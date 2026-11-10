//! Живой агент Конфигуратора по запросу (#577): `agent hold` ставит держателя на живого агента
//! цели проекта и поднимает его, если его нет; `agent release` снимает отметку держателя;
//! `agent stop` снимает живых агентов; `agent status` называет живых агентов учётной записи.
//!
//! Держатель — процесс или срок (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`); повторный
//! `hold` того же держателя второй отметки не ставит
//! (`INV.CLI.AN-AGENT-HOLD-IS-IDEMPOTENT-PER-HOLDER`); сверх предела, с `live: false` и в Job без
//! права выхода `hold` отказывает своим кодом и платформу не запускает
//! (`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`).

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::model::{AppConfig, DesignerAgentMode};
use crate::domain::capability::LiveAgentHolder;
use crate::domain::live_agent::{
    AgentHoldResult, AgentReleaseResult, AgentStatusResult, AgentStopRefusal, AgentStopResult,
    LiveAgentEntry,
};
use crate::platform::agent_keeper::client::{
    live_keepers, KeeperClient, KeeperStartError, LiveKeeper, Stopped,
};
use crate::platform::agent_keeper::protocol::{Holder, KeeperStatus};
use crate::platform::locator::UtilityType;
use crate::platform::service::{process_facts, started_at};
use crate::platform::service_record::ServiceRecord;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{existing_live_key, holder_receipt, live_target};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Имя держателя со сроком: у `agent hold --ttl` держатель один на агента, и повторный `hold`
/// продлевает его срок.
pub const TTL_HOLDER: &str = "agent-hold";

/// Расхождение времени старта процесса, которое ещё не отказ: ОС считает его точнее секунды, а
/// RFC 3339 вызывающего — до секунды.
const STARTED_AT_TOLERANCE: Duration = Duration::from_secs(1);

