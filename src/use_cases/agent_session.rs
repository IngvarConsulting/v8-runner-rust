//! Точка входа агента для сценариев: одна на команду, общая для `dump` и `build`.
//!
//! Сценарий открывает точку входа по конфигу — управляемую поднимает, к чужой
//! подключается, — получает каталог пользователя агента и работает с файлами через
//! него: агент читает и пишет только внутри своего `AgentBaseDir`, поэтому чужие
//! каталоги выставляются туда символической ссылкой. Здесь же живёт учёт
//! `generation-id`: токен поколения конфигурации, записанный после успешной загрузки
//! или выгрузки и сравниваемый перед следующей выгрузкой.

use crate::change_detection::runner_dirs::{runner_dirs_inside, RunnerDirs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::domain::status::GenerationAfter;
use crate::platform::process::{ProcessInterruption, ProcessInterruptionReason};
use crate::platform::result::PlatformCommandResult;

use serde::{Deserialize, Serialize};

use crate::config::model::{AppConfig, DesignerAgentMode};
use crate::domain::capability::{
    LiveAgentHolder, LiveAgentReason, LiveAgentReceipt, Provider, SessionEndpoint, SessionMode,
};
use crate::domain::source_set::SourceSetContext;
use crate::platform::agent::{
    self, agent_reads_host_key_at, AgentEndpoint, AgentError, AgentLaunch, AgentMessage,
    AgentReply, AgentSession, AgentSessionRequest, HostKeyExpectation, ManagedAgent, WaitPolicy,
};
use crate::platform::agent_keeper::client::{
    Credentials, KeeperClient, KeeperLaunch, KeeperStartError,
};
use crate::platform::agent_keeper::protocol::Holder;
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::copy_dir_recursively;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::interruption::{CommandFailure, Deferrals};

/// Открытая точка входа и учётные данные, которые ход её команд не показывает.
pub(crate) struct AgentHandle {
    point: AgentPoint,
    /// Известные секреты для маски хода: учётные данные базы, СУБД и кластера
    /// ([`progress_secrets`]). Учётные данные SSH агента — те же, что у базы.
    secrets: Vec<String>,
}

/// Точка входа: живой агент через сторожа, свой разовый процесс с сессией или только сессия
/// к чужому.
pub(crate) enum AgentPoint {
    /// Живой агент: процесс и сессию SSH держит сторож, команда владеет только подключением к
    /// базе (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`).
    Live(Box<LiveHandle>),
    /// Разовый агент команды и причина, по которой он не живой
    /// (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`).
    Managed(Box<ManagedAgent>, LiveAgentReason),
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

/// Живой агент команды: сессия команды у сторожа и всё, что команда о ней помнит сама.
pub(crate) struct LiveHandle {
    client: KeeperClient,
    /// `AgentBaseDir` агента в каталоге состояния учётной записи.
    base_dir: PathBuf,
    endpoint: AgentEndpoint,
    receipt: LiveAgentReceipt,
    /// Почему агент не останется, если команда его потеряла посреди запроса: квитанция
    /// называет это, когда сторож после `end` скажет, что агента снимает.
    failure: Option<LiveAgentReason>,
    /// Журнал разговора пишет команда, а не сторож.
    transcript: Option<std::fs::File>,
    secrets: Vec<String>,
}

impl LiveHandle {
    fn run(
        &mut self,
        command: &str,
        policy: &WaitPolicy,
        on_batch: &mut dyn FnMut(&[AgentMessage]),
    ) -> Result<AgentReply, AgentError> {
        use std::io::Write as _;
        let secrets: Vec<&str> = self.secrets.iter().map(String::as_str).collect();
        let transcript = &mut self.transcript;
        let started = std::time::Instant::now();
        // Кадры сторожа приходят по мере ответа агента, и каждый сразу уходит дальше — в журнал
        // разговора и в ход команды, а не копится до итога.
        let outcome = self
            .client
            .run(command, policy, &mut |batch: &[AgentMessage]| {
                if let Some(log) = transcript.as_mut() {
                    let _ = writeln!(log, "{}", agent::transcript_line(batch, &secrets));
                }
                on_batch(batch);
            });
        if let Ok(reply) = &outcome {
            agent::log_reply(command, &secrets, reply, started.elapsed());
        }
        if let Err(error) = &outcome {
            if let Some(reason) = failure_reason(error, self.client.interrupted()) {
                self.failure.get_or_insert(reason);
            }
        }
        outcome
    }

    /// Закрывает сессию команды; агент остаётся сторожу, если тот не решил его снять.
    /// Квитанция называет решение сторожа: `kept` — всегда по его `status` после `end`, а
    /// причина — по тому, что команда видела ([`failure_reason`]), иначе `removed` у сторожа,
    /// который снимает агента или ушёл, и `keeper_failed` у сторожа, который не ответил.
    fn finish(mut self, context: &ExecutionContext) {
        let ended = self.client.end();
        let client = &self.client;
        let fallback = ended_reason(&ended, |error| client.gone(error));
        self.receipt.kept = fallback.is_none();
        self.receipt.reason = fallback.map(|fallback| self.failure.unwrap_or(fallback));
        if let Ok(status) = &ended {
            self.receipt.holders = status.holders.iter().map(holder_receipt).collect();
        }
        context.note_live(self.receipt);
    }
}

/// Останется ли агент по ответу сторожа после `end`: `None` — останется; иначе причина, если
/// команда своей не видела. Сторож, который снимает агента или подтверждённо ушёл (`gone`), —
/// `removed`; не ответивший, но, может быть, живой — `keeper_failed`.
fn ended_reason(
    ended: &Result<crate::platform::agent_keeper::protocol::KeeperStatus, AgentError>,
    gone: impl FnOnce(&AgentError) -> bool,
) -> Option<LiveAgentReason> {
    match ended {
        Ok(status) if !status.closing => None,
        Ok(_) => Some(LiveAgentReason::Removed),
        Err(error) if gone(error) => Some(LiveAgentReason::Removed),
        Err(_) => Some(LiveAgentReason::KeeperFailed),
    }
}

/// Причина работать с разовым агентом, если живого взять нельзя; `None` — отказ команды.
fn one_shot_reason(error: &KeeperStartError) -> Option<LiveAgentReason> {
    match error {
        KeeperStartError::Job => Some(LiveAgentReason::Job),
        KeeperStartError::Limit { .. } => Some(LiveAgentReason::Limit),
        // Живого агента взять нельзя, а разовый работает и так: команда не падает.
        KeeperStartError::Busy { .. } => Some(LiveAgentReason::Busy),
        KeeperStartError::Unavailable { .. }
        | KeeperStartError::Agent(
            AgentError::LiveAgentGone { .. } | AgentError::KeeperUnreachable { .. },
        ) => Some(LiveAgentReason::KeeperFailed),
        KeeperStartError::Agent(_) => None,
    }
}

/// Причина в квитанции по отказу запроса живому агенту. `interrupted` — только отмена или срок,
/// при которых сторож снимает агента (`interrupted` у клиента); оборванный ответ или сессия —
/// `removed`; сторож, ушедший или не ответивший, — `keeper_failed`. Отказ самой команды агента
/// агента не снимает: причины нет.
fn failure_reason(error: &AgentError, interrupted: bool) -> Option<LiveAgentReason> {
    match error {
        AgentError::InvalidReply { .. } | AgentError::SessionClosed { .. } => {
            Some(LiveAgentReason::Removed)
        }
        AgentError::LiveAgentGone { .. } | AgentError::KeeperUnreachable { .. } => {
            Some(LiveAgentReason::KeeperFailed)
        }
        // Отмену и срок, при которых сторож снимает агента, классифицирует клиент
        // (`interrupted`): здесь вид отмены не разбирается.
        _ if interrupted => Some(LiveAgentReason::Interrupted),
        _ => None,
    }
}

/// Держатель живого агента в квитанции: процесс — номером и временем старта, срок — мгновением.
pub(crate) fn holder_receipt(holder: &Holder) -> LiveAgentHolder {
    let rfc3339 = |at: std::time::SystemTime| {
        chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    match holder {
        Holder::Process { pid, started } => LiveAgentHolder::Process {
            pid: *pid,
            started_at: crate::platform::service::started_at(*started).map(rfc3339),
        },
        Holder::Ttl { until_ms, .. } => LiveAgentHolder::Ttl {
            until: rfc3339(std::time::UNIX_EPOCH + Duration::from_millis(*until_ms)),
        },
    }
}

impl AgentHandle {
    /// SSH-сессия точки входа: для обмена файлами по SFTP. У живого агента её держит сторож, и
    /// файлы с ним идут только каталогом.
    pub(crate) fn ssh(&mut self) -> Result<&mut AgentSession, AppError> {
        match &mut self.point {
            AgentPoint::Live(_) => Err(AppError::capability(
                "the live agent exchanges files through its directory, not over SFTP",
            )),
            AgentPoint::Managed(agent, _) => Ok(agent.session()),
            AgentPoint::Attached { session, .. } | AgentPoint::Gate { session, .. } => Ok(session),
        }
    }

    /// Одна команда агенту.
    pub(crate) fn run(
        &mut self,
        command: &str,
        wait: &WaitPolicy,
    ) -> Result<AgentReply, AgentError> {
        let secrets: Vec<&str> = self.secrets.iter().map(String::as_str).collect();
        let mut progress = |batch: &[AgentMessage]| {
            crate::use_cases::progress::log_agent_messages(batch, &secrets)
        };
        match &mut self.point {
            AgentPoint::Live(live) => live.run(command, wait, &mut progress),
            AgentPoint::Managed(agent, _) => {
                agent.session().run_streaming(command, wait, &mut progress)
            }
            AgentPoint::Attached { session, .. } | AgentPoint::Gate { session, .. } => {
                session.run_streaming(command, wait, &mut progress)
            }
        }
    }

    /// Прерывание, отложенное в критической фазе за всю сессию.
    pub(crate) fn deferred_interruption(&self) -> Option<ProcessInterruptionReason> {
        match &self.point {
            AgentPoint::Live(live) => live.client.deferred_interruption(),
            AgentPoint::Managed(agent, _) => agent.session_ref().deferred_interruption(),
            AgentPoint::Attached { session, .. } | AgentPoint::Gate { session, .. } => {
                session.deferred_interruption()
            }
        }
    }

    /// Бросил ли второй сигнал команду чужого агента: свой агент и живой агент сторожа
    /// ожидания не бросают.
    pub(crate) fn abandoned(&self) -> bool {
        match &self.point {
            AgentPoint::Live(_) | AgentPoint::Managed(..) => false,
            AgentPoint::Attached { session, .. } | AgentPoint::Gate { session, .. } => {
                session.abandoned()
            }
        }
    }

    /// Прерывание, которое отложила последняя команда.
    pub(crate) fn last_command_deferral(&self) -> Option<ProcessInterruptionReason> {
        match &self.point {
            AgentPoint::Live(live) => live.client.last_command_deferral(),
            AgentPoint::Managed(agent, _) => agent.session_ref().last_command_deferral(),
            AgentPoint::Attached { session, .. } | AgentPoint::Gate { session, .. } => {
                session.last_command_deferral()
            }
        }
    }

    /// Режим и адрес, к которому открыта сессия: адрес — тот, к которому подключился
    /// SSH-клиент, напечатанный из хоста и порта; учётных данных в нём нет.
    pub(crate) fn endpoint(&self) -> SessionEndpoint {
        match &self.point {
            AgentPoint::Live(live) => session_endpoint(SessionMode::Managed, &live.endpoint),
            AgentPoint::Managed(agent, _) => {
                session_endpoint(SessionMode::Managed, agent.endpoint())
            }
            AgentPoint::Attached { session, .. } => {
                session_endpoint(SessionMode::Attached, session.endpoint())
            }
            AgentPoint::Gate { session, .. } => {
                session_endpoint(SessionMode::Gate, session.endpoint())
            }
        }
    }

    /// Канал обмена с точкой входа. У агента Конфигуратора это его каталог
    /// пользователя из карты `AgentBaseDir`, у шлюза автономного сервера — то, что
    /// объявил конфиг.
    pub(crate) fn exchange(&self, config: &AppConfig) -> Result<Exchange, AppError> {
        let base_dir = match &self.point {
            AgentPoint::Live(live) => live.base_dir.as_path(),
            AgentPoint::Managed(agent, _) => agent.base_dir(),
            AgentPoint::Attached { base_dir, .. } => base_dir,
            AgentPoint::Gate { exchange, .. } => return Ok(exchange.clone()),
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
    pub(crate) fn finish(self, context: &ExecutionContext, wait: &WaitPolicy) {
        match self.point {
            AgentPoint::Live(live) => live.finish(context),
            AgentPoint::Managed(agent, _) => agent.release_and_remove(),
            AgentPoint::Attached { session, .. } | AgentPoint::Gate { session, .. } => {
                session.release(wait)
            }
        }
    }
}

/// Точка входа для квитанции: адрес печатается из разобранных хоста и порта. Запись с
/// учётными данными до этого места не доходит — разбор адреса отказывает ей раньше.
fn session_endpoint(mode: SessionMode, endpoint: &AgentEndpoint) -> SessionEndpoint {
    SessionEndpoint {
        mode,
        address: endpoint.to_string(),
        live: None,
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
/// Класс по умолчанию — для команд без побочного эффекта; меняющие команды называют
/// свой класс через `run_critical`. У своего агента можно прервать чтение и снять процесс,
/// у чужого после отправки команды раннер ждёт итогового ответа, пока второй сигнал
/// (`ExecutionContext::abandonment`) не велит бросить ожидание.
/// Срока у агентской команды нет: команда его не имеет
/// (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE), а транспорту брать его неоткуда — замер
/// 15.09.2026 дал тишине занятого агента нижнюю границу и не дал верхней. Ожидание
/// заканчивает оператор, а мёртвый шлюз — таймаут записи TCP на keepalive-пакетах.
pub(crate) fn wait_policy(context: &ExecutionContext) -> WaitPolicy {
    WaitPolicy::from_step(context.process_policy(InterruptionSafetyClass::GracefulThenKill, None))
        .with_abandonment(context.abandonment())
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
    let handle = AgentHandle {
        point: open_handle(context, config, utilities, v8, transcript_log, wait)?,
        secrets: progress_secrets(config),
    };
    context.note_session(handle.endpoint());
    match &handle.point {
        AgentPoint::Live(live) => context.note_live(live.receipt.clone()),
        AgentPoint::Managed(_, reason) => context.note_live(LiveAgentReceipt {
            reused: false,
            holders: Vec::new(),
            kept: false,
            reason: Some(*reason),
        }),
        AgentPoint::Attached { .. } | AgentPoint::Gate { .. } => {}
    }
    Ok(handle)
}

/// Учётные данные, которых ни ход команды, ни журнал разговора с агентом не показывают:
/// пользователь и пароль базы — они же учётные данные SSH агента, — СУБД и администраторов
/// кластера. Список один на точку входа: его берут и [`AgentHandle`], и журнал живого агента.
fn progress_secrets(config: &AppConfig) -> Vec<String> {
    let infobase = &config.infobase;
    let dbms = infobase.dbms.as_ref();
    let cluster = infobase.cluster.as_ref();
    let cluster_agent = cluster.and_then(|cluster| cluster.agent.as_ref());
    let mut secrets: Vec<String> = [
        infobase.user.as_ref(),
        infobase.password.as_ref(),
        dbms.and_then(|dbms| dbms.user.as_ref()),
        dbms.and_then(|dbms| dbms.password.as_ref()),
        cluster.and_then(|cluster| cluster.user.as_ref()),
        cluster.and_then(|cluster| cluster.password.as_ref()),
        cluster_agent.and_then(|agent| agent.user.as_ref()),
        cluster_agent.and_then(|agent| agent.password.as_ref()),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .cloned()
    .collect();
    secrets.sort();
    secrets.dedup();
    secrets
}

/// Отказ агенту, выбранному рядом с объявленной строкой прямого шлюза, когда канала нет:
/// строка уже объявлена, и не хватает только канала.
const GATE_WITHOUT_A_CHANNEL: &str = "the agent works with a standalone server through its SSH gate, and files travel there only by a declared channel: set infobase.standalone.exchange to `sftp` (through the gate) or to `{ dir: … }` — the gate user's directory as the runner sees it";

fn open_handle(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    v8: Option<&Path>,
    transcript_log: PathBuf,
    wait: &WaitPolicy,
) -> Result<AgentPoint, AppError> {
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
        return Ok(AgentPoint::Gate { session, exchange });
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
            Ok(AgentPoint::Attached { session, base_dir })
        }
        DesignerAgentMode::Managed { port } => {
            let v8 = v8.ok_or_else(|| {
                AppError::EnvironmentUnavailable(
                    "the managed Designer agent needs the local platform; no 1cv8 was selected"
                        .to_owned(),
                )
            })?;
            let reason = if agent.live_enabled() {
                match open_live(context, config, v8, port, &transcript_log)? {
                    Ok(live) => return Ok(AgentPoint::Live(Box::new(live))),
                    Err(reason) => reason,
                }
            } else {
                // Живой агент выключен: живой агент этой рабочей копии, если остался, снимается
                // (`INV.CONFIG.A-LIVE-AGENT-IS-ON-UNLESS-TURNED-OFF`).
                remove_live_agent(config, v8);
                LiveAgentReason::LiveAgentOff
            };
            let launch = AgentLaunch {
                v8: v8.to_path_buf(),
                infobase_args: connection.infobase_args(),
                port,
                host_key: declared_launch_host_key(agent.host_key.clone())?,
                key_work_path: config.work_path.clone(),
                base_dir: config.work_path.join("agent").join("base"),
                workdir: config.work_path.join("agent").join("workdir"),
                process_log: transcript_log.with_extension("process"),
                user,
                password,
                transcript_log: Some(transcript_log),
            };
            let managed = ManagedAgent::launch(
                utilities.runner_for(UtilityType::V8),
                launch,
                Duration::from_millis(agent.startup_timeout_ms.max(1)),
                wait,
            )
            .map_err(AppError::from)?;
            Ok(AgentPoint::Managed(Box::new(managed), reason))
        }
    }
}

/// Ключ живого агента: строка подключения базы, платформа, пользователь базы, `workPath`, версия
/// раннера и учётная запись ОС (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`). Пароля
/// базы в ключе нет: учётные данные сторож сверяет с каждым запросом.
fn live_key(state: &Path, config: &AppConfig, v8: &Path) -> std::io::Result<String> {
    crate::support::machine::account::account_key(state, &live_key_parts(config, v8).as_parts())
}

/// Ключ живого агента, только если соль учётной записи уже есть: `None` без неё. Ничего не
/// создаёт.
pub(crate) fn existing_live_key(
    state: &Path,
    config: &AppConfig,
    v8: &Path,
) -> std::io::Result<Option<String>> {
    crate::support::machine::account::existing_account_key(
        state,
        &live_key_parts(config, v8).as_parts(),
    )
}

/// Части ключа живого агента. Пути идут байтами ОС (шестнадцатерично), а не их показом: два
/// разных пути с одинаковым показом — разные ключи.
struct LiveKeyParts {
    address: String,
    platform: String,
    user: String,
    work_path: String,
}

impl LiveKeyParts {
    fn as_parts(&self) -> [&str; 6] {
        [
            "live-agent",
            &self.address,
            &self.platform,
            &self.user,
            &self.work_path,
            env!("CARGO_PKG_VERSION"),
        ]
    }
}

fn live_key_parts(config: &AppConfig, v8: &Path) -> LiveKeyParts {
    let connection = config.v8_connection();
    LiveKeyParts {
        address: connection.infobase_args().join("\u{1f}"),
        platform: hex(v8.as_os_str().as_encoded_bytes()),
        user: connection.user.clone().unwrap_or_default(),
        work_path: hex(config.work_path.as_os_str().as_encoded_bytes()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Путь базы в каноническом виде: по нему находят живых агентов одной базы из разных рабочих
/// копий. Строка подключения иной формы может нести учётные данные (`Usr=`, `Pwd=`): в запись
/// агента она идёт только отпечатком SHA-256.
pub(crate) fn canonical_infobase(config: &AppConfig) -> String {
    use sha2::{Digest, Sha256};
    let connection = config.v8_connection();
    connection
        .snapshot_identity(&config.base_path)
        .unwrap_or_else(|| {
            let joined = connection.infobase_args().join("\u{1f}");
            format!("connection:{}", hex(&Sha256::digest(joined.as_bytes())))
        })
}

/// Что нужно, чтобы взять или поднять живого агента цели проекта: ключ, запуск без учётных
/// данных и учётные данные базы.
pub(crate) struct LiveTarget {
    pub key: String,
    pub launch: KeeperLaunch,
    pub credentials: Credentials,
}

/// Живой агент цели проекта: ключ (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`) и
/// запуск. Объявленный ключ хоста проверяется до запуска, как у разового агента.
pub(crate) fn live_target(
    config: &AppConfig,
    v8: &Path,
    port: Option<u16>,
) -> Result<LiveTarget, AppError> {
    let agent = &config.tools.designer_agent;
    let connection = config.v8_connection();
    let state = crate::support::machine::account::account_state_dir().map_err(|error| {
        AppError::EnvironmentUnavailable(format!(
            "the live Designer agent needs the account state directory: {error}; \
             set tools.designer_agent.live: false to work without it"
        ))
    })?;
    let key = live_key(&state, config, v8)
        .map_err(|error| AppError::Runtime(format!("cannot key the live agent: {error}")))?;
    let host_key = declared_launch_host_key(agent.host_key.clone())?;
    let launch = KeeperLaunch {
        v8: v8.to_path_buf(),
        infobase_args: connection.infobase_args(),
        port,
        host_key,
        startup_timeout_ms: agent.startup_timeout_ms.max(1),
        infobase: canonical_infobase(config),
        work_path: Some(config.work_path.clone()),
    };
    let credentials = Credentials {
        user: connection.user.clone().unwrap_or_default(),
        password: connection.password.clone().unwrap_or_default(),
    };
    Ok(LiveTarget {
        key,
        launch,
        credentials,
    })
}

/// Поднимает или берёт живого агента. `Ok(Err(reason))` — живого агента не будет, и команда
/// работает с разовым: раннер в Job без права выхода (`INV.PLATFORM.A-CLOSED-JOB-KEEPS-THE-AGENT-ONE-SHOT`)
/// или живых агентов уже на пределе (`INV.PLATFORM.A-COMMAND-OVER-THE-LIMIT-USES-A-ONE-SHOT-AGENT`).
fn open_live(
    context: &ExecutionContext,
    config: &AppConfig,
    v8: &Path,
    port: Option<u16>,
    transcript_log: &Path,
) -> Result<Result<LiveHandle, LiveAgentReason>, AppError> {
    let agent = &config.tools.designer_agent;
    let LiveTarget {
        key,
        launch,
        credentials,
    } = live_target(config, v8, port)?;
    let mut client = match KeeperClient::connect_or_start(
        &key,
        &launch,
        &credentials,
        agent.live_limit(),
        &context.cancellation(),
    ) {
        Ok(client) => client,
        Err(error) => {
            return match one_shot_reason(&error) {
                Some(reason) => {
                    tracing::warn!(error = %crate::support::error::chain(&error), ?reason, "the command works with a one-shot agent");
                    Ok(Err(reason))
                }
                None => match error {
                    KeeperStartError::Agent(error) => Err(AppError::from(error)),
                    other => Err(AppError::EnvironmentUnavailable(
                        crate::support::error::chain(&other),
                    )),
                },
            }
        }
    };
    let port = client.port().ok_or_else(|| {
        AppError::EnvironmentUnavailable("the live agent keeper names no agent port".to_owned())
    })?;
    let mut transcript = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(transcript_log)
        .ok();
    // Учётные данные сторожа — те же пользователь и пароль базы, что в общем списке.
    let secrets = progress_secrets(config);
    // Подключение к базе ведёт сторож: что агент сказал при нём, команда пишет в свой журнал,
    // как с разовым агентом, с той же маской учётных данных.
    let connect_reply = client.take_connect_reply();
    if let (Some(log), false) = (transcript.as_mut(), connect_reply.is_empty()) {
        use std::io::Write as _;
        let masked: Vec<&str> = secrets.iter().map(String::as_str).collect();
        let _ = writeln!(log, "{}", agent::transcript_line(&connect_reply, &masked));
    }
    let receipt = LiveAgentReceipt {
        reused: client.reused(),
        holders: Vec::new(),
        kept: true,
        reason: None,
    };
    Ok(Ok(LiveHandle {
        base_dir: client.base_dir(),
        endpoint: AgentEndpoint {
            host: crate::support::authority::Host::Address(std::net::IpAddr::V4(
                std::net::Ipv4Addr::LOCALHOST,
            )),
            port,
        },
        receipt,
        failure: None,
        transcript,
        secrets,
        client,
    }))
}

/// Снимает живого агента этой рабочей копии, если он есть: с `live: false` его не остаётся.
/// Отказ снятия команду не останавливает: она работает с разовым агентом и так.
/// Каталога состояния и соли учётной записи она не создаёт: без них живого агента нет.
fn remove_live_agent(config: &AppConfig, v8: &Path) {
    let Some(state) = crate::support::machine::account::existing_account_state_dir() else {
        return;
    };
    let key = match existing_live_key(&state, config, v8) {
        Ok(Some(key)) => key,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "the live agent of this working copy cannot be keyed");
            return;
        }
    };
    match crate::platform::agent_keeper::client::stop_keeper(&state, &key) {
        Ok(true) => tracing::info!("the live agent of this working copy is removed: live is off"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "the live agent of this working copy was not removed"),
    }
}

/// Объявленный ключ должен лежать по пути, с которого агент умеет его читать
/// (INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII).
fn declared_launch_host_key(declared: Option<PathBuf>) -> Result<Option<PathBuf>, AppError> {
    match &declared {
        Some(path) if !agent_reads_host_key_at(path) => {
            Err(AppError::EnvironmentUnavailable(format!(
                "tools.designer_agent.host-key '{}' is not an ASCII path: the Designer agent \
                 cannot read its host key there and exits; move the key to an ASCII path",
                path.display()
            )))
        }
        _ => Ok(declared),
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
/// нельзя, каталог копируется — медленно, но той же формы. Так можно отдать только то, что
/// агент читает; каталог, в который он пишет, выставляет [`expose_output_dir`].
pub(crate) fn expose_dir(
    user_dir: &Path,
    relative: &str,
    target: &Path,
    dirs: RunnerDirs<'_>,
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
    if link_dir(&target, &link).is_err() {
        copy_for_the_agent(&target, user_dir, dirs, &link).map_err(|error| {
            AppError::Runtime(format!(
                "failed to expose '{}' to the agent at '{}': {error}",
                target.display(),
                link.display()
            ))
        })?;
    }
    Ok(relative.to_owned())
}

/// Выставляет агенту каталог, в который он пишет. Копия здесь не годится: агент обновил бы её,
/// а цель осталась бы прежней при успешном ответе. Поэтому без ссылки это отказ, а не копия.
pub(crate) fn expose_output_dir(
    user_dir: &Path,
    relative: &str,
    target: &Path,
) -> Result<String, AppError> {
    expose_output_with(user_dir, relative, target, link_dir)
}

fn expose_output_with(
    user_dir: &Path,
    relative: &str,
    target: &Path,
    link_to: impl Fn(&Path, &Path) -> std::io::Result<()>,
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
            "target dir '{}' is not reachable: {error}",
            target.display()
        ))
    })?;
    let _ = crate::support::fs::remove_link(&link);
    link_to(&target, &link).map_err(|error| {
        AppError::EnvironmentUnavailable(format!(
            "the agent cannot write into '{}': no link can be created at '{}' ({error}); name another executor in providers.pull or, at a gate, use exchange: sftp",
            target.display(),
            link.display()
        ))
    })?;
    Ok(relative.to_owned())
}

/// Символическая ссылка, а на Windows без права на неё — точка соединения: она права не требует.
fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    symlink_dir(target, link).or_else(|symlink_error| {
        junction_dir(target, link).map_err(|junction_error| {
            std::io::Error::other(format!("{symlink_error}; {junction_error}"))
        })
    })
}

/// Точка соединения хранит локальный путь: на сетевом диске запись примет NTFS, но открыть такую
/// ссылку будет нельзя, а в каталоге обмена на чужом сервере она вела бы к его путям.
#[cfg(windows)]
fn is_on_a_local_disk(path: &Path) -> std::io::Result<bool> {
    use std::path::{Component, Prefix};
    Ok(matches!(
        path.canonicalize()?.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    ))
}

#[cfg(windows)]
fn junction_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    let directory = link.parent().unwrap_or(link);
    if !is_on_a_local_disk(target)? || !is_on_a_local_disk(directory)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "a junction links directories on local disks only",
        ));
    }
    junction::create(target, link).inspect_err(|_| {
        // Крейт сперва создаёт каталог и при отказе его оставляет.
        let _ = std::fs::remove_dir(link);
    })
}

