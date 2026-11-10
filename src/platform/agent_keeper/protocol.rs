//! Протокол сторожа: построчный JSON по каналу. Запрос — одна строка, ответ — поток кадров, по
//! строке на кадр. Одно подключение несёт один запрос: так разрыв подключения посреди `run`
//! однозначно значит «команда ушла». Сессия команды (`begin` … `end`) держит своё подключение
//! открытым всё время сессии: его разрыв значит, что команда ушла, не закрыв сессии.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::platform::agent::{AgentError, AgentErrorType, AgentMessage};

/// Держатель живого агента (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Holder {
    /// Процесс: держит, пока жив процесс с этим pid и этим временем старта
    /// ([`ProcessFacts::started`](crate::platform::service::ProcessFacts::started)).
    Process { pid: u32, started: u64 },
    /// Срок: держит до `until_ms` (миллисекунды эпохи UNIX, часы реального времени). `id`
    /// отличает держателей со сроком: повторный `hold` того же `id` продлевает срок.
    Ttl { id: String, until_ms: u64 },
}

impl std::fmt::Display for Holder {
    /// Держатель для людей: в отказе занятой базы и в ответе `agent status`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Process { pid, .. } => write!(f, "process {pid}"),
            Self::Ttl { id, .. } => write!(f, "'{id}' with a term"),
        }
    }
}

impl Holder {
    /// Тот же ли это держатель: процесс — по pid и времени старта, срок — по `id`.
    pub fn same(&self, other: &Holder) -> bool {
        match (self, other) {
            (
                Self::Process { pid, started },
                Self::Process {
                    pid: other_pid,
                    started: other_started,
                },
            ) => pid == other_pid && started == other_started,
            (Self::Ttl { id, .. }, Self::Ttl { id: other, .. }) => id == other,
            _ => false,
        }
    }
}

/// Запрос к сторожу.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Начало сессии команды: сторож подключает агента к базе (`common connect-ib`) и отвечает
    /// [`Frame::Begun`]. Подключение остаётся открытым, пока сессия идёт: строка
    /// [`Request::End`] по нему отключает агента от базы (`common disconnect-ib`) и отвечает
    /// [`Frame::Ended`]; разрыв подключения или любая другая строка значат, что команда ушла, и
    /// сторож отключает агента сам. `credentials` —
    /// [`credentials_digest`](super::credentials_digest).
    Begin {
        credentials: String,
    },
    /// Конец сессии команды: только по подключению, которое открыло её `begin`.
    End,
    /// Команда агенту в открытой сессии `session`. `critical` — команда меняет базу: уход
    /// клиента её не прерывает. Пока ответа нет, любая строка или разрыв от клиента значат,
    /// что он ушёл (клиент пишет [`Request::Cancel`], прежде чем уйти).
    Run {
        session: String,
        command: String,
        critical: bool,
        credentials: String,
    },
    /// Клиент бросает команду, ответа на которую ждёт: строка по подключению `run`.
    Cancel,
    Hold {
        holder: Holder,
    },
    Release {
        holder: Holder,
    },
    Status,
    /// Сторож снимает агента и выходит. `only_idle` — только если агент не выполняет команду и
    /// сессии команды нет, `only_unheld` — только если у него нет держателей. Условие
    /// проверяется под замком состояния сторожа, поэтому между проверкой и снятием команда
    /// сессию не откроет; не выполнено — отказ [`KeeperError::NotIdle`].
    Stop {
        #[serde(default)]
        only_idle: bool,
        #[serde(default)]
        only_unheld: bool,
    },
}

/// Состояние сторожа, как он его отвечает на `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperStatus {
    pub keeper_pid: u32,
    pub agent_pid: Option<u32>,
    /// Отпечаток ключа хоста, на котором закреплена сессия сторожа.
    pub fingerprint: Option<String>,
    /// Команда, которую агент выполняет сейчас; «занят» знает только сторож.
    pub busy: Option<String>,
    /// Агент подключён к базе: идёт сессия команды. Без неё агент базу не держит.
    #[serde(default)]
    pub connected: bool,
    /// Последняя команда открытой сессии: её называет отказ занятой базы между командами.
    #[serde(default)]
    pub last: Option<String>,
    pub holders: Vec<Holder>,
    /// Сторож уже решил снять агента: после сессии, которая идёт, агент не останется.
    #[serde(default)]
    pub closing: bool,
    /// Сколько агент простаивает: без команды и без сессии; занятый — ноль.
    #[serde(default)]
    pub idle_ms: u64,
}

impl KeeperStatus {
    /// Держит ли агент базу: выполняет команду или подключён к ней сессией команды.
    pub fn holds_the_infobase(&self) -> bool {
        self.connected || self.busy.is_some()
    }

