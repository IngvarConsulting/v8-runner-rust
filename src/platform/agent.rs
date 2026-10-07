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
use serde::Deserialize;
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
/// Команда, которой управляемый агент завершает работу.
/// Закрытие соединения с базой без завершения точки входа.
pub const DISCONNECT_COMMAND: &str = "common disconnect-ib";
pub const SHUTDOWN_COMMAND: &str = "common shutdown";
/// Адрес, который слушает управляемый агент: он живёт на машине раннера.
pub const MANAGED_LISTEN_HOST: std::net::IpAddr =
    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
/// Файл карты пользовательских каталогов в `AgentBaseDir`.
pub const BASE_DIR_MAP_FILE: &str = "agentbasedir.json";
const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
const WAIT_SLICE: Duration = Duration::from_millis(200);
/// Строка журнала: отмена бросила ответ команды, которую не обязаны дожидаться.
const AGENT_COMMAND_ABANDONED: &str = "agent command abandoned: the command was cancelled";

/// Интервал keepalive: даёт каналу трафик, на котором TCP способен заметить мёртвый шлюз.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// Тип сообщения агента по документации (Приложение 4, 4.7.8) плюс два, которых в ней
/// нет (живые ответы 8.3.27 от 15.09.2026): `extension-properties` — ответ агента на
/// `extensions properties get --extension=`, итоговый (после него `success` не
/// приходит); `generation-id` — промежуточное уведомление шлюза автономного сервера
/// внутри ответа на `update-db-cfg` (новый токен), итог команды приходит после него.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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

/// Одно сообщение из массива ответа.
#[derive(Debug, Clone, Deserialize)]
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
                message: message.message.clone().unwrap_or_default(),
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
            safety: policy.safety,
            work: policy.work,
        }
    }

    /// Та же политика, урезанная до срока очистки. Это единственное место, где у
    /// агентского ожидания срок вообще появляется: сама работа идёт без него, а вот
    /// завершение обязано закончиться — иначе прерванная сессия висела бы вечно.
    /// Берётся меньшее из запаса на завершение и уже стоящего срока, если тот есть.
    pub fn cleanup(&self) -> Self {
        let grace = Instant::now() + SHUTDOWN_GRACE;
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
    transcript: Option<std::fs::File>,
    ended: bool,
    /// Прерывание, защёлкнутое в критической фазе за время сессии; первое побеждает.
    /// Сессия помнит его, потому что результат платформы собирают после её закрытия.
    deferred_interruption: Option<ProcessInterruptionReason>,
    /// Прерывание, которое отложила последняя команда, — и когда её ответ не дочитан.
    command_deferral: Option<ProcessInterruptionReason>,
    /// Пришёл ли от агента хоть один JSON-массив. До него проза — баннер и приглашение
    /// shell, а не ответ: конец сессии после неё — закрытая сессия, а не неверный ответ.
    json_seen: bool,
}

