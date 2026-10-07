//! Точка входа агента для сценариев: одна на команду, общая для `dump` и `build`.
//!
//! Сценарий открывает точку входа по конфигу — управляемую поднимает, к чужой
//! подключается, — получает каталог пользователя агента и работает с файлами через
//! него: агент читает и пишет только внутри своего `AgentBaseDir`, поэтому чужие
//! каталоги выставляются туда символической ссылкой. Здесь же живёт учёт
//! `generation-id`: токен поколения конфигурации, записанный после успешной загрузки
//! или выгрузки и сравниваемый перед следующей выгрузкой.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::status::GenerationAfter;
use crate::platform::process::{ProcessInterruption, ProcessInterruptionReason};
use crate::platform::result::PlatformCommandResult;

use serde::{Deserialize, Serialize};

use crate::config::model::{AppConfig, DesignerAgentMode};
use crate::domain::capability::{Provider, SessionEndpoint, SessionMode};
use crate::domain::source_set::SourceSetContext;
use crate::platform::agent::{
    self, AgentEndpoint, AgentError, AgentLaunch, AgentSession, AgentSessionRequest,
    HostKeyExpectation, ManagedAgent, WaitPolicy,
};
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::copy_dir_recursively;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::interruption::{CommandFailure, Deferrals};

/// Открытая точка входа: свой процесс с сессией или только сессия к чужому.
pub(crate) enum AgentHandle {
    Managed(ManagedAgent),
    Attached {
        session: AgentSession,
        base_dir: PathBuf,
    },
    /// SSH-шлюз автономного сервера: тот же shell, файлы — объявленным каналом
    /// обмена (каталог пользователя шлюза или SFTP того же соединения).
    Gate {
        session: AgentSession,
        exchange: Exchange,
    },
}

/// Канал обмена файлами с точкой входа. Пути команд всегда относительны каталога
/// пользователя точки входа; канал решает, как файлы туда попадают и как
/// возвращаются: `Dir` — тот каталог, видимый раннеру (ссылки, копии, переносы),
/// `Sftp` — подсистема SFTP того же SSH-соединения (передача по сети).
#[derive(Debug, Clone)]
pub(crate) enum Exchange {
    Dir(PathBuf),
    Sftp,
}

impl AgentHandle {
    pub(crate) fn session(&mut self) -> &mut AgentSession {
        match self {
            Self::Managed(agent) => agent.session(),
            Self::Attached { session, .. } | Self::Gate { session, .. } => session,
        }
    }

    /// Режим и адрес, к которому открыта сессия: адрес — тот, к которому подключился
    /// SSH-клиент, напечатанный из хоста и порта; учётных данных в нём нет.
    pub(crate) fn endpoint(&self) -> SessionEndpoint {
        match self {
            Self::Managed(agent) => session_endpoint(SessionMode::Managed, agent.endpoint()),
            Self::Attached { session, .. } => {
                session_endpoint(SessionMode::Attached, session.endpoint())
            }
            Self::Gate { session, .. } => session_endpoint(SessionMode::Gate, session.endpoint()),
        }
    }

    /// Канал обмена с точкой входа. У агента Конфигуратора это его каталог
    /// пользователя из карты `AgentBaseDir`, у шлюза автономного сервера — то, что
    /// объявил конфиг.
    pub(crate) fn exchange(&self, config: &AppConfig) -> Result<Exchange, AppError> {
        let base_dir = match self {
            Self::Managed(agent) => agent.base_dir(),
            Self::Attached { base_dir, .. } => base_dir,
            Self::Gate { exchange, .. } => return Ok(exchange.clone()),
        };
        agent::user_dir(base_dir, &agent_user(config))
            .map(Exchange::Dir)
            .map_err(AppError::from)
    }

    /// Управляемый агент гасится, чужой — только отпускается: соединение с базой
    /// закрывается явно, иначе точка входа держит блокировку Конфигуратора и после
    /// разрыва SSH (шлюз `ibsrv` 8.3.27 держал её до перезапуска сервера — замер
    /// 15.09.2026). Ответ не важен: после `restore-ib` сессии уже нет.
    ///
    /// Срок очистки урезан: прежде `disconnect` наследовал весь остаток бюджета
    /// команды и мог держать её столько же ещё раз.
    pub(crate) fn finish(self, wait: &WaitPolicy) {
        match self {
            Self::Managed(agent) => agent.shutdown(wait),
            Self::Attached { session, .. } | Self::Gate { session, .. } => session.release(wait),
        }
    }
}

/// Точка входа для квитанции: адрес печатается из разобранных хоста и порта. Запись с
/// учётными данными до этого места не доходит — разбор адреса отказывает ей раньше.
fn session_endpoint(mode: SessionMode, endpoint: &AgentEndpoint) -> SessionEndpoint {
    SessionEndpoint {
        mode,
        address: endpoint.to_string(),
    }
}

/// После `update-db-cfg` автономный сервер 8.3.27 уходит в «refreshing» на 15–20 с и
/// всё это время отвергает SSH-логин (живой прогон 15.09.2026); в этом окне отказ
/// аутентификации — не неверный пароль, а занятой сервер. Поэтому к шлюзу стучатся
/// повторно в ограниченном окне; неверный пароль виден по тому, что окно истекло.
const GATE_REFRESH_WINDOW: Duration = Duration::from_secs(30);

