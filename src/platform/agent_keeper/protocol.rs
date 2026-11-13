//! Протокол сторожа: построчный JSON по каналу. Запрос — одна строка, ответ — поток кадров, по
//! строке на кадр. Одно подключение несёт один запрос: так разрыв подключения посреди `run`
//! однозначно значит «команда ушла». Сессия команды (`begin` … `end`) держит своё подключение
//! открытым всё время сессии: его разрыв значит, что команда ушла, не закрыв сессии.
//!
//! Подключение агента к базе живёт дольше сессии команды (#644,
//! `INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`): `begin` подключённого агента
//! `common connect-ib` не шлёт, `end` агента не отключает. Отключает его `disconnect`, сторож при
//! уходе команды без `end` и снятие агента.

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
    /// Начало сессии команды: сторож подключает агента к базе (`common connect-ib`), если тот
    /// ещё не подключён, и отвечает [`Frame::Begun`]. Подключение канала остаётся открытым, пока
    /// сессия идёт: строка [`Request::End`] по нему закрывает сессию — агент остаётся
    /// подключённым к базе — и отвечает [`Frame::Ended`]; разрыв подключения или любая другая
    /// строка значат, что команда ушла, и сторож отключает агента от базы сам. `credentials` —
    /// [`credentials_digest`](super::credentials_digest).
    Begin {
        credentials: String,
    },
    /// Конец сессии команды: только по подключению, которое открыло её `begin`.
    End,
    /// Подключает агента к базе без сессии команды и отвечает [`Frame::Attached`]: `push`
    /// подключает его, пока идёт разбор (#644). Пока агент подключается, он занят (`busy`):
    /// отключение по `status` его не выбирает, а условный `stop` не снимает.
    Attach {
        credentials: String,
    },
    /// Отключает агента от базы (`common disconnect-ib`), сам агент остаётся: так команда,
    /// которой база нужна монопольно, снимает его подключение. Агент выполняет команду или
    /// открыта сессия команды — отказ [`KeeperError::NotIdle`]. Ответ — [`Frame::Disconnected`].
    Disconnect,
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
    /// Ставит держателя. `credentials` — [`credentials_digest`](super::credentials_digest)
    /// команды: не тот, с которым поднят агент, — отказ [`KeeperError::CredentialsMismatch`], и
    /// сторож снимает агента, как при `begin`. Без поля (клиент прежней версии) не сверяется.
    Hold {
        holder: Holder,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<String>,
    },
    Release {
        holder: Holder,
    },
    Status,
    /// Сторож снимает агента и выходит. `only_idle` — только если агент не выполняет команду и
    /// сессии команды нет, `only_unheld` — только если у него нет держателей. Условие
    /// проверяется под замком состояния сторожа, поэтому между проверкой и снятием команда
    /// сессию не откроет; не выполнено — отказ [`KeeperError::NotIdle`]. Прежняя форма без
    /// полей (`{"op":"stop"}`) — безусловный `stop`.
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
    /// Идёт сессия команды.
    #[serde(default)]
    pub connected: bool,
    /// Агент подключён к базе: в сессии команды или между командами (#644). Сторож прежней
    /// сборки поля не шлёт: его агент между командами к базе не подключён.
    #[serde(default)]
    pub attached: bool,
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
    /// Агент свежий: его ещё не взяли ни `begin`, ни `hold`, и условный `stop` его не снимает.
    #[serde(default)]
    pub fresh: bool,
}

impl KeeperStatus {
    /// Занят ли агент базой: выполняет команду или идёт сессия команды. Агент, подключённый к
    /// базе между командами, не занят: его подключение снимает `disconnect`.
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
    /// Сессия команды открыта: агент подключён к базе. `messages` — ответ агента на
    /// подключение, уже с маской секретов: команда пишет его в свой журнал разговора, как с
    /// разовым агентом. Сторож прежней версии его не шлёт.
    Begun {
        session: String,
        #[serde(default)]
        messages: Vec<AgentMessage>,
    },
    /// Сессия команды закрыта; агент остаётся подключённым к базе.
    Ended,
    /// Агент подключён к базе без сессии команды (`attach`). `messages` — ответ агента на
    /// подключение; пусто — он уже был подключён.
    Attached {
        #[serde(default)]
        messages: Vec<AgentMessage>,
    },
    /// Ответ на `disconnect`: `was` — был ли агент подключён к базе.
    Disconnected {
        was: bool,
    },
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
    NotSent {
        command: String,
    },
    Abandoned {
        command: String,
    },
    Exchange {
        path: String,
        detail: String,
    },
    UnsafeEntryName {
        path: String,
        detail: String,
    },
    UserDirUnknown {
        base_dir: PathBuf,
        user: String,
        detail: String,
    },
    /// Отказ без своего варианта: отказ хранителя, код — «среда недоступна», слой
    /// `agent_start`. Так идут только отказы подъёма и самого хранителя.
    ///
    /// Перечень закрыт, как у [`KeeperError`]: вид, которого клиент не знает, кадр не читает, и
    /// клиент поступает с ним как с ушедшим агентом. Сторож другой сборки команде не служит —
    /// версия и штамп сборки входят в ключ живого агента.
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
            // Сегодня сторож этих отказов не порождает: он служит своему агенту, работу команды
            // не отмечает, а обмен файлами и каталог пользователя агента живут на стороне
            // команды. Свой вид, а не `Other`, — чтобы и тогда, когда породит, род ответа не
            // зависел от того, живой агент или разовый.
            AgentError::NotSent { command } => Self::NotSent { command },
            AgentError::Abandoned { command } => Self::Abandoned { command },
            AgentError::Exchange { path, detail } => Self::Exchange { path, detail },
            AgentError::UnsafeEntryName { path, detail } => Self::UnsafeEntryName { path, detail },
            AgentError::UserDirUnknown {
                base_dir,
                user,
                detail,
            } => Self::UserDirUnknown {
                base_dir,
                user,
                detail,
            },
            // Отказы подъёма и самого хранителя: на стороне команды — отказ хранителя.
            AgentError::Launch(_)
            | AgentError::LiveAgentGone { .. }
            | AgentError::Keeper { .. }
            | AgentError::KeeperUnreachable { .. } => Self::Other { message },
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
            Self::NotSent { command } => AgentError::NotSent { command },
            Self::Abandoned { command } => AgentError::Abandoned { command },
            Self::Exchange { path, detail } => AgentError::Exchange { path, detail },
            Self::UnsafeEntryName { path, detail } => AgentError::UnsafeEntryName { path, detail },
            Self::UserDirUnknown {
                base_dir,
                user,
                detail,
            } => AgentError::UserDirUnknown {
                base_dir,
                user,
                detail,
            },
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

