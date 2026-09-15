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
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use russh::client;
use russh::ChannelMsg;
use serde::Deserialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::platform::process::{ManagedSpawnMode, ProcessRequest, ProcessRunner};
use crate::platform::sftp::{self, SftpClient, SftpError};

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
pub const MANAGED_LISTEN_ADDRESS: &str = "127.0.0.1";
/// Файл карты пользовательских каталогов в `AgentBaseDir`.
pub const BASE_DIR_MAP_FILE: &str = "agentbasedir.json";
const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
const WAIT_SLICE: Duration = Duration::from_millis(200);

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
#[derive(Debug, Clone)]
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
    Cancelled { command: String },

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

    #[error("managed agent could not be launched: {0}")]
    Launch(#[source] crate::platform::process::ProcessError),

    #[error("managed agent did not accept a session within {timeout_ms} ms; last: {last}")]
    StartupTimedOut { timeout_ms: u64, last: String },

    #[error("agent base dir '{base_dir}' has no directory for user '{user}': {detail}")]
    UserDirUnknown {
        base_dir: PathBuf,
        user: String,
        detail: String,
    },
}

/// Точка входа: где слушает агент.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEndpoint {
    pub host: String,
    pub port: u16,
}

impl std::fmt::Display for AgentEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
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
}

/// Ожидание ответа: срок и отмена, переданные с границы команды.
#[derive(Debug, Clone)]
pub struct WaitPolicy {
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

impl Default for WaitPolicy {
    fn default() -> Self {
        Self {
            timeout: None,
            cancellation: CancellationToken::new(),
        }
    }
}

/// Обработчик событий SSH-клиента. Ключ хоста принимается: у управляемого агента его
/// создала платформа на этой же машине, у чужого — назвал пользователь в `attach`.
struct ClientEvents;

impl client::Handler for ClientEvents {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        debug!(key = ?server_public_key, "agent host key accepted");
        Ok(true)
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
        debug!(endpoint = %named, user = request.user.as_str(), "opening agent session");