fn open_gate_session(
    request: &AgentSessionRequest,
    wait: &WaitPolicy,
) -> Result<AgentSession, AppError> {
    let started = std::time::Instant::now();
    loop {
        match AgentSession::open(request, wait) {
            Ok(session) => return Ok(session),
            Err(
                error
                @ (AgentError::AuthenticationRejected { .. } | AgentError::Unreachable { .. }),
            ) if started.elapsed() < GATE_REFRESH_WINDOW => {
                // Отмена прекращает ожидание шлюза: сессии ещё нет, и ответ называет отмену,
                // а не последний отказ шлюза.
                wait.refuse_if_cancelled("open")?;
                tracing::debug!(%error, "gate is not accepting sessions yet; waiting");
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(error) => return Err(AppError::from(error)),
        }
    }
}

/// Отмена и класс безопасности сессии — с границы команды; срока там больше нет.
///
/// Класс здесь не теряется: агентский транспорт обязан различать команду, которую можно
/// бросить на полпути, и ту, что меняет базу. Класс по умолчанию — для команд без
/// побочного эффекта; меняющие команды называют свой класс сами, через `run_critical`.
/// Срока у агентской команды нет, и это выбор, а не упущение: команда его не имеет
/// (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE), а транспорту брать его неоткуда — замер
/// 15.09.2026 дал тишине занятого агента нижнюю границу и не дал верхней. Ожидание
/// заканчивает оператор, а мёртвый шлюз — таймаут записи TCP на keepalive-пакетах.
pub(crate) fn wait_policy(context: &ExecutionContext) -> WaitPolicy {
    WaitPolicy::from_step(context.process_policy(InterruptionSafetyClass::GracefulThenKill, None))
}

/// Журнал сессии рядом с журналами платформы, свежий на каждую команду.
pub(crate) fn transcript_log(config: &AppConfig, name: &str) -> Result<PathBuf, AppError> {
    let log_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        AppError::Runtime(format!("failed to create platform logs dir: {error}"))
    })?;
    let path = log_dir.join(format!("{name}-agent.log"));
    crate::support::fs::remove_path_if_exists(&path)
        .map_err(|error| AppError::Runtime(format!("failed to reset agent log: {error}")))?;
    Ok(path)
}

/// Открывает точку входа по конфигу: управляемую поднимает, к объявленной подключается.
/// `v8` — путь к `1cv8` из выбора исполнителя у управляемого агента; у чужого его нет.
///
/// Открытая точка входа отмечается в контексте команды: квитанция исполнителя называет
/// её оттуда, и другого источника адреса у квитанции нет.
pub(crate) fn connect(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    v8: Option<&Path>,
    transcript_log: PathBuf,
    wait: &WaitPolicy,
) -> Result<AgentHandle, AppError> {
    let handle = open_handle(config, utilities, v8, transcript_log, wait)?;
    context.note_session(handle.endpoint());
    Ok(handle)
}

/// Отказ агенту, выбранному рядом с объявленной строкой прямого шлюза, когда канала нет:
/// строка уже объявлена, и не хватает только канала.
const GATE_WITHOUT_A_CHANNEL: &str = "the agent works with a standalone server through its SSH gate, and files travel there only by a declared channel: set infobase.standalone.exchange to `sftp` (through the gate) or to `{ dir: … }` — the gate user's directory as the runner sees it";

fn open_handle(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    v8: Option<&Path>,
    transcript_log: PathBuf,
    wait: &WaitPolicy,
) -> Result<AgentHandle, AppError> {
    let agent = &config.tools.designer_agent;
    let connection = config.v8_connection();
    let user = connection.user.clone().unwrap_or_default();
    let password = connection.password.clone().unwrap_or_default();

    // Автономный сервер держит свой шлюз сам: раннер только подключается, и файлы
    // идут объявленным каналом — каталогом пользователя шлюза. Без канала сессия не
    // открывается: проверка конфигурации требует его, только когда строки прямого шлюза
    // нет, а агент мог достаться и при строке — ключом или не найдя Конфигуратора.
    if let Some(standalone) = config.infobase.standalone.as_ref() {
        let (host, port) = standalone.gate_endpoint().map_err(AppError::Validation)?;
        let exchange = if standalone.exchange_is_sftp() {
            Exchange::Sftp
        } else {
            Exchange::Dir(
                standalone
                    .exchange_dir()
                    .ok_or_else(|| AppError::Validation(GATE_WITHOUT_A_CHANNEL.to_owned()))?
                    .to_path_buf(),
            )
        };
        let request = AgentSessionRequest {
            endpoint: AgentEndpoint { host, port },
            user,
            password,
            transcript_log: Some(transcript_log),
            host_key: declared_expectation(standalone.host_fingerprint.as_deref())?,
        };
        let session = open_gate_session(&request, wait)?;
        return Ok(AgentHandle::Gate { session, exchange });
    }

    let mode = agent.mode().map_err(AppError::Validation)?;
    match mode {
        DesignerAgentMode::Attached { host, port } => {
            let base_dir = agent.base_dir.clone().ok_or_else(|| {
                AppError::capability(
                    "working through an attached agent exchanges files through its base dir and needs tools.designer_agent.base-dir".to_owned(),
                )
            })?;
            let request = AgentSessionRequest {
                endpoint: AgentEndpoint { host, port },
                user,
                password,
                transcript_log: Some(transcript_log),
                host_key: declared_expectation(agent.host_fingerprint.as_deref())?,
            };
            // Чужая точка входа не поднимается заново: недоступная — типизированный отказ.
            let session = AgentSession::open(&request, wait).map_err(AppError::from)?;
            Ok(AgentHandle::Attached { session, base_dir })
        }
        DesignerAgentMode::Managed { port } => {
            let v8 = v8.ok_or_else(|| {
                AppError::EnvironmentUnavailable(
                    "the managed Designer agent needs the local platform; no 1cv8 was selected"
                        .to_owned(),
                )
            })?;
            let launch = AgentLaunch {
                v8: v8.to_path_buf(),
                infobase_args: connection.infobase_args(),
                port,
                host_key: agent.host_key.clone(),
                base_dir: config.work_path.join("agent").join("base"),
                process_log: transcript_log.with_extension("process"),
            };
            let request = AgentSessionRequest {
                endpoint: launch.endpoint(),
                user,
                password,
                transcript_log: Some(transcript_log),
                // Тот же файл, что уезжает агенту в `/AgentSSHHostKey`: он публикует
                // ключ оттуда как есть, поэтому открытая часть файла и есть ожидание.
                host_key: launch
                    .host_key
                    .as_deref()
                    .map(HostKeyExpectation::of_host_key_file)
                    .unwrap_or_default(),
            };
            let managed = ManagedAgent::launch(
                utilities.runner_for(UtilityType::V8),
                &launch,
                request,
                Duration::from_millis(agent.startup_timeout_ms.max(1)),
                wait,
            )
            .map_err(AppError::from)?;
            Ok(AgentHandle::Managed(managed))
        }
    }
}