#[cfg(not(windows))]
fn junction_dir(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Набор в корне рабочей области держит каталог агента, `workPath` и `localPath` внутри себя:
/// копия не должна входить в собственное назначение и нести служебные каталоги, а метаданным с именем
/// вроде `build` глубже верхнего уровня в ней место.
fn copy_for_the_agent(
    target: &Path,
    user_dir: &Path,
    dirs: RunnerDirs<'_>,
    link: &Path,
) -> std::io::Result<()> {
    let agent_dir = user_dir.canonicalize()?;
    if agent_dir == target {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the agent directory is the directory to copy",
        ));
    }
    if !agent_dir.starts_with(target) {
        return copy_dir_recursively(target, link);
    }
    let mut excluded = vec![agent_dir];
    excluded.extend(runner_dirs_inside(target, dirs));
    std::fs::create_dir_all(link)?;
    for entry in crate::change_detection::scanner::top_level_source_entries(target, excluded) {
        let entry = entry?;
        let Ok(relative) = entry.path().strip_prefix(target) else {
            continue;
        };
        let destination = link.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination)?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &destination)?;
        }
    }
    Ok(())
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

/// Каталог для новых файлов, которые агент должен *написать* и раннер затем заберёт (каталог
/// набора, куда агент пишет сам, выставляет [`expose_output_dir`]): настоящий подкаталог
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
    let deferred = handle.deferred_interruption();
    handle.finish(context, &wait);
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
    run_critical_typed(handle, action, command, wait, deferrals)
        .map_err(|error| critical_failure(action, error))
}