/// Держатель `agent hold`, как его назвала командная строка.
#[derive(Debug, Clone)]
pub enum HolderRequest {
    Process {
        pid: u32,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    Ttl {
        seconds: u64,
    },
}

/// Область `agent stop` и `agent status`: агент цели проекта (из проекта берётся только ключ)
/// или все живые агенты учётной записи ОС (`--all`, проект не нужен).
#[derive(Debug, Clone, Copy)]
pub enum AgentScope<'a> {
    Project(&'a AppConfig),
    Account,
}

/// Чью отметку снимает `agent release`: процесса или держателя `agent hold --ttl`.
#[derive(Debug, Clone)]
pub enum ReleaseRequest {
    Process {
        pid: u32,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    Ttl,
}

/// `agent hold`.
pub fn hold(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &HolderRequest,
) -> Result<AgentHoldResult, AppError> {
    let started = Instant::now();
    let port = managed_port(config)?;
    let agent = &config.tools.designer_agent;
    if !agent.live_enabled() {
        return Err(refused(
            UseCaseErrorKind::LiveAgentOff,
            "tools.designer_agent.live is false: the runner keeps no live agent, so agent hold starts nothing; remove the key to hold one".to_owned(),
        ));
    }
    let holder = holder(request)?;
    let v8 = locate_v8(config)?;
    let target = live_target(config, &v8, port)?;
    let (client, holders) = match KeeperClient::start_and_hold(
        &target.key,
        &target.launch,
        &target.credentials,
        agent.live_limit(),
        &context.cancellation(),
        &holder,
    ) {
        Ok(taken) => taken,
        Err(KeeperStartError::Job) => {
            return Err(refused(
                UseCaseErrorKind::LiveAgentJob,
                "the runner runs in a Job that kills its processes on close and forbids leaving it: a live agent would die with the Job, so agent hold starts nothing".to_owned(),
            ))
        }
        Err(KeeperStartError::Limit { limit }) => {
            return Err(refused(
                UseCaseErrorKind::AgentLimit,
                format!(
                    "the OS account already keeps {limit} live agents (tools.designer_agent.live-max), and none of them is idle without holders: {}; release or stop one of them",
                    describe_live_agents()
                ),
            ))
        }
        Err(error @ (KeeperStartError::Busy { .. } | KeeperStartError::Unavailable { .. })) => {
            return Err(AppError::EnvironmentUnavailable(crate::support::error::chain(&error)))
        }
        Err(KeeperStartError::Agent(error)) => return Err(AppError::from(error)),
    };
    Ok(AgentHoldResult {
        key: client.key().to_owned(),
        reused: client.reused(),
        holders: holders.iter().map(holder_receipt).collect(),
        keeper_pid: client.keeper_pid(),
        agent_pid: client.agent_pid(),
        duration_ms: elapsed_ms(started),
    })
}

/// `agent release`: снимает отметку держателя с живого агента цели проекта. Отметки нет или
/// агента нет — успех без изменений.
pub fn release(
    _context: &ExecutionContext,
    config: &AppConfig,
    request: &ReleaseRequest,
) -> Result<AgentReleaseResult, AppError> {
    let started = Instant::now();
    let nothing = |key: Option<String>| AgentReleaseResult {
        key,
        released: false,
        holders: Vec::new(),
        duration_ms: elapsed_ms(started),
    };
    let Some((state, key)) = own_key(config)? else {
        return Ok(nothing(None));
    };
    let Some(client) = KeeperClient::attach(&state, &key).map_err(keeper_io)? else {
        return Ok(nothing(None));
    };
    let status = client.status().map_err(AppError::from)?;
    let marks: Vec<Holder> = status
        .holders
        .iter()
        .filter(|held| matches(request, held))
        .cloned()
        .collect();
    let mut holders = status.holders;
    for mark in &marks {
        holders = client.release(mark.clone()).map_err(AppError::from)?;
    }
    Ok(AgentReleaseResult {
        key: Some(key),
        released: !marks.is_empty(),
        holders: holders.iter().map(holder_receipt).collect(),
        duration_ms: elapsed_ms(started),
    })
}

/// `agent stop`: снимает живого агента цели проекта, с `all` — всех живых агентов учётной
/// записи. Агента, который выполняет критическую команду, сторож снимает после неё: не дождались
/// — агент назван с `closing: true`. Отмена команды кончает ожидание, а не сам `stop`. С `all`
/// отказ одного сторожа не останавливает обход: он назван в `refused`.
pub fn stop(
    context: &ExecutionContext,
    scope: AgentScope<'_>,
) -> Result<AgentStopResult, AppError> {
    let started = Instant::now();
    let nothing = |started: Instant| AgentStopResult {
        stopped: Vec::new(),
        refused: Vec::new(),
        duration_ms: elapsed_ms(started),
    };
    let Some(state) = crate::support::machine::account::existing_account_state_dir() else {
        return Ok(nothing(started));
    };
    let all = matches!(scope, AgentScope::Account);
    let own = match scope {
        AgentScope::Account => None,
        AgentScope::Project(config) => match own_key(config)? {
            Some((_, key)) => Some(key),
            None => return Ok(nothing(started)),
        },
    };
    let cancel = context.cancellation();
    let mut stopped = Vec::new();
    let mut refused = Vec::new();
    for (keeper, record) in live_keepers(&state) {
        if own.as_ref().is_some_and(|own| *own != keeper.key) {
            continue;
        }
        let status = keeper.client.status().ok();
        let mut found = entry(&keeper, &record, status.as_ref(), own.as_deref());
        match keeper.client.stop(&cancel) {
            Ok(Stopped::Removed | Stopped::InUse { .. }) => stopped.push(found),
            Ok(Stopped::Pending) => {
                found.closing = true;
                stopped.push(found);
            }
            Err(error) if all => refused.push(AgentStopRefusal {
                key: keeper.key.clone(),
                message: error.to_string(),
            }),
            Err(error) => {
                return Err(AppError::EnvironmentUnavailable(format!(
                    "the live agent {} was not stopped: {error}",
                    keeper.key
                )))
            }
        }
    }
    Ok(AgentStopResult {
        stopped,
        refused,
        duration_ms: elapsed_ms(started),
    })
}

/// `agent status`: живой агент цели проекта или, с `--all`, все живые агенты учётной записи.
/// Замка рабочего каталога не берёт и ничего не поднимает; сторож, который не ответил, назван
/// без состояния.
pub fn status(
    _context: &ExecutionContext,
    scope: AgentScope<'_>,
) -> Result<AgentStatusResult, AppError> {
    let started = Instant::now();
    let Some(state) = crate::support::machine::account::existing_account_state_dir() else {
        return Ok(AgentStatusResult {
            agents: Vec::new(),
            duration_ms: elapsed_ms(started),
        });
    };
    let own = match scope {
        AgentScope::Account => None,
        AgentScope::Project(config) => match own_key(config)? {
            Some((_, key)) => Some(key),
            None => {
                return Ok(AgentStatusResult {
                    agents: Vec::new(),
                    duration_ms: elapsed_ms(started),
                })
            }
        },
    };
    let agents = live_keepers(&state)
        .into_iter()
        .filter(|(keeper, _)| own.as_ref().is_none_or(|own| *own == keeper.key))
        .map(|(keeper, record)| {
            let status = match keeper.client.status() {
                Ok(status) => Some(status),
                Err(error) => {
                    tracing::warn!(key = %keeper.key, %error, "a live agent did not answer its status");
                    None
                }
            };
            entry(&keeper, &record, status.as_ref(), own.as_deref())
        })
        .collect();
    Ok(AgentStatusResult {
        agents,
        duration_ms: elapsed_ms(started),
    })
}

/// Порт управляемого агента; подключённому агенту и шлюзу автономного сервера живого агента
/// раннер не держит.
fn managed_port(config: &AppConfig) -> Result<Option<u16>, AppError> {
    if config.infobase.standalone.is_some() {
        return Err(AppError::capability(
            "a standalone server is reached through its own SSH gate: the runner keeps no live agent for it",
        ));
    }
    match config
        .tools
        .designer_agent
        .mode()
        .map_err(AppError::Validation)?
    {
        DesignerAgentMode::Attached { .. } => Err(AppError::capability(
            "tools.designer_agent.attach names an agent the runner did not start: the runner keeps no live agent for it",
        )),
        DesignerAgentMode::Managed { port } => Ok(port),
    }
}

fn locate_v8(config: &AppConfig) -> Result<PathBuf, AppError> {
    PlatformUtilities::from_config(config)
        .locate(UtilityType::V8)
        .map(|location| location.path)
        .map_err(AppError::from)
}

/// Каталог состояния и ключ живого агента цели проекта, если они уже есть: ничего не создаёт.
fn own_key(config: &AppConfig) -> Result<Option<(PathBuf, String)>, AppError> {
    managed_port(config)?;
    let Some(state) = crate::support::machine::account::existing_account_state_dir() else {
        return Ok(None);
    };
    let v8 = locate_v8(config)?;
    let key = existing_live_key(&state, config, &v8)
        .map_err(|error| AppError::Runtime(format!("cannot key the live agent: {error}")))?;
    Ok(key.map(|key| (state, key)))
}

/// Держатель из запроса. Процесс должен быть жив и своей учётной записи ОС, а названное время
/// старта — совпадать с тем, что называет ОС, с точностью до секунды.
fn holder(request: &HolderRequest) -> Result<Holder, AppError> {
    match request {
        HolderRequest::Process {
            pid,
            started_at: expected,
        } => {
            let facts = process_facts(*pid).ok_or_else(|| {
                AppError::Validation(format!(
                    "process {pid} is not running: it cannot hold the live agent"
                ))
            })?;
            match crate::support::machine::account::runs_as_this_account(*pid) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(AppError::Validation(format!(
                        "process {pid} runs as another OS account: it cannot hold the live agent of this one"
                    )))
                }
                Err(error) => {
                    return Err(AppError::Validation(format!(
                        "the OS account of process {pid} is not known: {error}"
                    )))
                }
            }
            if let Some(expected) = expected {
                let actual = started_at(facts.started).ok_or_else(|| {
                    AppError::Validation(format!(
                        "the start time of process {pid} is not known to the runner: hold it without --holder-started-at"
                    ))
                })?;
                let expected = SystemTime::from(*expected);
                let apart = actual
                    .duration_since(expected)
                    .or_else(|_| expected.duration_since(actual))
                    .unwrap_or_default();
                if apart > STARTED_AT_TOLERANCE {
                    return Err(AppError::Validation(format!(
                        "process {pid} started at {}, not at {}: the pid now belongs to another process",
                        rfc3339(actual),
                        rfc3339(expected)
                    )));
                }
            }
            Ok(Holder::Process {
                pid: *pid,
                started: facts.started,
            })
        }
        HolderRequest::Ttl { seconds } => Ok(Holder::Ttl {
            id: TTL_HOLDER.to_owned(),
            until_ms: now_ms().saturating_add(seconds.saturating_mul(1000)),
        }),
    }
}