impl AgentSession {
    /// Открывает сессию и переводит её в машинный режим. Успех означает, что
    /// аутентификация прошла и агент ответил JSON, — этим и доказывается готовность.
    pub fn open(request: &AgentSessionRequest, policy: &WaitPolicy) -> Result<Self, AgentError> {
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
                let mut connection = client::connect(
                    config,
                    (host.as_str(), endpoint.port),
                    ClientEvents {
                        expectation: expectation.clone(),
                        presented: presented.clone(),
                    },
                )
                .await
                .map_err(|error| match error {
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
            transcript,
            ended: false,
            deferred_interruption: None,
            command_deferral: None,
            json_seen: false,
        };
        // Режим ответа и подключение к базе — служебные команды открытия сессии: работы
        // команды они не отмечают.
        let service = policy.without_work();
        session.run(JSON_MODE_COMMAND, &service)?.outcome()?;
        session.run(CONNECT_COMMAND, &service)?.outcome()?;
        Ok(session)
    }

    /// Одна команда — последовательность JSON-массивов до первого с итоговым
    /// сообщением: долгие команды шлют прогресс и журнал отдельными массивами
    /// (замер 15.09.2026: `load-config-from-files` — `progress`, `progress`, …, `success`).
    /// Команду запроса после отмены не отправляет: отказ `NotSent`.
    pub fn run(&mut self, command: &str, policy: &WaitPolicy) -> Result<AgentReply, AgentError> {
        self.command_deferral = None;
        // Команду запроса после отмены агенту не отдают, как раннер не запускает процесс:
        // отмена до работы — безопасная точка. Служебные команды — открытие, закрытие —
        // идут и после неё, иначе сессию было бы не закрыть.
        if policy.work.is_some() && policy.cancellation.is_cancelled() {
            return Err(AgentError::NotSent {
                command: command.to_owned(),
            });
        }
        self.send(command)?;
        // Команда ушла агенту: это работа команды, чем бы она ни кончилась. Служебные
        // команды сессии приходят без отметки.
        if let Some(work) = &policy.work {
            work.mark_work_given();
        }
        let mut messages = Vec::new();
        loop {
            let raw = self.read_reply(command, policy)?;
            let batch = parse_batch(&raw)?;
            let done = batch.iter().any(AgentMessage::is_terminal);
            messages.extend(batch);
            if done {
                break;
            }
        }
        let reply = AgentReply { messages };
        debug!(command, messages = reply.messages.len(), "agent replied");
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
    pub fn release(mut self, policy: &WaitPolicy) {
        self.discard_stale();
        let _ = self.run(DISCONNECT_COMMAND, &policy.cleanup().without_work());
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
                Err(error) => {
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

    /// Просит агента завершиться и закрывает сессию; возвращает ответ, если он был.
    /// Завершение — служебная команда: работы команды оно не отмечает.
    pub fn shutdown(mut self, policy: &WaitPolicy) -> Option<AgentReply> {
        // Ответ на shutdown может и не прийти — сессию закрывает сам агент; ждать его
        // дольше короткого срока незачем, даже если бюджет команды не ограничен.
        let capped = policy.cleanup().without_work();
        self.discard_stale();
        // Агент может закрыть соединение, не ответив: EOF здесь — не отказ.
        let reply = self.run(SHUTDOWN_COMMAND, &capped).ok();
        self.disconnect();
        reply
    }

    /// Недочитанное от прежней команды — например, неверный ответ, на котором она
    /// кончилась, — ответом на служебную команду (`disconnect-ib`, `shutdown`) не является:
    /// без сброса разбор упал бы на нём сразу (`InvalidReply`), не дождавшись ответа агента.
    /// Сброшенное остаётся в журнале сессии.
    fn discard_stale(&mut self) {
        let stale = std::mem::take(&mut self.pending);
        if let Some(log) = self.transcript.as_mut() {
            let _ = log.write_all(&stale);
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
        if let Some(log) = self.transcript.as_mut() {
            let _ = writeln!(log, "> {command}");
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

    /// Прерывание, защёлкнутое в критической фазе за время сессии.
    pub fn deferred_interruption(&self) -> Option<ProcessInterruptionReason> {
        self.deferred_interruption
    }

    /// Критическая фаза откладывает отмену или истёкший срок, и сессия помнит это в тот же
    /// миг: ответ, который потом не дочитан, этого факта не теряет. Первое побеждает.
    fn defer(&mut self, command: &str, reason: ProcessInterruptionReason) {
        if self.command_deferral.is_none() {
            warn!(command, reason = ?reason, "{}", CRITICAL_INTERRUPTION_DEFERRED);
        }
        self.command_deferral.get_or_insert(reason);
        self.deferred_interruption.get_or_insert(reason);
    }

    /// Читает канал до первого полного JSON-массива. Всё до открывающей скобки — не
    /// ответ (баннер или приглашение до JSON-режима) и записывается только в журнал.
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
        loop {
            if let Some(reply) = self.take_complete_array()? {
                if let Some(log) = self.transcript.as_mut() {
                    let _ = log.write_all(&reply);
                    let _ = log.write_all(b"\n");
                }
                return Ok(reply);
            }
            if self.ended {
                return Err(self.ended_without_reply());
            }
            if policy.cancellation.is_cancelled() {
                if !critical {
                    // Служебные команды уборки тоже бросаются по отмене, но строка журнала —
                    // о брошенной работе команды.
                    if policy.work.is_some() {
                        info!(command, "{}", AGENT_COMMAND_ABANDONED);
                    }
                    // Ответ читается после отправки: команда запроса уже работа команды.
                    return Err(AgentError::Cancelled {
                        command: command.to_owned(),
                        delivered: policy.work.is_some(),
                    });
                }
                self.defer(command, ProcessInterruptionReason::Cancelled);
            }
            let wait = match policy.deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match (remaining.is_zero(), critical) {
                        (true, false) => {
                            return Err(AgentError::TimedOut {
                                command: command.to_owned(),
                                timeout_ms: started.elapsed().as_millis() as u64,
                            })
                        }
                        (true, true) => {
                            self.defer(command, ProcessInterruptionReason::TimedOut);
                            WAIT_SLICE
                        }
                        (false, _) => remaining.min(WAIT_SLICE),
                    }
                }
                None => WAIT_SLICE,
            };
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
            let _ = log.write_all(&skipped);
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
                head: head_of(&self.pending),
            },
            None => AgentError::SessionClosed {
                endpoint: self.endpoint.to_string(),
                stderr: self.stderr_text(),
            },
        }
    }

    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// Снимает с начала буфера всё до открывающей скобки. Эти байты — не ответ (баннер или
/// приглашение до JSON-режима): они идут только в журнал и итогом команды не становятся.
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
            detail: error.to_string(),
            head: head_of(pending),
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
        detail: error.to_string(),
        head: head_of(raw),
    })
}

fn head_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(200).collect()
}

