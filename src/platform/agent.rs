//! Агентский shell Конфигуратора через системный `ssh`.
//!
//! Раннер не носит SSH-клиента в себе: сессию открывает `ssh -T` без псевдотерминала,
//! пароль отдаёт через `SSH_ASKPASS` (форсированный `SSH_ASKPASS_REQUIRE=force`), а
//! команды пишет по одной в открытый stdin. Первая команда переводит агент в JSON
//! без приглашения; после неё каждый ответ — один JSON-массив, и границу ответа даёт
//! сам разбор, а не поиск приглашения.
//!
//! Решение по ответу принимается по `type` и закрытому множеству `error-type`;
//! `message` переносится как улика.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::platform::process::{ManagedSpawnMode, ProcessRequest, ProcessRunner};

/// Первая команда любой сессии: без неё ответы — проза с приглашением.
pub const JSON_MODE_COMMAND: &str = "options set --show-prompt=no --output-format=json";
/// Команда, которой управляемый агент завершает работу.
pub const SHUTDOWN_COMMAND: &str = "common shutdown";
/// Адрес, который слушает управляемый агент: он живёт на машине раннера.
pub const MANAGED_LISTEN_ADDRESS: &str = "127.0.0.1";
/// Файл карты пользовательских каталогов в `AgentBaseDir`.
pub const BASE_DIR_MAP_FILE: &str = "agentbasedir.json";
/// Имя переменной, из которой askpass-скрипт читает пароль: сам пароль в файл не
/// попадает.
const PASSWORD_ENV: &str = "V8_RUNNER_AGENT_PASSWORD";
const READ_CHUNK: usize = 4096;
const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Тип сообщения агента по документации (Приложение 4, 4.7.8).
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

