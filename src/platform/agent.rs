//! Агентский shell Конфигуратора по SSH из процесса раннера.
//!
//! SSH-клиент встроен (`russh`): сессия не зависит от внешнего `ssh`, его версии и
//! способа передать пароль. Раннер открывает канал без псевдотерминала, первой
//! командой переводит агент в JSON без приглашения и дальше пишет команды по одной;
//! каждый ответ — один JSON-массив, и его границу даёт сам разбор.
//!
//! Решение по ответу принимается по `type` и закрытому множеству `error-type`;
//! `message` переносится как улика. Отказы до первого ответа типизирует библиотека:
//! соединение, рукопожатие, аутентификация и канал различимы без чтения прозы.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::platform::process::{
    ProcessExecutionPolicy, ProcessInterruptionReason, ProcessInterruptionSafety, WorkGiven,
    CRITICAL_INTERRUPTION_DEFERRED,
};

use russh::client;
use russh::keys::ssh_key::{Fingerprint, HashAlg};
use russh::ChannelMsg;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::platform::process::{ManagedSpawnMode, ProcessRequest, ProcessRunner};
use crate::platform::sftp::{self, SftpClient, SftpError};
use crate::support::authority::Host;

/// Первая команда любой сессии: без неё ответы — проза с приглашением.
pub const JSON_MODE_COMMAND: &str = "options set --show-prompt=no --output-format=json";
/// Вторая команда сессии: без подключения к базе любая команда `config` отвечает
/// `DesignerNotConnectedToInfoBase` (замер 15.09.2026).
pub const CONNECT_COMMAND: &str = "common connect-ib";
/// Закрытие соединения с базой без завершения точки входа.
pub const DISCONNECT_COMMAND: &str = "common disconnect-ib";
/// Адрес, который слушает управляемый агент: он живёт на машине раннера.
pub const MANAGED_LISTEN_HOST: std::net::IpAddr =
    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
/// Файл карты пользовательских каталогов в `AgentBaseDir`.
pub const BASE_DIR_MAP_FILE: &str = "agentbasedir.json";
const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const CLEANUP_GRACE: Duration = Duration::from_secs(30);
/// Сколько управляемый агент ждут ответить на `common disconnect-ib`: команда отвечает за
/// 40–190 мс (замер 07.11.2026); дольше агент её уже не выполнит, и процесс снимается.
const RELEASE_GRACE: Duration = Duration::from_secs(5);
const WAIT_SLICE: Duration = Duration::from_millis(200);
/// Строка журнала: отмена бросила ответ команды, которую не обязаны дожидаться.
pub(crate) const AGENT_COMMAND_ABANDONED: &str =
    "agent command abandoned: the command was cancelled";
/// Строка журнала первого сигнала у чужого агента: раннер ждёт итог и говорит, как перестать.
const EXTERNAL_AGENT_WAITING_FOR_TERMINAL: &str =
    "interruption requested after sending a command to an external agent; waiting for terminal reply; \
     interrupt again to stop waiting (the agent may keep running the command)";
/// Строка журнала второго сигнала: ожидание итога чужого агента брошено.
const EXTERNAL_AGENT_COMMAND_ABANDONED: &str =
    "external agent command abandoned: the runner stopped waiting for its terminal reply";

/// Интервал keepalive: даёт каналу трафик, на котором TCP способен заметить мёртвый шлюз.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// Тип сообщения агента по документации (Приложение 4, 4.7.8) плюс два, которых в ней
/// нет (живые ответы 8.3.27 от 15.09.2026): `extension-properties` — ответ агента на
/// `extensions properties get --extension=`, итоговый (после него `success` не
/// приходит); `generation-id` — промежуточное уведомление шлюза автономного сервера
/// внутри ответа на `update-db-cfg` (новый токен), итог команды приходит после него.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMessageType {
    Log,
    Success,
    Error,
    Canceled,
    Question,
    Dbstru,
    LoadingIssue,
    Progress,
    ExtensionInfo,
    ExtensionProperties,
    GenerationId,
}

/// Закрытое множество `error-type`; всё вне его сохраняется дословно, а не теряется.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentErrorType {
    UnknownError,
    DesignerNotConnectedToInfoBase,
    DesignerAlreadyConnectedToInfoBase,
    CommandFormatError,
    DbRestructInfo,
    InfoBaseNotFound,
    AdministrationAccessRightRequired,
    ConfigFilesError,
    DesignerAlreadyStarted,
    InfoBaseExclusiveLockRequired,
    LanguageNotFound,
    ExtensionWithDataIsActive,
    ExtensionNotFound,
    Other(String),
}

impl AgentErrorType {
    pub fn parse(value: &str) -> Self {
        match value {
            "UnknownError" => Self::UnknownError,
            "DesignerNotConnectedToInfoBase" => Self::DesignerNotConnectedToInfoBase,
            "DesignerAlreadyConnectedToInfoBase" => Self::DesignerAlreadyConnectedToInfoBase,
            "CommandFormatError" => Self::CommandFormatError,
            "DBRestructInfo" => Self::DbRestructInfo,
            "InfoBaseNotFound" => Self::InfoBaseNotFound,
            "AdministrationAccessRightRequired" => Self::AdministrationAccessRightRequired,
            "ConfigFilesError" => Self::ConfigFilesError,
            "DesignerAlreadyStarted" => Self::DesignerAlreadyStarted,
            "InfoBaseExclusiveLockRequired" => Self::InfoBaseExclusiveLockRequired,
            "LanguageNotFound" => Self::LanguageNotFound,
            "ExtensionWithDataIsActive" => Self::ExtensionWithDataIsActive,
            "ExtensionNotFound" => Self::ExtensionNotFound,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::UnknownError => "UnknownError",
            Self::DesignerNotConnectedToInfoBase => "DesignerNotConnectedToInfoBase",
            Self::DesignerAlreadyConnectedToInfoBase => "DesignerAlreadyConnectedToInfoBase",
            Self::CommandFormatError => "CommandFormatError",
            Self::DbRestructInfo => "DBRestructInfo",
            Self::InfoBaseNotFound => "InfoBaseNotFound",
            Self::AdministrationAccessRightRequired => "AdministrationAccessRightRequired",
            Self::ConfigFilesError => "ConfigFilesError",
            Self::DesignerAlreadyStarted => "DesignerAlreadyStarted",
            Self::InfoBaseExclusiveLockRequired => "InfoBaseExclusiveLockRequired",
            Self::LanguageNotFound => "LanguageNotFound",
            Self::ExtensionWithDataIsActive => "ExtensionWithDataIsActive",
            Self::ExtensionNotFound => "ExtensionNotFound",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl<'de> Deserialize<'de> for AgentErrorType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|value| Self::parse(&value))
    }
}

impl Serialize for AgentErrorType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Одно сообщение из массива ответа.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentMessage {
    #[serde(rename = "type")]
    pub kind: AgentMessageType,
    #[serde(rename = "error-type", default)]
    pub error_type: Option<AgentErrorType>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
}

/// Ответ агента на одну команду: массив сообщений и итог, выведенный из него.
#[derive(Debug, Clone, Default)]
pub struct AgentReply {
    pub messages: Vec<AgentMessage>,
}

impl AgentMessage {
    /// Сообщение, которым команда заканчивается; прогресс и журнал — промежуточные.
    /// `extension-properties` — тоже итог: на `properties get --extension=` агент
    /// отвечает только им, без `success` (живой ответ 15.09.2026).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.kind,
            AgentMessageType::Success
                | AgentMessageType::Error
                | AgentMessageType::Canceled
                | AgentMessageType::Question
                | AgentMessageType::ExtensionProperties
        )
    }
}

impl AgentReply {
    /// Итог команды: успех с телом или типизированный отказ. Прогресс и журнал итогом
    /// не являются; вопрос агента — отказ, раннер на вопросы не отвечает.
    pub fn outcome(&self) -> Result<Option<&serde_json::Value>, AgentError> {
        let terminal = self
            .messages
            .iter()
            .rev()
            .find(|message| message.is_terminal());
        match terminal {
            Some(message)
                if matches!(
                    message.kind,
                    AgentMessageType::Success | AgentMessageType::ExtensionProperties
                ) =>
            {
                Ok(message.body.as_ref())
            }
            Some(message) if message.kind == AgentMessageType::Error => Err(AgentError::Command {
                error_type: message
                    .error_type
                    .clone()
                    .unwrap_or(AgentErrorType::UnknownError),
                message: self.failure_message(message),
            }),
            Some(message) if message.kind == AgentMessageType::Canceled => {
                Err(AgentError::Canceled {
                    message: message.message.clone().unwrap_or_default(),
                })
            }
            Some(message) => Err(AgentError::Question {
                message: message.message.clone().unwrap_or_default(),
            }),
            None => Err(AgentError::NoTerminalMessage {
                count: self.messages.len(),
            }),
        }
    }

    /// Typed causes and nearby file logs have separate shares of the bounded
    /// diagnostic budget; terminal prose cannot be displaced by either.
    fn failure_message(&self, terminal: &AgentMessage) -> String {
        let mut result = excerpt(terminal.message.as_deref().unwrap_or_default(), 2048);
        let end = self
            .messages
            .iter()
            .position(|message| std::ptr::eq(message, terminal))
            .unwrap_or(self.messages.len());
        // Reserve one global truncation marker, including its separator. Each
        // selected message also pays for its separator before consuming text.
        let mut remaining = 6144 - "\n[truncated]".chars().count();
        let mut selected = Vec::new();
        let mut omitted = false;
        for kind in [AgentMessageType::LoadingIssue, AgentMessageType::Log] {
            let mut budget = if kind == AgentMessageType::LoadingIssue {
                remaining.min(4096)
            } else {
                remaining
            };
            for (index, message) in self.messages[..end].iter().enumerate().rev() {
                if message.kind != kind {
                    continue;
                }
                let Some(text) = message.message.as_deref().filter(|text| !text.is_empty()) else {
                    continue;
                };
                if budget <= 1 {
                    omitted = true;
                    continue;
                }
                let mut chars = text.chars();
                let part: String = chars.by_ref().take(budget - 1).collect();
                omitted |= chars.next().is_some();
                let cost = part.chars().count() + 1;
                budget -= cost;
                remaining -= cost;
                selected.push((index, part));
            }
        }
        selected.sort_unstable_by_key(|(index, _)| *index);
        for (_, part) in selected {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&part);
        }
        if omitted {
            result.push_str("\n[truncated]");
        }
        result
    }

    /// Журнал ответа как улика: все `message` по порядку.
    pub fn transcript(&self) -> String {
        self.messages
            .iter()
            .filter_map(|message| message.message.as_deref())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Отказы агентского пути. Каждый различим без чтения прозы.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent at {endpoint} is unreachable: {source}")]
    Unreachable {
        endpoint: String,
        #[source]
        source: std::io::Error,
    },

    #[error("SSH handshake with the agent at {endpoint} failed: {source}")]
    Handshake {
        endpoint: String,
        #[source]
        source: russh::Error,
    },

    #[error("agent at {endpoint} rejected the credentials of user '{user}'")]
    AuthenticationRejected { endpoint: String, user: String },

    #[error(
        "agent at {endpoint} presented {presented} as its host key, but {expected} was expected"
    )]
    HostKeyRejected {
        endpoint: String,
        expected: String,
        presented: String,
    },

    #[error("agent at {endpoint} did not open a shell channel: {source}")]
    Channel {
        endpoint: String,
        #[source]
        source: russh::Error,
    },

    #[error("agent session at {endpoint} failed while sending a command: {source}")]
    Transport {
        endpoint: String,
        #[source]
        source: russh::Error,
    },

    #[error("agent session at {endpoint} ended before a reply; agent said: {stderr}")]
    SessionClosed { endpoint: String, stderr: String },

    #[error("agent did not answer '{command}' within {timeout_ms} ms")]
    TimedOut { command: String, timeout_ms: u64 },

    #[error("agent session cancelled while waiting for '{command}'")]
    Cancelled {
        command: String,
        /// Доставлена ли агенту команда запроса — работа команды. Служебные команды сессии
        /// и ожидание до её открытия работы не несут.
        delivered: bool,
    },

    /// Второй сигнал бросил ожидание итога команды чужого агента: раннер его не дождался, а
    /// агент, возможно, ещё её выполняет. Это не отмена — конец команды не наблюдался.
    #[error(
        "stopped waiting for the agent after a second interrupt: command '{command}' was \
         abandoned; the agent may still be running it and may still hold the infobase"
    )]
    Abandoned { command: String },

    /// Команда запроса, которую после отмены агенту не отдали: работы он не получил, и в
    /// базе она ничего не начинала.
    #[error("agent command '{command}' was not sent: the command was cancelled")]
    NotSent { command: String },

    #[error("agent reply is not a JSON message array: {detail}; head: {head}")]
    InvalidReply { detail: String, head: String },

    #[error("agent reply has {count} messages and no terminal one")]
    NoTerminalMessage { count: usize },

    #[error("agent command failed [{}]: {message}", error_type.as_str())]
    Command {
        error_type: AgentErrorType,
        message: String,
    },

    #[error("agent command was cancelled: {message}")]
    Canceled { message: String },

    #[error("agent asked a question the runner does not answer: {message}")]
    Question { message: String },

    /// Ни один путь для одноразового ключа хоста агент не прочтёт
    /// (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`).
    #[error(
        "the Designer agent reads its host key only from an ASCII path, and none was available \
         (tried: {}); move the project to an ASCII path or declare tools.designer_agent.host-key \
         at one",
        .tried.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    HostKeyPathNotReadable { tried: Vec<PathBuf> },

    #[error("failed to prepare the agent workspace '{path}': {source}")]
    Workspace {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Обмен файлами по SFTP той же точки входа не удался: путь на её стороне и
    /// её ответ дословно.
    #[error("sftp exchange with the agent failed at '{path}': {detail}")]
    Exchange { path: String, detail: String },

    #[error("sftp exchange with the agent failed at '{path}': {detail}")]
    SftpStatus {
        path: String,
        code: u32,
        detail: String,
    },

    /// Точка входа назвала запись каталога именем, непригодным как компонент пути.
    /// Отдельный вид, а не текст внутри `Exchange`: по нему решают, и решать по прозе
    /// нельзя (`DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES`).
    #[error("entry point returned an unusable directory entry name at '{path}': {detail}")]
    UnsafeEntryName { path: String, detail: String },

    #[error("managed agent could not be launched: {0}")]
    Launch(#[source] crate::platform::process::ProcessError),

    /// Порт управляемого агента занят другим процессом: свободный порт раннер выбирает
    /// перед запуском, и между выбором и запуском его мог занять кто-то ещё.
    #[error(
        "port {port} of the managed agent is taken by another process ({detail}): run the command again for another free port, or declare a free tools.designer_agent.port"
    )]
    PortTaken { port: u16, detail: String },

    /// Система не дала свободного порта на адресе управляемого агента.
    #[error(
        "the system gave no free port on {MANAGED_LISTEN_HOST} for the managed agent: {source}; declare tools.designer_agent.port"
    )]
    NoFreePort {
        #[source]
        source: std::io::Error,
    },

    /// На порту управляемого агента ответил SSH-сервер с чужим ключом хоста: это не тот
    /// агент, которого поднял раннер, и сессия к нему не открывается.
    #[error(
        "{endpoint} answered with host key {presented}, not with the key the runner handed to the agent it launched ({expected}): another SSH server holds the port, and the runner does not talk to it"
    )]
    ForeignAgentOnPort {
        endpoint: String,
        expected: String,
        presented: String,
    },

    #[error("managed agent did not accept a session within {timeout_ms} ms; last: {last}")]
    StartupTimedOut { timeout_ms: u64, last: String },

    /// Процесс агента завершился, не приняв ни одной сессии. Код выхода причину не
    /// различает (замер #429: и занятый порт, и нечитаемый ключ — rc=0), поэтому ответ
    /// приводит строку `/Out` агента как есть.
    #[error("managed agent exited ({status}) before accepting a session; /Out: {out}")]
    ExitedBeforeSession { status: String, out: String },

    /// Живого агента сторож снял или снимает: команда поднимает нового
    /// (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`).
    #[error("{detail}; a new live agent is needed")]
    LiveAgentGone { detail: String },

    /// Отказ сторожа живого агента.
    #[error("the live agent keeper failed: {detail}")]
    Keeper { detail: String },

    /// Канала сторожа нет или он не принимает подключение: сторож ещё не поднялся или уже ушёл.
    #[error("the live agent keeper is not reachable at {channel}: {source}")]
    KeeperUnreachable {
        channel: String,
        #[source]
        source: std::io::Error,
    },

    #[error("agent base dir '{base_dir}' has no directory for user '{user}': {detail}")]
    UserDirUnknown {
        base_dir: PathBuf,
        user: String,
        detail: String,
    },
}