/// То же, что `run_critical`, но отказ остаётся типизированным: сценарий, которому важен род
/// отказа, решает по нему, а не по тексту. Текст отказа даёт `critical_failure`.
pub(crate) fn run_critical_typed(
    handle: &mut AgentHandle,
    action: &str,
    command: &str,
    wait: &WaitPolicy,
    deferrals: &mut Deferrals,
) -> Result<agent::AgentReply, AgentError> {
    let outcome = run_checked(handle, command, &wait.critical());
    let deferral = handle
        .last_command_deferral()
        .map(ProcessInterruption::deferred);
    deferrals.note_outcome(action, &outcome, deferral);
    outcome
}

/// Отказ критической команды в словах сценария.
pub(crate) fn critical_failure(action: &str, error: AgentError) -> AppError {
    AppError::from(error).with_context(format!("agent {action}"))
}

/// Одна команда с проверкой итога; журнал ответа возвращается как улика.
pub(crate) fn run_command(
    handle: &mut AgentHandle,
    command: &str,
    wait: &WaitPolicy,
) -> Result<agent::AgentReply, AppError> {
    run_checked(handle, command, wait).map_err(AppError::from)
}

fn run_checked(
    handle: &mut AgentHandle,
    command: &str,
    wait: &WaitPolicy,
) -> Result<agent::AgentReply, AgentError> {
    let reply = handle.run(command, wait)?;
    reply.outcome()?;
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
    dirs: RunnerDirs<'_>,
) -> Result<String, AppError> {
    match exchange {
        Exchange::Dir(user_dir) => expose_dir(user_dir, relative, local, dirs),
        Exchange::Sftp => handle
            .ssh()?
            .sftp_put_dir(local, relative)
            .map(|()| relative.to_owned())
            .map_err(AppError::from),
    }
}