impl AgentReply {
    /// Итог команды: успех с телом или типизированный отказ. Прогресс и журнал итогом
    /// не являются; вопрос агента — отказ, раннер на вопросы не отвечает.
    pub fn outcome(&self) -> Result<Option<&serde_json::Value>, AgentError> {
        let terminal = self.messages.iter().rev().find(|message| {
            matches!(
                message.kind,
                AgentMessageType::Success
                    | AgentMessageType::Error
                    | AgentMessageType::Canceled
                    | AgentMessageType::Question
            )
        });
        match terminal {
            Some(message) if message.kind == AgentMessageType::Success => Ok(message.body.as_ref()),
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
    #[error("ssh client could not be started ({ssh}): {source}")]
    SshSpawn {
        ssh: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("agent at {endpoint} is unreachable: {source}")]
    Unreachable {
        endpoint: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "agent at {endpoint} accepted the connection, but the session ended before a reply (ssh exit {exit_code:?}); ssh said: {stderr}"
    )]
    SessionClosed {
        endpoint: String,
        exit_code: Option<i32>,
        stderr: String,
    },

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

    #[error("failed to write the agent session stdin: {0}")]
    Stdin(#[source] std::io::Error),

    #[error("failed to prepare the agent workspace '{path}': {source}")]
    Workspace {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

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

/// Как открыть сессию: клиент, точка входа, учётные данные и рабочий каталог для
/// askpass-скрипта. Пустая пара — законные учётные данные базы без пользователей.
#[derive(Debug, Clone)]
pub struct AgentSessionRequest {
    pub ssh: PathBuf,
    pub endpoint: AgentEndpoint,
    pub user: String,
    pub password: String,
    pub askpass_dir: PathBuf,
    pub transcript_log: Option<PathBuf>,
}

/// Аргументы `ssh` для сессии агента. Хост-ключ агента не проверяется: он либо
/// сгенерирован платформой на этой же машине, либо назван пользователем в `attach`.
pub fn ssh_args(endpoint: &AgentEndpoint, user: &str) -> Vec<String> {
    #[cfg(windows)]
    let known_hosts = "UserKnownHostsFile=NUL";
    #[cfg(not(windows))]
    let known_hosts = "UserKnownHostsFile=/dev/null";
    vec![
        "-T".to_owned(),
        "-o".to_owned(),
        "StrictHostKeyChecking=no".to_owned(),
        "-o".to_owned(),
        known_hosts.to_owned(),
        "-o".to_owned(),
        "LogLevel=ERROR".to_owned(),
        "-o".to_owned(),
        "PreferredAuthentications=password,keyboard-interactive".to_owned(),
        "-o".to_owned(),
        "NumberOfPasswordPrompts=1".to_owned(),
        "-p".to_owned(),
        endpoint.port.to_string(),
        "-l".to_owned(),
        user.to_owned(),
        endpoint.host.clone(),
    ]
}

/// Открытая сессия: живой `ssh`, читатель ответов и журнал stderr.
pub struct AgentSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    stderr: Arc<Mutex<Vec<u8>>>,
    endpoint: AgentEndpoint,
    transcript: Option<std::fs::File>,
    ended: bool,
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

impl AgentSession {
    /// Открывает сессию и переводит её в машинный режим. Успех означает, что
    /// аутентификация прошла и агент ответил JSON, — этим и доказывается готовность.
    pub fn open(request: &AgentSessionRequest, policy: &WaitPolicy) -> Result<Self, AgentError> {
        let askpass = write_askpass_script(&request.askpass_dir)?;
        let mut command = Command::new(&request.ssh);
        command
            .args(ssh_args(&request.endpoint, &request.user))
            .env("SSH_ASKPASS", &askpass)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(PASSWORD_ENV, &request.password)
            .env_remove("SSH_AUTH_SOCK")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // `SSH_ASKPASS_REQUIRE=force` не смотрит на DISPLAY, но старые клиенты без
        // переменной молча спрашивают у терминала — пусть DISPLAY будет непустым.
        if std::env::var_os("DISPLAY").is_none() {
            command.env("DISPLAY", "none:0");
        }
        debug!(endpoint = %request.endpoint, user = request.user.as_str(), "opening agent session");
        let mut child = command.spawn().map_err(|source| AgentError::SshSpawn {
            ssh: request.ssh.clone(),
            source,
        })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            let mut stdout = stdout;
            let mut chunk = [0u8; READ_CHUNK];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if sender.send(chunk[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stderr_sink = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut stderr_pipe = stderr_pipe;
            let mut chunk = [0u8; READ_CHUNK];
            loop {
                match stderr_pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if let Ok(mut sink) = stderr_sink.lock() {
                            sink.extend_from_slice(&chunk[..read]);
                        }
                    }
                }
            }
        });
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
            child,
            stdin,
            stdout: receiver,
            pending: Vec::new(),
            stderr,
            endpoint: request.endpoint.clone(),
            transcript,
            ended: false,
        };
        let reply = session.run(JSON_MODE_COMMAND, policy)?;
        reply.outcome()?;
        Ok(session)
    }

    /// Одна команда — один ответ-массив.
    pub fn run(&mut self, command: &str, policy: &WaitPolicy) -> Result<AgentReply, AgentError> {
        self.send(command)?;
        let raw = self.read_reply(command, policy)?;
        let messages: Vec<AgentMessage> =
            serde_json::from_slice(&raw).map_err(|error| AgentError::InvalidReply {
                detail: error.to_string(),
                head: head_of(&raw),
            })?;
        let reply = AgentReply { messages };
        debug!(command, messages = reply.messages.len(), "agent replied");
        Ok(reply)
    }

    /// Закрывает сессию, не трогая агента: у чужого процесса раннер не хозяин.
    pub fn close(mut self) {
        self.stdin.take();
        self.ended = true;
        let _ = self.child.wait();
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
        let reply = self.run(SHUTDOWN_COMMAND, &capped).ok();
        self.stdin.take();
        self.ended = true;
        let _ = self.child.wait();
        reply
    }

    fn send(&mut self, command: &str) -> Result<(), AgentError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| AgentError::SessionClosed {
                endpoint: self.endpoint.to_string(),
                exit_code: None,
                stderr: "stdin is already closed".to_owned(),
            })?;
        if let Some(log) = self.transcript.as_mut() {
            let _ = writeln!(log, "> {command}");
        }
        stdin
            .write_all(command.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
            .map_err(AgentError::Stdin)
    }

