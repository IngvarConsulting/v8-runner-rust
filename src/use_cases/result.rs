use std::fmt;

use crate::domain::next_step::NextStep;
use crate::support::error::{AppError, CapabilityReason};

const VALIDATION_EXIT_CODE: i32 = 2;
const RUNTIME_EXIT_CODE: i32 = 3;
const PLATFORM_EXIT_CODE: i32 = 4;

/// Stable use-case error class used by transport adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseCaseErrorKind {
    /// Операция здесь не выполняется; причина уточняет, почему именно.
    Capability(CapabilityReason),
    Environment,
    WorkspaceBusy,
    InvalidOutput,
    Cancelled,
    TimedOut,
    Validation,
    Runtime,
    Platform,
}

impl UseCaseErrorKind {
    /// Maps the error kind to the CLI exit code.
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Capability(_) => VALIDATION_EXIT_CODE,
            Self::Environment => VALIDATION_EXIT_CODE,
            Self::WorkspaceBusy => RUNTIME_EXIT_CODE,
            Self::InvalidOutput | Self::Cancelled | Self::TimedOut => PLATFORM_EXIT_CODE,
            Self::Validation => VALIDATION_EXIT_CODE,
            Self::Runtime => RUNTIME_EXIT_CODE,
            Self::Platform => PLATFORM_EXIT_CODE,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Capability(_) => "capability unavailable",
            Self::Environment => "environment unavailable",
            Self::WorkspaceBusy => "workspace busy",
            Self::InvalidOutput => "invalid output",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed out",
            Self::Validation => "validation error",
            Self::Runtime => "runtime error",
            Self::Platform => "platform error",
        }
    }
}

/// Transport-neutral error metadata returned by use cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCaseError {
    kind: UseCaseErrorKind,
    message: String,
    /// Шаг, которым вызывающий выходит из отказа. Живёт здесь и только здесь: конверт его
    /// печатает, а транспорт не выдумывает.
    next: Option<Box<NextStep>>,
}

impl UseCaseError {
    /// Creates a new use-case error.
    pub fn new(kind: UseCaseErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            next: None,
        }
    }

    /// Называет шаг, которым вызывающий выходит из отказа.
    #[must_use]
    pub fn with_next(mut self, next: NextStep) -> Self {
        self.next = Some(Box::new(next));
        self
    }

    /// Шаг, названный отказом, если он есть.
    pub fn next(&self) -> Option<&NextStep> {
        self.next.as_deref()
    }

    /// Returns the error kind.
    pub const fn kind(&self) -> UseCaseErrorKind {
        self.kind
    }

    /// Returns the message without the prefixed kind label.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the CLI exit code associated with this error kind.
    pub const fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }
}

impl fmt::Display for UseCaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.message)
    }
}