        let (connection, channel) =
            runtime.block_on(async {
                // Без keepalive: агент однопоточен и, занятый долгой командой, не отвечает
                // на глобальные запросы — три неотвеченных keepalive рвали сессию на 46-й
                // секунде выгрузки УТ (замер 15.09.2026). Зависание ловит срок команды.
                let config = Arc::new(client::Config {
                    inactivity_timeout: None,
                    keepalive_interval: None,
                    ..client::Config::default()
                });
                let mut connection = client::connect(
                    config,
                    (endpoint.host.as_str(), endpoint.port),
                    ClientEvents,
                )
                .await
                .map_err(|error| match error {
                    russh::Error::IO(source) => AgentError::Unreachable {
                        endpoint: named.clone(),
                        source,
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
        };
        session.run(JSON_MODE_COMMAND, policy)?.outcome()?;
        session.run(CONNECT_COMMAND, policy)?.outcome()?;
        Ok(session)
    }

    /// Одна команда — последовательность JSON-массивов до первого с итоговым
    /// сообщением: долгие команды шлют прогресс и журнал отдельными массивами
    /// (замер 15.09.2026: `load-config-from-files` — `progress`, `progress`, …, `success`).
    pub fn run(&mut self, command: &str, policy: &WaitPolicy) -> Result<AgentReply, AgentError> {
        self.send(command)?;
        let mut messages = Vec::new();
        loop {
            let raw = self.read_reply(command, policy)?;
            let batch: Vec<AgentMessage> =
                serde_json::from_slice(&raw).map_err(|error| AgentError::InvalidReply {
                    detail: error.to_string(),
                    head: head_of(&raw),
                })?;
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

    /// Закрывает сессию, не трогая агента: у чужого процесса раннер не хозяин.
    pub fn close(mut self) {
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
    pub fn shutdown(mut self, policy: &WaitPolicy) -> Option<AgentReply> {
        // Ответ на shutdown может и не прийти — сессию закрывает сам агент; ждать его
        // дольше короткого срока незачем, даже если бюджет команды не ограничен.
        let capped = WaitPolicy {
            timeout: Some(
                policy
                    .timeout
                    .map_or(SHUTDOWN_GRACE, |timeout| timeout.min(SHUTDOWN_GRACE)),
            ),
            cancellation: policy.cancellation.clone(),
        };
        // Агент может закрыть соединение, не ответив: EOF здесь — не отказ.
        let reply = self.run(SHUTDOWN_COMMAND, &capped).ok();
        self.disconnect();
        reply
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

    /// Читает канал до первого полного JSON-массива. Всё до открывающей скобки — не
    /// ответ (баннер или приглашение до JSON-режима) и записывается только в журнал.
    fn read_reply(&mut self, command: &str, policy: &WaitPolicy) -> Result<Vec<u8>, AgentError> {
        let started = Instant::now();
        loop {
            if let Some(reply) = self.take_complete_array()? {
                if let Some(log) = self.transcript.as_mut() {
                    let _ = log.write_all(&reply);
                    let _ = log.write_all(b"\n");
                }
                return Ok(reply);
            }
            if self.ended {
                return Err(AgentError::SessionClosed {
                    endpoint: self.endpoint.to_string(),
                    stderr: self.stderr_text(),
                });
            }
            if policy.cancellation.is_cancelled() {
                return Err(AgentError::Cancelled {
                    command: command.to_owned(),
                });
            }
            let wait = match policy.timeout {
                Some(timeout) => {
                    let elapsed = started.elapsed();
                    if elapsed >= timeout {
                        return Err(AgentError::TimedOut {
                            command: command.to_owned(),
                            timeout_ms: timeout.as_millis() as u64,
                        });
                    }
                    (timeout - elapsed).min(WAIT_SLICE)
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
        let Some(start) = self.pending.iter().position(|byte| *byte == b'[') else {
            return Ok(None);
        };
        if start > 0 {
            let skipped = self.pending.drain(..start).collect::<Vec<_>>();
            if let Some(log) = self.transcript.as_mut() {
                let _ = log.write_all(&skipped);
            }
        }
        let mut stream = serde_json::Deserializer::from_slice(&self.pending)
            .into_iter::<serde::de::IgnoredAny>();
        match stream.next() {
            Some(Ok(_)) => {
                let end = stream.byte_offset();
                let reply = self.pending.drain(..end).collect::<Vec<_>>();
                Ok(Some(reply))
            }
            Some(Err(error)) if error.is_eof() => Ok(None),
            Some(Err(error)) => Err(AgentError::InvalidReply {
                detail: error.to_string(),
                head: head_of(&self.pending),
            }),
            None => Ok(None),
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

fn head_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(200).collect()
}

/// Запуск Конфигуратора в агентском режиме. Из ключей базы берётся только адрес:
/// `/N` и `/P` в этом режиме игнорируются, учётные данные идут через SSH.
#[derive(Debug, Clone)]
pub struct AgentLaunch {
    pub v8: PathBuf,
    pub infobase_args: Vec<String>,
    pub port: u16,
    pub host_key: Option<PathBuf>,
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
        args.push(MANAGED_LISTEN_ADDRESS.to_owned());
        match self.host_key.as_ref() {
            Some(key) => {
                args.push("/AgentSSHHostKey".to_owned());
                args.push(key.display().to_string());
            }
            None => args.push("/AgentSSHHostKeyAuto".to_owned()),
        }
        args.push("/AgentBaseDir".to_owned());
        args.push(self.base_dir.display().to_string());
        args
    }

    pub fn endpoint(&self) -> AgentEndpoint {
        AgentEndpoint {
            host: MANAGED_LISTEN_ADDRESS.to_owned(),
            port: self.port,
        }
    }
}

/// Агент, которого раннер поднял сам: процесс и сессия к нему живут вместе.
pub struct ManagedAgent {
    process: Option<crate::platform::process::ManagedSpawnResult>,
    session: Option<AgentSession>,
    base_dir: PathBuf,
}

impl ManagedAgent {
    /// Поднимает процесс и ждёт, пока он примет аутентифицированную сессию. Пока порт
    /// не принимает соединений, агент ещё поднимается; любой другой отказ повтором
    /// не лечится и останавливает ожидание сразу.
    pub fn launch(
        runner: &dyn ProcessRunner,
        launch: &AgentLaunch,
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
            .spawn_managed(&request, ManagedSpawnMode::Wait)
            .map_err(AgentError::Launch)?;
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
                Err(error) => {
                    process.terminate();
                    return Err(error);
                }
            };
            if policy.cancellation.is_cancelled() {
                process.terminate();
                return Err(AgentError::Cancelled {
                    command: "open".to_owned(),
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
        })
    }

    pub fn session(&mut self) -> &mut AgentSession {
        self.session.as_mut().expect("session lives with the agent")
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
            let grace = crate::platform::process::ProcessExecutionPolicy {
                timeout: Some(Duration::from_secs(15)),
                cancellation: CancellationToken::new(),
                safety: crate::platform::process::ProcessInterruptionSafety::Interruptible,
                graceful_shutdown_timeout: Duration::from_millis(250),
            };
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

    /// Живой зонд SFTP шлюза: `V8_GATE_PROBE=host:port V8_GATE_USER=u V8_GATE_PASSWORD=p
    /// cargo test -- --ignored gate_sftp_probe --nocapture`.
    #[test]
    #[ignore]
    fn gate_sftp_probe() {
        let Ok(endpoint) = std::env::var("V8_GATE_PROBE") else {
            return;
        };
        let (host, port) = endpoint.rsplit_once(':').expect("host:port");
        let request = AgentSessionRequest {
            endpoint: AgentEndpoint {
                host: host.to_owned(),
                port: port.parse().expect("port"),
            },
            user: std::env::var("V8_GATE_USER").unwrap_or_default(),
            password: std::env::var("V8_GATE_PASSWORD").unwrap_or_default(),
            transcript_log: None,
        };
        let wait = WaitPolicy {
            timeout: Some(Duration::from_secs(60)),
            cancellation: CancellationToken::new(),
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
        let _ = session.run("common disconnect-ib", &wait);
        session.close();
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

    #[test]
    fn launch_args_carry_only_the_agent_keys_and_the_infobase_address() {
        let launch = AgentLaunch {
            v8: PathBuf::from("/opt/1cv8/1cv8"),
            infobase_args: vec!["/F".to_owned(), "/tmp/ib".to_owned()],
            port: 1543,
            host_key: None,
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
                "/AgentSSHHostKeyAuto",
                "/AgentBaseDir",
                "/work/agent",
            ]
        );
    }

    #[test]
    fn a_dead_endpoint_is_unreachable_not_a_handshake_failure() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let request = AgentSessionRequest {
            endpoint: AgentEndpoint {
                host: "127.0.0.1".to_owned(),
                port,
            },
            user: String::new(),
            password: String::new(),
            transcript_log: None,
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
