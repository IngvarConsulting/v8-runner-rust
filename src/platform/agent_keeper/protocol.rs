//! Протокол сторожа: построчный JSON по каналу. Запрос — одна строка, ответ — поток кадров, по
//! строке на кадр. Одно подключение несёт один запрос: так разрыв подключения посреди `run`
//! однозначно значит «команда ушла».

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::platform::agent::{AgentError, AgentMessage};

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
    /// Команда агенту. `critical` — команда меняет базу: разрыв подключения её не прерывает.
    /// `credentials` — [`credentials_digest`](super::credentials_digest) учётных данных команды.
    Run {
        command: String,
        critical: bool,
        credentials: String,
    },
    Hold {
        holder: Holder,
    },
    Release {
        holder: Holder,
    },
    Status,
    Stop,
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
    pub holders: Vec<Holder>,
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
    /// Сторож снял агента и выходит.
    Stopped,
}

impl Frame {
    /// Кадр, после которого ответ на запрос окончен.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Batch { .. })
    }
}

/// Отказ сторожа в виде, который переживает канал.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum KeeperError {
    /// Агент не поднялся: текст отказа запуска.
    Launch { message: String },
    /// Учётные данные команды не те, с которыми открыта сессия: сторож снимает агента.
    CredentialsMismatch,
    /// Сторож снимает агента и новых запросов не берёт.
    Closing,
    /// Сессия агента кончилась без ответа.
    SessionClosed { endpoint: String, stderr: String },
    /// Ответ агента не читается как массив сообщений.
    InvalidReply { detail: String, head: String },
    /// Другой отказ сессии агента: его текст.
    Agent { message: String },
    /// Запрос не читается.
    Protocol { message: String },
}

impl KeeperError {
    /// Отказ сессии агента в виде для канала: то, по чему решает команда, остаётся типом.
    pub fn from_agent(error: AgentError) -> Self {
        match error {
            AgentError::SessionClosed { endpoint, stderr } => {
                Self::SessionClosed { endpoint, stderr }
            }
            AgentError::InvalidReply { detail, head } => Self::InvalidReply { detail, head },
            other => Self::Agent {
                message: other.to_string(),
            },
        }
    }

    /// Отказ сторожа отказом агентского пути на стороне команды.
    pub fn into_agent(self) -> AgentError {
        match self {
            Self::SessionClosed { endpoint, stderr } => {
                AgentError::SessionClosed { endpoint, stderr }
            }
            Self::InvalidReply { detail, head } => AgentError::InvalidReply { detail, head },
            Self::CredentialsMismatch => AgentError::LiveAgentGone {
                detail: "the live agent was opened with other credentials and is removed"
                    .to_owned(),
            },
            Self::Closing => AgentError::LiveAgentGone {
                detail: "the live agent is being removed".to_owned(),
            },
            Self::Launch { message } => AgentError::Keeper {
                detail: format!("the live agent did not start: {message}"),
            },
            Self::Agent { message } | Self::Protocol { message } => {
                AgentError::Keeper { detail: message }
            }
        }
    }
}

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

/// Читает одну строку JSON; `None` — канал закрыт до строки.
pub async fn read_line<R, T>(reader: &mut R) -> std::io::Result<Option<T>>
where
    R: AsyncBufRead + Unpin + ?Sized,
    T: for<'de> Deserialize<'de>,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_and_a_frame_travel_as_one_line_of_json() {
        let request = Request::Run {
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
}