/// Ожидание из объявленного отпечатка. Не объявлен — ожидания нет.
fn declared_expectation(fingerprint: Option<&str>) -> Result<HostKeyExpectation, AppError> {
    match fingerprint {
        Some(declared) => HostKeyExpectation::declared(declared).map_err(AppError::Validation),
        None => Ok(HostKeyExpectation::Unpinned),
    }
}

/// Значение параметра агентской команды. Пробелы в значении экранируются кавычками —
/// грамматика shell документацией не описана, у путей раннера пробелов нет.
pub(crate) fn argument(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

/// Имя пользователя, под которым открыта сессия: пустое у базы без пользователей.
pub(crate) fn agent_user(config: &AppConfig) -> String {
    config.infobase.user.clone().unwrap_or_default()
}

/// Уникальный ключ одного прогона для имён внутри каталога пользователя агента.
pub(crate) fn run_id() -> String {
    format!(
        "{}-{:x}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

/// Выставляет чужой каталог внутрь каталога пользователя агента под относительным
/// именем и возвращает это имя для команды. Агент ходит по символическим ссылкам
/// (замер 15.09.2026: выгрузка и загрузка через ссылку); там, где ссылку создать
/// нельзя, каталог копируется — медленно, но той же формы.
pub(crate) fn expose_dir(
    user_dir: &Path,
    relative: &str,
    target: &Path,
) -> Result<String, AppError> {
    let link = user_dir.join(relative);
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Runtime(format!(
                "failed to prepare the agent exchange dir '{}': {error}",
                parent.display()
            ))
        })?;
    }
    let target = target.canonicalize().map_err(|error| {
        AppError::Runtime(format!(
            "source dir '{}' is not reachable: {error}",
            target.display()
        ))
    })?;
    let _ = crate::support::fs::remove_path_if_exists(&link);
    if symlink_dir(&target, &link).is_err() {
        copy_dir_recursively(&target, &link).map_err(|error| {
            AppError::Runtime(format!(
                "failed to expose '{}' to the agent at '{}': {error}",
                target.display(),
                link.display()
            ))
        })?;
    }
    Ok(relative.to_owned())
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// Кладёт чужой файл внутрь каталога пользователя агента под относительным именем:
/// жёсткой ссылкой, если файловая система одна, иначе копией. Через символическую
/// ссылку агент файлы не видит («Файл не обнаружен», замер 15.09.2026) — в отличие от
/// каталогов.
pub(crate) fn expose_file(
    user_dir: &Path,
    relative: &str,
    target: &Path,
) -> Result<String, AppError> {
    let link = user_dir.join(relative);
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Runtime(format!(
                "failed to prepare the agent exchange dir '{}': {error}",
                parent.display()
            ))
        })?;
    }
    let _ = crate::support::fs::remove_path_if_exists(&link);
    if std::fs::hard_link(target, &link).is_err() {
        std::fs::copy(target, &link).map_err(|error| {
            AppError::Runtime(format!(
                "failed to expose '{}' to the agent at '{}': {error}",
                target.display(),
                link.display()
            ))
        })?;
    }
    Ok(relative.to_owned())
}