/// По SFTP передаём файлы списка и корневые описатели. Когда сам корень в
/// списке, дополнительно передаём файлы объявленных объектов: платформа
/// может прочитать их из каталога, даже если их нет в списке загрузки.
pub(crate) fn stage_dir_partially(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
    local: &Path,
    files: &[PathBuf],
    dirs: RunnerDirs<'_>,
) -> Result<String, AppError> {
    let Exchange::Sftp = exchange else {
        return stage_dir(handle, exchange, relative, local, dirs);
    };
    use crate::change_detection::partial_load::{
        relative_paths, root_dependencies, CONFIGURATION_XML,
    };

    let configuration = local.join(CONFIGURATION_XML);
    let mut selected = files.to_vec();
    if files.contains(&configuration) {
        selected.extend(
            root_dependencies(local)
                .map_err(|error| AppError::SourceLoadNotStarted(error.to_string()))?,
        );
    }
    selected.extend(
        [
            configuration,
            local.join(crate::use_cases::ignored_files::VERSION_FILE_NAME),
        ]
        .into_iter()
        .filter(|path| path.is_file()),
    );
    selected.sort_unstable();
    selected.dedup();
    let paths = relative_paths(&selected, local)
        .map_err(|error| AppError::SourceLoadNotStarted(error.to_string()))?;
    let session = handle.ssh()?;
    session.sftp_mkdir_all(relative).map_err(AppError::from)?;
    for inside in paths {
        let file = local.join(&inside);
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
            let session = handle.ssh()?;
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
            let session = handle.ssh()?;
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
            .ssh()?
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
            let session = handle.ssh()?;
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
            let session = handle.ssh()?;
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
            let session = handle.ssh()?;
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
///
/// Ссылку снимает и после брошенной команды: это отрезает агента от чужого каталога, а не
/// убирает за ним.
pub(crate) fn unstage(handle: &mut AgentHandle, exchange: &Exchange, relative: &str) {
    match exchange {
        Exchange::Dir(user_dir) => withdraw_dir(user_dir, relative),
        Exchange::Sftp => tidy(handle, exchange, relative),
    }
}

/// Убирает след прогона со стороны точки входа: путь и опустевшие родители.
///
/// След брошенной команды остаётся: агент, возможно, ещё пишет туда, и удаление шло бы
/// наперегонки с ним — через SFTP это десятки секунд и всё равно отказ (живой прогон #296).
pub(crate) fn tidy(handle: &mut AgentHandle, exchange: &Exchange, relative: &str) {
    if handle.abandoned() {
        tracing::debug!(
            path = relative,
            "the abandoned agent command keeps its exchange path"
        );
        return;
    }
    match exchange {
        Exchange::Dir(user_dir) => tidy_run_path(user_dir, relative),
        Exchange::Sftp => {
            if let Ok(session) = handle.ssh() {
                let _ = session.sftp_remove_all(relative);
                session.sftp_remove_empty_parents(relative);
            }
        }
    }
}

/// Снимает выставленный каталог: ссылку — как ссылку, копию — целиком.
pub(crate) fn withdraw_dir(user_dir: &Path, relative: &str) {
    let link = user_dir.join(relative);
    match std::fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let _ = crate::support::fs::remove_link(&link);
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
    handle: &mut AgentHandle,
    extension: Option<&str>,
    wait: &WaitPolicy,
) -> Result<String, AppError> {
    let mut command = String::from("config generation-id");
    if let Some(extension) = extension {
        command.push_str(&format!(" --extension={}", argument(extension)));
    }
    let reply = handle.run(&command, wait).map_err(AppError::from)?;
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
    /// Инструмент, которым получен токен: токены разных инструментов сравнимы только там,
    /// где это замерено ([`TokenScope`],
    /// `INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`).
    pub tool: Provider,
    /// Что было сделано, когда токен записан.
    pub after: GenerationAfter,
    /// Применена ли загрузка к конфигурации базы данных. `false` — загружено без
    /// применения (`push --no-apply` или неудачное применение): поле пишется только тогда,
    /// и запись без него — прежняя запись, она применённая
    /// (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`).
    #[serde(default = "applied_by_default", skip_serializing_if = "is_applied")]
    pub applied: bool,
    pub recorded_at: String,
    /// Привязка памяти набора (база, каталог, назначение, имя): запись другой пары чужая.
    pub identity: String,
}

const fn applied_by_default() -> bool {
    true
}

#[allow(clippy::trivially_copy_pass_by_ref, reason = "подпись задаёт serde")]
const fn is_applied(applied: &bool) -> bool {
    *applied
}

/// Применена ли загрузка, после которой пишется запись поколения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyMark {
    Applied,
    /// Загружено без применения: запись несёт `applied: false`.
    Unapplied,
}

impl GenerationRecord {
    /// Признак применения записи.
    pub(crate) const fn mark(&self) -> ApplyMark {
        if self.applied {
            ApplyMark::Applied
        } else {
            ApplyMark::Unapplied
        }
    }
}

/// Ответ записи на вопрос «менялась ли база с прошлого чтения».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenerationComparison {
    /// Сравнимый инструмент отдал тот же токен: основную конфигурацию не трогали.
    Unchanged,
    /// Сравнимый инструмент отдал другой токен: была запись, возможно того же самого.
    Changed,
    /// Токен получен несравнимым инструментом: ответа нет — ни совпадения, ни расхождения.
    NoAnswer,
}