    /// Читает поток до первого полного JSON-массива. Всё до открывающей скобки — не
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
                return Err(self.closed_error());
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
                    (timeout - elapsed).min(Duration::from_millis(200))
                }
                None => Duration::from_millis(200),
            };
            match self.stdout.recv_timeout(wait) {
                Ok(chunk) => self.pending.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                }
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

    fn closed_error(&mut self) -> AgentError {
        let exit_code = self.child.wait().ok().and_then(|status| status.code());
        let stderr = self
            .stderr
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .unwrap_or_default();
        AgentError::SessionClosed {
            endpoint: self.endpoint.to_string(),
            exit_code,
            stderr,
        }
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        self.stdin.take();
        if !self.ended {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Точка входа принимает TCP-соединение. Это единственный структурный признак
/// «там кто-то слушает»: ошибку соединения типизирует ОС, а `ssh` любую свою неудачу
/// — от недоступного хоста до отвергнутого пароля — сообщает одним кодом 255 и
/// прозой, на которую раннер решений не принимает.
pub fn probe_reachable(endpoint: &AgentEndpoint, timeout: Duration) -> Result<(), AgentError> {
    let address = format!("{endpoint}");
    let resolved = std::net::ToSocketAddrs::to_socket_addrs(&address)
        .and_then(|mut addresses| {
            addresses.next().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no address resolved")
            })
        })
        .map_err(|source| AgentError::Unreachable {
            endpoint: address.clone(),
            source,
        })?;
    std::net::TcpStream::connect_timeout(&resolved, timeout)
        .map(drop)
        .map_err(|source| AgentError::Unreachable {
            endpoint: address,
            source,
        })
}

fn head_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(200).collect()
}

/// Пишет askpass-скрипт, который отдаёт пароль из окружения: файл с паролем на диске
/// не появляется, а сам скрипт одинаков для любой сессии.
fn write_askpass_script(dir: &Path) -> Result<PathBuf, AgentError> {
    std::fs::create_dir_all(dir).map_err(|source| AgentError::Workspace {
        path: dir.to_path_buf(),
        source,
    })?;
    #[cfg(windows)]
    let (name, body) = (
        "askpass.cmd",
        format!("@echo off\r\necho %{PASSWORD_ENV}%\r\n"),
    );
    #[cfg(not(windows))]
    let (name, body) = (
        "askpass.sh",
        format!("#!/bin/sh\nprintf '%s\\n' \"${PASSWORD_ENV}\"\n"),
    );
    let path = dir.join(name);
    std::fs::write(&path, body).map_err(|source| AgentError::Workspace {
        path: path.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| AgentError::Workspace {
                path: path.clone(),
                source,
            },
        )?;
    }
    Ok(path)
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
    /// Поднимает процесс и ждёт, пока он примет аутентифицированную сессию. Отказ в
    /// доступе останавливает ожидание сразу: повтор с теми же учётными данными не
    /// поможет; «не дозвонились» — повод ждать дальше до срока.
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

        // Пока порт не принимает соединений, агент ещё поднимается; как только принял,
        // сессия открывается один раз — отказ после этого повтором не лечится.
        let started = Instant::now();
        let mut last;
        loop {
            match probe_reachable(&session.endpoint, PROBE_TIMEOUT) {
                Ok(()) => break,
                Err(error) => last = error.to_string(),
            }
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
        }
        let opened = match AgentSession::open(&session, policy) {
            Ok(opened) => opened,
            Err(error) => {
                process.terminate();
                return Err(error);
            }
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
    fn ssh_args_request_no_pty_and_pass_an_empty_login_verbatim() {
        let args = ssh_args(
            &AgentEndpoint {
                host: "127.0.0.1".to_owned(),
                port: 2222,
            },
            "",
        );
        assert_eq!(args[0], "-T");
        let login = args.iter().position(|arg| arg == "-l").expect("-l");
        assert_eq!(args[login + 1], "");
        assert_eq!(args[args.len() - 1], "127.0.0.1");
    }

    #[test]
    fn reachability_is_decided_by_the_socket_not_by_ssh_prose() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let endpoint = AgentEndpoint {
            host: "127.0.0.1".to_owned(),
            port,
        };
        assert!(probe_reachable(&endpoint, Duration::from_secs(1)).is_ok());
        drop(listener);
        assert!(matches!(
            probe_reachable(&endpoint, Duration::from_secs(1)),
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