/// Отметка держателя из запроса: процесс — по pid и, если названо, по времени старта; срок —
/// держатель `agent hold --ttl`.
fn matches(request: &ReleaseRequest, held: &Holder) -> bool {
    match (request, held) {
        (
            ReleaseRequest::Process {
                pid,
                started_at: expected,
            },
            Holder::Process {
                pid: held_pid,
                started,
            },
        ) => {
            pid == held_pid
                && expected.is_none_or(|expected| {
                    started_at(*started).is_none_or(|actual| {
                        let expected = SystemTime::from(expected);
                        actual
                            .duration_since(expected)
                            .or_else(|_| expected.duration_since(actual))
                            .unwrap_or_default()
                            <= STARTED_AT_TOLERANCE
                    })
                })
        }
        (ReleaseRequest::Ttl, Holder::Ttl { id, .. }) => id == TTL_HOLDER,
        _ => false,
    }
}

fn entry(
    keeper: &LiveKeeper,
    record: &ServiceRecord,
    status: Option<&KeeperStatus>,
    own: Option<&str>,
) -> LiveAgentEntry {
    LiveAgentEntry {
        key: keeper.key.clone(),
        infobase: record.agent.as_ref().map(|agent| agent.infobase.clone()),
        this_project: own == Some(keeper.key.as_str()),
        keeper_pid: status.map_or(record.pid.unwrap_or_default(), |status| status.keeper_pid),
        agent_pid: status
            .and_then(|status| status.agent_pid)
            .or_else(|| record.agent.as_ref().and_then(|agent| agent.pid)),
        holders: status
            .map(|status| status.holders.iter().map(holder_receipt).collect())
            .unwrap_or_default(),
        busy: status.and_then(|status| status.command().map(str::to_owned)),
        connected: status.is_some_and(|status| status.connected),
        idle_ms: status.map_or(0, |status| status.idle_ms),
        closing: status.is_some_and(|status| status.closing),
    }
}

