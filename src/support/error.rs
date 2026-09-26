use crate::config::loader::ConfigLoadError;
use crate::platform::agent::AgentError;
use crate::platform::designer::DesignerError;
use crate::platform::download::DownloadError;
use crate::platform::edt::EdtError;
use crate::platform::edt_session::EdtSessionError;
use crate::platform::enterprise::EnterpriseError;
use crate::platform::ibcmd::IbcmdError;
use crate::platform::interactive::InteractiveProcessError;
use crate::platform::locator::LocatorError;
use crate::platform::process::ProcessError;
use thiserror::Error;

/// Почему операция здесь не выполняется. Род отказа один — `capability`, — а причина
/// различает случаи, которые сайт называет разными словами: предмет не тот навсегда, цель
/// не та, или раннер пока не умеет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityReason {
    /// Операция здесь не выполняется, и точнее сказать нечего.
    Unavailable,
    /// Предмет не тот, и другим он не станет.
    #[allow(
        dead_code,
        reason = "код заведён в наборе; производитель приходит со своей задачей"
    )]
    Subject,
    /// Не для этой цели.
    Target,
    /// Пока не умеет.
    Soon,
}

/// Отказ по возможности: причина и текст человеку.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRefusal {
    pub reason: CapabilityReason,
    pub message: String,
}

impl std::fmt::Display for CapabilityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Где команду остановила отмена. Код и род отказа от этого не зависят — отмена одна, — а
/// ответ, который пишет прерывание, называет по нему фазу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelledAt {
    /// На безопасной точке: команда сама проверила отмену между шагами или исполнитель ещё
    /// не получил её работы — процесс не запущен, команда запроса не отправлена. Работа
    /// команды не оборвана.
    Boundary,
    /// Посреди работы исполнителя: запущенный процесс снят или ответ на отправленную
    /// команду брошен.
    Work,
}