/// Чьи токены поколения сравнимы у записи набора.
///
/// Замер #559 (файловые базы, 8.3.24, 8.3.27, 8.5.4): токен основной конфигурации у агента,
/// Конфигуратора и `ibcmd` один и тот же в каждом состоянии — как есть, после применённой
/// правки, загруженной без применения и применённой снова. У расширения токены инструментов
/// различаются (один читает загруженное, другой применённое), кластер и автономный сервер не
/// замерены: там токен сравнивается только внутри своего инструмента.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenScope {
    /// Только тот же инструмент.
    OwnTool,
    /// Любой из агента, Конфигуратора и `ibcmd`.
    AnyTool,
}

impl TokenScope {
    /// Сравним ли токен инструмента `read` с токеном, записанным инструментом `recorded`.
    pub(crate) fn compares(self, recorded: Provider, read: Provider) -> bool {
        recorded == read
            || (self == Self::AnyTool && Self::measured(recorded) && Self::measured(read))
    }

    /// Отвечает ли инструмент поколением, которое раннер записывает: агент, Конфигуратор и
    /// `ibcmd`.
    pub(crate) const fn measured(tool: Provider) -> bool {
        matches!(tool, Provider::Agent | Provider::Designer | Provider::Ibcmd)
    }
}

impl GenerationRecord {
    /// Сравнивает записанный токен с прочитанным сейчас инструментом `tool` в пределах
    /// `scope`. Токен пустой базы сравнивается как любой другой.
    pub(crate) fn compare(
        &self,
        tool: Provider,
        token: &str,
        scope: TokenScope,
    ) -> GenerationComparison {
        if !scope.compares(self.tool, tool) {
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

/// Журнал поколений базы: `localPath/infobases/<база>/generation.json`, запись на набор.
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
    pub(crate) fn of(context: &SourceSetContext) -> Option<Self> {
        Some(Self {
            file: context.generation_file()?,
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
    #[cfg(test)]
    pub(crate) fn record(
        &self,
        tool: Provider,
        token: &str,
        after: GenerationAfter,
    ) -> Result<(), AppError> {
        self.record_as(tool, token, after, ApplyMark::Applied)
    }

    /// Записывает поколение с признаком применения: `applied: false` — загружено без
    /// применения к конфигурации базы данных.
    pub(crate) fn record_as(
        &self,
        tool: Provider,
        token: &str,
        after: GenerationAfter,
        mark: ApplyMark,
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
            applied: mark == ApplyMark::Applied,
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

    fn keeper_status(closing: bool) -> crate::platform::agent_keeper::protocol::KeeperStatus {
        crate::platform::agent_keeper::protocol::KeeperStatus {
            keeper_pid: 1,
            agent_pid: Some(2),
            fingerprint: None,
            busy: None,
            connected: false,
            last: None,
            holders: Vec::new(),
            closing,
            idle_ms: 0,
            fresh: false,
        }
    }

    fn unreachable(kind: std::io::ErrorKind) -> AgentError {
        AgentError::KeeperUnreachable {
            channel: "c".to_owned(),
            source: std::io::Error::new(kind, "x"),
        }
    }

    /// `kept` в квитанции — по ответу сторожа после `end`: агент, которого сторож снимает или
    /// который ушёл, — `removed`; сторож, не ответивший в срок, — `keeper_failed`, а не
    /// `removed` (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`).
    #[test]
    fn the_receipt_says_removed_only_when_the_keeper_removes_the_agent() {
        assert_eq!(ended_reason(&Ok(keeper_status(false)), |_| false), None);
        assert_eq!(
            ended_reason(&Ok(keeper_status(true)), |_| false),
            Some(LiveAgentReason::Removed)
        );
        assert_eq!(
            ended_reason(&Err(unreachable(std::io::ErrorKind::NotFound)), |_| true),
            Some(LiveAgentReason::Removed)
        );
        assert_eq!(
            ended_reason(&Err(unreachable(std::io::ErrorKind::TimedOut)), |_| false),
            Some(LiveAgentReason::KeeperFailed)
        );
    }

    /// Живой агент, занятый сессией другой команды, команду не роняет: она работает с разовым, и
    /// квитанция говорит `busy`; сторож, который не поднялся, — `keeper_failed`
    /// (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`).
    #[test]
    fn a_busy_live_agent_makes_the_command_one_shot_with_reason_busy() {
        assert_eq!(
            one_shot_reason(&KeeperStartError::Busy {
                command: Some("config dump-cfg".to_owned())
            }),
            Some(LiveAgentReason::Busy)
        );
        assert_eq!(
            one_shot_reason(&KeeperStartError::Unavailable {
                detail: "d".to_owned(),
                source: None
            }),
            Some(LiveAgentReason::KeeperFailed)
        );
        assert_eq!(
            one_shot_reason(&KeeperStartError::Agent(AgentError::LiveAgentGone {
                detail: "d".to_owned()
            })),
            Some(LiveAgentReason::KeeperFailed)
        );
        assert_eq!(
            one_shot_reason(&KeeperStartError::Agent(AgentError::PortTaken {
                port: 1,
                detail: "d".to_owned()
            })),
            None
        );
    }

    /// `interrupted` — только отмена или срок, при которых сторож снимает агента; оборванный
    /// ответ — `removed`, пропавший сторож — `keeper_failed`; отказ команды агента причины не даёт.
    #[test]
    fn a_failed_request_names_its_own_reason() {
        let timed_out = AgentError::TimedOut {
            command: "c".to_owned(),
            timeout_ms: 0,
        };
        assert_eq!(
            failure_reason(&timed_out, true),
            Some(LiveAgentReason::Interrupted)
        );
        assert_eq!(failure_reason(&timed_out, false), None);
        assert_eq!(
            failure_reason(
                &AgentError::InvalidReply {
                    detail: "d".to_owned(),
                    head: "h".to_owned()
                },
                false
            ),
            Some(LiveAgentReason::Removed)
        );
        assert_eq!(
            failure_reason(&unreachable(std::io::ErrorKind::BrokenPipe), true),
            Some(LiveAgentReason::KeeperFailed)
        );
        assert_eq!(
            failure_reason(
                &AgentError::Command {
                    error_type: crate::platform::agent::AgentErrorType::parse("X"),
                    message: "m".to_owned()
                },
                false
            ),
            None
        );
    }

    /// Объявленный ключ хоста по пути не в ASCII — отказ до запуска, называющий путь
    /// (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`).
    #[cfg(windows)]
    #[test]
    fn a_declared_host_key_outside_ascii_is_refused_before_the_launch() {
        let declared = PathBuf::from("ключи").join("host_ed25519");
        let refused = declared_launch_host_key(Some(declared)).expect_err("refused");
        assert!(
            matches!(&refused, AppError::EnvironmentUnavailable(message)
                if message.contains("ключи") && message.contains("ASCII")),
            "{refused}"
        );
        assert!(matches!(
            declared_launch_host_key(Some(PathBuf::from("keys/host_ed25519"))),
            Ok(Some(_))
        ));
    }

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
                managed: None,
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

    /// Цель выгрузки без ссылки — отказ: копию агент обновил бы при успешном ответе, а цель
    /// осталась бы прежней.
    #[test]
    fn an_output_directory_without_a_link_is_refused_and_not_copied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let user_dir = dir.path().join("0");
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target).expect("target");
        std::fs::write(target.join("Configuration.xml"), "<x/>").expect("marker");

        let error = expose_output_with(&user_dir, "target/run", &target, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .expect_err("no link, no output directory");

        assert!(matches!(error, AppError::EnvironmentUnavailable(_)));
        assert!(!user_dir.join("target/run").exists());
    }

    /// То, что агент пишет в выставленный каталог, оказывается в цели, а снятие ссылки цель
    /// не трогает.
    #[test]
    fn a_write_through_the_exposed_output_directory_lands_in_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let user_dir = dir.path().join("0");
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target).expect("target");

        let relative = expose_output_dir(&user_dir, "target/run", &target).expect("expose");
        std::fs::write(user_dir.join(&relative).join("Module.bsl"), "new").expect("write");
        withdraw_dir(&user_dir, &relative);

        assert_eq!(
            std::fs::read_to_string(target.join("Module.bsl")).expect("in the target"),
            "new"
        );
        assert!(!user_dir.join(&relative).exists());
    }

    /// Точка соединения не требует права на символические ссылки: это путь по умолчанию на
    /// Windows обычного пользователя.
    #[cfg(windows)]
    #[test]
    fn a_junction_carries_writes_and_its_removal_keeps_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target).expect("target");
        let target = target.canonicalize().expect("canonical");
        let link = dir.path().join("link");

        junction_dir(&target, &link).expect("junction");
        std::fs::write(link.join("Module.bsl"), "new").expect("write");
        crate::support::fs::remove_path_if_exists(&link).expect("remove");

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(target.join("Module.bsl").is_file());
    }

    /// На сетевом пути точка соединения создалась бы, но не открывалась: это отказ, и для чтения
    /// остаётся копия.
    #[cfg(windows)]
    #[test]
    fn a_junction_is_refused_for_a_target_off_the_local_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(is_on_a_local_disk(dir.path()).expect("local"));
        assert!(!is_on_a_local_disk(std::path::Path::new(r"\\localhost\C$")).unwrap_or(false));
    }