/// Каталог для файлов, которые агент должен *написать*: настоящий подкаталог
/// каталога пользователя — через символическую ссылку агент файлы не пишет.
pub(crate) fn output_dir(user_dir: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let dir = user_dir.join(relative);
    std::fs::create_dir_all(&dir).map_err(|error| {
        AppError::Runtime(format!(
            "failed to prepare the agent output dir '{}': {error}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

/// Одна сессия на операцию: открыть, выполнить, закрыть — и при отказе тоже. Прерывание,
/// которое сессия отложила в критической фазе, исход несёт и у удачи, и у отказа.
pub(crate) fn with_session(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: Option<&Path>,
    log: PathBuf,
    work: impl FnOnce(&mut AgentHandle, &WaitPolicy, &Exchange) -> Result<String, AppError>,
) -> Result<PlatformCommandResult, CommandFailure> {
    let wait = wait_policy(context);
    let mut utilities = PlatformUtilities::from_config(config);
    let mut handle = connect(context, config, &mut utilities, v8, log.clone(), &wait)
        .map_err(CommandFailure::without_deferral)?;
    let outcome = handle
        .exchange(config)
        .and_then(|exchange| work(&mut handle, &wait, &exchange));
    let deferred = handle.session().deferred_interruption();
    handle.finish(&wait);
    match outcome {
        Ok(transcript) => Ok(platform_result(transcript, log, deferred)),
        Err(error) => Err(CommandFailure::after(
            error,
            deferred.map(ProcessInterruption::deferred),
        )),
    }
}

/// Команда, которая меняет базу: фаза критическая, отмену и истёкший срок команда
/// откладывает до своего исхода. Отложенное попадает в `deferrals` и у удачи, и у отказа:
/// сессия помнит его и тогда, когда ответ не дочитан. Сессия отдаёт этот факт только до
/// следующей команды, и читает его только этот помощник.
pub(crate) fn run_critical(
    handle: &mut AgentHandle,
    action: &str,
    command: &str,
    wait: &WaitPolicy,
    deferrals: &mut Deferrals,
) -> Result<agent::AgentReply, AppError> {
    let outcome = run_command(handle, command, &wait.critical());
    let deferral = handle
        .session()
        .last_command_deferral()
        .map(ProcessInterruption::deferred);
    deferrals.note_outcome(action, &outcome, deferral);
    outcome
}

/// Одна команда с проверкой итога; журнал ответа возвращается как улика.
pub(crate) fn run_command(
    handle: &mut AgentHandle,
    command: &str,
    wait: &WaitPolicy,
) -> Result<agent::AgentReply, AppError> {
    let reply = handle
        .session()
        .run(command, wait)
        .map_err(AppError::from)?;
    reply.outcome().map_err(AppError::from)?;
    Ok(reply)
}

/// Ответ агента в форме результата платформы: код 0, журнал сообщений — как stdout,
/// путь журнала сессии — как платформенный журнал.
pub(crate) fn platform_result(
    transcript: String,
    log: PathBuf,
    interruption: Option<ProcessInterruptionReason>,
) -> PlatformCommandResult {
    PlatformCommandResult {
        process: crate::platform::process::ProcessResult {
            exit_code: 0,
            stdout: transcript,
            stderr: String::new(),
            // Отложенное прерывание едет тем же полем, что и у процессов платформы,
            // поэтому о нём рассказывают уже существующие помощники, а не второй путь.
            interruption: interruption.map(ProcessInterruption::deferred),
        },
        platform_log_path: Some(log),
        platform_log: None,
        platform_log_read_error: None,
    }
}

/// Убирает след прогона из каталога пользователя: сам путь и опустевшие родители до
/// каталога пользователя. В каталоге чужой точки входа раннер следов не оставляет.
pub(crate) fn tidy_run_path(user_dir: &Path, relative: &str) {
    let path = user_dir.join(relative);
    let _ = crate::support::fs::remove_path_if_exists(&path);
    let mut parent = path.parent();
    while let Some(dir) = parent.filter(|dir| *dir != user_dir && dir.starts_with(user_dir)) {
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        parent = dir.parent();
    }
}

/// Каталог раннера — на сторону точки входа под относительным именем: в её каталог
/// ссылкой (копией, где ссылки нет), по SFTP — передачей.
pub(crate) fn stage_dir(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    local: &Path,
) -> Result<String, AppError> {
    match exchange {
        Exchange::Dir(user_dir) => expose_dir(user_dir, relative, local),
        Exchange::Sftp => handle
            .session()
            .sftp_put_dir(local, relative)
            .map(|()| relative.to_owned())
            .map_err(AppError::from),
    }
}

/// Часть каталога раннера — на сторону точки входа: корневые описатели и названные
/// файлы, каждый под своим относительным путём. По сети частичной загрузке хватает
/// `Configuration.xml`, `ConfigDumpInfo.xml` и самих изменённых файлов (замер
/// 16.09.2026 на агенте Конфигуратора 8.3.27); в каталог точки входа, видимый
/// раннеру, каталог по-прежнему выставляется целиком ссылкой.
pub(crate) fn stage_dir_partially(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    local: &Path,
    files: &[PathBuf],
) -> Result<String, AppError> {
    let Exchange::Sftp = exchange else {
        return stage_dir(handle, exchange, relative, local);
    };
    let session = handle.session();
    session.sftp_mkdir_all(relative).map_err(AppError::from)?;
    let mut selected: Vec<PathBuf> = ["Configuration.xml", "ConfigDumpInfo.xml"]
        .iter()
        .map(|name| local.join(name))
        .filter(|path| path.is_file())
        .collect();
    selected.extend(files.iter().cloned());
    for file in selected {
        let inside = file.strip_prefix(local).map_err(|_| {
            AppError::Runtime(format!(
                "partial load file '{}' lies outside its source root '{}'",
                file.display(),
                local.display()
            ))
        })?;
        let remote = format!(
            "{relative}/{}",
            inside.display().to_string().replace('\\', "/")
        );
        if let Some((parent, _)) = remote.rsplit_once('/') {
            session.sftp_mkdir_all(parent).map_err(AppError::from)?;
        }
        session
            .sftp_put_file(&file, &remote)
            .map_err(AppError::from)?;
    }
    Ok(relative.to_owned())
}

/// Файл раннера — на сторону точки входа под относительным именем.
pub(crate) fn stage_file(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    local: &Path,
) -> Result<String, AppError> {
    match exchange {
        Exchange::Dir(user_dir) => expose_file(user_dir, relative, local),
        Exchange::Sftp => {
            let session = handle.session();
            if let Some((parent, _)) = relative.rsplit_once('/') {
                session.sftp_mkdir_all(parent).map_err(AppError::from)?;
            }
            session
                .sftp_put_file(local, relative)
                .map(|()| relative.to_owned())
                .map_err(AppError::from)
        }
    }
}

/// Текст — на сторону точки входа под относительным именем (списки объектов и путей).
pub(crate) fn write_text(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    text: &str,
) -> Result<(), AppError> {
    write_bytes(handle, exchange, relative, text.as_bytes())
}

/// Байты — на сторону точки входа под относительным именем.
pub(crate) fn write_bytes(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    bytes: &[u8],
) -> Result<(), AppError> {
    match exchange {
        Exchange::Dir(user_dir) => {
            let path = user_dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    AppError::Runtime(format!(
                        "failed to prepare the agent exchange dir '{}': {error}",
                        parent.display()
                    ))
                })?;
            }
            std::fs::write(&path, bytes).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to write '{}' for the agent: {error}",
                    path.display()
                ))
            })
        }
        Exchange::Sftp => {
            let session = handle.session();
            if let Some((parent, _)) = relative.rsplit_once('/') {
                session.sftp_mkdir_all(parent).map_err(AppError::from)?;
            }
            session.sftp_write(relative, bytes).map_err(AppError::from)
        }
    }
}