impl From<AppError> for UseCaseError {
    fn from(value: AppError) -> Self {
        match value {
            AppError::CapabilityUnavailable(refusal) => Self::new(
                UseCaseErrorKind::Capability(refusal.reason),
                refusal.message,
            ),
            AppError::EnvironmentUnavailable(message) => {
                Self::new(UseCaseErrorKind::Environment, message)
            }
            AppError::WorkspaceBusy(message) => Self::new(UseCaseErrorKind::WorkspaceBusy, message),
            AppError::Cancelled(message) => Self::new(UseCaseErrorKind::Cancelled, message),
            AppError::TimedOut(message) => Self::new(UseCaseErrorKind::TimedOut, message),
            AppError::InvalidOutput(message) => Self::new(UseCaseErrorKind::InvalidOutput, message),
            AppError::Validation(message) => Self::new(UseCaseErrorKind::Validation, message),
            AppError::ValidationIbcmd(error) => {
                Self::new(UseCaseErrorKind::Validation, error.to_string())
            }
            AppError::ValidationIbcmdContext { context, source } => {
                Self::new(UseCaseErrorKind::Validation, format!("{context}; {source}"))
            }
            AppError::Runtime(message) => Self::new(UseCaseErrorKind::Runtime, message),
            AppError::Platform(message) => Self::new(UseCaseErrorKind::Platform, message),
            AppError::PlatformDesigner(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformDesignerContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformLocator(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformProcess(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformLocatorContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformProcessContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformEdt(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformEdtContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformEdtSession(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformEdtSessionContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::Config(error) => Self::new(UseCaseErrorKind::Validation, error.to_string()),
            AppError::ConfigContext { context, source } => {
                Self::new(UseCaseErrorKind::Validation, format!("{context}; {source}"))
            }
        }
    }
}

/// A failed use-case execution with structured payload and transport-neutral error metadata.
#[derive(Debug, Clone)]
pub struct UseCaseFailure<T> {
    pub error: UseCaseError,
    pub payload: Option<T>,
}

impl<T> UseCaseFailure<T> {
    /// Creates a failure that should still be rendered as a structured command payload.
    pub fn with_payload(error: impl Into<UseCaseError>, payload: T) -> Self {
        Self {
            error: error.into(),
            payload: Some(payload),
        }
    }

    /// Creates a failure that should not emit a structured command payload.
    pub fn without_payload(error: impl Into<UseCaseError>) -> Self {
        Self {
            error: error.into(),
            payload: None,
        }
    }
}

/// The transport-neutral result contract for use-case execution.
pub type UseCaseResult<T> = Result<T, UseCaseFailure<T>>;

/// Полезная нагрузка любого исхода: сам результат или тот, что несёт отказ.
pub(crate) fn payload_mut<T>(outcome: &mut UseCaseResult<T>) -> Option<&mut T> {
    match outcome {
        Ok(result) => Some(result),
        Err(failure) => failure.payload.as_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::{UseCaseError, UseCaseErrorKind};
    use crate::config::loader::ConfigLoadError;
    use crate::platform::designer::DesignerError;
    use crate::platform::edt_session::EdtSessionError;
    use crate::platform::ibcmd::IbcmdError;
    use crate::platform::process::ProcessError;
    use crate::support::error::{AppError, CapabilityReason};

    #[test]
    fn use_case_error_kinds_keep_stable_cli_exit_codes() {
        for reason in [
            CapabilityReason::Unavailable,
            CapabilityReason::Subject,
            CapabilityReason::Target,
            CapabilityReason::Soon,
        ] {
            assert_eq!(
                UseCaseErrorKind::Capability(reason).exit_code(),
                2,
                "{reason:?}"
            );
        }
        assert_eq!(UseCaseErrorKind::Environment.exit_code(), 2);
        assert_eq!(UseCaseErrorKind::WorkspaceBusy.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::InvalidOutput.exit_code(), 4);
        assert_eq!(UseCaseErrorKind::Cancelled.exit_code(), 4);
        assert_eq!(UseCaseErrorKind::TimedOut.exit_code(), 4);
        assert_eq!(UseCaseErrorKind::Validation.exit_code(), 2);
        assert_eq!(UseCaseErrorKind::Runtime.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::Platform.exit_code(), 4);
    }

    #[test]
    fn process_app_error_normalizes_only_at_adapter_boundary() {
        let error = UseCaseError::from(AppError::PlatformProcess(ProcessError::SpawnFailed {
            cmd: "1cv8c ENTERPRISE".to_owned(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing binary"),
        }));

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert_eq!(error.exit_code(), 4);
        assert!(error.message().contains("failed to spawn process"));
        assert!(error.message().contains("1cv8c ENTERPRISE"));
    }

    #[test]
    fn generic_process_timeout_keeps_the_existing_platform_error_kind() {
        let error = UseCaseError::from(AppError::PlatformProcess(ProcessError::TimedOut {
            cmd: "1cv8 DESIGNER".to_owned(),
            timeout_ms: 100,
        }));

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
    }

    #[test]
    fn contextual_config_errors_stay_validation_errors() {
        let app_error = AppError::Config(ConfigLoadError::NotFound("v8project.yaml".to_owned()))
            .with_context("failed to load project config");
        assert!(matches!(app_error, AppError::ConfigContext { .. }));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(error.message().contains("failed to load project config"));
        assert!(error.message().contains("v8project.yaml"));
    }

    #[test]
    fn contextual_edt_session_errors_stay_platform_errors() {
        let app_error = AppError::from(EdtSessionError::QueueFull)
            .with_context("failed to acquire EDT session");
        assert!(matches!(
            app_error,
            AppError::PlatformEdtSessionContext { .. }
        ));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert!(error.message().contains("failed to acquire EDT session"));
        assert!(error.message().contains("shared EDT queue is full"));
    }

    #[test]
    fn contextual_ibcmd_validation_errors_keep_typed_source() {
        let app_error = AppError::from(IbcmdError::MissingServerDbmsField("kind"))
            .with_context("failed to build ibcmd connection");
        assert!(matches!(app_error, AppError::ValidationIbcmdContext { .. }));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(error.message().contains("failed to build ibcmd connection"));
        assert!(error.message().contains("infobase.dbms.kind"));
    }

    #[test]
    fn contextual_designer_errors_keep_typed_platform_source() {
        let app_error = AppError::from(DesignerError::UtilityNotFound("1cv8".to_owned()))
            .with_context("failed to resolve designer utility");
        assert!(matches!(
            app_error,
            AppError::PlatformDesignerContext { .. }
        ));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert!(error
            .message()
            .contains("failed to resolve designer utility"));
        assert!(error.message().contains("designer utility not found"));
    }
}