    /// Сторож понимает запросы клиента прежней версии: `stop` без условий и `hold` без
    /// отпечатка учётных данных.
    #[test]
    fn the_former_stop_and_hold_forms_are_read() {
        let stop: Request = serde_json::from_str(r#"{"op":"stop"}"#).unwrap();
        assert!(matches!(
            stop,
            Request::Stop {
                only_idle: false,
                only_unheld: false
            }
        ));
        let hold: Request = serde_json::from_str(
            r#"{"op":"hold","holder":{"kind":"process","pid":7,"started":1}}"#,
        )
        .unwrap();
        assert!(matches!(
            hold,
            Request::Hold {
                credentials: None,
                ..
            }
        ));
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
            AgentError::NotSent {
                command: "dump".to_owned(),
            },
            AgentError::Abandoned {
                command: "dump".to_owned(),
            },
            AgentError::Exchange {
                path: "p".to_owned(),
                detail: "d".to_owned(),
            },
            AgentError::UnsafeEntryName {
                path: "p".to_owned(),
                detail: "d".to_owned(),
            },
            AgentError::UserDirUnknown {
                base_dir: PathBuf::from("b"),
                user: "u".to_owned(),
                detail: "d".to_owned(),
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

    /// Номер вида отказа агента. Совпадение без `_`: новый вид не компилируется, пока ему не
    /// дан номер, а тест ниже требует, чтобы перечень назвал каждый номер.
    const fn variant_index(error: &AgentError) -> usize {
        match error {
            AgentError::Unreachable { .. } => 0,
            AgentError::Handshake { .. } => 1,
            AgentError::AuthenticationRejected { .. } => 2,
            AgentError::HostKeyRejected { .. } => 3,
            AgentError::Channel { .. } => 4,
            AgentError::Transport { .. } => 5,
            AgentError::SessionClosed { .. } => 6,
            AgentError::TimedOut { .. } => 7,
            AgentError::Cancelled { .. } => 8,
            AgentError::Abandoned { .. } => 9,
            AgentError::NotSent { .. } => 10,
            AgentError::InvalidReply { .. } => 11,
            AgentError::NoTerminalMessage { .. } => 12,
            AgentError::Command { .. } => 13,
            AgentError::Canceled { .. } => 14,
            AgentError::Question { .. } => 15,
            AgentError::HostKeyPathNotReadable { .. } => 16,
            AgentError::Workspace { .. } => 17,
            AgentError::Exchange { .. } => 18,
            AgentError::UnsafeEntryName { .. } => 19,
            AgentError::Launch(_) => 20,
            AgentError::PortTaken { .. } => 21,
            AgentError::NoFreePort { .. } => 22,
            AgentError::ForeignAgentOnPort { .. } => 23,
            AgentError::StartupTimedOut { .. } => 24,
            AgentError::ExitedBeforeSession { .. } => 25,
            AgentError::LiveAgentGone { .. } => 26,
            AgentError::Keeper { .. } => 27,
            AgentError::KeeperUnreachable { .. } => 28,
            AgentError::UserDirUnknown { .. } => 29,
        }
    }
    const AGENT_ERROR_VARIANTS: usize = 30;

    /// Каждый вид отказа агента, один раз.
    fn every_agent_error() -> Vec<AgentError> {
        let io = || std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let endpoint = || "127.0.0.1:7".to_owned();
        vec![
            AgentError::Unreachable {
                endpoint: endpoint(),
                source: io(),
            },
            AgentError::Handshake {
                endpoint: endpoint(),
                source: russh::Error::Disconnect,
            },
            AgentError::AuthenticationRejected {
                endpoint: endpoint(),
                user: "u".to_owned(),
            },
            AgentError::HostKeyRejected {
                endpoint: endpoint(),
                expected: "a".to_owned(),
                presented: "b".to_owned(),
            },
            AgentError::Channel {
                endpoint: endpoint(),
                source: russh::Error::Disconnect,
            },
            AgentError::Transport {
                endpoint: endpoint(),
                source: russh::Error::Disconnect,
            },
            AgentError::SessionClosed {
                endpoint: endpoint(),
                stderr: String::new(),
            },
            AgentError::TimedOut {
                command: "c".to_owned(),
                timeout_ms: 5,
            },
            AgentError::Cancelled {
                command: "c".to_owned(),
                delivered: true,
            },
            AgentError::Abandoned {
                command: "c".to_owned(),
            },
            AgentError::NotSent {
                command: "c".to_owned(),
            },
            AgentError::InvalidReply {
                detail: "d".to_owned(),
                head: "h".to_owned(),
            },
            AgentError::NoTerminalMessage { count: 1 },
            AgentError::Command {
                error_type: AgentErrorType::InfoBaseNotFound,
                message: "m".to_owned(),
            },
            AgentError::Canceled {
                message: "m".to_owned(),
            },
            AgentError::Question {
                message: "m".to_owned(),
            },
            AgentError::HostKeyPathNotReadable {
                tried: vec![PathBuf::from("a")],
            },
            AgentError::Workspace {
                path: PathBuf::from("w"),
                source: io(),
            },
            AgentError::Exchange {
                path: "p".to_owned(),
                detail: "d".to_owned(),
            },
            AgentError::UnsafeEntryName {
                path: "p".to_owned(),
                detail: "d".to_owned(),
            },
            AgentError::Launch(crate::platform::process::ProcessError::SpawnFailed {
                cmd: "1cv8".to_owned(),
                source: io(),
            }),
            AgentError::PortTaken {
                port: 7,
                detail: "taken".to_owned(),
            },
            AgentError::NoFreePort { source: io() },
            AgentError::ForeignAgentOnPort {
                endpoint: endpoint(),
                expected: "a".to_owned(),
                presented: "b".to_owned(),
            },
            AgentError::StartupTimedOut {
                timeout_ms: 5,
                last: "x".to_owned(),
            },
            AgentError::ExitedBeforeSession {
                status: "1".to_owned(),
                out: "o".to_owned(),
            },
            AgentError::LiveAgentGone {
                detail: "gone".to_owned(),
            },
            AgentError::Keeper {
                detail: "k".to_owned(),
            },
            AgentError::KeeperUnreachable {
                channel: "pipe".to_owned(),
                source: io(),
            },
            AgentError::UserDirUnknown {
                base_dir: PathBuf::from("b"),
                user: "u".to_owned(),
                detail: "d".to_owned(),
            },
        ]
    }

    /// Слой отказа окружения переживает канал хранителя живого агента: живой агент или
    /// разовый — `error.layer` тот же (`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`).
    /// Род переживает канал тоже, у каждого вида без исключений: код ответа команды не зависит
    /// от того, живой агент или разовый.
    #[test]
    fn an_environment_layer_and_a_kind_survive_the_keeper_channel() {
        use crate::support::error::AppError;
        use crate::use_cases::result::UseCaseError;

        let through_channel = |error: AgentError| {
            let line = serde_json::to_string(&KeeperError::from_agent(error)).unwrap();
            serde_json::from_str::<KeeperError>(&line)
                .unwrap()
                .into_agent()
        };
        let mut named: Vec<usize> = every_agent_error().iter().map(variant_index).collect();
        named.sort_unstable();
        named.dedup();
        assert_eq!(
            named,
            (0..AGENT_ERROR_VARIANTS).collect::<Vec<_>>(),
            "the list names every kind of agent failure once"
        );
        let mut layered = 0;
        for (direct, sent) in every_agent_error().into_iter().zip(every_agent_error()) {
            let shown = direct.to_string();
            let direct = UseCaseError::from(AppError::from(direct));
            let back = UseCaseError::from(AppError::from(through_channel(sent)));
            assert_eq!(back.kind(), direct.kind(), "{shown}");
            assert_eq!(
                back.environment_layer(),
                direct.environment_layer(),
                "{shown}"
            );
            layered += usize::from(direct.environment_layer().is_some());
        }
        assert_eq!(
            layered, 15,
            "every environment refusal of the agent crossed the channel"
        );
    }

    /// Отказ подъёма и самого хранителя идёт как `other` и читается отказом хранителя с тем же
    /// текстом.
    #[test]
    fn a_launch_or_keeper_failure_travels_as_other_with_its_text() {
        let line = r#"{"kind":"other","message":"agent command 'dump' was not sent: the command was cancelled"}"#;
        let failure = serde_json::from_str::<AgentFailure>(line).expect("read");
        assert!(matches!(
            failure.into_agent(),
            AgentError::Keeper { detail }
                if detail == "agent command 'dump' was not sent: the command was cancelled"
        ));
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