/// Каталог на стороне точки входа, в который она будет писать.
pub(crate) fn make_output_dir(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
) -> Result<(), AppError> {
    match exchange {
        Exchange::Dir(user_dir) => output_dir(user_dir, relative).map(|_| ()),
        Exchange::Sftp => handle
            .session()
            .sftp_mkdir_all(relative)
            .map_err(AppError::from),
    }
}

/// Каталог, который точка входа написала, — в локальный путь (родители создаются);
/// на стороне точки входа его больше нет.
pub(crate) fn collect_dir(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    destination: &Path,
) -> Result<(), AppError> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Runtime(format!("failed to prepare '{}': {error}", parent.display()))
        })?;
    }
    match exchange {
        Exchange::Dir(user_dir) => {
            let produced = user_dir.join(relative);
            if !produced.is_dir() {
                return Err(AppError::Platform(format!(
                    "agent reported success but wrote nothing into '{}'",
                    produced.display()
                )));
            }
            let _ = crate::support::fs::remove_path_if_exists(destination);
            crate::support::fs::move_dir(&produced, destination).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to collect the agent's dir '{}': {error}",
                    produced.display()
                ))
            })
        }
        Exchange::Sftp => {
            let session = handle.session();
            let _ = crate::support::fs::remove_path_if_exists(destination);
            session
                .sftp_get_dir(relative, destination)
                .map_err(|error| {
                    AppError::Platform(format!(
                        "agent reported success but its dir '{relative}' could not be collected: {error}"
                    ))
                })?;
            session.sftp_remove_all(relative).map_err(AppError::from)
        }
    }
}

/// Каталог, который точка входа написала, — поверх существующего локального
/// (файлы перезаписываются, лишние остаются); на стороне точки входа его больше нет.
pub(crate) fn collect_into_dir(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    existing: &Path,
) -> Result<(), AppError> {
    match exchange {
        Exchange::Dir(user_dir) => {
            let produced = user_dir.join(relative);
            copy_dir_recursively(&produced, existing).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to collect the agent's dir '{}': {error}",
                    produced.display()
                ))
            })?;
            tidy_run_path(user_dir, relative);
            Ok(())
        }
        Exchange::Sftp => {
            let session = handle.session();
            session
                .sftp_get_dir(relative, existing)
                .map_err(AppError::from)?;
            session.sftp_remove_all(relative).map_err(AppError::from)
        }
    }
}

/// Файл, который точка входа написала, — в локальный путь; на её стороне его больше нет.
pub(crate) fn collect_file(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    destination: &Path,
) -> Result<(), AppError> {
    match exchange {
        Exchange::Dir(user_dir) => {
            let produced = user_dir.join(relative);
            if !produced.is_file() {
                return Err(AppError::Platform(format!(
                    "agent reported success but wrote no file at '{}'",
                    produced.display()
                )));
            }
            crate::support::fs::move_file(&produced, destination).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to collect the agent's file '{}': {error}",
                    produced.display()
                ))
            })
        }
        Exchange::Sftp => {
            let session = handle.session();
            session
                .sftp_get_file(relative, destination)
                .map_err(|error| {
                    AppError::Platform(format!(
                        "agent reported success but its file '{relative}' could not be collected: {error}"
                    ))
                })?;
            session.sftp_remove_all(relative).map_err(AppError::from)
        }
    }
}

/// Убирает выставленный каталог со стороны точки входа (ссылку — как ссылку).
pub(crate) fn unstage(handle: &mut AgentHandle, exchange: &Exchange, relative: &str) {
    match exchange {
        Exchange::Dir(user_dir) => withdraw_dir(user_dir, relative),
        Exchange::Sftp => tidy(handle, exchange, relative),
    }
}

/// Убирает след прогона со стороны точки входа: путь и опустевшие родители.
pub(crate) fn tidy(handle: &mut AgentHandle, exchange: &Exchange, relative: &str) {
    match exchange {
        Exchange::Dir(user_dir) => tidy_run_path(user_dir, relative),
        Exchange::Sftp => {
            let session = handle.session();
            let _ = session.sftp_remove_all(relative);
            session.sftp_remove_empty_parents(relative);
        }
    }
}

/// Снимает выставленный каталог: ссылку — как ссылку, копию — целиком.
pub(crate) fn withdraw_dir(user_dir: &Path, relative: &str) {
    let link = user_dir.join(relative);
    match std::fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let _ = std::fs::remove_file(&link);
        }
        Ok(metadata) if metadata.is_dir() => {
            let _ = std::fs::remove_dir_all(&link);
        }
        _ => {}
    }
    tidy_run_path(user_dir, relative);
}