/// Живые агенты учётной записи с держателями — для отказа `agent_limit`.
fn describe_live_agents() -> String {
    let Some(state) = crate::support::machine::account::existing_account_state_dir() else {
        return "none answered".to_owned();
    };
    let described: Vec<String> = live_keepers(&state)
        .into_iter()
        .map(|(keeper, record)| {
            let infobase = record
                .agent
                .as_ref()
                .map_or("an unknown infobase", |agent| agent.infobase.as_str())
                .to_owned();
            match keeper.client.status() {
                Ok(status) if status.holders.is_empty() => format!(
                    "{} on {infobase} running '{}'",
                    keeper.key,
                    status.command().unwrap_or("nothing")
                ),
                Ok(status) => format!(
                    "{} on {infobase} held by {}",
                    keeper.key,
                    LiveAgentHolder::list_text(
                        &status
                            .holders
                            .iter()
                            .map(holder_receipt)
                            .collect::<Vec<_>>()
                    )
                ),
                Err(_) => format!("{} on {infobase}, not answering", keeper.key),
            }
        })
        .collect();
    if described.is_empty() {
        "none answered".to_owned()
    } else {
        described.join("; ")
    }
}

fn refused(kind: UseCaseErrorKind, message: String) -> AppError {
    AppError::Refused(Box::new(UseCaseError::new(kind, message)))
}

fn keeper_io(error: std::io::Error) -> AppError {
    AppError::EnvironmentUnavailable(format!("the live agent registry cannot be read: {error}"))
}

fn rfc3339(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