impl CancelledAt {
    /// Отмена, заставшая исполнителя: оборвана работа, только если она была передана.
    pub const fn after(delivered: bool) -> Self {
        if delivered {
            Self::Work
        } else {
            Self::Boundary
        }
    }
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("capability unavailable: {0}")]
    CapabilityUnavailable(CapabilityRefusal),

    #[error("environment unavailable: {0}")]
    EnvironmentUnavailable(String),

    #[error("workspace busy: {0}")]
    WorkspaceBusy(String),

    #[error("cancelled: {message}")]
    Cancelled { message: String, at: CancelledAt },

    #[error("timed out: {0}")]
    TimedOut(String),

    #[error("invalid platform output: {0}")]
    InvalidOutput(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("validation error: {0}")]
    ValidationIbcmd(#[source] IbcmdError),

    #[error("validation error: {context}; {source}")]
    ValidationIbcmdContext {
        context: String,
        #[source]
        source: IbcmdError,
    },

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("platform error: {0}")]
    Platform(String),

    #[error("platform error: {0}")]
    PlatformDesigner(#[source] DesignerError),

    #[error("platform error: {context}; {source}")]
    PlatformDesignerContext {
        context: String,
        #[source]
        source: DesignerError,
    },

    #[error("platform error: {0}")]
    PlatformLocator(#[from] LocatorError),

    #[error("platform error: {0}")]
    PlatformProcess(#[from] ProcessError),

    #[error("platform error: {context}; {source}")]
    PlatformLocatorContext {
        context: String,
        #[source]
        source: LocatorError,
    },

    #[error("platform error: {context}; {source}")]
    PlatformProcessContext {
        context: String,
        #[source]
        source: ProcessError,
    },

    #[error("platform error: {0}")]
    PlatformEdt(#[source] EdtError),

    #[error("platform error: {context}; {source}")]
    PlatformEdtContext {
        context: String,
        #[source]
        source: EdtError,
    },

    #[error("platform error: {0}")]
    PlatformEdtSession(#[source] EdtSessionError),

    #[error("platform error: {context}; {source}")]
    PlatformEdtSessionContext {
        context: String,
        #[source]
        source: EdtSessionError,
    },

    #[error(transparent)]
    Config(#[from] ConfigLoadError),

    #[error("validation error: {context}; {source}")]
    ConfigContext {
        context: String,
        #[source]
        source: ConfigLoadError,
    },
}

impl AppError {
    /// Отказ по возможности без уточнения причины.
    pub fn capability(message: impl Into<String>) -> Self {
        Self::capability_for(CapabilityReason::Unavailable, message)
    }

    /// Отказ по возможности с названной причиной: её видит и код отказа в конверте.
    pub fn capability_for(reason: CapabilityReason, message: impl Into<String>) -> Self {
        Self::CapabilityUnavailable(CapabilityRefusal {
            reason,
            message: message.into(),
        })
    }

    /// Отмена ли это и где она остановила команду — по самой ошибке, а не по сигналу:
    /// сигнал, пришедший во время чужого отказа, отказа отменой не делает. Здесь и только
    /// здесь отмена узнаётся, как бы глубоко платформа её ни завернула. Истёкший срок —
    /// не отмена.
    pub fn cancellation(&self) -> Option<CancelledAt> {
        match self {
            Self::Cancelled { at, .. } => Some(*at),
            Self::PlatformProcess(source) | Self::PlatformProcessContext { source, .. } => {
                process_cancellation(source)
            }
            Self::PlatformDesigner(source) | Self::PlatformDesignerContext { source, .. } => {
                match source {
                    DesignerError::Spawn(error) => process_cancellation(error),
                    DesignerError::UtilityNotFound(_) | DesignerError::StaleLogCleanup { .. } => {
                        None
                    }
                }
            }
            Self::ValidationIbcmd(source) | Self::ValidationIbcmdContext { source, .. } => {
                match source {
                    IbcmdError::Spawn(error) => process_cancellation(error),
                    IbcmdError::MissingServerDbmsField(_) => None,
                }
            }
            Self::PlatformEdt(source) | Self::PlatformEdtContext { source, .. } => match source {
                EdtError::Spawn(error) => process_cancellation(error),
                EdtError::Interactive(error) => interactive_cancellation(error),
                EdtError::SharedSession(error) => session_cancellation(error),
                EdtError::PrepareWorkspace { .. } => None,
            },
            Self::PlatformEdtSession(source) | Self::PlatformEdtSessionContext { source, .. } => {
                session_cancellation(source)
            }
            Self::CapabilityUnavailable(_)
            | Self::EnvironmentUnavailable(_)
            | Self::WorkspaceBusy(_)
            | Self::TimedOut(_)
            | Self::InvalidOutput(_)
            | Self::Validation(_)
            | Self::Runtime(_)
            | Self::Platform(_)
            | Self::PlatformLocator(_)
            | Self::PlatformLocatorContext { .. }
            | Self::Config(_)
            | Self::ConfigContext { .. } => None,
        }
    }

    pub fn with_context(self, context: impl Into<String>) -> Self {
        let context = context.into();
        match self {
            // Причина переживает уточнение текста: иначе `target` молча становится общим.
            Self::CapabilityUnavailable(refusal) => {
                Self::CapabilityUnavailable(CapabilityRefusal {
                    reason: refusal.reason,
                    message: format!("{context}; {}", refusal.message),
                })
            }
            Self::EnvironmentUnavailable(message) => {
                Self::EnvironmentUnavailable(format!("{context}; {message}"))
            }
            Self::WorkspaceBusy(message) => Self::WorkspaceBusy(format!("{context}; {message}")),
            Self::Cancelled { message, at } => Self::Cancelled {
                message: format!("{context}; {message}"),
                at,
            },
            Self::TimedOut(message) => Self::TimedOut(format!("{context}; {message}")),
            Self::InvalidOutput(message) => Self::InvalidOutput(format!("{context}; {message}")),
            Self::Validation(message) => Self::Validation(format!("{context}; {message}")),
            Self::ValidationIbcmd(source) => Self::ValidationIbcmdContext { context, source },
            Self::ValidationIbcmdContext {
                context: existing,
                source,
            } => Self::ValidationIbcmdContext {
                context: format!("{context}; {existing}"),
                source,
            },
            Self::Runtime(message) => Self::Runtime(format!("{context}; {message}")),
            Self::Platform(message) => Self::Platform(format!("{context}; {message}")),
            Self::PlatformDesigner(source) => Self::PlatformDesignerContext { context, source },
            Self::PlatformDesignerContext {
                context: existing,
                source,
            } => Self::PlatformDesignerContext {
                context: format!("{context}; {existing}"),
                source,
            },
            Self::PlatformLocator(source) => Self::PlatformLocatorContext { context, source },
            Self::PlatformProcess(source) => Self::PlatformProcessContext { context, source },
            Self::PlatformEdt(source) => Self::PlatformEdtContext { context, source },
            Self::PlatformEdtSession(source) => Self::PlatformEdtSessionContext { context, source },
            Self::PlatformLocatorContext {
                context: existing,
                source,
            } => Self::PlatformLocatorContext {
                context: format!("{context}; {existing}"),
                source,
            },
            Self::PlatformProcessContext {
                context: existing,
                source,
            } => Self::PlatformProcessContext {
                context: format!("{context}; {existing}"),
                source,
            },
            Self::PlatformEdtContext {
                context: existing,
                source,
            } => Self::PlatformEdtContext {
                context: format!("{context}; {existing}"),
                source,
            },
            Self::PlatformEdtSessionContext {
                context: existing,
                source,
            } => Self::PlatformEdtSessionContext {
                context: format!("{context}; {existing}"),
                source,
            },
            Self::Config(source) => Self::ConfigContext { context, source },
            Self::ConfigContext {
                context: existing,
                source,
            } => Self::ConfigContext {
                context: format!("{context}; {existing}"),
                source,
            },
        }
    }
}

fn process_cancellation(error: &ProcessError) -> Option<CancelledAt> {
    match error {
        ProcessError::Cancelled { delivered, .. } => Some(CancelledAt::after(*delivered)),
        ProcessError::SpawnFailed { .. }
        | ProcessError::StartupCheckFailed { .. }
        | ProcessError::ExitedEarly { .. }
        | ProcessError::StdoutLogIo { .. }
        | ProcessError::StderrLogIo { .. }
        | ProcessError::TimedOut { .. }
        | ProcessError::ManagedSpawnUnsupported { .. } => None,
    }
}

fn interactive_cancellation(error: &InteractiveProcessError) -> Option<CancelledAt> {
    match error {
        InteractiveProcessError::CommandCancelled { delivered, .. } => {
            Some(CancelledAt::after(*delivered))
        }
        InteractiveProcessError::SpawnFailed { .. }
        | InteractiveProcessError::MissingStdin { .. }
        | InteractiveProcessError::MissingStdout { .. }
        | InteractiveProcessError::MissingStderr { .. }
        | InteractiveProcessError::StartupTimeout { .. }
        | InteractiveProcessError::CommandTimeout { .. }
        | InteractiveProcessError::ProcessExited { .. }
        | InteractiveProcessError::Poisoned
        | InteractiveProcessError::Terminated
        | InteractiveProcessError::StdinWriteFailed { .. }
        | InteractiveProcessError::StdinFlushFailed { .. }
        | InteractiveProcessError::StreamReadFailed { .. }
        | InteractiveProcessError::WaitFailed { .. }
        | InteractiveProcessError::KillFailed { .. } => None,
    }
}

fn session_cancellation(error: &EdtSessionError) -> Option<CancelledAt> {
    match error {
        EdtSessionError::QueuedCancelled => Some(CancelledAt::Boundary),
        EdtSessionError::RunningCancelled { delivered } => Some(CancelledAt::after(*delivered)),
        EdtSessionError::QueueFull
        | EdtSessionError::QueuedTimeout
        | EdtSessionError::RunningTimeout
        | EdtSessionError::StartupFailed { .. }
        | EdtSessionError::SessionFailed { .. }
        | EdtSessionError::DrainedByRestartOrShutdown { .. }
        | EdtSessionError::InternalFailure { .. } => None,
    }
}

impl From<IbcmdError> for AppError {
    fn from(error: IbcmdError) -> Self {
        match error {
            IbcmdError::MissingServerDbmsField(_) => Self::ValidationIbcmd(error),
            IbcmdError::Spawn(error) => Self::PlatformProcess(error),
        }
    }
}

impl From<DesignerError> for AppError {
    fn from(error: DesignerError) -> Self {
        match error {
            DesignerError::UtilityNotFound(_) | DesignerError::StaleLogCleanup { .. } => {
                Self::PlatformDesigner(error)
            }
            DesignerError::Spawn(error) => Self::PlatformProcess(error),
        }
    }
}

impl From<EdtError> for AppError {
    fn from(error: EdtError) -> Self {
        match error {
            EdtError::Spawn(error) => Self::PlatformProcess(error),
            error @ EdtError::PrepareWorkspace { .. } => Self::PlatformEdt(error),
            error @ EdtError::Interactive(_) => Self::PlatformEdt(error),
            error @ EdtError::SharedSession(_) => Self::PlatformEdt(error),
        }
    }
}

impl From<EdtSessionError> for AppError {
    fn from(error: EdtSessionError) -> Self {
        Self::PlatformEdtSession(error)
    }
}

impl From<EnterpriseError> for AppError {
    fn from(error: EnterpriseError) -> Self {
        match error {
            EnterpriseError::Spawn(error) => Self::PlatformProcess(error),
        }
    }
}

/// Отказы агента раскладываются по родам раннера: среда, срок, отмена, платформа.
impl From<AgentError> for AppError {
    fn from(error: AgentError) -> Self {
        match error {
            AgentError::TimedOut { .. } => Self::TimedOut(error.to_string()),
            AgentError::Cancelled { delivered, .. } => Self::Cancelled {
                message: error.to_string(),
                at: CancelledAt::after(delivered),
            },
            AgentError::Command { .. }
            | AgentError::Canceled { .. }
            | AgentError::Question { .. }
            | AgentError::NoTerminalMessage { .. }
            | AgentError::InvalidReply { .. }
            | AgentError::SessionClosed { .. }
            | AgentError::Transport { .. }
            | AgentError::UserDirUnknown { .. }
            | AgentError::UnsafeEntryName { .. }
            | AgentError::Exchange { .. } => Self::Platform(error.to_string()),
            AgentError::Workspace { .. } => Self::Runtime(error.to_string()),
            AgentError::Unreachable { .. }
            | AgentError::Handshake { .. }
            | AgentError::AuthenticationRejected { .. }
            // Тот же класс, что и отвергнутые учётные данные: сервер ответил, но работать
            // с этой точкой входа как объявлено нельзя.
            | AgentError::HostKeyRejected { .. }
            | AgentError::Channel { .. }
            | AgentError::Launch(_)
            | AgentError::StartupTimedOut { .. } => Self::EnvironmentUnavailable(error.to_string()),
        }
    }
}

/// Отмена загрузки — отмена на границе: исполнителя у загрузки нет, а файлы ложатся на
/// место только после неё, так что оборванной работы она не оставляет. Прочие отказы
/// загрузки — отказы выполнения.
impl From<DownloadError> for AppError {
    fn from(error: DownloadError) -> Self {
        match error {
            DownloadError::Cancelled => Self::Cancelled {
                message: error.to_string(),
                at: CancelledAt::Boundary,
            },
            DownloadError::Client(_)
            | DownloadError::Request { .. }
            | DownloadError::Status { .. }
            | DownloadError::Read { .. }
            | DownloadError::ResponseTooLarge { .. }
            | DownloadError::TimedOut { .. }
            | DownloadError::InvalidUtf8(_)
            | DownloadError::InsecureScheme { .. }
            | DownloadError::UnusableUrl { .. }
            | DownloadError::Runtime(_) => Self::Runtime(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, CancelledAt};
    use crate::platform::agent::AgentError;
    use crate::platform::designer::DesignerError;
    use crate::platform::download::DownloadError;
    use crate::platform::edt::EdtError;
    use crate::platform::edt_session::EdtSessionError;
    use crate::platform::ibcmd::IbcmdError;
    use crate::platform::interactive::InteractiveProcessError;
    use crate::platform::process::ProcessError;

    fn process(delivered: bool) -> ProcessError {
        ProcessError::Cancelled {
            cmd: "1cv8 DESIGNER".to_owned(),
            delivered,
        }
    }

    /// Отмену узнаёт сама ошибка, как глубоко её ни завернули, и называет, где она остановила
    /// команду: оборвана работа исполнителя, только если он её получил.
    #[test]
    fn a_cancellation_is_recognised_through_every_wrapper() {
        for delivered in [true, false] {
            let at = Some(CancelledAt::after(delivered));
            let cases = [
                AppError::PlatformProcess(process(delivered)),
                AppError::PlatformProcess(process(delivered)).with_context("export failed"),
                AppError::from(DesignerError::Spawn(process(delivered))),
                AppError::PlatformDesigner(DesignerError::Spawn(process(delivered))),
                AppError::from(IbcmdError::Spawn(process(delivered))),
                AppError::ValidationIbcmd(IbcmdError::Spawn(process(delivered))),
                AppError::from(EdtError::Spawn(process(delivered))),
                AppError::PlatformEdt(EdtError::Interactive(
                    InteractiveProcessError::CommandCancelled {
                        command: "export".to_owned(),
                        stdout: String::new(),
                        stderr: String::new(),
                        delivered,
                    },
                )),
                AppError::PlatformEdt(EdtError::SharedSession(EdtSessionError::RunningCancelled {
                    delivered,
                })),
                AppError::from(EdtSessionError::RunningCancelled { delivered })
                    .with_context("syntax"),
                AppError::from(AgentError::Cancelled {
                    command: "load-config".to_owned(),
                    delivered,
                }),
            ];
            for error in cases {
                assert_eq!(error.cancellation(), at, "{error:?}");
            }
        }
        for error in [
            AppError::Cancelled {
                message: "safe point".to_owned(),
                at: CancelledAt::Boundary,
            }
            .with_context("dump"),
            AppError::from(EdtSessionError::QueuedCancelled),
            AppError::from(DownloadError::Cancelled),
        ] {
            assert_eq!(
                error.cancellation(),
                Some(CancelledAt::Boundary),
                "{error:?}"
            );
        }
    }

    /// Истёкший срок — не отмена, и прочий отказ, пришедший при ожидающей отмене, — тоже.
    #[test]
    fn a_timeout_or_an_unrelated_failure_is_not_a_cancellation() {
        for error in [
            AppError::PlatformProcess(ProcessError::TimedOut {
                cmd: "1cv8 DESIGNER".to_owned(),
                timeout_ms: 100,
            }),
            AppError::from(AgentError::TimedOut {
                command: "dump-config".to_owned(),
                timeout_ms: 100,
            }),
            AppError::from(EdtSessionError::RunningTimeout),
            AppError::from(EdtSessionError::QueuedTimeout),
            AppError::PlatformEdt(EdtError::Interactive(
                InteractiveProcessError::CommandTimeout {
                    command: "export".to_owned(),
                    timeout_ms: 100,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            )),
            AppError::from(DownloadError::TimedOut { timeout_ms: 100 }),
            AppError::Runtime("publication failed".to_owned()),
            AppError::from(AgentError::Canceled {
                message: "the agent cancelled the command itself".to_owned(),
            }),
        ] {
            assert_eq!(error.cancellation(), None, "{error:?}");
        }
    }
}
