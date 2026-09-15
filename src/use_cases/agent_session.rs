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

use serde::{Deserialize, Serialize};

use crate::config::model::{AppConfig, DesignerAgentMode};
use crate::platform::agent::{
    self, AgentEndpoint, AgentError, AgentLaunch, AgentSession, AgentSessionRequest, ManagedAgent,
    WaitPolicy,
};
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::copy_dir_recursively;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};

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
            .map_err(map_agent_error)
    }

    /// Управляемый агент гасится, чужой — только отпускается: соединение с базой
    /// закрывается явно, иначе точка входа держит блокировку Конфигуратора и после
    /// разрыва SSH (шлюз `ibsrv` 8.3.27 держал её до перезапуска сервера — замер
    /// 15.09.2026). Ответ не важен: после `restore-ib` сессии уже нет.
    pub(crate) fn finish(self, wait: &WaitPolicy) {
        match self {
            Self::Managed(agent) => agent.shutdown(wait),
            Self::Attached { mut session, .. } | Self::Gate { mut session, .. } => {
                let _ = session.run(agent::DISCONNECT_COMMAND, wait);
                session.close();
            }
        }
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
            ) if started.elapsed() < GATE_REFRESH_WINDOW && !wait.cancellation.is_cancelled() => {
                tracing::debug!(%error, "gate is not accepting sessions yet; waiting");
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(error) => return Err(map_agent_error(error)),
        }
    }
}

/// Срок и отмена сессии — с границы команды.
pub(crate) fn wait_policy(context: &ExecutionContext) -> WaitPolicy {
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
    WaitPolicy {
        timeout: policy.timeout,
        cancellation: policy.cancellation.clone(),
    }
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
pub(crate) fn connect(
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
    // идут объявленным каналом — каталогом пользователя шлюза.
    if let Some(standalone) = config.infobase.standalone.as_ref() {
        let (host, port) = standalone.gate_endpoint().map_err(AppError::Validation)?;
        let exchange = if standalone.exchange_is_sftp() {
            Exchange::Sftp
        } else {
            Exchange::Dir(
                standalone
                    .exchange_dir()
                    .ok_or_else(|| {
                        AppError::CapabilityUnavailable(
                            "files travel to a standalone server only through a declared channel; set infobase.standalone.exchange".to_owned(),
                        )
                    })?
                    .to_path_buf(),
            )
        };
        let request = AgentSessionRequest {
            endpoint: AgentEndpoint { host, port },
            user,
            password,
            transcript_log: Some(transcript_log),
        };
        let session = open_gate_session(&request, wait)?;
        return Ok(AgentHandle::Gate { session, exchange });
    }

    let mode = agent.mode().map_err(AppError::Validation)?;
    match mode {
        DesignerAgentMode::Attached { host, port } => {
            let base_dir = agent.base_dir.clone().ok_or_else(|| {
                AppError::CapabilityUnavailable(
                    "working through an attached agent exchanges files through its base dir and needs tools.designer_agent.base-dir".to_owned(),
                )
            })?;
            let request = AgentSessionRequest {
                endpoint: AgentEndpoint { host, port },
                user,
                password,
                transcript_log: Some(transcript_log),
            };
            // Чужая точка входа не поднимается заново: недоступная — типизированный отказ.
            let session = AgentSession::open(&request, wait).map_err(map_agent_error)?;
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
            };
            let managed = ManagedAgent::launch(
                utilities.runner_for(UtilityType::V8),
                &launch,
                request,
                Duration::from_millis(agent.startup_timeout_ms.max(1)),
                wait,
            )
            .map_err(map_agent_error)?;
            Ok(AgentHandle::Managed(managed))
        }
    }
}