    /// Копия корня в каталог агента внутри него же заканчивается, не несёт `workPath` и
    /// служебных каталогов, а форму с именем служебного каталога сохраняет.
    #[test]
    fn a_root_copied_for_the_agent_leaves_the_agent_directory_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        let root = root.canonicalize().expect("canonical root");
        let work = root.join("service-area");
        let local = root.join("personal");
        let user_dir = work.join("agent").join("0");
        std::fs::create_dir_all(&user_dir).expect("agent dir");
        std::fs::write(work.join("state.txt"), "state").expect("state");
        std::fs::create_dir_all(local.join("db")).expect("local");
        std::fs::write(local.join("db").join("1Cv8.1CD"), "base").expect("personal base");
        std::fs::create_dir_all(root.join(".build")).expect("service dir");
        std::fs::write(root.join(".build").join("lock"), "lock").expect("lock");
        let form = root
            .join("Catalogs")
            .join("Item")
            .join("Forms")
            .join("build");
        std::fs::create_dir_all(&form).expect("form");
        std::fs::write(form.join("Form.xml"), "<x/>").expect("form file");
        std::fs::write(root.join("Configuration.xml"), "<x/>").expect("marker");
        let link = user_dir.join("run").join("main");
        std::fs::create_dir_all(link.parent().expect("parent")).expect("parent");