/// Токен поколения конфигурации у агента: `config generation-id [--extension=<имя>]`.
///
/// Тело ответа — строка из 40 hex; сравнивать её можно только на равенство
/// (Приложение 7, `/GetConfigGenerationID`).
pub(crate) fn generation_id(
    session: &mut AgentSession,
    extension: Option<&str>,
    wait: &WaitPolicy,
) -> Result<String, AppError> {
    let mut command = String::from("config generation-id");
    if let Some(extension) = extension {
        command.push_str(&format!(" --extension={}", argument(extension)));
    }
    let reply = session.run(&command, wait).map_err(AppError::from)?;
    let body = reply.outcome().map_err(AppError::from)?;
    body.and_then(|value| value.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::InvalidOutput(
                "agent answered generation-id without a token in the body".to_owned(),
            )
        })
}

/// Запись о поколении конфигурации, увиденном после последней удачной операции.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GenerationRecord {
    pub token: String,
    /// Инструмент, которым получен токен: токены разных инструментов несравнимы
    /// (`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`).
    pub tool: Provider,
    /// Что было сделано, когда токен записан.
    pub after: GenerationAfter,
    pub recorded_at: String,
    /// Привязка памяти набора (база, каталог, назначение, имя): запись другой пары чужая.
    pub identity: String,
}

/// Ответ записи на вопрос «менялась ли база с прошлого чтения».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenerationComparison {
    /// Тот же инструмент отдал тот же токен: основную конфигурацию не трогали.
    Unchanged,
    /// Тот же инструмент отдал другой токен: была запись, возможно того же самого.
    Changed,
    /// Токен получен другим инструментом: ответа нет — ни совпадения, ни расхождения.
    NoAnswer,
}

impl GenerationRecord {
    /// Сравнивает записанный токен с прочитанным сейчас, только внутри одного
    /// инструмента. Токен пустой базы сравнивается как любой другой.
    pub(crate) fn compare(&self, tool: Provider, token: &str) -> GenerationComparison {
        if self.tool != tool {
            GenerationComparison::NoAnswer
        } else if self.token == token {
            GenerationComparison::Unchanged
        } else {
            GenerationComparison::Changed
        }
    }
}

/// Что журнал помнит о поколении набора.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Recorded {
    Nothing,
    Ours(GenerationRecord),
    /// Запись сделана для другой пары «база ↔ каталог»: её привязка без учётных данных.
    Foreign {
        identity: String,
    },
}

/// Журнал поколений базы: `workPath/infobases/<база>/generation.json`, запись на набор.
///
/// Запись годится только для той пары «база ↔ каталог», для которой сделана: запись с
/// другой привязкой чужая и основанием не выгружать не служит. У набора без памяти о
/// базе журнала нет, и агент выгружает всегда.
pub(crate) struct GenerationLedger {
    file: PathBuf,
    source_set: String,
    identity: String,
}

impl GenerationLedger {
    pub(crate) fn of(context: &SourceSetContext, work_path: &Path) -> Option<Self> {
        Some(Self {
            file: context.generation_file(work_path)?,
            source_set: context.name().to_owned(),
            identity: context.storage_identity()?.to_owned(),
        })
    }