/// Точка входа: где слушает агент. Хост типизирован — читает его `support::authority`,
/// и вопрос «это адрес IPv6?» здесь не задаётся заново.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEndpoint {
    pub host: Host,
    pub port: u16,
}

/// `host:port` для журнала и отказа; числовой адрес печатает `SocketAddr`, и скобки у
/// IPv6 ставит он — как их и объявляют.
impl std::fmt::Display for AgentEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.host {
            Host::Address(address) => {
                std::fmt::Display::fmt(&std::net::SocketAddr::new(*address, self.port), f)
            }
            Host::Name(name) => write!(f, "{name}:{}", self.port),
        }
    }
}

/// Как открыть сессию: точка входа и учётные данные. Пустая пара — законные учётные
/// данные базы без пользователей.
#[derive(Debug, Clone)]
pub struct AgentSessionRequest {
    pub endpoint: AgentEndpoint,
    pub user: String,
    pub password: String,
    pub transcript_log: Option<PathBuf>,

    /// Чей ключ считать своим на том конце.
    pub host_key: HostKeyExpectation,
}

/// Ожидание ответа: отмена, класс безопасности и — если он вообще есть — срок.
///
/// С границы команды срок больше не приходит: у команды его нет
/// (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`). Поле остаётся ради очистки, которая
/// заводит свой собственный срок в `cleanup`, и потому остаётся абсолютным: одна команда
/// агента читает канал столько раз, сколько батчей пришлёт агент (`progress`, `progress`,
/// …, `success`), и длительность, отсчитываемая заново на каждом чтении, ограничивала бы
/// батч, а не работу целиком.
#[derive(Debug, Clone)]
pub struct WaitPolicy {
    pub deadline: Option<Instant>,
    pub cancellation: CancellationToken,
    /// Второй сигнал: перестать ждать итог команды чужого агента, уже отменённой первым.
    /// Свой агент и служебные команды его не слушают.
    abandonment: CancellationToken,
    /// Класс безопасности меняет только сама платформа (`critical`, `cleanup`): критическую
    /// политику сценарий берёт у `critical()` там, где отсрочку потом называют
    /// (`agent_session::run_critical`, команды внутри `with_session`).
    safety: ProcessInterruptionSafety,
    /// Куда отметить, что команда запроса отправлена агенту. Служебные команды сессии —
    /// подключение к базе, закрытие — идут без отметки, и снимает её сама платформа
    /// (`without_work`): снаружи поле не видно.
    work: Option<WorkGiven>,
}

impl WaitPolicy {
    /// Ожидание команды запроса по политике шага команды: отмена, класс безопасности и
    /// отметка работы — оттуда, срок — если он у шага есть.
    pub fn from_step(policy: ProcessExecutionPolicy) -> Self {
        Self {
            deadline: policy.timeout.map(|timeout| Instant::now() + timeout),
            cancellation: policy.cancellation,
            abandonment: CancellationToken::new(),
            safety: policy.safety,
            work: policy.work,
        }
    }

    /// Та же политика со вторым сигналом команды: по нему ожидание итога чужого агента
    /// бросается.
    pub fn with_abandonment(self, abandonment: CancellationToken) -> Self {
        Self {
            abandonment,
            ..self
        }
    }

    /// Та же политика, урезанная до срока очистки. Это единственное место, где у
    /// агентского ожидания срок вообще появляется: сама работа идёт без него, а вот
    /// завершение обязано закончиться — иначе прерванная сессия висела бы вечно.
    /// Берётся меньшее из запаса на завершение и уже стоящего срока, если тот есть.
    pub fn cleanup(&self) -> Self {
        self.cleanup_within(CLEANUP_GRACE)
    }

    /// Срок отпускания своего агента, который отмена команды не обрывает: агент, снятый
    /// посреди `disconnect-ib`, мог бы держать базу дольше, чем стоят эти секунды.
    fn release_window(grace: Duration) -> Self {
        Self {
            deadline: Some(Instant::now() + grace),
            cancellation: CancellationToken::new(),
            abandonment: CancellationToken::new(),
            safety: ProcessInterruptionSafety::Interruptible,
            work: None,
        }
    }

    fn cleanup_within(&self, grace: Duration) -> Self {
        let grace = Instant::now() + grace;
        Self {
            deadline: Some(self.deadline.map_or(grace, |deadline| deadline.min(grace))),
            // Очистка не наследует критический класс: иначе она перестала бы слушать
            // собственный срок и завершение могло бы не закончиться никогда.
            safety: ProcessInterruptionSafety::Interruptible,
            ..self.clone()
        }
    }

    /// Отказ ждать дальше, если пришла отмена: сессии ещё нет, и работы команда не дала.
    pub fn refuse_if_cancelled(&self, command: &str) -> Result<(), AgentError> {
        if self.cancellation.is_cancelled() {
            return Err(AgentError::Cancelled {
                command: command.to_owned(),
                delivered: false,
            });
        }
        Ok(())
    }

    /// Та же политика для служебной команды сессии: работы команды она не отмечает.
    fn without_work(&self) -> Self {
        Self {
            work: None,
            ..self.clone()
        }
    }

    /// Ожидание команды, которую сторож живого агента выполняет по запросу клиента: без срока,
    /// отменяет его сам сторож, когда клиент ушёл посреди некритической команды. Работы команды
    /// оно не отмечает: её отмечает клиент в своём процессе.
    pub(in crate::platform) fn for_keeper(cancellation: CancellationToken, critical: bool) -> Self {
        Self {
            deadline: None,
            cancellation,
            abandonment: CancellationToken::new(),
            safety: if critical {
                ProcessInterruptionSafety::CriticalNonAbortable
            } else {
                ProcessInterruptionSafety::Interruptible
            },
            work: None,
        }
    }

    /// Ожидание `common disconnect-ib`, которым сторож отключает живого агента от базы после
    /// сессии команды: тот же короткий срок, что у отпускания своего агента.
    pub(in crate::platform) fn for_keeper_release() -> Self {
        Self::release_window(RELEASE_GRACE)
    }

    /// Объявлена ли фаза критической: её команду доводят до исхода.
    pub fn is_critical(&self) -> bool {
        matches!(self.safety, ProcessInterruptionSafety::CriticalNonAbortable)
    }

    /// Отмечает работу команды, если отметка есть: команда ушла агенту через сторожа.
    pub fn mark_work_given(&self) {
        if let Some(work) = &self.work {
            work.mark_work_given();
        }
    }

    /// Та же политика со сроком и отменой, но фаза объявлена критической: команда,
    /// меняющая информационную базу, доводится до исхода, а прерывание записывается
    /// и отдаётся вызывающему отложенным предупреждением.
    pub fn critical(&self) -> Self {
        Self {
            safety: ProcessInterruptionSafety::CriticalNonAbortable,
            ..self.clone()
        }
    }
}

/// Только для тестов: в работе политику ожидания строит контекст команды.
#[cfg(test)]
impl Default for WaitPolicy {
    fn default() -> Self {
        Self {
            deadline: None,
            cancellation: CancellationToken::new(),
            abandonment: CancellationToken::new(),
            safety: ProcessInterruptionSafety::Interruptible,
            work: Some(WorkGiven::for_command()),
        }
    }
}

/// Настройка SSH-клиента.
///
/// Keepalive включён, но обрыв по неотвеченным keepalive выключен (`keepalive_max: 0`), и
/// это не половинчатость, а единственная работающая комбинация.
///
/// Обрыв по счётчику уже пробовали и сняли: агент однопоточен и, занятый долгой командой,
/// не шлёт по каналу ничего — три неотвеченных keepalive рвали сессию на 46-й секунде
/// выгрузки УТ (замер 15.09.2026, `DEC.2026-09-14.AGENT-ENDPOINT-IS-MANAGED-OR-ATTACHED`).
/// Клиент `russh` сбрасывает счётчик на любых данных от сервера, так что порог означал бы
/// «сколько агенту позволено молчать»; измерение даёт этой тишине нижнюю границу и не даёт
/// верхней, а выбирать порог по догадке — значит снова рвать живую работу.
///
/// Сами пакеты при этом нужны: без них по каналу в тишине не идёт ничего, и полуоткрытое
/// соединение с мёртвым шлюзом не замечает никто — ни TCP, которому нечем ошибиться, ни
/// раннер, ждущий терминального сообщения в критической фазе. С keepalive запись рано или
/// поздно упирается в таймаут TCP, канал закрывается, и ожидание получает конец.
fn ssh_client_config() -> client::Config {
    client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: 0,
        ..client::Config::default()
    }
}

/// Чего раннер ждёт от ключа хоста на том конце.
#[derive(Debug, Clone, Default)]
pub enum HostKeyExpectation {
    /// Ожидания нет: ключ принимается и называется, чтобы владелец мог его закрепить.
    #[default]
    Unpinned,

    /// Ключ объявлен: принимается только он.
    Pinned(Fingerprint),
}

impl HostKeyExpectation {
    /// Ожидание из отпечатка, объявленного в конфигурации.
    pub fn declared(fingerprint: &str) -> Result<Self, String> {
        Fingerprint::from_str(fingerprint)
            .map(HostKeyExpectation::Pinned)
            .map_err(|_| format!("not an SSH key fingerprint: {fingerprint}"))
    }

    /// Ожидание из того самого файла, который раннер отдал платформе.
    ///
    /// Агент публикует ключ из переданного файла как есть: на 8.3.27.1859 замерен
    /// явный ED25519 (`references/1c/designer-agent/request-surface.md`). Поэтому
    /// открытая часть этого файла и есть то, что предъявит агент.
    ///
    /// Нечитаемый файл ожидания не даёт: ключ мог быть под паролем, которого у раннера
    /// нет, а платформа его спросит. Отказывать здесь значило бы ломать работающий
    /// запуск ради проверки, поэтому такой случай проходит как `Unpinned` — вслух.
    pub fn of_host_key_file(path: &Path) -> Self {
        match russh::keys::load_secret_key(path, None) {
            Ok(key) => HostKeyExpectation::Pinned(key.public_key().fingerprint(HashAlg::Sha256)),
            Err(error) => {
                warn!(
                    path = %path.display(),
                    %error,
                    "the managed agent host key cannot be read, so its identity is not checked"
                );
                HostKeyExpectation::Unpinned
            }
        }
    }
}

/// Обработчик событий SSH-клиента.
///
/// Ключ хоста сверяется здесь и больше нигде: это единственное место, где библиотека
/// спрашивает раннер, тот ли сервер ответил.
struct ClientEvents {
    expectation: HostKeyExpectation,

    /// Куда лечь увиденному ключу. Обработчик уезжает в `connect` по значению, а
    /// разбираться с отказом приходится снаружи — иначе причина осталась бы внутри.
    presented: Arc<Mutex<Option<String>>>,
}