    /// Команда, которой агент занят: выполняемая или последняя в открытой сессии.
    pub fn command(&self) -> Option<&str> {
        self.busy.as_deref().or(self.last.as_deref())
    }
}

/// Кадр ответа.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum Frame {
    /// Один JSON-массив ответа агента, уже с маской секретов.
    Batch {
        messages: Vec<AgentMessage>,
    },
    /// Конец ответа агента: итог читают по сообщениям.
    Done,
    Error {
        error: KeeperError,
    },
    Status {
        status: KeeperStatus,
    },
    /// Ответ на `hold` и `release`: держатели после запроса.
    Holders {
        holders: Vec<Holder>,
    },
    /// Сессия команды открыта: агент подключён к базе.
    Begun {
        session: String,
    },
    /// Сессия команды закрыта: агент отключён от базы.
    Ended,
    /// Сторож снял агента и выходит.
    Stopped,
}

impl Frame {
    /// Кадр, после которого ответ на запрос окончен.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Batch { .. })
    }
}

/// Отказ сторожа в виде, который переживает канал: закрытый перечень, по которому решают.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum KeeperError {
    /// Агент не поднялся.
    Launch { failure: AgentFailure },
    /// Учётные данные команды не те, с которыми открыта сессия: сторож снимает агента.
    CredentialsMismatch,
    /// Сторож снимает агента и новых сессий не берёт.
    Closing,
    /// Сессии, которую назвал `run`, нет.
    NoSession,
    /// Сессию другой команды ещё не закрыли.
    Busy { command: Option<String> },
    /// Процесса держателя нет или он не тот: держателем он быть не может.
    HolderGone,
    /// `stop` с условием: агент выполняет команду, подключён сессией команды или его держат.
    NotIdle {
        command: Option<String>,
        holders: Vec<Holder>,
    },
    /// Отказ сессии агента.
    Agent { failure: AgentFailure },
    /// Запрос не читается.
    Protocol { message: String },
}

/// Отказ агентского пути, перенесённый через канал. Варианты повторяют [`AgentError`] там, где
/// по ним решают или где от них зависит код ответа: команда получает тот же код, что и с
/// разовым агентом.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentFailure {
    PortTaken {
        port: u16,
        detail: String,
    },
    NoFreePort {
        detail: String,
    },
    Unreachable {
        endpoint: String,
        detail: String,
    },
    AuthenticationRejected {
        endpoint: String,
        user: String,
    },
    HostKeyRejected {
        endpoint: String,
        expected: String,
        presented: String,
    },
    ForeignAgentOnPort {
        endpoint: String,
        expected: String,
        presented: String,
    },
    StartupTimedOut {
        timeout_ms: u64,
        last: String,
    },
    ExitedBeforeSession {
        status: String,
        out: String,
    },
    HostKeyPathNotReadable {
        tried: Vec<PathBuf>,
    },
    Workspace {
        path: PathBuf,
        detail: String,
    },
    SessionClosed {
        endpoint: String,
        stderr: String,
    },
    InvalidReply {
        detail: String,
        head: String,
    },
    NoTerminalMessage {
        count: usize,
    },
    TimedOut {
        command: String,
        timeout_ms: u64,
    },
    Cancelled {
        command: String,
        delivered: bool,
    },
    Command {
        error_type: String,
        message: String,
    },
    Canceled {
        message: String,
    },
    Question {
        message: String,
    },
    /// Отказ без своего варианта: решать по нему нечего, код — «среда недоступна».
    Other {
        message: String,
    },
}