/// Отказы агента раскладываются по типам раннера: среда, срок, отмена, платформа.
pub(crate) fn map_agent_error(error: AgentError) -> AppError {
    match error {
        AgentError::TimedOut { .. } => AppError::TimedOut(error.to_string()),
        AgentError::Cancelled { .. } => AppError::Cancelled(error.to_string()),
        AgentError::Command { .. }
        | AgentError::Canceled { .. }
        | AgentError::Question { .. }
        | AgentError::NoTerminalMessage { .. }
        | AgentError::InvalidReply { .. }
        | AgentError::SessionClosed { .. }
        | AgentError::Transport { .. }
        | AgentError::UserDirUnknown { .. }
        | AgentError::Exchange { .. } => AppError::Platform(error.to_string()),
        AgentError::Workspace { .. } => AppError::Runtime(error.to_string()),
        AgentError::Unreachable { .. }
        | AgentError::Handshake { .. }
        | AgentError::AuthenticationRejected { .. }
        | AgentError::Channel { .. }
        | AgentError::Launch(_)
        | AgentError::StartupTimedOut { .. } => AppError::EnvironmentUnavailable(error.to_string()),
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

/// Копия чужого каталога внутри каталога пользователя агента: для команд с файловыми
/// параметрами, которые через ссылку не разрешаются.
pub(crate) fn copy_dir_in(
    user_dir: &Path,
    relative: &str,
    target: &Path,
) -> Result<String, AppError> {
    let copy = user_dir.join(relative);
    let _ = crate::support::fs::remove_path_if_exists(&copy);
    crate::support::fs::copy_dir_recursively(target, &copy).map_err(|error| {
        AppError::Runtime(format!(
            "failed to copy '{}' into the agent dir '{}': {error}",
            target.display(),
            copy.display()
        ))
    })?;
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

/// Одна команда с проверкой итога; журнал ответа возвращается как улика.
pub(crate) fn run_command(
    handle: &mut AgentHandle,
    command: &str,
    wait: &WaitPolicy,
) -> Result<agent::AgentReply, AppError> {
    let reply = handle
        .session()
        .run(command, wait)
        .map_err(map_agent_error)?;
    reply.outcome().map_err(map_agent_error)?;
    Ok(reply)
}

/// Ответ агента в форме результата платформы: код 0, журнал сообщений — как stdout,
/// путь журнала сессии — как платформенный журнал.
pub(crate) fn platform_result(
    transcript: String,
    log: PathBuf,
) -> crate::platform::result::PlatformCommandResult {
    crate::platform::result::PlatformCommandResult {
        process: crate::platform::process::ProcessResult {
            exit_code: 0,
            stdout: transcript,
            stderr: String::new(),
            interruption: None,
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
            .map_err(map_agent_error),
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
    session.sftp_mkdir_all(relative).map_err(map_agent_error)?;
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
            session.sftp_mkdir_all(parent).map_err(map_agent_error)?;
        }
        session
            .sftp_put_file(&file, &remote)
            .map_err(map_agent_error)?;
    }
    Ok(relative.to_owned())
}

/// То же, но всегда копией: для команд с файловыми параметрами, которые через ссылку
/// точка входа не разрешает.
pub(crate) fn stage_copy_dir(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    local: &Path,
) -> Result<String, AppError> {
    match exchange {
        Exchange::Dir(user_dir) => copy_dir_in(user_dir, relative, local),
        Exchange::Sftp => stage_dir(handle, exchange, relative, local),
    }
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
                session.sftp_mkdir_all(parent).map_err(map_agent_error)?;
            }
            session
                .sftp_put_file(local, relative)
                .map(|()| relative.to_owned())
                .map_err(map_agent_error)
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
                session.sftp_mkdir_all(parent).map_err(map_agent_error)?;
            }
            session.sftp_write(relative, bytes).map_err(map_agent_error)
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
            .map_err(map_agent_error),
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
            session.sftp_remove_all(relative).map_err(map_agent_error)
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
                .map_err(map_agent_error)?;
            session.sftp_remove_all(relative).map_err(map_agent_error)
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
            session.sftp_remove_all(relative).map_err(map_agent_error)
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
    let reply = session.run(&command, wait).map_err(map_agent_error)?;
    let body = reply.outcome().map_err(map_agent_error)?;
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
    /// Что было сделано, когда токен записан: `build` или `dump`.
    pub after: String,
    pub recorded_at: String,
}

/// Учёт поколений по предмету: файл на каждый набор исходников под `workPath`.
pub(crate) struct GenerationLedger {
    dir: PathBuf,
}

impl GenerationLedger {
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self::at(config.work_path.join("agent").join("generation"))
    }

    fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, source_set: &str) -> PathBuf {
        self.dir.join(format!("{source_set}.json"))
    }

    pub(crate) fn read(&self, source_set: &str) -> Option<GenerationRecord> {
        let text = std::fs::read_to_string(self.path(source_set)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub(crate) fn record(
        &self,
        source_set: &str,
        token: &str,
        after: &str,
    ) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the generation ledger '{}': {error}",
                self.dir.display()
            ))
        })?;
        let record = GenerationRecord {
            token: token.to_owned(),
            after: after.to_owned(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        };
        let text = serde_json::to_string_pretty(&record)
            .map_err(|error| AppError::Runtime(format!("failed to encode generation: {error}")))?;
        std::fs::write(self.path(source_set), text).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write the generation ledger '{}': {error}",
                self.path(source_set).display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_ledger_keeps_one_record_per_source_set() {
        let root = tempfile::tempdir().expect("tempdir");
        let ledger = GenerationLedger::at(root.path().join("generation"));
        assert!(ledger.read("main").is_none());
        ledger.record("main", "abc", "build").expect("record");
        let record = ledger.read("main").expect("record");
        assert_eq!(record.token, "abc");
        assert_eq!(record.after, "build");
        assert!(ledger.read("ext").is_none());
    }
}