        copy_for_the_agent(
            &root,
            &user_dir,
            RunnerDirs::new(&work, Some(&local)),
            &link,
        )
        .expect("copy");

        assert!(link.join("Configuration.xml").is_file());
        assert!(link.join("Catalogs/Item/Forms/build/Form.xml").is_file());
        assert!(!link.join("service-area").exists());
        assert!(!link.join("personal").exists());
        assert!(!link.join(".build").exists());
    }

    #[test]
    fn a_symlinked_dir_is_exposed_under_the_user_dir_and_withdrawn_as_a_link() {
        let root = tempfile::tempdir().expect("tempdir");
        let user_dir = root.path().join("0");
        let sources = root.path().join("src");
        std::fs::create_dir_all(&sources).expect("sources");
        std::fs::write(sources.join("Configuration.xml"), "<x/>").expect("marker");

        let relative = expose_dir(
            &user_dir,
            "build/run/main",
            &sources,
            RunnerDirs::new(root.path(), None),
        )
        .expect("expose");
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
            .with_infobase_memory(&root.join(".local"), "origin", identity.to_owned());
        GenerationLedger::of(&context).expect("a remembered base")
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
            .join(".local/infobases/origin/generation.json")
            .is_file());
    }

    /// Признак «не применено» пишется только тогда, когда он есть: прежняя запись без поля
    /// читается применённой, и прежний раннер, не знающий поля, читает новую запись как
    /// обычную запись отправки (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`).
    #[test]
    fn a_record_without_the_applied_mark_reads_as_applied() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a");
        main.record(Provider::Designer, "abc", GenerationAfter::Build)
            .expect("record");
        let text =
            std::fs::read_to_string(root.path().join(".local/infobases/origin/generation.json"))
                .expect("ledger");
        assert!(!text.contains("applied"), "{text}");
        assert!(recorded(&main).applied);

        main.record_as(
            Provider::Designer,
            "def",
            GenerationAfter::Build,
            ApplyMark::Unapplied,
        )
        .expect("record");
        let text =
            std::fs::read_to_string(root.path().join(".local/infobases/origin/generation.json"))
                .expect("ledger");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(value["main"]["applied"], false, "{text}");
        assert_eq!(value["main"]["after"], "build", "{text}");
        assert!(!recorded(&main).applied);
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
            record.compare(Provider::Agent, "abc", TokenScope::OwnTool),
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
            recorded(&main).compare(Provider::Ibcmd, "def", TokenScope::OwnTool),
            GenerationComparison::Changed
        );
    }

    /// Токен другого инструмента там, где токены сравниваются только внутри своего, —
    /// отсутствие ответа: ни совпадения при равных значениях, ни расхождения при разных.
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
                record.compare(tool, token, TokenScope::OwnTool),
                GenerationComparison::NoAnswer,
                "{tool} {token}"
            );
        }
    }

    /// Основная конфигурация файловой базы (#559): запись любого из агента, Конфигуратора и
    /// `ibcmd` сравнивается с токеном, прочитанным любым другим из них, — и совпадением, и
    /// расхождением. Инструмент без замера и там сравним только с собой.
    #[test]
    fn a_token_of_another_measured_tool_is_compared_where_tokens_are_shared() {
        let root = tempfile::tempdir().expect("tempdir");
        let main = ledger(root.path(), "main", "base-a");
        for recorded_by in [Provider::Agent, Provider::Designer, Provider::Ibcmd] {
            main.record(recorded_by, "abc", GenerationAfter::Build)
                .expect("record");
            let record = recorded(&main);
            for read_by in [Provider::Agent, Provider::Designer, Provider::Ibcmd] {
                assert_eq!(
                    record.compare(read_by, "abc", TokenScope::AnyTool),
                    GenerationComparison::Unchanged,
                    "{recorded_by} then {read_by}"
                );
                assert_eq!(
                    record.compare(read_by, "def", TokenScope::AnyTool),
                    GenerationComparison::Changed,
                    "{recorded_by} then {read_by}"
                );
            }
            assert_eq!(
                record.compare(Provider::IbcmdRs, "abc", TokenScope::AnyTool),
                GenerationComparison::NoAnswer
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
                recorded(&main).compare(Provider::Ibcmd, token, TokenScope::OwnTool),
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
        let file = root.path().join(".local/infobases/origin/generation.json");
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
        let file = root.path().join(".local/infobases/origin/generation.json");
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
        assert!(GenerationLedger::of(&unbound).is_none());
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