/// Запуск Конфигуратора в агентском режиме. Из ключей базы берётся только адрес:
/// `/N` и `/P` в этом режиме игнорируются, учётные данные идут через SSH.
#[derive(Debug)]
pub struct AgentLaunch {
    pub v8: PathBuf,
    pub infobase_args: Vec<String>,
    pub port: u16,
    /// Ключ хоста, который агент получит в `/AgentSSHHostKey`. Ключа платформы
    /// (`/AgentSSHHostKeyAuto`) раннер не берёт: сессия закрепляется на отданном ключе.
    pub host_key: LaunchHostKey,
    pub base_dir: PathBuf,
    /// Куда зеркалится stdout/stderr процесса агента: улика при неудачном запуске.
    pub process_log: PathBuf,
}

impl AgentLaunch {
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["DESIGNER".to_owned()];
        args.extend(self.infobase_args.iter().cloned());
        args.push("/AgentMode".to_owned());
        args.push("/AgentPort".to_owned());
        args.push(self.port.to_string());
        args.push("/AgentListenAddress".to_owned());
        args.push(MANAGED_LISTEN_HOST.to_string());
        args.push("/AgentSSHHostKey".to_owned());
        args.push(self.host_key.path().display().to_string());
        args.push("/AgentBaseDir".to_owned());
        args.push(self.base_dir.display().to_string());
        args
    }

    pub fn endpoint(&self) -> AgentEndpoint {
        AgentEndpoint {
            host: Host::Address(MANAGED_LISTEN_HOST),
            port: self.port,
        }
    }
}

/// Агент, которого раннер поднял сам: процесс и сессия к нему живут вместе.
pub struct ManagedAgent {
    process: Option<crate::platform::process::ManagedSpawnResult>,
    session: Option<AgentSession>,
    base_dir: PathBuf,
    /// Ключ хоста этого запуска: одноразовый файл живёт, пока жив агент.
    _host_key: LaunchHostKey,
}

