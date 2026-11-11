use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::next_step::NextStep;
use crate::domain::platforms::PlatformReleaseRefusal;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Stable machine-readable code for MCP-facing service failures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpErrorCode {
    InvalidArgument,
    UnsupportedValue,
    RuntimeFailure,
    PlatformFailure,
    Internal,
}

/// High-level business error class surfaced by the MCP service layer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpBusinessErrorKind {
    Validation,
    Runtime,
    Platform,
}

impl From<UseCaseErrorKind> for McpBusinessErrorKind {
    fn from(value: UseCaseErrorKind) -> Self {
        match value {
            UseCaseErrorKind::Capability(_) => Self::Runtime,
            UseCaseErrorKind::Environment(_)
            | UseCaseErrorKind::WorkspaceBusy
            | UseCaseErrorKind::InfobaseBusy
            | UseCaseErrorKind::InfobaseSessions
            | UseCaseErrorKind::InfobaseDesignerBusy
            | UseCaseErrorKind::NonFastForward
            | UseCaseErrorKind::UntrackedBase
            | UseCaseErrorKind::AgentLimit
            | UseCaseErrorKind::LiveAgentOff
            | UseCaseErrorKind::LiveAgentJob => Self::Runtime,
            UseCaseErrorKind::InvalidOutput
            | UseCaseErrorKind::Cancelled(_)
            | UseCaseErrorKind::TimedOut => Self::Platform,
            UseCaseErrorKind::Validation
            | UseCaseErrorKind::SourceSetIgnoredByGit
            | UseCaseErrorKind::WorkPathOnAnotherVolume => Self::Validation,
            UseCaseErrorKind::Runtime => Self::Runtime,
            UseCaseErrorKind::Platform => Self::Platform,
        }
    }
}

/// Structured business error metadata returned by the MCP service layer.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct McpBusinessError {
    pub code: McpErrorCode,
    pub kind: McpBusinessErrorKind,
    pub message: String,
    /// Шаг, которым вызывающий выходит из отказа. Род и код у MCP свои и уже, а шаг —
    /// тот же самый: он про предмет, а не про транспорт.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<NextStep>,
    /// Формат исходников и опись установок отказа выпуска: код у MCP — `runtime_failure`,
    /// а поле то же, что у CLI (`INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_release: Option<Box<PlatformReleaseRefusal>>,
}

impl McpBusinessError {
    /// Maps a use-case error into a stable MCP-facing business error.
    pub fn from_use_case(error: &UseCaseError) -> Self {
        let kind = error.kind();
        let code = match kind {
            UseCaseErrorKind::Capability(_) => McpErrorCode::RuntimeFailure,
            UseCaseErrorKind::Environment(_)
            | UseCaseErrorKind::WorkspaceBusy
            | UseCaseErrorKind::InfobaseBusy
            | UseCaseErrorKind::InfobaseSessions
            | UseCaseErrorKind::InfobaseDesignerBusy
            | UseCaseErrorKind::NonFastForward
            | UseCaseErrorKind::UntrackedBase
            | UseCaseErrorKind::AgentLimit
            | UseCaseErrorKind::LiveAgentOff
            | UseCaseErrorKind::LiveAgentJob => McpErrorCode::RuntimeFailure,
            UseCaseErrorKind::InvalidOutput
            | UseCaseErrorKind::Cancelled(_)
            | UseCaseErrorKind::TimedOut => McpErrorCode::PlatformFailure,
            UseCaseErrorKind::Validation
            | UseCaseErrorKind::SourceSetIgnoredByGit
            | UseCaseErrorKind::WorkPathOnAnotherVolume => McpErrorCode::InvalidArgument,
            UseCaseErrorKind::Runtime => McpErrorCode::RuntimeFailure,
            UseCaseErrorKind::Platform => McpErrorCode::PlatformFailure,
        };
        Self {
            code,
            kind: kind.into(),
            message: error.message().to_owned(),
            next: error.next().cloned(),
            platform_release: error.platform_release().cloned().map(Box::new),
        }
    }
}

/// Structured business failure with a failure-shaped MCP response payload.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct McpBusinessFailure<T> {
    pub error: McpBusinessError,
    pub response: T,
}