impl AgentFailure {
    /// Отказ агента в виде для канала.
    pub fn from_agent(error: AgentError) -> Self {
        let message = error.to_string();
        match error {
            AgentError::PortTaken { port, detail } => Self::PortTaken { port, detail },
            AgentError::NoFreePort { source } => Self::NoFreePort {
                detail: source.to_string(),
            },
            AgentError::Unreachable { endpoint, source } => Self::Unreachable {
                endpoint,
                detail: source.to_string(),
            },
            AgentError::Handshake { endpoint, .. } | AgentError::Channel { endpoint, .. } => {
                Self::Unreachable {
                    endpoint,
                    detail: message,
                }
            }
            // Оборванная отправка — тот же класс, что закрытая сессия: отказ платформы.
            AgentError::Transport { endpoint, .. } => Self::SessionClosed {
                endpoint,
                stderr: message,
            },
            AgentError::AuthenticationRejected { endpoint, user } => {
                Self::AuthenticationRejected { endpoint, user }
            }
            AgentError::HostKeyRejected {
                endpoint,
                expected,
                presented,
            } => Self::HostKeyRejected {
                endpoint,
                expected,
                presented,
            },
            AgentError::ForeignAgentOnPort {
                endpoint,
                expected,
                presented,
            } => Self::ForeignAgentOnPort {
                endpoint,
                expected,
                presented,
            },
            AgentError::StartupTimedOut { timeout_ms, last } => {
                Self::StartupTimedOut { timeout_ms, last }
            }
            AgentError::ExitedBeforeSession { status, out } => {
                Self::ExitedBeforeSession { status, out }
            }
            AgentError::HostKeyPathNotReadable { tried } => Self::HostKeyPathNotReadable { tried },
            AgentError::Workspace { path, source } => Self::Workspace {
                path,
                detail: source.to_string(),
            },
            AgentError::SessionClosed { endpoint, stderr } => {
                Self::SessionClosed { endpoint, stderr }
            }
            AgentError::InvalidReply { detail, head } => Self::InvalidReply { detail, head },
            AgentError::NoTerminalMessage { count } => Self::NoTerminalMessage { count },
            AgentError::TimedOut {
                command,
                timeout_ms,
            } => Self::TimedOut {
                command,
                timeout_ms,
            },
            AgentError::Cancelled { command, delivered } => Self::Cancelled { command, delivered },
            AgentError::Command {
                error_type,
                message,
            } => Self::Command {
                error_type: error_type.as_str().to_owned(),
                message,
            },
            AgentError::Canceled { message } => Self::Canceled { message },
            AgentError::Question { message } => Self::Question { message },
            AgentError::NotSent { .. }
            | AgentError::Exchange { .. }
            | AgentError::UnsafeEntryName { .. }
            | AgentError::Launch(_)
            | AgentError::LiveAgentGone { .. }
            | AgentError::Keeper { .. }
            | AgentError::KeeperUnreachable { .. }
            | AgentError::UserDirUnknown { .. } => Self::Other { message },
        }
    }

    /// Отказ агента на стороне команды.
    pub fn into_agent(self) -> AgentError {
        match self {
            Self::PortTaken { port, detail } => AgentError::PortTaken { port, detail },
            Self::NoFreePort { detail } => AgentError::NoFreePort {
                source: std::io::Error::other(detail),
            },
            Self::Unreachable { endpoint, detail } => AgentError::Unreachable {
                endpoint,
                source: std::io::Error::other(detail),
            },
            Self::AuthenticationRejected { endpoint, user } => {
                AgentError::AuthenticationRejected { endpoint, user }
            }
            Self::HostKeyRejected {
                endpoint,
                expected,
                presented,
            } => AgentError::HostKeyRejected {
                endpoint,
                expected,
                presented,
            },
            Self::ForeignAgentOnPort {
                endpoint,
                expected,
                presented,
            } => AgentError::ForeignAgentOnPort {
                endpoint,
                expected,
                presented,
            },
            Self::StartupTimedOut { timeout_ms, last } => {
                AgentError::StartupTimedOut { timeout_ms, last }
            }
            Self::ExitedBeforeSession { status, out } => {
                AgentError::ExitedBeforeSession { status, out }
            }
            Self::HostKeyPathNotReadable { tried } => AgentError::HostKeyPathNotReadable { tried },
            Self::Workspace { path, detail } => AgentError::Workspace {
                path,
                source: std::io::Error::other(detail),
            },
            Self::SessionClosed { endpoint, stderr } => {
                AgentError::SessionClosed { endpoint, stderr }
            }
            Self::InvalidReply { detail, head } => AgentError::InvalidReply { detail, head },
            Self::NoTerminalMessage { count } => AgentError::NoTerminalMessage { count },
            Self::TimedOut {
                command,
                timeout_ms,
            } => AgentError::TimedOut {
                command,
                timeout_ms,
            },
            Self::Cancelled { command, delivered } => AgentError::Cancelled { command, delivered },
            Self::Command {
                error_type,
                message,
            } => AgentError::Command {
                error_type: AgentErrorType::parse(&error_type),
                message,
            },
            Self::Canceled { message } => AgentError::Canceled { message },
            Self::Question { message } => AgentError::Question { message },
            Self::Other { message } => AgentError::Keeper { detail: message },
        }
    }
}

impl KeeperError {
    /// Отказ сессии агента в виде для канала.
    pub fn from_agent(error: AgentError) -> Self {
        Self::Agent {
            failure: AgentFailure::from_agent(error),
        }
    }