impl ManagedAgent {
    /// Поднимает процесс и ждёт, пока он примет аутентифицированную сессию. Пока порт
    /// не принимает соединений, агент ещё поднимается; любой другой отказ повтором
    /// не лечится и останавливает ожидание сразу.
    pub fn launch(
        runner: &dyn ProcessRunner,
        launch: AgentLaunch,
        session: AgentSessionRequest,
        startup_timeout: Duration,
        policy: &WaitPolicy,
    ) -> Result<Self, AgentError> {
        std::fs::create_dir_all(&launch.base_dir).map_err(|source| AgentError::Workspace {
            path: launch.base_dir.clone(),
            source,
        })?;
        let request = ProcessRequest {
            program: launch.v8.clone(),
            args: launch.args(),
            workdir: None,
            stdout_log_path: Some(launch.process_log.with_extension("stdout.log")),
            stderr_log_path: Some(launch.process_log.with_extension("stderr.log")),
            startup_probe: Some(Duration::from_millis(300)),
        };
        let process = runner
            // Процесс агента — подъём сессии, а не работа команды: отметки у него нет.
            .spawn_managed(&request, ManagedSpawnMode::Wait, None)
            .map_err(|error| match port_taken(launch.port) {
                Some(detail) => AgentError::PortTaken {
                    port: launch.port,
                    detail,
                },
                None => AgentError::Launch(error),
            })?;
        debug!(
            pid = process.pid(),
            port = launch.port,
            "designer agent launched"
        );

        let started = Instant::now();
        let opened = loop {
            let last = match AgentSession::open(&session, policy) {
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
            base_dir: launch.base_dir,
            _host_key: launch.host_key,
        })
    }

    pub fn session(&mut self) -> &mut AgentSession {
        self.session.as_mut().expect("session lives with the agent")
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

    /// Просит агента завершиться и ждёт процесс; не вышедший вовремя — снимается.
    pub fn shutdown(mut self, policy: &WaitPolicy) {
        if let Some(session) = self.session.take() {
            session.shutdown(policy);
        }
        if let Some(process) = self.process.take() {
            // Ожидание выхода агента — служебное: работы команды оно не отмечает.
            let grace = ProcessExecutionPolicy::platform_step(
                Some(Duration::from_secs(15)),
                CancellationToken::new(),
                ProcessInterruptionSafety::Interruptible,
            );
            match process.wait_for_exit(&grace) {
                Ok(outcome) if outcome.timed_out => {
                    warn!("designer agent ignored shutdown and was terminated")
                }
                Ok(_) => {}
                Err(error) => warn!(error = %error, "designer agent shutdown wait failed"),
            }
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

/// Занят ли порт на адресе управляемого агента: `Some` — чем, `None` — свободен.
fn port_taken(port: u16) -> Option<String> {
    std::net::TcpListener::bind((MANAGED_LISTEN_HOST, port))
        .err()
        .map(|error| error.to_string())
}

/// Свободный порт на адресе управляемого агента: система назначает его привязке к
/// порту `0`, привязка тут же закрывается, и номер уходит агенту в `/AgentPort`.
pub fn free_managed_port() -> Result<u16, AgentError> {
    std::net::TcpListener::bind((MANAGED_LISTEN_HOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|source| AgentError::NoFreePort { source })
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
}

/// Ключ хоста, который получает управляемый агент.
#[derive(Debug)]
pub enum LaunchHostKey {
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
            let _ = std::fs::remove_file(entry.path());
        }
    }
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
        let key = russh::keys::PrivateKey::new(
            russh::keys::ssh_key::private::KeypairData::Ed25519(
                russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&seed),
            ),
            "v8-runner managed agent",
        )
        .map_err(|error| invalid(error.to_string()))?;
        let text = key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .map_err(|error| invalid(error.to_string()))?;
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
        file.write_all(text.as_bytes()).map_err(workspace)?;
        Ok(Self {
            path,
            fingerprint: key.public_key().fingerprint(HashAlg::Sha256),
        })
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

    #[test]
    fn launch_args_carry_only_the_agent_keys_and_the_infobase_address() {
        let launch = AgentLaunch {
            v8: PathBuf::from("/opt/1cv8/1cv8"),
            infobase_args: vec!["/F".to_owned(), "/tmp/ib".to_owned()],
            port: 1543,
            host_key: LaunchHostKey::Declared(PathBuf::from("/work/host_key")),
            base_dir: PathBuf::from("/work/agent"),
            process_log: PathBuf::from("/work/logs/agent"),
        };
        assert_eq!(
            launch.args(),
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
            ]
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
            port: 1543,
            host_key: LaunchHostKey::Declared(PathBuf::from("/work/host_key")),
            base_dir: PathBuf::from("/work/agent"),
            process_log: PathBuf::from("/work/logs/agent"),
        };
        let args = launch.args();
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