impl client::Handler for ClientEvents {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key,
            // Сертификат не проверяется, а отвергается. Библиотека спрашивает про него
            // *вместо* ключа, поэтому принять сертификат, внутри которого лежит нужный
            // ключ, значило бы обойти закрепление: за сертификатом стоят удостоверяющий
            // центр, срок и principals, которых раннер не смотрит. Агент сертификатов
            // и не предъявляет.
            russh::keys::PublicKeyOrCertificate::Certificate(_) => {
                *self.presented.lock().expect("host key slot") =
                    Some("a host certificate".to_owned());
                return Ok(false);
            }
        };

        match &self.expectation {
            HostKeyExpectation::Unpinned => {
                let presented = key.fingerprint(HashAlg::Sha256);
                *self.presented.lock().expect("host key slot") = Some(presented.to_string());
                warn!(
                    fingerprint = %presented,
                    "the agent host key is not checked because none was declared; \
                     declare this fingerprint to pin it"
                );
                Ok(true)
            }
            HostKeyExpectation::Pinned(expected) => {
                // Считается тем же алгоритмом, каким записано ожидание: отпечатки разных
                // алгоритмов не равны никогда, и `SHA512:` сверялся бы с `SHA256:` вечно
                // не сходясь — то есть объявленный ключ читался бы как подменённый.
                let presented = key.fingerprint(expected.algorithm());
                *self.presented.lock().expect("host key slot") = Some(presented.to_string());
                Ok(*expected == presented)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AgentOwnership {
    Managed,
    External,
}

/// Подключать ли агента к базе, открывая сессию.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AtOpen {
    /// `common connect-ib` сразу после машинного режима.
    Connect,
    /// Без подключения: агент поднят, но базы не держит, пока его не подключат.
    Idle,
}

/// Открытая сессия: соединение, канал shell и накопленный, ещё не разобранный ответ.
pub struct AgentSession {
    runtime: tokio::runtime::Runtime,
    connection: client::Handle<ClientEvents>,
    channel: russh::Channel<client::Msg>,
    /// Подсистема SFTP той же точки входа, открывается при первой надобности.
    sftp: Option<SftpClient<russh::ChannelStream<client::Msg>>>,
    pending: Vec<u8>,
    stderr: Vec<u8>,
    endpoint: AgentEndpoint,
    ownership: AgentOwnership,
    transcript: Option<std::fs::File>,
    known_secrets: Vec<String>,
    ended: bool,
    /// Прерывание, защёлкнутое в критической фазе за время сессии; первое побеждает.
    /// Сессия помнит его, потому что результат платформы собирают после её закрытия.
    deferred_interruption: Option<ProcessInterruptionReason>,
    /// Прерывание, которое отложила последняя команда, — и когда её ответ не дочитан.
    command_deferral: Option<ProcessInterruptionReason>,
    /// Прерывание, пришедшее после отправки команды чужому агенту, и когда его заметили:
    /// снять команду раннер не может, поэтому дочитывает её итог через все батчи ответа.
    /// Сбрасывает его следующая команда.
    awaited_interruption: Option<(ProcessInterruptionReason, Instant)>,
    /// Бросил ли второй сигнал команду этой сессии: агент, возможно, ещё пишет в свой
    /// каталог, и убирать за ним след нельзя.
    abandoned: bool,
    /// Пришёл ли от агента хоть один JSON-массив. До него проза — баннер и приглашение
    /// shell, а не ответ: конец сессии после неё — закрытая сессия, а не неверный ответ.
    json_seen: bool,
}

impl AgentSession {
    /// Открывает сессию и переводит её в машинный режим. Успех означает, что
    /// аутентификация прошла и агент ответил JSON, — этим и доказывается готовность.
    pub fn open(request: &AgentSessionRequest, policy: &WaitPolicy) -> Result<Self, AgentError> {
        Self::open_within(
            request,
            policy,
            AgentOwnership::External,
            None,
            AtOpen::Connect,
        )
    }

    /// То же, но рукопожатие транспорта ограничено `handshake`: слушатель, который принял
    /// соединение и молчит, — не агент, и его ответа ждать нельзя. Истёкший срок отвечает
    /// `Unreachable`, как не принятое соединение.
    fn open_within(
        request: &AgentSessionRequest,
        policy: &WaitPolicy,
        ownership: AgentOwnership,
        handshake: Option<Duration>,
        at_open: AtOpen,
    ) -> Result<Self, AgentError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| AgentError::Workspace {
                path: PathBuf::from("<tokio runtime>"),
                source,
            })?;
        let endpoint = request.endpoint.clone();
        let named = endpoint.to_string();
        let host = endpoint.host.to_string();
        // Имя пользователя базы в журнал не идёт, как и у Конфигуратора (`/N ***`): журнал
        // действий читают те, кому учётных данных базы не давали.
        debug!(endpoint = %named, "opening agent session");

        let expectation = request.host_key.clone();
        let presented: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let (connection, channel) =
            runtime.block_on(async {
                let config = Arc::new(ssh_client_config());
                let connect = client::connect(
                    config,
                    (host.as_str(), endpoint.port),
                    ClientEvents {
                        expectation: expectation.clone(),
                        presented: presented.clone(),
                    },
                );
                let connected = match handshake {
                    Some(limit) => tokio::time::timeout(limit, connect).await.map_err(|_| {
                        AgentError::Unreachable {
                            endpoint: named.clone(),
                            source: std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                format!("no SSH answer within {} ms", limit.as_millis()),
                            ),
                        }
                    })?,
                    None => connect.await,
                };
                let mut connection = connected.map_err(|error| match error {
                    russh::Error::IO(source) => AgentError::Unreachable {
                        endpoint: named.clone(),
                        source,
                    },
                    // Отказ по ключу — не сбой рукопожатия: сервер ответил исправно,
                    // просто это не тот сервер. Причина едет отдельно, потому что через
                    // `Result<bool, _>` обработчика она пройти не может.
                    russh::Error::UnknownKey => AgentError::HostKeyRejected {
                        endpoint: named.clone(),
                        expected: match &expectation {
                            HostKeyExpectation::Pinned(fingerprint) => fingerprint.to_string(),
                            HostKeyExpectation::Unpinned => "any public key".to_owned(),
                        },
                        presented: presented
                            .lock()
                            .expect("host key slot")
                            .clone()
                            .unwrap_or_else(|| "nothing".to_owned()),
                    },
                    source => AgentError::Handshake {
                        endpoint: named.clone(),
                        source,
                    },
                })?;
                let auth = connection
                    .authenticate_password(request.user.clone(), request.password.clone())
                    .await
                    .map_err(|source| AgentError::Handshake {
                        endpoint: named.clone(),
                        source,
                    })?;
                if !auth.success() {
                    return Err(AgentError::AuthenticationRejected {
                        endpoint: named.clone(),
                        user: request.user.clone(),
                    });
                }
                let channel = connection.channel_open_session().await.map_err(|source| {
                    AgentError::Channel {
                        endpoint: named.clone(),
                        source,
                    }
                })?;
                // Без псевдотерминала: с ним агент рвёт сессию (замер 13.09.2026).
                channel
                    .request_shell(true)
                    .await
                    .map_err(|source| AgentError::Channel {
                        endpoint: named.clone(),
                        source,
                    })?;
                Ok((connection, channel))
            })?;

        let transcript = match request.transcript_log.as_ref() {
            Some(path) => Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|source| AgentError::Workspace {
                        path: path.clone(),
                        source,
                    })?,
            ),
            None => None,
        };

        let mut session = Self {
            runtime,
            connection,
            channel,
            sftp: None,
            pending: Vec::new(),
            stderr: Vec::new(),
            endpoint,
            ownership,
            transcript,
            known_secrets: [&request.user, &request.password]
                .into_iter()
                .filter(|value| !value.is_empty())
                .cloned()
                .collect(),
            ended: false,
            deferred_interruption: None,
            command_deferral: None,
            awaited_interruption: None,
            abandoned: false,
            json_seen: false,
        };
        // Режим ответа и подключение к базе — служебные команды открытия сессии: работы
        // команды они не отмечают.
        let service = policy.without_work();
        session.run(JSON_MODE_COMMAND, &service)?.outcome()?;
        if at_open == AtOpen::Connect {
            session.connect_infobase(&service)?;
        }
        Ok(session)
    }

    /// Подключает агента к базе (`common connect-ib`). Ответ «уже подключён»
    /// (`DesignerAlreadyConnectedToInfoBase`) — тоже подключение: так отвечает агент, которого
    /// подключили раньше (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`). Служебная
    /// команда работы команды не отмечает.
    pub fn connect_infobase(&mut self, policy: &WaitPolicy) -> Result<(), AgentError> {
        self.connect_infobase_reporting(policy, &mut |_| {})
    }

    /// То же, что `connect_infobase`, но ответ агента на подключение, уже с маской секретов,
    /// отдаётся `on_batch`: сторож живого агента передаёт его в журнал команды.
    pub fn connect_infobase_reporting(
        &mut self,
        policy: &WaitPolicy,
        on_batch: &mut dyn FnMut(&[AgentMessage]),
    ) -> Result<(), AgentError> {
        match self
            .run_streaming(CONNECT_COMMAND, &policy.without_work(), on_batch)?
            .outcome()
        {
            Ok(_)
            | Err(AgentError::Command {
                error_type: AgentErrorType::DesignerAlreadyConnectedToInfoBase,
                ..
            }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Отключает агента от базы (`common disconnect-ib`), не закрывая сессии: так живой агент
    /// отпускает базу другому исполнителю (#644). Служебная команда работы команды не отмечает.
    pub fn disconnect_infobase(&mut self, policy: &WaitPolicy) -> Result<(), AgentError> {
        self.run(DISCONNECT_COMMAND, &policy.without_work())?
            .outcome()
            .map(|_| ())
    }

    /// Одна команда — последовательность JSON-массивов до первого с итоговым
    /// сообщением: долгие команды шлют прогресс и журнал отдельными массивами
    /// (замер 15.09.2026: `load-config-from-files` — `progress`, `progress`, …, `success`).
    /// Команду запроса после отмены не отправляет: отказ `NotSent`.
    pub fn run(&mut self, command: &str, policy: &WaitPolicy) -> Result<AgentReply, AgentError> {
        self.run_streaming(command, policy, &mut |_| {})
    }

    /// То же, что `run`, но каждый массив ответа, уже с маской секретов, сначала отдаётся
    /// `on_batch`: сторож живого агента передаёт их команде по мере прихода.
    pub fn run_streaming(
        &mut self,
        command: &str,
        policy: &WaitPolicy,
        on_batch: &mut dyn FnMut(&[AgentMessage]),
    ) -> Result<AgentReply, AgentError> {
        self.command_deferral = None;
        self.awaited_interruption = None;
        // Команду запроса после отмены агенту не отдают, как раннер не запускает процесс:
        // отмена до работы — безопасная точка. Служебные команды — открытие, закрытие —
        // идут и после неё, иначе сессию было бы не закрыть.
        if policy.work.is_some() && policy.cancellation.is_cancelled() {
            return Err(AgentError::NotSent {
                command: self.display_command(command),
            });
        }
        let started = Instant::now();
        self.send(command)?;
        // Команда ушла агенту: это работа команды, чем бы она ни кончилась. Служебные
        // команды сессии приходят без отметки.
        if let Some(work) = &policy.work {
            work.mark_work_given();
        }
        let mut messages = Vec::new();
        loop {
            let raw = self.read_reply(command, policy)?;
            let mut batch = parse_batch(&raw)?;
            let secrets: Vec<&str> = self.known_secrets.iter().map(String::as_str).collect();
            if let Some(log) = self.transcript.as_mut() {
                let mut display = serde_json::to_value(&batch).expect("agent messages serialize");
                mask_agent_display(&mut display, &secrets);
                let _ = writeln!(log, "{display}");
            }
            for message in &mut batch {
                if let Some(text) = &mut message.message {
                    *text = crate::platform::secrets::mask_text(text, &secrets);
                }
            }
            on_batch(&batch);
            let done = batch.iter().any(AgentMessage::is_terminal);
            messages.extend(batch);
            if done {
                break;
            }
        }
        let reply = AgentReply { messages };
        let secrets: Vec<&str> = self.known_secrets.iter().map(String::as_str).collect();
        log_reply(command, &secrets, &reply, started.elapsed());
        if let Some((reason, observed)) = self.awaited_interruption {
            // Вопрос останавливает чтение, но не подтверждает конец команды. Отказ
            // агента сохраняет свою причину; явная отмена агентом подтверждает конец.
            match reply.outcome() {
                Ok(_) | Err(AgentError::Canceled { .. }) => {}
                Err(error) => return Err(error),
            }
            return Err(match reason {
                ProcessInterruptionReason::Cancelled => AgentError::Cancelled {
                    command: self.display_command(command),
                    delivered: true,
                },
                ProcessInterruptionReason::TimedOut => AgentError::TimedOut {
                    command: self.display_command(command),
                    timeout_ms: observed.saturating_duration_since(started).as_millis() as u64,
                },
            });
        }
        Ok(reply)
    }

    /// Точка входа, к которой сессия подключена.
    pub fn endpoint(&self) -> &AgentEndpoint {
        &self.endpoint
    }

    /// Прерывание, которое отложила последняя команда, — и тогда, когда её ответ не
    /// дочитан или оказался отказом. Читают его сразу после `run`: следующая команда его
    /// сбрасывает.
    pub fn last_command_deferral(&self) -> Option<ProcessInterruptionReason> {
        self.command_deferral
    }

    /// Отпускает чужой агент, не трогая его процесса — у чужого процесса раннер не хозяин:
    /// закрывает соединение с базой служебной командой — иначе точка входа держит
    /// блокировку Конфигуратора и после разрыва SSH — и саму сессию. Ответ не важен, работы
    /// команды это не отмечает.
    pub fn release(self, policy: &WaitPolicy) {
        self.release_under(&policy.cleanup());
    }

    fn release_under(mut self, cleanup: &WaitPolicy) {
        self.discard_stale();
        if let Err(error) = self.run(DISCONNECT_COMMAND, &cleanup.without_work()) {
            debug!(%error, "agent did not confirm common disconnect-ib");
        }
        self.disconnect();
    }

    /// SFTP той же точки входа: второй канал того же соединения (агент допускает один
    /// shell и несколько SFTP-клиентов). Пути — относительно корня, который точка входа
    /// отдаёт как каталог пользователя (замер 15.09.2026 на шлюзе `ibsrv`).
    fn sftp(&mut self) -> Result<(), AgentError> {
        if self.sftp.is_none() {
            let connection = &self.connection;
            let client = self
                .runtime
                .block_on(async {
                    let channel = connection
                        .channel_open_session()
                        .await
                        .map_err(|error| SftpError::Io(error.to_string()))?;
                    channel
                        .request_subsystem(true, "sftp")
                        .await
                        .map_err(|error| SftpError::Io(error.to_string()))?;
                    SftpClient::init(channel.into_stream()).await
                })
                .map_err(|error| AgentError::Exchange {
                    path: "/".to_owned(),
                    detail: format!("cannot open the sftp subsystem: {error}"),
                })?;
            self.sftp = Some(client);
        }
        Ok(())
    }

    /// Одна операция SFTP. Обрыв канала подсистемы — повод открыть её заново и
    /// повторить операцию один раз; отказ самой точки входа возвращается как есть.
    fn sftp_call<T>(
        &mut self,
        path: &str,
        call: impl Fn(
            &mut SftpClient<russh::ChannelStream<client::Msg>>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, SftpError>> + '_>>,
    ) -> Result<T, AgentError> {
        let mut reopened = false;
        loop {
            self.sftp()?;
            let runtime = &self.runtime;
            let client = self.sftp.as_mut().expect("sftp is open");
            match runtime.block_on(call(client)) {
                Ok(value) => return Ok(value),
                Err(error) if error.is_channel_loss() && !reopened => {
                    debug!(%error, "sftp channel is gone; reopening the subsystem");
                    self.drop_sftp();
                    reopened = true;
                }
                Err(error @ SftpError::UnsafeName { .. }) => {
                    return Err(AgentError::UnsafeEntryName {
                        path: path.to_owned(),
                        detail: error.to_string(),
                    })
                }
                Err(error @ SftpError::Status { code, .. }) => {
                    return Err(AgentError::SftpStatus {
                        path: path.to_owned(),
                        code,
                        detail: error.to_string(),
                    })
                }
                Err(error @ SftpError::Io(_)) => {
                    return Err(AgentError::Exchange {
                        path: path.to_owned(),
                        detail: error.to_string(),
                    })
                }
            }
        }
    }

    /// Поток канала отпускается только внутри контекста runtime: его освобождение
    /// обращается к реактору tokio, и вне контекста это паника, а не ошибка.
    fn drop_sftp(&mut self) {
        let _entered = self.runtime.enter();
        self.sftp = None;
    }

    /// Путь на стороне точки входа — всегда от корня SFTP: относительные пути шлюз
    /// `ibsrv` 8.3.27 не разрешает («No such file or directory» на `mkdir dump`),
    /// а корень он отдаёт как каталог пользователя (замер 15.09.2026).
    fn sftp_path(remote: &str) -> String {
        format!("/{}", remote.trim_start_matches('/'))
    }

    /// Каталог на стороне точки входа, со всеми родителями. Существование не
    /// спрашивается: на отсутствующий путь шлюз `ibsrv` 8.3.27 отвечает общим
    /// `Failure`, а не `NoSuchFile`, так что «есть ли» по коду не узнать; создание
    /// существующего каталога — не ошибка.
    pub fn sftp_mkdir_all(&mut self, remote: &str) -> Result<(), AgentError> {
        let mut prefix = String::new();
        for segment in remote.split('/').filter(|segment| !segment.is_empty()) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            let path = Self::sftp_path(&prefix);
            let created = self.sftp_call(&prefix, move |client| {
                let path = path.clone();
                Box::pin(async move { client.mkdir(&path).await })
            });
            if let Err(error) = created {
                debug!(%error, "sftp mkdir refused; assuming the dir exists");
            }
        }
        Ok(())
    }

    /// Файл целиком на сторону точки входа.
    ///
    /// Флаги открытия — по убыванию строгости: создать и обрезать, создать, только
    /// записать. Точка входа вправе принимать не все сочетания (шлюз `ibsrv` 8.3.27
    /// на запись не отвечает ни одним — замер 15.09.2026), и отказ на последнем
    /// сочетании — отказ канала с кодом точки входа, а не догадка о его причине.
    pub fn sftp_write(&mut self, remote: &str, data: &[u8]) -> Result<(), AgentError> {
        let attempts = [
            sftp::OPEN_WRITE | sftp::OPEN_CREATE | sftp::OPEN_TRUNCATE,
            sftp::OPEN_WRITE | sftp::OPEN_CREATE,
            sftp::OPEN_WRITE,
        ];
        let mut last = None;
        for flags in attempts {
            let data = data.to_vec();
            let path = Self::sftp_path(remote);
            let written = self.sftp_call(remote, move |client| {
                let (path, data) = (path.clone(), data.clone());
                Box::pin(async move { client.write_file(&path, flags, &data).await })
            });
            match written {
                Ok(()) => return Ok(()),
                Err(error) => last = Some(error),
            }
        }
        Err(last.expect("at least one attempt"))
    }

    /// Файл целиком со стороны точки входа.
    pub fn sftp_read(&mut self, remote: &str) -> Result<Vec<u8>, AgentError> {
        let path = Self::sftp_path(remote);
        self.sftp_call(remote, move |client| {
            let path = path.clone();
            Box::pin(async move { client.read_file(&path).await })
        })
    }

    /// Локальный файл — на сторону точки входа.
    pub fn sftp_put_file(&mut self, local: &Path, remote: &str) -> Result<(), AgentError> {
        let data = std::fs::read(local).map_err(|error| AgentError::Exchange {
            path: remote.to_owned(),
            detail: format!("cannot read '{}': {error}", local.display()),
        })?;
        self.sftp_write(remote, &data)
    }

    /// Файл со стороны точки входа — в локальный путь (родители создаются).
    pub fn sftp_get_file(&mut self, remote: &str, local: &Path) -> Result<(), AgentError> {
        let data = self.sftp_read(remote)?;
        if let Some(parent) = local.parent() {
            std::fs::create_dir_all(parent).map_err(|error| AgentError::Exchange {
                path: remote.to_owned(),
                detail: format!("cannot create '{}': {error}", parent.display()),
            })?;
        }
        std::fs::write(local, data).map_err(|error| AgentError::Exchange {
            path: remote.to_owned(),
            detail: format!("cannot write '{}': {error}", local.display()),
        })
    }

    /// Локальный каталог целиком — на сторону точки входа (символические ссылки
    /// разыменовываются).
    pub fn sftp_put_dir(&mut self, local: &Path, remote: &str) -> Result<(), AgentError> {
        self.sftp_mkdir_all(remote)?;
        let entries = std::fs::read_dir(local).map_err(|error| AgentError::Exchange {
            path: remote.to_owned(),
            detail: format!("cannot list '{}': {error}", local.display()),
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| AgentError::Exchange {
                path: remote.to_owned(),
                detail: format!("cannot list '{}': {error}", local.display()),
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_remote = format!("{remote}/{name}");
            let child_local = entry.path();
            if child_local.is_dir() {
                self.sftp_put_dir(&child_local, &child_remote)?;
            } else {
                self.sftp_put_file(&child_local, &child_remote)?;
            }
        }
        Ok(())
    }

    /// Каталог со стороны точки входа — поверх локального (существующие файлы
    /// перезаписываются, лишние не трогаются).
    pub fn sftp_get_dir(&mut self, remote: &str, local: &Path) -> Result<(), AgentError> {
        std::fs::create_dir_all(local).map_err(|error| AgentError::Exchange {
            path: remote.to_owned(),
            detail: format!("cannot create '{}': {error}", local.display()),
        })?;
        for (name, is_dir) in self.sftp_list(remote)? {
            let child_remote = format!("{remote}/{name}");
            let child_local = local.join(&name);
            if is_dir {
                self.sftp_get_dir(&child_remote, &child_local)?;
            } else {
                self.sftp_get_file(&child_remote, &child_local)?;
            }
        }
        Ok(())
    }

    /// Имена и вид записей каталога на стороне точки входа.
    fn sftp_list(&mut self, remote: &str) -> Result<Vec<(String, bool)>, AgentError> {
        let path = Self::sftp_path(remote);
        let entries = self.sftp_call(remote, move |client| {
            let path = path.clone();
            Box::pin(async move { client.list_dir(&path).await })
        })?;
        Ok(entries
            .into_iter()
            .map(|entry| (entry.name, entry.is_dir))
            .collect())
    }

    /// Убирает путь на стороне точки входа целиком; отсутствующий — не ошибка.
    /// Вид пути не спрашивается (см. `sftp_mkdir_all`): каталог узнаётся по тому, что
    /// его удалось перечислить, остальное убирается как файл.
    pub fn sftp_remove_all(&mut self, remote: &str) -> Result<(), AgentError> {
        let path = Self::sftp_path(remote);
        match self.sftp_list(remote) {
            // Отказ по имени — не «это файл, а не каталог»: рекурсию останавливают,
            // иначе непригодное имя тихо превратилось бы в попытку удалить путь как
            // файл и настоящая причина осталась бы только в debug-журнале.
            Err(error @ AgentError::UnsafeEntryName { .. }) => Err(error),
            Ok(entries) => {
                for (name, _) in entries {
                    self.sftp_remove_all(&format!("{remote}/{name}"))?;
                }
                let removed = self.sftp_call(remote, move |client| {
                    let path = path.clone();
                    Box::pin(async move { client.rmdir(&path).await })
                });
                if let Err(error) = removed {
                    debug!(%error, "sftp rmdir refused");
                }
                Ok(())
            }
            Err(_) => {
                let removed = self.sftp_call(remote, move |client| {
                    let path = path.clone();
                    Box::pin(async move { client.remove(&path).await })
                });
                if let Err(error) = removed {
                    debug!(%error, "sftp rm refused; assuming the path is absent");
                }
                Ok(())
            }
        }
    }

    /// Убирает опустевшие родительские каталоги пути, не доходя до корня.
    pub fn sftp_remove_empty_parents(&mut self, remote: &str) {
        let mut path = remote.to_owned();
        while let Some((parent, _)) = path.rsplit_once('/') {
            let parent = parent.to_owned();
            if parent.is_empty() {
                break;
            }
            let remote_parent = Self::sftp_path(&parent);
            let removed = self
                .sftp_call(&parent, move |client| {
                    let remote_parent = remote_parent.clone();
                    Box::pin(async move { client.rmdir(&remote_parent).await })
                })
                .is_ok();
            if !removed {
                break;
            }
            path = parent;
        }
    }

    /// Недочитанное от прежней команды — например, неверный ответ, на котором она
    /// кончилась, — ответом на служебную команду (`disconnect-ib`) не является:
    /// без сброса разбор упал бы на нём сразу (`InvalidReply`), не дождавшись ответа агента.
    /// Журнал сохраняет только размер сброшенного: неразобранные байты могут содержать секреты.
    fn discard_stale(&mut self) {
        let stale = std::mem::take(&mut self.pending);
        if let Some(log) = self.transcript.as_mut() {
            let _ = writeln!(log, "[stale agent bytes omitted: {} bytes]", stale.len());
        }
    }

    fn disconnect(&mut self) {
        self.drop_sftp();
        if self.ended {
            return;
        }
        self.ended = true;
        let channel = &self.channel;
        let connection = &self.connection;
        self.runtime.block_on(async {
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let _ = channel.eof().await;
                let _ = channel.close().await;
                let _ = connection
                    .disconnect(russh::Disconnect::ByApplication, "", "")
                    .await;
            })
            .await;
        });
    }

    fn send(&mut self, command: &str) -> Result<(), AgentError> {
        if self.ended {
            return Err(AgentError::SessionClosed {
                endpoint: self.endpoint.to_string(),
                stderr: self.stderr_text(),
            });
        }
        let display = self.display_command(command);
        if let Some(log) = self.transcript.as_mut() {
            let _ = writeln!(log, "> {display}");
        }
        let payload = format!("{command}\n");
        let channel = &self.channel;
        self.runtime
            .block_on(async { channel.data_bytes(payload.into_bytes()).await })
            .map_err(|source| AgentError::Transport {
                endpoint: self.endpoint.to_string(),
                source,
            })
    }

    /// Бросил ли второй сигнал команду этой сессии: тогда агент, возможно, ещё её
    /// выполняет, и след прогона на его стороне остаётся ему.
    pub fn abandoned(&self) -> bool {
        self.abandoned
    }

    /// Прерывание, защёлкнутое в критической фазе за время сессии.
    pub fn deferred_interruption(&self) -> Option<ProcessInterruptionReason> {
        self.deferred_interruption
    }

    /// Критическая фаза откладывает отмену или истёкший срок, и сессия помнит это в тот же
    /// миг: ответ, который потом не дочитан, этого факта не теряет. Первое побеждает.
    fn defer(&mut self, command: &str, reason: ProcessInterruptionReason) {
        if self.command_deferral.is_none() {
            warn!(command = %self.display_command(command), reason = ?reason, "{}", CRITICAL_INTERRUPTION_DEFERRED);
        }
        self.command_deferral.get_or_insert(reason);
        self.deferred_interruption.get_or_insert(reason);
    }

    /// Читает канал до первого полного JSON-массива. Всё до открывающей скобки — не
    /// ответ (баннер или приглашение до JSON-режима); в журнал идёт только его размер.
    fn read_reply(&mut self, command: &str, policy: &WaitPolicy) -> Result<Vec<u8>, AgentError> {
        let started = Instant::now();
        // Критическая фаза меняет базу, и бросать её на полпути дороже, чем ждать
        // (`DEC.2026-04-20.A-MUTATING-CRITICAL-PHASE-IS-NOT-HARD-KILLED`): отмена и
        // истёкший срок записываются, а команда ждёт исхода. Ожидание ограничивает
        // смерть канала — ровно так же, как на пути Конфигуратора его ограничивает
        // выход процесса.
        let critical = matches!(
            policy.safety,
            ProcessInterruptionSafety::CriticalNonAbortable
        );
        // Чужой процесс раннер не может снять и дождаться. Только доставленная
        // команда запроса ждёт своего итога; служебная очистка сохраняет свой предел.
        // Ожидание ограничивает второй сигнал (`policy.abandonment`): тогда команда брошена.
        let wait_for_external_end =
            self.ownership == AgentOwnership::External && policy.work.is_some() && !critical;
        loop {
            let reply = self.take_complete_array()?;
            let now = Instant::now();
            let remaining = policy
                .deadline
                .map(|deadline| deadline.saturating_duration_since(now));
            let reason = if policy.cancellation.is_cancelled() {
                Some(ProcessInterruptionReason::Cancelled)
            } else if remaining == Some(Duration::ZERO) {
                Some(ProcessInterruptionReason::TimedOut)
            } else {
                None
            };
            let starts_waiting = wait_for_external_end && self.awaited_interruption.is_none();
            if let Some(reason) = reason.filter(|_| starts_waiting) {
                warn!(command = %self.display_command(command), ?reason, "{}", EXTERNAL_AGENT_WAITING_FOR_TERMINAL);
                self.awaited_interruption = Some((reason, now));
            }
            if let Some(reply) = reply {
                return Ok(reply);
            }
            if self.ended {
                return Err(self.ended_without_reply());
            }
            // Второй сигнал бросает только уже начатое ожидание: без первого его нет.
            if self.awaited_interruption.is_some() && policy.abandonment.is_cancelled() {
                warn!(command = %self.display_command(command), "{}", EXTERNAL_AGENT_COMMAND_ABANDONED);
                self.abandoned = true;
                return Err(AgentError::Abandoned {
                    command: self.display_command(command),
                });
            }
            // Свой агент и служебные команды прерываются сразу; чужой ждёт итога выше.
            let interrupts_now = !wait_for_external_end;
            if let Some(reason) = reason.filter(|_| interrupts_now) {
                match reason {
                    reason if critical => self.defer(command, reason),
                    ProcessInterruptionReason::Cancelled => {
                        if policy.work.is_some() {
                            info!(command = %self.display_command(command), "{}", AGENT_COMMAND_ABANDONED);
                        }
                        return Err(AgentError::Cancelled {
                            command: self.display_command(command),
                            delivered: policy.work.is_some(),
                        });
                    }
                    ProcessInterruptionReason::TimedOut => {
                        return Err(AgentError::TimedOut {
                            command: self.display_command(command),
                            timeout_ms: started.elapsed().as_millis() as u64,
                        });
                    }
                }
            }
            let wait = remaining
                .filter(|remaining| !remaining.is_zero())
                .map_or(WAIT_SLICE, |remaining| remaining.min(WAIT_SLICE));
            let channel = &mut self.channel;
            let event = self
                .runtime
                .block_on(async { tokio::time::timeout(wait, channel.wait()).await });
            match event {
                Err(_elapsed) => {}
                Ok(Some(ChannelMsg::Data { data })) => self.pending.extend_from_slice(&data),
                Ok(Some(ChannelMsg::ExtendedData { data, .. })) => {
                    self.stderr.extend_from_slice(&data)
                }
                Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Ok(None) => self.ended = true,
                Ok(Some(_)) => {}
            }
        }
    }

    fn take_complete_array(&mut self) -> Result<Option<Vec<u8>>, AgentError> {
        let skipped = skip_to_array(&mut self.pending);
        if let Some(log) = self.transcript.as_mut() {
            if !skipped.is_empty() {
                let _ = writeln!(log, "[agent banner omitted: {} bytes]", skipped.len());
            }
        }
        let reply = take_array(&mut self.pending)?;
        self.json_seen |= reply.is_some();
        Ok(reply)
    }

    /// Канал кончился, а полного массива нет. Недочитанные байты — проза вместо массива или
    /// оборванный массив — неверный ответ; без них сессия просто закрылась.
    fn ended_without_reply(&self) -> AgentError {
        match unread_reply(&self.pending, self.json_seen) {
            Some(detail) => AgentError::InvalidReply {
                detail: detail.to_owned(),
                head: omitted_output(&self.pending),
            },
            None => AgentError::SessionClosed {
                endpoint: self.endpoint.to_string(),
                stderr: self.stderr_text(),
            },
        }
    }

    fn display_command(&self, command: &str) -> String {
        let secrets: Vec<&str> = self.known_secrets.iter().map(String::as_str).collect();
        crate::platform::secrets::mask_text(command, &secrets)
    }

    fn stderr_text(&self) -> String {
        if self.stderr.is_empty() {
            String::new()
        } else {
            omitted_output(&self.stderr)
        }
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// Снимает с начала буфера всё до открывающей скобки. Эти байты — не ответ (баннер или
/// приглашение до JSON-режима): в журнал идёт их размер, итогом команды они не становятся.
/// Без скобки буфер остаётся как есть: массив может прийти следующим пакетом.
fn skip_to_array(pending: &mut Vec<u8>) -> Vec<u8> {
    match pending.iter().position(|byte| *byte == b'[') {
        Some(start) => pending.drain(..start).collect(),
        None => Vec::new(),
    }
}

/// Снимает с буфера, начатого скобкой, первый полный JSON-массив. Скобка, за которой не
/// JSON, — неверный ответ, а не повод читать дальше.
fn take_array(pending: &mut Vec<u8>) -> Result<Option<Vec<u8>>, AgentError> {
    if pending.first() != Some(&b'[') {
        return Ok(None);
    }
    let mut stream =
        serde_json::Deserializer::from_slice(pending).into_iter::<serde::de::IgnoredAny>();
    match stream.next() {
        Some(Ok(_)) => {
            let end = stream.byte_offset();
            Ok(Some(pending.drain(..end).collect()))
        }
        Some(Err(error)) if error.is_eof() => Ok(None),
        Some(Err(error)) => Err(AgentError::InvalidReply {
            detail: safe_json_error(&error),
            head: omitted_output(pending),
        }),
        None => Ok(None),
    }
}

/// Что осталось в буфере, когда канал кончился: оборванный массив — неверный ответ, проза —
/// тоже, но только после первого JSON-массива сессии (`json_seen`): до него это баннер и
/// приглашение shell. Пустота и пробельные байты ответом не были.
fn unread_reply(pending: &[u8], json_seen: bool) -> Option<&'static str> {
    if pending.iter().all(u8::is_ascii_whitespace) {
        None
    } else if pending.first() == Some(&b'[') {
        Some("the reply was cut off when the session ended")
    } else if json_seen {
        Some("the session ended after text that is not a message array")
    } else {
        None
    }
}

/// Массив ответа — только массив сообщений известного типа: всё иное — неверный ответ.
fn parse_batch(raw: &[u8]) -> Result<Vec<AgentMessage>, AgentError> {
    serde_json::from_slice(raw).map_err(|error| AgentError::InvalidReply {
        detail: safe_json_error(&error),
        head: omitted_output(raw),
    })
}

fn safe_json_error(error: &serde_json::Error) -> String {
    format!(
        "invalid agent message array ({:?}, line {}, column {})",
        error.classify(),
        error.line(),
        error.column()
    )
}

fn omitted_output(bytes: &[u8]) -> String {
    format!("[unparsed agent output omitted: {} bytes]", bytes.len())
}

/// Bound the displayed text without first joining the complete transcript.
fn excerpt(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.push_str("\n[truncated]");
    }
    result
}

/// Отладочная строка `agent replied` об одной команде агента: команда с маской секретов, число
/// сообщений ответа и время от отправки до последнего кадра. Её пишут и разовая сессия, и
/// живой агент, чтобы трасса показывала, сколько шла загрузка и сколько применение.
pub(crate) fn log_reply(command: &str, secrets: &[&str], reply: &AgentReply, elapsed: Duration) {
    debug!(
        command = %crate::platform::secrets::mask_text(command, secrets),
        messages = reply.messages.len(),
        elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        "agent replied"
    );
}

/// Protocol values stay intact; only this display copy masks decoded strings and keys.
/// Строка журнала разговора для одного массива ответа: с маской секретов, как её пишет сессия.
/// Её же пишет команда, которая говорит с живым агентом через сторожа.
pub fn transcript_line(batch: &[AgentMessage], secrets: &[&str]) -> String {
    let mut display = serde_json::to_value(batch).expect("agent messages serialize");
    mask_agent_display(&mut display, secrets);
    display.to_string()
}

fn mask_agent_display(value: &mut serde_json::Value, secrets: &[&str]) {
    match value {
        serde_json::Value::String(text) => {
            *text = crate::platform::secrets::mask_text(text, secrets)
        }
        serde_json::Value::Array(values) => {
            for value in values {
                mask_agent_display(value, secrets);
            }
        }
        serde_json::Value::Object(values) => {
            *values = std::mem::take(values)
                .into_iter()
                .map(|(key, mut value)| {
                    mask_agent_display(&mut value, secrets);
                    (crate::platform::secrets::mask_text(&key, secrets), value)
                })
                .collect();
        }
        _ => {}
    }
}

/// Запуск Конфигуратора в агентском режиме. Из ключей базы берётся только адрес:
/// `/N` и `/P` в этом режиме игнорируются, учётные данные идут через SSH.
pub struct AgentLaunch {
    pub v8: PathBuf,
    pub infobase_args: Vec<String>,
    /// Объявленный порт. Без него каждый запуск получает новый свободный порт.
    pub port: Option<u16>,
    /// Объявленный ключ для `/AgentSSHHostKey`. Иначе каждая попытка получает одноразовый.
    pub host_key: Option<PathBuf>,
    /// Корень для одноразовых ключей до применения ASCII fallback.
    pub key_work_path: PathBuf,
    pub user: String,
    pub password: String,
    pub transcript_log: Option<PathBuf>,
    pub base_dir: PathBuf,
    /// Текущий каталог процесса агента. Агент пишет туда файл настроек пользователя
    /// (`1cv8u.pfl`) при первой сессии, поэтому он свой, внутри `workPath`, а не каталог,
    /// из которого запущен раннер (`INV.PLATFORM.A-MANAGED-AGENT-RUNS-IN-ITS-OWN-DIRECTORY`).
    pub workdir: PathBuf,
    /// Куда зеркалится stdout/stderr процесса агента: улика при неудачном запуске. Рядом
    /// лежит `/Out` агента (`.out.log`): причину выхода до первой сессии агент пишет только
    /// туда.
    pub process_log: PathBuf,
}

impl AgentLaunch {
    fn args(&self, port: u16, host_key: &Path) -> Vec<String> {
        let mut args = vec!["DESIGNER".to_owned()];
        args.extend(self.infobase_args.iter().cloned());
        args.push("/AgentMode".to_owned());
        args.push("/AgentPort".to_owned());
        args.push(port.to_string());
        args.push("/AgentListenAddress".to_owned());
        args.push(MANAGED_LISTEN_HOST.to_string());
        args.push("/AgentSSHHostKey".to_owned());
        args.push(host_key.display().to_string());
        args.push("/AgentBaseDir".to_owned());
        args.push(self.base_dir.display().to_string());
        args.push("/Out".to_owned());
        args.push(self.out_file().display().to_string());
        args
    }

    /// Запрос процесса агента: программа, ключи, свой текущий каталог и журналы.
    fn request(&self, port: u16, host_key: &Path) -> ProcessRequest {
        ProcessRequest {
            program: self.v8.clone(),
            args: self.args(port, host_key),
            workdir: Some(self.workdir.clone()),
            stdout_log_path: Some(self.process_log.with_extension("stdout.log")),
            stderr_log_path: Some(self.process_log.with_extension("stderr.log")),
            startup_probe: Some(Duration::from_millis(300)),
            windows_verbatim_args: Vec::new(),
        }
    }

    /// Файл `/Out` агента.
    fn out_file(&self) -> PathBuf {
        self.process_log.with_extension("out.log")
    }

    fn host_key_for_attempt(&self) -> Result<LaunchHostKey, AgentError> {
        match &self.host_key {
            Some(path) => Ok(LaunchHostKey::Declared(path.clone())),
            None => EphemeralHostKey::create_for(&self.key_work_path, &host_key_fallback_roots())
                .map(LaunchHostKey::OneTime),
        }
    }
}

/// Агент, которого раннер поднял сам: процесс и сессия к нему живут вместе.
pub struct ManagedAgent {
    process: Option<crate::platform::process::ManagedSpawnResult>,
    session: Option<AgentSession>,
    base_dir: PathBuf,
    /// Ключ хоста этого запуска: одноразовый файл живёт, пока жив агент.
    host_key: LaunchHostKey,
}

impl ManagedAgent {
    /// Поднимает процесс и ждёт аутентифицированную сессию. При коллизии на
    /// автоматически выбранном порту один раз поднимает процесс на другом порту.
    pub fn launch(
        runner: &dyn ProcessRunner,
        launch: AgentLaunch,
        startup_timeout: Duration,
        policy: &WaitPolicy,
    ) -> Result<Self, AgentError> {
        Self::launch_in(
            runner,
            launch,
            startup_timeout,
            policy,
            ManagedSpawnMode::Wait,
            AtOpen::Connect,
        )
    }

    /// То же для сторожа живого агента: процесс агента умирает вместе со сторожем
    /// (`INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`), поэтому звать это можно только из
    /// потока, который живёт столько же, сколько сторож ([`ManagedSpawnMode::Tied`]).
    /// `at_open` — подключать ли агента к базе при подъёме: агента, поднятого заранее для
    /// `push`, не подключают, пока разбор не нашёл работы (#574).
    pub fn launch_for_keeper(
        runner: &dyn ProcessRunner,
        launch: AgentLaunch,
        startup_timeout: Duration,
        policy: &WaitPolicy,
        at_open: AtOpen,
    ) -> Result<Self, AgentError> {
        Self::launch_in(
            runner,
            launch,
            startup_timeout,
            policy,
            ManagedSpawnMode::Tied,
            at_open,
        )
    }

    fn launch_in(
        runner: &dyn ProcessRunner,
        launch: AgentLaunch,
        startup_timeout: Duration,
        policy: &WaitPolicy,
        mode: ManagedSpawnMode,
        at_open: AtOpen,
    ) -> Result<Self, AgentError> {
        let port = match launch.port {
            Some(port) => port,
            None => free_managed_port(None)?,
        };
        let started = Instant::now();
        match Self::launch_once(
            runner,
            &launch,
            port,
            started,
            startup_timeout,
            policy,
            mode,
            at_open,
        ) {
            Err(AgentError::ForeignAgentOnPort { .. } | AgentError::PortTaken { .. })
                if launch.port.is_none()
                    && started.elapsed() < startup_timeout
                    && policy
                        .deadline
                        .is_none_or(|deadline| Instant::now() < deadline) =>
            {
                policy.refuse_if_cancelled("open")?;
                debug!(port, "managed agent port collision; retrying on a new port");
                let next_port = free_managed_port(Some(port))?;
                Self::launch_once(
                    runner,
                    &launch,
                    next_port,
                    started,
                    startup_timeout,
                    policy,
                    mode,
                    at_open,
                )
            }
            result => result,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_once(
        runner: &dyn ProcessRunner,
        launch: &AgentLaunch,
        port: u16,
        started: Instant,
        startup_timeout: Duration,
        policy: &WaitPolicy,
        mode: ManagedSpawnMode,
        at_open: AtOpen,
    ) -> Result<Self, AgentError> {
        let host_key = launch.host_key_for_attempt()?;
        let session = AgentSessionRequest {
            endpoint: AgentEndpoint {
                host: Host::Address(MANAGED_LISTEN_HOST),
                port,
            },
            user: launch.user.clone(),
            password: launch.password.clone(),
            transcript_log: launch.transcript_log.clone(),
            host_key: host_key.expectation(),
        };
        for dir in [&launch.base_dir, &launch.workdir] {
            std::fs::create_dir_all(dir).map_err(|source| AgentError::Workspace {
                path: dir.clone(),
                source,
            })?;
        }
        // `/Out` прошлого запуска не должен стать причиной этого.
        crate::support::fs::remove_path_if_exists(&launch.out_file()).map_err(|source| {
            AgentError::Workspace {
                path: launch.out_file(),
                source,
            }
        })?;
        let request = launch.request(port, host_key.path());
        let mut process = runner
            // Процесс агента — подъём сессии, а не работа команды: отметки у него нет.
            .spawn_managed(&request, mode, None)
            .map_err(|error| match port_taken(port) {
                Some(detail) => AgentError::PortTaken { port, detail },
                None => AgentError::Launch(error),
            })?;
        debug!(pid = process.pid(), port, "designer agent launched");

        let opened = loop {
            let last = match AgentSession::open_within(
                &session,
                policy,
                AgentOwnership::Managed,
                Some(HANDSHAKE_PROBE),
                at_open,
            ) {
                Ok(opened) => break opened,
                Err(error @ AgentError::Unreachable { .. }) => error.to_string(),
                Err(AgentError::HostKeyRejected {
                    endpoint,
                    expected,
                    presented,
                }) => {
                    process.terminate();
                    return Err(AgentError::ForeignAgentOnPort {
                        endpoint,
                        expected,
                        presented,
                    });
                }
                Err(error) => {
                    process.terminate();
                    return Err(error);
                }
            };
            if policy.cancellation.is_cancelled() {
                process.terminate();
                return Err(AgentError::Cancelled {
                    command: "open".to_owned(),
                    delivered: false,
                });
            }
            // Агент, вышедший до первой сессии, сессию уже не примет: ждать срок незачем.
            // Порт, занятый после выхода, — структурный признак «порт занят»; прочее
            // называет `/Out` агента.
            if let Some(status) = process.exited() {
                let out = out_evidence(&launch.out_file());
                return Err(match port_taken(port) {
                    Some(detail) => AgentError::PortTaken {
                        port,
                        detail: format!("{detail}; /Out: {out}"),
                    },
                    None => AgentError::ExitedBeforeSession {
                        status: status.to_string(),
                        out,
                    },
                });
            }
            if started.elapsed() >= startup_timeout {
                process.terminate();
                return Err(AgentError::StartupTimedOut {
                    timeout_ms: startup_timeout.as_millis() as u64,
                    last,
                });
            }
            thread::sleep(RETRY_INTERVAL);
        };
        Ok(Self {
            process: Some(process),
            session: Some(opened),
            base_dir: launch.base_dir.clone(),
            host_key,
        })
    }

    pub fn session(&mut self) -> &mut AgentSession {
        self.session.as_mut().expect("session lives with the agent")
    }

    pub fn session_ref(&self) -> &AgentSession {
        self.session.as_ref().expect("session lives with the agent")
    }

    /// Точка входа сессии с поднятым агентом.
    pub fn endpoint(&self) -> &AgentEndpoint {
        self.session
            .as_ref()
            .expect("session lives with the agent")
            .endpoint()
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Номер процесса агента.
    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().map(|process| process.pid())
    }

    /// Ожидание ключа хоста, на котором закреплена сессия: отпечаток идёт в запись живого агента.
    pub fn host_key_expectation(&self) -> HostKeyExpectation {
        self.host_key.expectation()
    }

    /// Закрывает соединение агента с базой (`common disconnect-ib`) и сессию и снимает процесс,
    /// не дожидаясь, что он выйдет сам: `common shutdown` агенту не отдаётся
    /// (`INV.PLATFORM.A-MANAGED-AGENT-IS-RELEASED-AND-REMOVED`). Отказ или молчание
    /// `disconnect-ib` снятия не отменяют, исхода команды они не меняют; отмена команды
    /// срока отпускания не обрывает.
    ///
    /// Процесс снимается его job-объектом (на Unix — `SIGKILL` группы) без ожидания выхода:
    /// команда отвечает сразу, а снятие дерева система доводит сама (#637).
    pub fn release_and_remove(mut self) {
        if let Some(session) = self.session.take() {
            session.release_under(&WaitPolicy::release_window(RELEASE_GRACE));
        }
        if let Some(process) = self.process.take() {
            process.remove();
        }
    }
}

impl Drop for ManagedAgent {
    fn drop(&mut self) {
        // Сессия закрывается первой, чтобы процесс не ждал клиента при снятии.
        self.session.take();
        self.process.take();
    }
}

/// Сколько знаков `/Out` агента приводит отказ.
const OUT_EVIDENCE_CHARS: usize = 400;

/// Текст `/Out` агента для отказа — улика, а не решение; нет файла или текста — пометка.
fn out_evidence(path: &Path) -> String {
    crate::support::fs::read_platform_log(path)
        .ok()
        .and_then(|text| crate::support::fs::evidence_lines(&text))
        .map_or_else(
            || "no /Out text".to_owned(),
            |text| text.chars().take(OUT_EVIDENCE_CHARS).collect(),
        )
}

/// Срок рукопожатия при подъёме управляемого агента: поднявшийся агент присылает баннер
/// SSH сразу, а молчащий слушатель на порту не должен держать ожидание.
const HANDSHAKE_PROBE: Duration = Duration::from_secs(2);

/// Занят ли порт на адресе управляемого агента: `Some` — чем, `None` — свободен.
fn port_taken(port: u16) -> Option<String> {
    std::net::TcpListener::bind((MANAGED_LISTEN_HOST, port))
        .err()
        .map(|error| error.to_string())
}

/// Свободный порт на адресе управляемого агента: система назначает его привязке к
/// порту `0`, привязка тут же закрывается, и номер уходит агенту в `/AgentPort`.
fn free_managed_port(exclude: Option<u16>) -> Result<u16, AgentError> {
    let pick = || -> std::io::Result<_> {
        let listener = std::net::TcpListener::bind((MANAGED_LISTEN_HOST, 0))?;
        let port = listener.local_addr()?.port();
        if Some(port) == exclude {
            // Держим исключённый порт, пока ОС выбирает другой.
            return Ok(std::net::TcpListener::bind((MANAGED_LISTEN_HOST, 0))?
                .local_addr()?
                .port());
        }
        Ok(port)
    };
    pick().map_err(|source| AgentError::NoFreePort { source })
}

/// Одноразовый ключ хоста управляемого агента.
///
/// Без объявленного `host-key` раннер создаёт ED25519-ключ на каждый запуск и отдаёт его
/// агенту тем же `/AgentSSHHostKey`, что и объявленный; ожидание сессии закрепляется на
/// его отпечатке, поэтому другой SSH-сервер на том же порту отвергается. Файл доступен
/// только владельцу и удаляется вместе с агентом.
#[derive(Debug)]
pub struct EphemeralHostKey {
    path: PathBuf,
    fingerprint: Fingerprint,
    /// Свой каталог этого ключа вне `workPath`: создан только для владельца и уходит вместе с
    /// ключом (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`).
    private_dir: Option<PathBuf>,
}

/// Ключ хоста, который получает управляемый агент.
#[derive(Debug)]
enum LaunchHostKey {
    /// `tools.designer_agent.host-key`: файл владельца, раннер его не трогает.
    Declared(PathBuf),
    /// Одноразовый ключ этого запуска.
    OneTime(EphemeralHostKey),
}

impl LaunchHostKey {
    pub fn path(&self) -> &Path {
        match self {
            Self::Declared(path) => path,
            Self::OneTime(key) => key.path(),
        }
    }

    /// Ожидание сессии: открытая часть того самого ключа, что ушёл агенту.
    pub fn expectation(&self) -> HostKeyExpectation {
        match self {
            Self::Declared(path) => HostKeyExpectation::of_host_key_file(path),
            Self::OneTime(key) => key.expectation(),
        }
    }
}

/// Ключ ED25519 из зерна и его текст в формате OpenSSH.
fn ed25519_key(
    seed: &[u8; 32],
    comment: &str,
) -> Result<(russh::keys::PrivateKey, String), String> {
    let key = russh::keys::PrivateKey::new(
        russh::keys::ssh_key::private::KeypairData::Ed25519(
            russh::keys::ssh_key::private::Ed25519Keypair::from_seed(seed),
        ),
        comment,
    )
    .map_err(|error| error.to_string())?;
    let text = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|error| error.to_string())?;
    Ok((key, text.to_string()))
}

/// Ключ хоста для SSH-шлюза службы. Живёт вместе с её записью: от запуска к запуску он тот же, и
/// закреплённый отпечаток остаётся верным.
pub fn write_service_host_key(path: &Path) -> std::io::Result<()> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(std::io::Error::other)?;
    let (_, text) =
        ed25519_key(&seed, "v8-runner managed server").map_err(std::io::Error::other)?;
    crate::support::fs::write_file_atomically(path, |file| file.write_all(text.as_bytes()))
}

/// Отпечаток ключа хоста в виде, который принимает `standalone.host-fingerprint`.
pub fn host_key_fingerprint(path: &Path) -> Option<String> {
    let key = russh::keys::load_secret_key(path, None).ok()?;
    Some(key.public_key().fingerprint(HashAlg::Sha256).to_string())
}

/// Префикс имени одноразового ключа; за ним — номер процесса раннера, который ключ создал.
const ONE_TIME_KEY_PREFIX: &str = "host-key-";

/// Убирает одноразовые ключи, чей раннер уже не работает: файл, переживший аварийный выход
/// (`panic = "abort"`, снятый процесс), не копится в `workPath`. Ключи живых раннеров —
/// и этого тоже — остаются.
fn remove_orphaned_keys(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(owner) = name
            .to_str()
            .and_then(|name| name.strip_prefix(ONE_TIME_KEY_PREFIX))
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if !crate::support::machine::is_process_alive(owner) {
            // Свой каталог ключа вне `workPath` уходит целиком; чужой не даст удалить его ACL.
            let path = entry.path();
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
    }
}

/// Читает ли агент Конфигуратора ключ хоста по этому пути (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`).
/// На Windows — только по пути в ASCII: с кириллицей он отвечает «Фатальная ошибка SSH-сервера:
/// Ошибка установки ключа хоста» и выходит (замер #582). Другие ОС не замерены, и путь не
/// ограничивается.
pub(crate) fn agent_reads_host_key_at(path: &Path) -> bool {
    !cfg!(windows) || path.to_str().is_some_and(str::is_ascii)
}

/// Корни, где ложится одноразовый ключ, когда агент не прочтёт его в `workPath`: временный
/// каталог ОС, на Windows ещё `%ProgramData%\v8-runner`.
pub(crate) fn host_key_fallback_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if cfg!(windows) {
        if let Some(data) = std::env::var_os("ProgramData") {
            roots.push(PathBuf::from(data).join("v8-runner"));
        }
    }
    roots
}

impl EphemeralHostKey {
    pub fn create(dir: &Path) -> Result<Self, AgentError> {
        let workspace = |source| AgentError::Workspace {
            path: dir.to_path_buf(),
            source,
        };
        std::fs::create_dir_all(dir).map_err(workspace)?;
        let invalid = |detail: String| {
            workspace(std::io::Error::new(std::io::ErrorKind::InvalidData, detail))
        };
        // Зерно ключа — из системного генератора (`getrandom`, тот же крейт, что уже в сборке
        // у `uuid`); его отказ — отказ запуска агента, а не паника.
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|error| AgentError::Workspace {
            path: dir.to_path_buf(),
            source: std::io::Error::other(format!(
                "the system random generator gave no seed for the one-time host key: {error}"
            )),
        })?;
        let (key, text) = ed25519_key(&seed, "v8-runner managed agent").map_err(invalid)?;
        remove_orphaned_keys(dir);
        let path = dir.join(format!(
            "{ONE_TIME_KEY_PREFIX}{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(workspace)?;
        if let Err(error) = file.write_all(text.as_bytes()) {
            // Файл уже создан, а владельца, который удалит его при сбросе, ещё нет.
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(workspace(error));
        }
        Ok(Self {
            path,
            fingerprint: key.public_key().fingerprint(HashAlg::Sha256),
            private_dir: None,
        })
    }

    /// Одноразовый ключ для агента рабочей копии `work_path`
    /// (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`). Агент читает путь `workPath` — ключ там. Иначе
    /// ключ ложится в новый каталог со случайным именем под первым корнем из `roots`, путь к
    /// которому агент прочтёт. Каталог создаётся только для владельца и только этим вызовом, так
    /// что заранее подложенный чужой каталог не годится. Ни одного такого корня — отказ до
    /// запуска со всеми испробованными путями.
    pub fn create_for(work_path: &Path, roots: &[PathBuf]) -> Result<Self, AgentError> {
        let own = work_path.join("agent").join("host-keys");
        if agent_reads_host_key_at(&own) {
            return Self::create(&own);
        }
        let mut tried = vec![own];
        for root in roots {
            let dir = root.join(format!(
                "{ONE_TIME_KEY_PREFIX}{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            if !agent_reads_host_key_at(&dir) {
                tried.push(dir);
                continue;
            }
            remove_orphaned_keys(root);
            let made = std::fs::create_dir_all(root)
                .and_then(|()| crate::support::temp::create_private_dir(&dir));
            if let Err(error) = made {
                debug!(dir = %dir.display(), %error, "host key directory refused");
                tried.push(dir);
                continue;
            }
            return match Self::create(&dir) {
                Ok(mut key) => {
                    key.private_dir = Some(dir);
                    Ok(key)
                }
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&dir);
                    Err(error)
                }
            };
        }
        Err(AgentError::HostKeyPathNotReadable { tried })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ожидание сессии: только этот ключ.
    pub fn expectation(&self) -> HostKeyExpectation {
        HostKeyExpectation::Pinned(self.fingerprint)
    }
}

impl Drop for EphemeralHostKey {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(dir) = &self.private_dir {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// Карта каталогов пользователей внутри `AgentBaseDir`, как её пишет платформа.
#[derive(Debug, Deserialize)]
struct BaseDirMap {
    #[serde(rename = "usersInfo", default)]
    users_info: VecDeque<BaseDirUser>,
}

#[derive(Debug, Deserialize)]
struct BaseDirUser {
    #[serde(default)]
    name: String,
    dir: String,
}

/// Каталог, относительно которого агент трактует пути команд для данного
/// пользователя. Локальный агент отдаёт результат через диск, и раскладку задаёт
/// платформа: `<AgentBaseDir>/agentbasedir.json` → `<AgentBaseDir>/<dir>`.
pub fn user_dir(base_dir: &Path, user: &str) -> Result<PathBuf, AgentError> {
    let map_path = base_dir.join(BASE_DIR_MAP_FILE);
    let text = std::fs::read_to_string(&map_path).map_err(|error| AgentError::UserDirUnknown {
        base_dir: base_dir.to_path_buf(),
        user: user.to_owned(),
        detail: format!("cannot read {}: {error}", map_path.display()),
    })?;
    let map: BaseDirMap =
        serde_json::from_str(&text).map_err(|error| AgentError::UserDirUnknown {
            base_dir: base_dir.to_path_buf(),
            user: user.to_owned(),
            detail: format!("{} is not the expected map: {error}", map_path.display()),
        })?;
    map.users_info
        .iter()
        .find(|entry| entry.name == user)
        .map(|entry| base_dir.join(&entry.dir))
        .ok_or_else(|| AgentError::UserDirUnknown {
            base_dir: base_dir.to_path_buf(),
            user: user.to_owned(),
            detail: format!(
                "{} lists [{}]",
                map_path.display(),
                map.users_info
                    .iter()
                    .map(|entry| format!("'{}' → {}", entry.name, entry.dir))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Критический класс объявляет только фазу: срок и отмена остаются теми же, иначе
    /// меняющая команда получила бы собственный бюджет вместо остатка общего.
    #[test]
    fn a_critical_policy_keeps_the_deadline_and_the_cancellation() {
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(7);
        let base = WaitPolicy {
            deadline: Some(deadline),
            cancellation: cancellation.clone(),
            abandonment: CancellationToken::new(),
            safety: ProcessInterruptionSafety::GracefulThenKill,
            work: Some(WorkGiven::for_command()),
        };

        let critical = base.critical();

        assert_eq!(critical.deadline, Some(deadline));
        assert!(matches!(
            critical.safety,
            ProcessInterruptionSafety::CriticalNonAbortable
        ));
        assert!(!critical.cancellation.is_cancelled());
        cancellation.cancel();
        assert!(
            critical.cancellation.is_cancelled(),
            "critical policy must observe the same cancellation token, not a fresh one"
        );
    }

    /// Keepalive идёт, а обрыв по его счётчику — нет. Порог означал бы «сколько агенту
    /// позволено молчать», а занятый агент молчит: три неотвеченных keepalive уже рвали
    /// сессию на 46-й секунде выгрузки УТ (замер 15.09.2026). Измерение даёт этой тишине
    /// нижнюю границу и не даёт верхней, поэтому любой порог здесь — догадка, которая
    /// снова оборвёт живую работу.
    #[test]
    fn keepalive_runs_without_a_teardown_threshold() {
        let config = ssh_client_config();

        assert_eq!(
            config.keepalive_interval,
            Some(KEEPALIVE_INTERVAL),
            "without keepalive traffic a half-open channel to a dead gate is noticed by nobody"
        );
        assert_eq!(
            config.keepalive_max, 0,
            "a non-zero threshold tears the session down while the agent is merely busy"
        );
        assert_eq!(
            config.inactivity_timeout, None,
            "nothing bounds how long an agent operation may take: the command has no deadline \
             and the measurement gives agent silence no upper bound to pick a threshold from"
        );
    }

    /// Очистка входит в срок команды и не заводит собственного: она берёт меньшее из
    /// остатка бюджета и запаса на завершение.
    #[test]
    fn a_cleanup_policy_never_outlives_the_command_budget() {
        let soon = Instant::now() + Duration::from_millis(50);
        let capped = WaitPolicy {
            deadline: Some(soon),
            cancellation: CancellationToken::new(),
            abandonment: CancellationToken::new(),
            safety: ProcessInterruptionSafety::GracefulThenKill,
            work: Some(WorkGiven::for_command()),
        }
        .cleanup();
        assert_eq!(
            capped.deadline,
            Some(soon),
            "a remaining budget shorter than the grace must win"
        );

        let far = Instant::now() + Duration::from_secs(3_600);
        let bounded = WaitPolicy {
            deadline: Some(far),
            cancellation: CancellationToken::new(),
            abandonment: CancellationToken::new(),
            safety: ProcessInterruptionSafety::GracefulThenKill,
            work: Some(WorkGiven::for_command()),
        }
        .cleanup();
        let bounded = bounded.deadline.expect("cleanup always has a deadline");
        assert!(bounded < far, "cleanup must not inherit the whole budget");

        let unbounded = WaitPolicy::default().cleanup();
        assert!(
            unbounded.deadline.is_some(),
            "cleanup must be bounded even when the command has no deadline"
        );
    }

    #[test]
    fn reply_outcome_is_decided_by_type_and_error_type_not_by_prose() {
        let reply: Vec<AgentMessage> = serde_json::from_str(
            r#"[{"type":"log","message":"Ошибка: всё плохо"},{"type":"success","message":""}]"#,
        )
        .expect("parse");
        let reply = AgentReply { messages: reply };
        assert!(reply.outcome().is_ok());

        let reply: Vec<AgentMessage> = serde_json::from_str(
            r#"[{"type":"error","error-type":"InfoBaseNotFound","message":"Успешно"}]"#,
        )
        .expect("parse");
        let reply = AgentReply { messages: reply };
        match reply.outcome() {
            Err(AgentError::Command { error_type, .. }) => {
                assert_eq!(error_type, AgentErrorType::InfoBaseNotFound)
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    /// Проза, в которой встретилась скобка, — неверный ответ, а не успех и не повод ждать.
    #[test]
    fn prose_in_place_of_a_message_array_is_an_invalid_reply() {
        for prose in [
            "Выгрузка конфигурации успешно завершена [100%]\n",
            "Configuration dumped successfully [OK]\n",
            "[success]\n",
        ] {
            let mut pending = prose.as_bytes().to_vec();
            skip_to_array(&mut pending);
            match take_array(&mut pending) {
                Err(AgentError::InvalidReply { .. }) => {}
                other => panic!("{prose:?} gave {other:?}"),
            }
        }
    }

    /// Проза без массива ответом не становится: она уходит в журнал, а итога нет.
    #[test]
    fn prose_without_an_array_is_not_a_reply() {
        for prose in [
            "Выгрузка конфигурации успешно завершена\n",
            "Success\n",
            "{\"type\":\"success\"}\n",
        ] {
            let mut pending = prose.as_bytes().to_vec();
            assert!(skip_to_array(&mut pending).is_empty(), "{prose:?}");
            let reply = take_array(&mut pending).expect("no bracket, no verdict");
            assert!(reply.is_none(), "{prose:?}");
        }

        let mut pending = b"designer> Success\n[{\"type\":\"error\"}]".to_vec();
        assert_eq!(skip_to_array(&mut pending), b"designer> Success\n");
        let reply = take_array(&mut pending).expect("array after prose");
        let messages = parse_batch(&reply.expect("the array")).expect("messages");
        let reply = AgentReply { messages };
        assert!(
            matches!(reply.outcome(), Err(AgentError::Command { .. })),
            "the prose before the array decides nothing"
        );
    }

    /// Конец канала посреди массива или после прозы, пришедшей вслед за JSON, — неверный
    /// ответ; пустой конец и баннер до первого массива — закрытая сессия.
    #[test]
    fn a_reply_cut_off_or_left_as_prose_at_the_end_of_the_session_is_an_invalid_reply() {
        let cut_off = br#"[{"type":"progress"},{"type":"succ"#;
        let prose = "Выгрузка успешно завершена\n".as_bytes();
        for json_seen in [false, true] {
            assert_eq!(unread_reply(b"", json_seen), None);
            assert_eq!(unread_reply(b" \r\n", json_seen), None);
            assert!(unread_reply(cut_off, json_seen).is_some());
        }
        assert!(unread_reply(prose, true).is_some());
    }

    /// Баннер и приглашение shell до JSON-режима — не ответ: сессия, закрытая после них,
    /// остаётся закрытой сессией, а не неверным ответом.
    #[test]
    fn a_banner_before_the_first_array_then_the_end_of_the_session_is_a_closed_session() {
        assert_eq!(unread_reply(b"1C Designer Shell\ndesigner> ", false), None);
    }

    /// Сообщение неизвестного типа или без типа и массив не из сообщений — неверный ответ.
    #[test]
    fn a_message_of_an_unknown_or_missing_type_is_an_invalid_reply() {
        for raw in [
            r#"[{"type":"done","message":"Успешно"}]"#,
            r#"[{"message":"Успешно"}]"#,
            r#"[{"type":"success","error-type":42}]"#,
            r#"["Успешно"]"#,
            r#"[true]"#,
        ] {
            match parse_batch(raw.as_bytes()) {
                Err(AgentError::InvalidReply { .. }) => {}
                other => panic!("{raw} gave {other:?}"),
            }
        }
    }

    /// Ответ без итогового сообщения — отказ, а не успех, о чём бы ни говорил журнал.
    #[test]
    fn a_reply_without_a_terminal_message_is_a_refusal() {
        for raw in [
            "[]",
            r#"[{"type":"log","message":"Успешно"}]"#,
            r#"[{"type":"progress","message":"100%"},{"type":"log","message":"Success"}]"#,
        ] {
            let reply = AgentReply {
                messages: parse_batch(raw.as_bytes()).expect("messages"),
            };
            match reply.outcome() {
                Err(AgentError::NoTerminalMessage { .. }) => {}
                other => panic!("{raw} gave {other:?}"),
            }
        }
    }

    /// Ошибка без `error-type` остаётся отказом: рода у неё нет, а успехом она не становится.
    #[test]
    fn an_error_without_an_error_type_is_still_a_refusal() {
        let reply = AgentReply {
            messages: parse_batch(r#"[{"type":"error","message":"Успешно"}]"#.as_bytes())
                .expect("messages"),
        };
        match reply.outcome() {
            Err(AgentError::Command { error_type, .. }) => {
                assert_eq!(error_type, AgentErrorType::UnknownError)
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    /// Живой зонд SFTP шлюза: `V8_GATE_PROBE=host:port V8_GATE_USER=u V8_GATE_PASSWORD=p
    /// cargo test -- --ignored gate_sftp_probe --nocapture`.
    #[test]
    #[ignore]
    fn gate_sftp_probe() {
        let Ok(endpoint) = std::env::var("V8_GATE_PROBE") else {
            return;
        };
        let (host, port) =
            crate::support::authority::host_and_port_of_authority(&endpoint).expect("host:port");
        let request = AgentSessionRequest {
            endpoint: AgentEndpoint {
                host,
                port: port.expect("port"),
            },
            user: std::env::var("V8_GATE_USER").unwrap_or_default(),
            password: std::env::var("V8_GATE_PASSWORD").unwrap_or_default(),
            transcript_log: None,
            host_key: HostKeyExpectation::Unpinned,
        };
        let wait = WaitPolicy {
            deadline: Some(Instant::now() + Duration::from_secs(60)),
            cancellation: CancellationToken::new(),
            abandonment: CancellationToken::new(),
            safety: ProcessInterruptionSafety::Interruptible,
            work: Some(WorkGiven::for_command()),
        };
        let mut session = AgentSession::open(&request, &wait).expect("open");
        eprintln!(
            "connect-ib: {:?}",
            session
                .run("common connect-ib", &wait)
                .map(|r| r.messages.len())
        );
        let before = std::env::var("V8_GATE_PROBE_MODE").as_deref() != Ok("after");
        if before {
            eprintln!("mkdir: {:?}", session.sftp_mkdir_all("probe/x"));
            eprintln!("list before command: {:?}", session.sftp_list("probe"));
        }
        eprintln!(
            "dump-cfg: {:?}",
            session
                .run("config dump-cfg --file=probe/x/a.cf", &wait)
                .map(|r| r.messages.len())
        );
        eprintln!("list after command: {:?}", session.sftp_list("probe/x"));
        eprintln!(
            "read after command: {:?}",
            session.sftp_read("probe/x/a.cf").map(|b| b.len())
        );
        session.drop_sftp();
        eprintln!("list after reopen: {:?}", session.sftp_list("probe/x"));
        eprintln!(
            "read after reopen: {:?}",
            session.sftp_read("probe/x/a.cf").map(|b| b.len())
        );
        eprintln!("remove: {:?}", session.sftp_remove_all("probe"));
        session.release(&wait);
    }

    /// Шлюз автономного сервера внутри ответа на `update-db-cfg` шлёт уведомление
    /// `generation-id`; оно не итог — итог команды приходит после него. Приняв его за
    /// итог, читатель сдвинул бы все следующие ответы на одну команду (живой прогон
    /// 15.09.2026).
    #[test]
    fn a_generation_id_notice_of_the_gate_does_not_end_the_reply() {
        let notice: AgentMessage = serde_json::from_str(
            r#"{"body":"9d8827bb07b15c4b84da5a76ddd83d4600000000","type":"generation-id"}"#,
        )
        .expect("message");
        assert!(!notice.is_terminal());
        let reply = AgentReply {
            messages: serde_json::from_str(
                r#"[{"message":"Принятие изменений...","type":"log"},{"body":"9d88","type":"generation-id"},{"message":"Обновление конфигурации базы данных успешно завершено","type":"log"},{"type":"success"}]"#,
            )
            .expect("messages"),
        };
        assert!(reply.outcome().is_ok());
    }

    /// На одно расширение агент отвечает записью без `success`: она и есть итог.
    #[test]
    fn an_extension_properties_message_alone_ends_the_reply() {
        let reply = AgentReply {
            messages: serde_json::from_str(
                r#"[{"type":"extension-properties","body":{"name":"Зонд"}}]"#,
            )
            .expect("messages"),
        };
        assert!(reply.messages[0].is_terminal());
        assert_eq!(
            reply
                .outcome()
                .expect("outcome")
                .and_then(|body| body.get("name")),
            Some(&serde_json::Value::String("Зонд".to_owned()))
        );
    }

    #[test]
    fn command_error_keeps_preceding_file_diagnostics() {
        for terminal in ["", "Configuration files could not be loaded"] {
            let reply = AgentReply {
                messages: serde_json::from_value(serde_json::json!([
                    {"type":"log", "message":"Reports/Sales/Forms/Main/Ext/Form.xml"},
                    {"type":"loading-issue", "message":"Invalid data path: Rows.Reference"},
                    {"type":"error", "error-type":"ConfigFilesError", "message":terminal}
                ]))
                .expect("messages"),
            };
            match reply.outcome().expect_err("the terminal error decides") {
                AgentError::Command {
                    error_type,
                    message,
                } => {
                    assert_eq!(error_type, AgentErrorType::ConfigFilesError);
                    assert!(
                        message.contains("Reports/Sales/Forms/Main/Ext/Form.xml"),
                        "{message}"
                    );
                    assert!(
                        message.contains("Invalid data path: Rows.Reference"),
                        "{message}"
                    );
                    assert!(message.contains(terminal), "{message}");
                }
                other => panic!("wrong refusal: {other}"),
            }
        }
    }

    #[test]
    fn failure_diagnostics_keep_the_nearest_file_and_typed_cause_after_long_logs() {
        let reply = AgentReply {
            messages: serde_json::from_value(serde_json::json!([
                {"type":"log", "message":"noise".repeat(1400)},
                {"type":"log", "message":"Reports/Sales/Forms/Main/Ext/Form.xml"},
                {"type":"loading-issue", "message":"Invalid data path: Rows.Reference"},
                {"type":"error", "error-type":"ConfigFilesError", "message":""}
            ]))
            .expect("messages"),
        };
        let AgentError::Command {
            error_type,
            message,
        } = reply.outcome().unwrap_err()
        else {
            panic!("expected a typed command error");
        };
        assert_eq!(error_type, AgentErrorType::ConfigFilesError);
        let file = message
            .find("Reports/Sales/Forms/Main/Ext/Form.xml")
            .expect("closest filename must survive preceding logs");
        let cause = message
            .find("Invalid data path: Rows.Reference")
            .expect("typed cause must survive preceding logs");
        assert!(file < cause, "diagnostics retain chronological order");
        assert!(message.contains("[truncated]"));
        assert!(message.chars().count() <= 6144);
    }

    #[test]
    fn typed_diagnostics_reserve_log_space_and_return_their_unused_share() {
        for long_typed in [false, true] {
            let log = format!("closest-file.xml {} end-of-file-context", "л".repeat(5000));
            let cause = if long_typed {
                format!("closest typed cause {}", "п".repeat(7000))
            } else {
                "closest typed cause".to_owned()
            };
            let reply = AgentReply {
                messages: serde_json::from_value(serde_json::json!([
                    {"type":"loading-issue", "message":"older typed cause"},
                    {"type":"log", "message":"older log"},
                    {"type":"log", "message":log},
                    {"type":"loading-issue", "message":cause},
                    {"type":"error", "error-type":"ConfigFilesError", "message":""}
                ]))
                .expect("messages"),
            };
            let AgentError::Command { message, .. } = reply.outcome().unwrap_err() else {
                panic!("expected command error");
            };
            assert!(
                message.find("closest-file.xml").unwrap()
                    < message.find("closest typed cause").unwrap()
            );
            assert!(message.chars().count() <= 6144);
            if long_typed {
                assert!(!message.contains("older typed cause"));
                assert!(!message.contains("older log"));
                assert!(message.chars().filter(|ch| *ch == 'л').count() >= 1900);
                assert!(message.contains("[truncated]"));
            } else {
                assert!(
                    message.contains("end-of-file-context"),
                    "unused typed capacity goes to logs"
                );
            }
        }
    }

    #[test]
    fn long_agent_diagnostics_keep_the_terminal_message_with_a_bounded_unicode_excerpt() {
        let reply = AgentReply {
            messages: serde_json::from_value(serde_json::json!([
                {"type":"log", "message":"д".repeat(20_000)},
                {"type":"error", "error-type":"ConfigFilesError", "message":"terminal cause"}
            ]))
            .expect("messages"),
        };
        match reply.outcome().expect_err("refusal") {
            AgentError::Command { message, .. } => {
                assert!(
                    message.starts_with("terminal cause"),
                    "terminal must have its own budget"
                );
                assert!(message.contains("[truncated]"));
                assert!(message.chars().count() < 8_300);
            }
            other => panic!("wrong refusal: {other}"),
        }
        let success = AgentReply {
            messages: serde_json::from_value(serde_json::json!([
                {"type":"log", "message":"ConfigFilesError: alarming prose"},
                {"type":"success", "body":{"unchanged":true}}
            ]))
            .expect("messages"),
        };
        assert_eq!(success.outcome().unwrap().unwrap()["unchanged"], true);
    }

    #[test]
    fn agent_display_masks_decoded_values_and_keys_without_changing_protocol_body() {
        let secret = format!("private{}{}{}пароль", '"', '\\', '\n');
        let original = serde_json::json!({
            "type":"error", "error-type":"ConfigFilesError",
            "message": format!("Forms/Main/Ext/Form.xml: {secret}"),
            "body": {(secret.clone()): format!("quoted value: {secret}"), "source":"Forms/Main/Ext/Form.xml"}
        });
        let encoded = original.to_string().replace("private", "\\u0070rivate");
        let decoded: serde_json::Value = serde_json::from_str(&encoded).expect("escaped JSON");
        assert_eq!(decoded, original);
        let mut display = decoded.clone();
        mask_agent_display(&mut display, &[&secret]);
        assert_eq!(decoded, original, "semantic fields must stay intact");
        let text = display.to_string();
        assert!(!text.contains("private"), "{text}");
        assert!(!text.contains("rivate"), "{text}");
        assert!(!text.contains("пароль"), "{text}");
        assert!(text.contains("Forms/Main/Ext/Form.xml"));
        assert_eq!(display["error-type"], "ConfigFilesError");
        assert!(
            display["body"].get("***").is_some(),
            "decoded object key is masked"
        );
    }

    #[test]
    fn malformed_agent_output_never_echoes_the_unparsed_input() {
        for raw in [
            br#"[{"type":"secret-password"}]"#.as_slice(),
            br#"[{"type":"error",not-json-secret-password}]"#.as_slice(),
        ] {
            let error = parse_batch(raw).expect_err("not a message array");
            let shown = error.to_string();
            assert!(!shown.contains("secret-password"), "{shown}");
            assert!(shown.contains("omitted"), "{shown}");
            assert!(matches!(error, AgentError::InvalidReply { .. }));
        }
        let mut malformed = b"[secret-password]".to_vec();
        let error = take_array(&mut malformed).expect_err("malformed JSON");
        assert!(!error.to_string().contains("secret-password"));
        assert_eq!(
            malformed, b"[secret-password]",
            "display must not alter input"
        );
    }

    #[test]
    fn many_short_diagnostics_share_the_same_bounded_budget() {
        let mut messages: Vec<AgentMessage> = (0..20_000)
            .map(|_| AgentMessage {
                kind: AgentMessageType::Log,
                error_type: None,
                message: Some("д".into()),
                body: None,
            })
            .collect();
        messages.push(AgentMessage {
            kind: AgentMessageType::Error,
            error_type: Some(AgentErrorType::ConfigFilesError),
            message: Some("terminal cause".into()),
            body: None,
        });
        match (AgentReply { messages }).outcome().unwrap_err() {
            AgentError::Command { message, .. } => {
                assert!(message.starts_with("terminal cause"));
                assert!(message.contains("[truncated]"));
                assert!(message.chars().count() < 8_300);
            }
            other => panic!("wrong refusal: {other}"),
        }
    }

    #[test]
    fn unknown_error_type_is_kept_verbatim() {
        let parsed: AgentMessage =
            serde_json::from_str(r#"{"type":"error","error-type":"SomethingNew","message":""}"#)
                .expect("parse");
        assert_eq!(
            parsed.error_type,
            Some(AgentErrorType::Other("SomethingNew".to_owned()))
        );
    }

    /// Одноразовый ключ: файл только для владельца, удаляется вместе с ключом; ключи
    /// неживых раннеров следующий ключ убирает, ключи живых оставляет.
    #[test]
    fn a_one_time_host_key_lives_with_its_owner_and_sweeps_orphans() {
        let dir = tempfile::tempdir().expect("dir");
        let orphan = dir
            .path()
            .join(format!("{ONE_TIME_KEY_PREFIX}{}-orphan", u32::MAX));
        let alive = dir
            .path()
            .join(format!("{ONE_TIME_KEY_PREFIX}{}-alive", std::process::id()));
        let foreign = dir.path().join("someone-elses-file");
        for path in [&orphan, &alive, &foreign] {
            std::fs::write(path, "key").expect("file");
        }

        let key = EphemeralHostKey::create(dir.path()).expect("key");

        assert!(!orphan.exists(), "a key of a gone runner stays");
        assert!(alive.exists() && foreign.exists());
        assert!(key.path().is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key.path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(matches!(key.expectation(), HostKeyExpectation::Pinned(_)));
        let path = key.path().to_path_buf();
        drop(key);
        assert!(!path.exists(), "the one-time key outlived its owner");
    }

    /// Агент не читает ключ хоста по пути с кириллицей: при таком `workPath` одноразовый ключ
    /// ложится в новый каталог под корнем с путём в ASCII, только для владельца, и уходит вместе
    /// с каталогом; без такого корня — отказ со всеми испробованными путями
    /// (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`).
    #[cfg(windows)]
    #[test]
    fn a_one_time_host_key_leaves_a_cyrillic_work_path_for_a_private_ascii_directory() {
        let temp = tempfile::tempdir().expect("dir");
        let ascii_work = temp.path().join("work");
        let key = EphemeralHostKey::create_for(&ascii_work, &[]).expect("key in workPath");
        assert!(key
            .path()
            .starts_with(ascii_work.join("agent").join("host-keys")));
        drop(key);

        let cyrillic_work = temp.path().join("Рабочая папка").join("ws3");
        let root = temp.path().join("keys");
        // Каталог, подложенный заранее под похожим именем, ключ не получает.
        let planted = root.join(format!(
            "{ONE_TIME_KEY_PREFIX}{}-planted",
            std::process::id()
        ));
        std::fs::create_dir_all(&planted).expect("planted");
        let key = EphemeralHostKey::create_for(&cyrillic_work, &[root.clone()]).expect("key");
        assert!(
            agent_reads_host_key_at(key.path()),
            "{}",
            key.path().display()
        );
        assert!(key.path().starts_with(&root));
        assert!(!key.path().starts_with(&planted));
        let dir = key.path().parent().expect("dir").to_path_buf();
        assert_ne!(dir, root);
        drop(key);
        assert!(!dir.exists(), "the key directory outlived the key");

        let refused = EphemeralHostKey::create_for(&cyrillic_work, &[temp.path().join("ключи")])
            .err()
            .expect("refused");
        assert!(
            matches!(&refused, AgentError::HostKeyPathNotReadable { tried } if tried.len() == 2),
            "{refused}"
        );
    }

    #[test]
    fn launch_args_carry_only_the_agent_keys_and_the_infobase_address() {
        let launch = AgentLaunch {
            v8: PathBuf::from("/opt/1cv8/1cv8"),
            infobase_args: vec!["/F".to_owned(), "/tmp/ib".to_owned()],
            port: Some(1543),
            host_key: Some(PathBuf::from("/work/host_key")),
            key_work_path: PathBuf::from("/work"),
            user: String::new(),
            password: String::new(),
            transcript_log: None,
            base_dir: PathBuf::from("/work/agent"),
            workdir: PathBuf::from("/work/agent/workdir"),
            process_log: PathBuf::from("/work/logs/agent"),
        };
        assert_eq!(
            launch.args(1543, Path::new("/work/host_key")),
            vec![
                "DESIGNER",
                "/F",
                "/tmp/ib",
                "/AgentMode",
                "/AgentPort",
                "1543",
                "/AgentListenAddress",
                "127.0.0.1",
                "/AgentSSHHostKey",
                "/work/host_key",
                "/AgentBaseDir",
                "/work/agent",
                "/Out",
                "/work/logs/agent.out.log",
            ]
        );
    }

    /// Агент работает в своём каталоге внутри `workPath`: файл настроек пользователя,
    /// который он пишет в текущий каталог при первой сессии, не попадает туда, откуда
    /// запущен раннер.
    #[test]
    fn a_managed_agent_runs_in_its_own_directory() {
        let launch = AgentLaunch {
            v8: PathBuf::from("/opt/1cv8/1cv8"),
            infobase_args: vec!["/F".to_owned(), "/tmp/ib".to_owned()],
            port: Some(1543),
            host_key: Some(PathBuf::from("/work/host_key")),
            key_work_path: PathBuf::from("/work"),
            user: String::new(),
            password: String::new(),
            transcript_log: None,
            base_dir: PathBuf::from("/work/agent/base"),
            workdir: PathBuf::from("/work/agent/workdir"),
            process_log: PathBuf::from("/work/logs/agent"),
        };
        let request = launch.request(1543, Path::new("/work/host_key"));
        assert_eq!(
            request.workdir.as_deref(),
            Some(Path::new("/work/agent/workdir"))
        );
    }

    /// Адрес кластерной базы приходит к агенту в той же форме, в какой уходит
    /// Конфигуратору пакетно: `/S host\name` (#55). Агент разбирает строку не сам —
    /// он получает готовый адрес от `V8Connection`, и эти два пути не расходятся.
    #[test]
    fn a_cluster_address_reaches_the_agent_in_the_platform_form() {
        let connection = crate::platform::connection::V8Connection::from_connection_string(
            "Srvr=srv:1541;Ref=demo",
        );
        let launch = AgentLaunch {
            v8: PathBuf::from("/opt/1cv8/1cv8"),
            infobase_args: connection.infobase_args(),
            port: Some(1543),
            host_key: Some(PathBuf::from("/work/host_key")),
            key_work_path: PathBuf::from("/work"),
            user: String::new(),
            password: String::new(),
            transcript_log: None,
            base_dir: PathBuf::from("/work/agent"),
            workdir: PathBuf::from("/work/agent/workdir"),
            process_log: PathBuf::from("/work/logs/agent"),
        };
        let args = launch.args(1543, Path::new("/work/host_key"));
        assert_eq!(&args[..3], ["DESIGNER", "/S", "srv:1541\\demo"], "{args:?}");
        assert!(
            !args.iter().any(|arg| arg == "/IBConnectionString"),
            "{args:?}"
        );
        // Реквизиты в этом режиме не передаются: их несёт SSH.
        assert!(
            !args.iter().any(|arg| arg == "/N" || arg == "/P"),
            "{args:?}"
        );
    }

    /// Журнал и отказ печатают точку входа так, как её объявляют: адрес IPv6 — в
    /// скобках; соединению при этом уходит голый адрес.
    #[test]
    fn an_endpoint_prints_a_v6_address_in_brackets() {
        let v6 = AgentEndpoint {
            host: Host::Address("::1".parse().expect("v6")),
            port: 1543,
        };
        assert_eq!(v6.to_string(), "[::1]:1543");
        assert_eq!(v6.host.to_string(), "::1");
        let v4 = AgentEndpoint {
            host: Host::Address(MANAGED_LISTEN_HOST),
            port: 1543,
        };
        assert_eq!(v4.to_string(), "127.0.0.1:1543");
        let name = AgentEndpoint {
            host: Host::Name("srv.example".to_owned()),
            port: 1543,
        };
        assert_eq!(name.to_string(), "srv.example:1543");
    }

    #[test]
    fn a_dead_endpoint_is_unreachable_not_a_handshake_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let request = AgentSessionRequest {
            endpoint: AgentEndpoint {
                host: Host::Address(MANAGED_LISTEN_HOST),
                port,
            },
            user: String::new(),
            password: String::new(),
            transcript_log: None,
            host_key: HostKeyExpectation::Unpinned,
        };
        assert!(matches!(
            AgentSession::open(&request, &WaitPolicy::default()),
            Err(AgentError::Unreachable { .. })
        ));
    }

    #[test]
    fn user_dir_follows_the_platform_map() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(BASE_DIR_MAP_FILE),
            r#"{"usersInfo":[{"name":"","dir":"0"},{"name":"Admin","dir":"1"}]}"#,
        )
        .expect("map");
        assert_eq!(user_dir(dir.path(), "").expect("dir"), dir.path().join("0"));
        assert_eq!(
            user_dir(dir.path(), "Admin").expect("dir"),
            dir.path().join("1")
        );
        assert!(matches!(
            user_dir(dir.path(), "Nobody"),
            Err(AgentError::UserDirUnknown { .. })
        ));
    }
}