impl<T> McpBusinessFailure<T> {
    /// Creates a new business failure for the specified response payload.
    pub fn new(error: McpBusinessError, response: T) -> Self {
        Self { error, response }
    }
}

/// Non-business service error that must not be surfaced as a business response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct McpInternalError {
    pub code: McpErrorCode,
    pub message: String,
}

impl McpInternalError {
    /// Creates a new internal MCP service error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: McpErrorCode::Internal,
            message: message.into(),
        }
    }
}

/// Top-level MCP service failure contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServiceError<T> {
    Business(McpBusinessFailure<T>),
    Internal(McpInternalError),
}

/// Top-level MCP service result contract.
pub type McpServiceResult<T> = Result<T, McpServiceError<T>>;

impl From<McpErrorCode> for crate::command_envelope::ErrorCode {
    /// У MCP свой словарь кодов; на проводе он остаётся подмножеством одного закрытого
    /// набора, поэтому переводится, а не печатается строкой.
    fn from(value: McpErrorCode) -> Self {
        match value {
            McpErrorCode::InvalidArgument => Self::InvalidArgument,
            McpErrorCode::UnsupportedValue => Self::UnsupportedValue,
            McpErrorCode::RuntimeFailure => Self::RuntimeFailure,
            McpErrorCode::PlatformFailure => Self::PlatformFailure,
            // До конверта этот код не доходит: внутренняя ошибка отвечает не ответом
            // команды, а ошибкой протокола. Ветка есть ради полноты перевода.
            McpErrorCode::Internal => Self::RuntimeFailure,
        }
    }
}

impl From<McpBusinessErrorKind> for crate::command_envelope::ErrorKind {
    fn from(value: McpBusinessErrorKind) -> Self {
        match value {
            McpBusinessErrorKind::Validation => Self::Validation,
            McpBusinessErrorKind::Runtime => Self::Runtime,
            McpBusinessErrorKind::Platform => Self::Platform,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{McpBusinessError, McpBusinessErrorKind, McpErrorCode};
    use crate::platform::locator::{LocatorError, UtilityType};
    use crate::support::error::AppError;
    use crate::use_cases::result::{EnvironmentReason, UseCaseError, UseCaseErrorKind};

    /// У MCP нет рода `environment`: отсутствующая утилита отвечает `runtime_failure`, а не
    /// `platform_failure` (INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE).
    #[test]
    fn a_missing_utility_answers_a_runtime_failure_over_mcp() {
        let error = UseCaseError::from(AppError::from(LocatorError::NotFound {
            utility: UtilityType::Ibcmd,
            detail: None,
        }));
        assert_eq!(
            error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable),
            "{error}"
        );

        let mcp = McpBusinessError::from_use_case(&error);

        assert_eq!(mcp.code, McpErrorCode::RuntimeFailure);
        assert_eq!(mcp.kind, McpBusinessErrorKind::Runtime);
        assert_eq!(mcp.message, error.message());
    }

    /// Нет выпуска формата исходников: у MCP код тот же `runtime_failure`, а формат и опись
    /// установок едут тем же полем, что в конверте CLI.
    #[test]
    fn a_missing_release_answers_a_runtime_failure_with_its_inventory_over_mcp() {
        use crate::domain::platforms::SourceFormatInfo;
        use crate::platform::locator::{PlatformReleaseMissing, RequiredRelease};

        let error = UseCaseError::from(AppError::from(LocatorError::PlatformReleaseMissing(
            Box::new(PlatformReleaseMissing {
                required: RequiredRelease {
                    releases: Vec::new(),
                    source_format: SourceFormatInfo {
                        version: Some("2.21".to_owned()),
                        file: "/project/src/Configuration.xml".into(),
                        release: Some("8.5.1".to_owned()),
                    },
                    unreadable: None,
                },
                installations: Vec::new(),
            }),
        )));
        assert_eq!(
            error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::PlatformReleaseMissing)
        );

        let mcp = McpBusinessError::from_use_case(&error);

        assert_eq!(mcp.code, McpErrorCode::RuntimeFailure);
        assert_eq!(mcp.kind, McpBusinessErrorKind::Runtime);
        assert_eq!(
            mcp.platform_release
                .as_ref()
                .and_then(|release| release.source_format.release.as_deref()),
            Some("8.5.1")
        );
    }
}