    /// Записи журнала как есть: запись набора разбирается при чтении, чтобы запись,
    /// которую разобрать нельзя, не лишала памяти остальные наборы.
    fn records(&self) -> serde_json::Map<String, serde_json::Value> {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Запись набора. Привязка читается до полного разбора: запись другой пары называется
    /// чужой, даже если разобрать её целиком нельзя. Запись своей пары без имени инструмента
    /// или неразборчивая — отсутствие ответа: её как будто нет.
    pub(crate) fn read(&self) -> Recorded {
        let Some(value) = self.records().remove(&self.source_set) else {
            return Recorded::Nothing;
        };
        if let Some(identity) = value.get("identity").and_then(serde_json::Value::as_str) {
            if identity != self.identity {
                return Recorded::Foreign {
                    identity: identity.to_owned(),
                };
            }
        }
        match serde_json::from_value::<GenerationRecord>(value) {
            Ok(record) if record.identity == self.identity => Recorded::Ours(record),
            Ok(_) => Recorded::Nothing,
            Err(error) => {
                tracing::debug!(
                    ledger = %self.file.display(),
                    source_set = %self.source_set,
                    %error,
                    "generation record is not readable; treated as no answer"
                );
                Recorded::Nothing
            }
        }
    }

    /// Записывает поколение набора, сохраняя записи остальных наборов базы.
    ///
    /// Чтение, правка и запись журнала идут без своего замка: журнал лежит под `workPath`,
    /// а всякая команда держит замок `workPath` до конца
    /// (`INV.WIRE.A-BUSY-WORKSPACE-ANSWERS-WORKSPACE-BUSY`), и две команды в одном журнале
    /// не пишут. Если бы запись другого набора всё же потерялась, следующая выгрузка этого
    /// набора не пропустилась бы по поколению — лишняя выгрузка, а не потеря правок.
    pub(crate) fn record(
        &self,
        tool: Provider,
        token: &str,
        after: GenerationAfter,
    ) -> Result<(), AppError> {
        let dir = self.file.parent().ok_or_else(|| {
            AppError::Runtime(format!(
                "the generation ledger '{}' has no parent directory",
                self.file.display()
            ))
        })?;
        std::fs::create_dir_all(dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the generation ledger directory '{}': {error}",
                dir.display()
            ))
        })?;
        let record = serde_json::to_value(GenerationRecord {
            token: token.to_owned(),
            tool,
            after,
            recorded_at: chrono::Utc::now().to_rfc3339(),
            identity: self.identity.clone(),
        })
        .map_err(|error| AppError::Runtime(format!("failed to encode generation: {error}")))?;
        let mut records = self.records();
        records.insert(self.source_set.clone(), record);
        self.write(&records)
    }

    /// Стирает запись набора, сохраняя остальные: после загрузки, о которой инструмент не
    /// ответил поколением, прежний токен описывает уже не ту базу. `true` — запись была.
    pub(crate) fn forget(&self) -> Result<bool, AppError> {
        let mut records = self.records();
        if records.remove(&self.source_set).is_none() {
            return Ok(false);
        }
        self.write(&records).map(|()| true)
    }

    fn write(&self, records: &serde_json::Map<String, serde_json::Value>) -> Result<(), AppError> {
        let text = serde_json::to_vec_pretty(records)
            .map_err(|error| AppError::Runtime(format!("failed to encode generation: {error}")))?;
        crate::support::fs::write_file_atomically(&self.file, |file| {
            std::io::Write::write_all(file, &text)
        })
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to write the generation ledger '{}': {error}",
                self.file.display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Адрес квитанции — `host:port` без учётных данных. Запись с `user:pass@` не
    /// разбирается ни у чужого агента, ни у шлюза, поэтому до адреса не доходит; разобранная
    /// печатается хостом и портом, IPv6 — в скобках.
    #[test]
    fn a_receipt_address_never_carries_credentials() {
        for record in ["user:secret@127.0.0.1:1543", "agent@gate.example:1543"] {
            let attached = crate::config::model::DesignerAgentConfig {
                attach: Some(record.to_owned()),
                ..Default::default()
            };
            assert!(attached.mode().is_err(), "{record} was accepted as attach");
            let standalone: crate::config::model::StandaloneConfig =
                serde_yaml::from_str(&format!("gate: '{record}'")).expect("standalone yaml");
            assert!(
                standalone.gate_endpoint().is_err(),
                "{record} was accepted as gate"
            );
        }

        for (record, address) in [
            ("127.0.0.1:1543", "127.0.0.1:1543"),
            ("Gate.Example:22", "gate.example:22"),
            ("[::1]:1543", "[::1]:1543"),
        ] {
            let (host, port) = crate::config::model::StandaloneConfig {
                gate: Some(record.to_owned()),
                host_fingerprint: None,
                exchange: None,
            }
            .gate_endpoint()
            .expect("gate record");
            let endpoint = session_endpoint(SessionMode::Gate, &AgentEndpoint { host, port });
            assert_eq!(endpoint.address, address);
        }
    }

    #[test]
    fn a_symlinked_dir_is_exposed_under_the_user_dir_and_withdrawn_as_a_link() {
        let root = tempfile::tempdir().expect("tempdir");
        let user_dir = root.path().join("0");
        let sources = root.path().join("src");
        std::fs::create_dir_all(&sources).expect("sources");
        std::fs::write(sources.join("Configuration.xml"), "<x/>").expect("marker");

        let relative = expose_dir(&user_dir, "build/run/main", &sources).expect("expose");
        assert_eq!(relative, "build/run/main");
        assert!(user_dir.join("build/run/main/Configuration.xml").is_file());

        withdraw_dir(&user_dir, "build/run/main");
        assert!(!user_dir.join("build/run/main").exists());
        assert!(
            sources.join("Configuration.xml").is_file(),
            "withdrawing the link must not touch the sources"
        );
    }

    fn ledger(root: &Path, set: &str, identity: &str) -> GenerationLedger {
        let context = SourceSetContext::new(set, root.join(set), "designer-x")
            .with_infobase_memory("origin", identity.to_owned());
        GenerationLedger::of(&context, &root.join("work")).expect("a remembered base")
    }

    #[test]
    fn the_ledger_keeps_one_record_per_source_set_under_the_base() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a main");
        assert_eq!(main.read(), Recorded::Nothing);
        main.record(Provider::Agent, "abc", GenerationAfter::Build)
            .expect("record");
        let ext = ledger(root.path(), "ext", "base-a ext");
        ext.record(Provider::Agent, "def", GenerationAfter::Dump)
            .expect("record");
        let Recorded::Ours(record) = main.read() else {
            panic!("the record of the same pair");
        };
        assert_eq!(record.token, "abc");
        assert_eq!(record.after, GenerationAfter::Build);
        assert!(matches!(ext.read(), Recorded::Ours(record) if record.token == "def"));
        assert!(root
            .path()
            .join("work/infobases/origin/generation.json")
            .is_file());
    }

    fn recorded(ledger: &GenerationLedger) -> GenerationRecord {
        match ledger.read() {
            Recorded::Ours(record) => record,
            other => panic!("the record of the same pair: {other:?}"),
        }
    }

    #[test]
    fn the_same_tool_with_the_same_token_is_unchanged() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a");
        main.record(Provider::Agent, "abc", GenerationAfter::Build)
            .expect("record");
        let record = recorded(&main);
        assert_eq!(record.tool, Provider::Agent);
        assert_eq!(
            record.compare(Provider::Agent, "abc"),
            GenerationComparison::Unchanged
        );
    }

    #[test]
    fn the_same_tool_with_another_token_is_changed() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a");
        main.record(Provider::Ibcmd, "abc", GenerationAfter::Dump)
            .expect("record");
        assert_eq!(
            recorded(&main).compare(Provider::Ibcmd, "def"),
            GenerationComparison::Changed
        );
    }

    /// Токен другого инструмента — отсутствие ответа: ни совпадения при равных значениях,
    /// ни расхождения при разных.
    #[test]
    fn a_token_of_another_tool_is_no_answer() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a");
        main.record(Provider::Designer, "abc", GenerationAfter::Build)
            .expect("record");
        let record = recorded(&main);
        for (tool, token) in [
            (Provider::Agent, "abc"),
            (Provider::Ibcmd, "abc"),
            (Provider::Agent, "def"),
        ] {
            assert_eq!(
                record.compare(tool, token),
                GenerationComparison::NoAnswer,
                "{tool} {token}"
            );
        }
    }

    /// Токен пустой базы сравнивается как любой другой: сорок нулей (8.3.27) и постоянное
    /// значение пустой базы на 8.5.4 у того же инструмента совпадают сами с собой.
    #[test]
    fn an_empty_base_token_is_compared_like_any_other() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a");
        for token in [
            "0000000000000000000000000000000000000000",
            "2af84151e959af78eab1cb38d137eedf33543af5",
        ] {
            main.record(Provider::Ibcmd, token, GenerationAfter::Build)
                .expect("record");
            assert_eq!(
                recorded(&main).compare(Provider::Ibcmd, token),
                GenerationComparison::Unchanged,
                "{token}"
            );
        }
    }

    /// Запись другой пары без имени инструмента всё равно называется чужой: привязка
    /// читается до полного разбора.
    #[test]
    fn a_foreign_record_without_a_tool_is_still_named_foreign() {
        let root = tempfile::tempdir().expect("tempdir");
        let file = root.path().join("work/infobases/origin/generation.json");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(
            &file,
            serde_json::json!({
                "main": {
                    "token": "abc",
                    "after": "dump",
                    "recorded_at": "2026-10-01T00:00:00+00:00",
                    "identity": "base-a",
                }
            })
            .to_string(),
        )
        .expect("record without a tool");
        assert_eq!(
            ledger(root.path(), "main", "base-b").read(),
            Recorded::Foreign {
                identity: "base-a".to_owned()
            }
        );
        assert_eq!(
            ledger(root.path(), "main", "base-a").read(),
            Recorded::Nothing
        );
    }

    /// Запись без имени инструмента читается как отсутствие ответа, а записи других
    /// наборов остаются годными.
    #[test]
    fn a_record_without_a_tool_is_no_answer_and_spares_the_other_sets() {
        let root = tempfile::tempdir().expect("tempdir");
        let ext = ledger(root.path(), "ext", "base-a ext");
        ext.record(Provider::Agent, "def", GenerationAfter::Dump)
            .expect("record");
        let file = root.path().join("work/infobases/origin/generation.json");
        let mut journal: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("ledger"))
                .expect("ledger json");
        journal["main"] = serde_json::json!({
            "token": "abc",
            "after": "build",
            "recorded_at": "2026-10-01T00:00:00+00:00",
            "identity": "base-a main",
        });
        std::fs::write(&file, journal.to_string()).expect("old record");

        let main = ledger(root.path(), "main", "base-a main");
        assert_eq!(main.read(), Recorded::Nothing);
        assert!(matches!(ext.read(), Recorded::Ours(record) if record.token == "def"));
        main.record(Provider::Agent, "abc", GenerationAfter::Dump)
            .expect("record");
        assert_eq!(recorded(&main).tool, Provider::Agent);
        assert!(matches!(ext.read(), Recorded::Ours(record) if record.token == "def"));
    }

    /// Запись о поколении, сделанная для другой пары «база ↔ каталог», чужая: агент не
    /// считает по ней, что выгружать нечего.
    #[test]
    fn a_generation_recorded_for_another_pair_is_not_used() {
        let root = tempfile::tempdir().expect("tempdir");
        ledger(root.path(), "main", "base-a")
            .record(Provider::Agent, "abc", GenerationAfter::Dump)
            .expect("record");
        let retargeted = ledger(root.path(), "main", "base-b");
        assert_eq!(
            retargeted.read(),
            Recorded::Foreign {
                identity: "base-a".to_owned()
            }
        );
        let unbound = SourceSetContext::new("main", root.path().join("main"), "designer-main")
            .without_memory();
        assert!(GenerationLedger::of(&unbound, &root.path().join("work")).is_none());
    }

    /// Отложенное прерывание едет тем же полем, что и у процессов платформы: иначе о нём
    /// рассказывал бы второй путь, а учёт отложенных отмен (`Deferrals`) для агентских
    /// результатов не срабатывал бы никогда.
    #[test]
    fn an_agent_result_reports_a_deferred_interruption_like_any_platform_result() {
        let noted = |action: &str, result: &crate::platform::result::PlatformCommandResult| {
            crate::use_cases::interruption::collecting_deferrals(|deferrals| {
                deferrals.note_result(action, result);
                Ok(())
            })
            .map(|((), warnings)| warnings)
            .expect("noting a finished command does not fail")
        };
        let quiet = platform_result("ok".to_owned(), PathBuf::from("/tmp/agent.log"), None);
        assert!(quiet.process.interruption.is_none());
        assert!(noted("build", &quiet).is_empty());

        let latched = platform_result(
            "ok".to_owned(),
            PathBuf::from("/tmp/agent.log"),
            Some(ProcessInterruptionReason::Cancelled),
        );

        let interruption = latched
            .process
            .interruption
            .expect("a latched interruption must reach the platform result");
        assert_eq!(interruption.reason, ProcessInterruptionReason::Cancelled);
        assert_eq!(
            interruption.action,
            crate::platform::process::ProcessInterruptionAction::Deferred
        );

        let warnings = noted("update_db_cfg", &latched);
        let [warning] = warnings.as_slice() else {
            panic!("the ledger must name an agent result's deferral: {warnings:?}");
        };
        assert!(warning.contains("update_db_cfg"), "{warning}");
        assert!(warning.contains("critical phase"), "{warning}");
    }
}