    /// Отказ сторожа отказом агентского пути на стороне команды.
    pub fn into_agent(self) -> AgentError {
        match self {
            Self::Launch { failure } | Self::Agent { failure } => failure.into_agent(),
            Self::CredentialsMismatch => AgentError::LiveAgentGone {
                detail: "the live agent was opened with other credentials and is removed"
                    .to_owned(),
            },
            Self::Closing => AgentError::LiveAgentGone {
                detail: "the live agent is being removed".to_owned(),
            },
            Self::NoSession => AgentError::LiveAgentGone {
                detail: "the live agent has no session of this command".to_owned(),
            },
            Self::Busy { command } => AgentError::Keeper {
                detail: format!(
                    "the live agent still serves another command{}",
                    command.map_or_else(String::new, |command| format!(": {command}"))
                ),
            },
            Self::HolderGone => AgentError::Keeper {
                detail: "the holder process is not running".to_owned(),
            },
            Self::NotIdle { command, .. } => AgentError::Keeper {
                detail: format!(
                    "the live agent is in use{}",
                    command.map_or_else(String::new, |command| format!(": {command}"))
                ),
            },
            Self::Protocol { message } => AgentError::Keeper { detail: message },
        }
    }
}

/// Предел строки протокола: длиннее строк ни сторож, ни клиент не читают.
pub const LINE_LIMIT: u64 = 1024 * 1024;

/// Пишет значение одной строкой JSON и сбрасывает буфер.
pub async fn write_line<W, T>(writer: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin + ?Sized,
    T: Serialize,
{
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Читает одну строку JSON не длиннее [`LINE_LIMIT`]; `None` — канал закрыт до строки.
pub async fn read_line<R, T>(reader: &mut R) -> std::io::Result<Option<T>>
where
    R: AsyncBufRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut line = Vec::new();
    let read = (&mut *reader)
        .take(LINE_LIMIT)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') && read as u64 >= LINE_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("a protocol line is longer than {LINE_LIMIT} bytes"),
        ));
    }
    serde_json::from_slice(line.trim_ascii_end())
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_and_a_frame_travel_as_one_line_of_json() {
        let request = Request::Run {
            session: "s".to_owned(),
            command: "config dump-config-to-files --dir \"a\nb\"".to_owned(),
            critical: true,
            credentials: "00".to_owned(),
        };
        let line = serde_json::to_string(&request).unwrap();
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains("\"op\":\"run\""), "{line}");
        let frame = Frame::Error {
            error: KeeperError::CredentialsMismatch,
        };
        let line = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            line,
            r#"{"frame":"error","error":{"error":"credentials_mismatch"}}"#
        );
    }

    #[test]
    fn a_holder_is_the_same_by_its_identity_not_by_its_term() {
        let process = Holder::Process { pid: 7, started: 1 };
        assert!(process.same(&Holder::Process { pid: 7, started: 1 }));
        assert!(!process.same(&Holder::Process { pid: 7, started: 2 }));
        let ttl = Holder::Ttl {
            id: "ci".to_owned(),
            until_ms: 1,
        };
        assert!(ttl.same(&Holder::Ttl {
            id: "ci".to_owned(),
            until_ms: 9
        }));
        assert!(!ttl.same(&process));
    }

    /// Отказ агента переживает канал тем же видом: код ответа команды не зависит от того,
    /// живой агент или разовый.
    #[test]
    fn an_agent_failure_keeps_its_kind_across_the_channel() {
        let cases = [
            AgentError::PortTaken {
                port: 7,
                detail: "taken".to_owned(),
            },
            AgentError::AuthenticationRejected {
                endpoint: "127.0.0.1:7".to_owned(),
                user: "u".to_owned(),
            },
            AgentError::StartupTimedOut {
                timeout_ms: 5,
                last: "x".to_owned(),
            },
            AgentError::HostKeyPathNotReadable {
                tried: vec![PathBuf::from("a")],
            },
            AgentError::ExitedBeforeSession {
                status: "1".to_owned(),
                out: "o".to_owned(),
            },
            AgentError::InvalidReply {
                detail: "d".to_owned(),
                head: "h".to_owned(),
            },
            AgentError::Command {
                error_type: AgentErrorType::InfoBaseNotFound,
                message: "m".to_owned(),
            },
        ];
        for error in cases {
            let shown = error.to_string();
            let kind = std::mem::discriminant(&error);
            let line = serde_json::to_string(&KeeperError::from_agent(error)).unwrap();
            let back = serde_json::from_str::<KeeperError>(&line)
                .unwrap()
                .into_agent();
            assert_eq!(std::mem::discriminant(&back), kind, "{line}");
            assert_eq!(back.to_string(), shown);
        }
    }

    /// Строка длиннее предела не читается: сторож не копит её в памяти.
    #[test]
    fn a_line_over_the_limit_is_refused() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let long = vec![b'a'; usize::try_from(LINE_LIMIT).unwrap() + 10];
        let outcome = runtime.block_on(async {
            let mut reader = tokio::io::BufReader::new(long.as_slice());
            read_line::<_, serde_json::Value>(&mut reader).await
        });
        assert_eq!(
            outcome.err().map(|error| error.kind()),
            Some(std::io::ErrorKind::InvalidData)
        );
    }
}
