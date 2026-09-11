//! Reads and changes the extension composition of the configured infobase.
//!
//! This is the only family in the runner where the provider choice is settled by the
//! platform rather than by `builder`: Designer has no batch key that reports installed
//! extensions, so every operation here is IBCMD-only and says so when IBCMD is absent.

use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::extensions::{
    ExtensionInventoryResult, ExtensionsResult, ExtensionsStep, InstalledExtension,
};
use crate::platform::extension_inventory::parse_extension_inventory;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::request::{ExtensionInventoryRequest, ExtensionInventoryScope};
use crate::use_cases::result::{UseCaseError, UseCaseFailure, UseCaseResult};
use tracing::debug;

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExtensionInventoryRequest,
) -> UseCaseResult<ExtensionInventoryResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        "executing extension inventory use case"
    );
    let started = Instant::now();
    let connection = IbcmdConnection::from_infobase(&config.infobase)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    let mut utilities = PlatformUtilities::from_config(config);
    let binary = utilities
        .locate(UtilityType::Ibcmd)
        .map(|location| location.path)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    let dsl = IbcmdDsl::new(binary, connection, utilities.runner_for(UtilityType::Ibcmd))
        .with_execution_policy(
            context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        );

    let platform_result = match &request.scope {
        ExtensionInventoryScope::All => dsl.infobase_extension_list(),
        ExtensionInventoryScope::Named { name } => dsl.infobase_extension_info(name),
    }
    .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;

    validate_success(&platform_result).map_err(UseCaseFailure::without_payload)?;
    let extensions =
        read_inventory(&platform_result, request).map_err(UseCaseFailure::without_payload)?;

    Ok(ExtensionInventoryResult {
        ok: true,
        extensions,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn validate_success(result: &PlatformCommandResult) -> Result<(), AppError> {
    if result.process.exit_code == 0 {
        return Ok(());
    }
    let mut details = vec![format!(
        "platform extension read failed with exit code {}",
        result.process.exit_code
    )];
    for (label, value) in [
        ("stdout", result.process.stdout.as_str()),
        ("stderr", result.process.stderr.as_str()),
    ] {
        let value = value.trim();
        if !value.is_empty() {
            details.push(format!("{label}: {value}"));
        }
    }
    Err(AppError::Platform(details.join("; ")))
}

/// Reads the inventory, and for a named request proves the record is the one asked for.
///
/// The platform answers `info --name` with the same record shape as `list`, so without
/// this check a renamed or substituted record would be reported as the requested one.
fn read_inventory(
    result: &PlatformCommandResult,
    request: &ExtensionInventoryRequest,
) -> Result<Vec<InstalledExtension>, AppError> {
    let extensions = parse_extension_inventory(&result.process.stdout).map_err(|error| {
        AppError::InvalidOutput(format!(
            "cannot read the platform extension inventory: {error}"
        ))
    })?;
    if let ExtensionInventoryScope::Named { name } = &request.scope {
        if !extensions
            .iter()
            .any(|extension| extension.name.eq_ignore_ascii_case(name))
        {
            return Err(AppError::InvalidOutput(format!(
                "platform reported an extension inventory without the requested '{name}'"
            )));
        }
    }
    Ok(extensions)
}

/// Change to the extension composition of the infobase.
///
/// Each variant is one platform command; there is no combined "ensure" shape, because a
/// caller that means create and a caller that means update must fail differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionChangeRequest {
    Create {
        name: String,
        name_prefix: String,
        synonym: Option<String>,
        purpose: Option<String>,
    },
    Delete {
        name: String,
    },
    SetActive {
        name: String,
        active: bool,
    },
}

impl ExtensionChangeRequest {
    fn target(&self) -> &str {
        match self {
            Self::Create { name, .. } | Self::Delete { name } | Self::SetActive { name, .. } => {
                name
            }
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Delete { .. } => "delete",
            Self::SetActive { active: true, .. } => "activate",
            Self::SetActive { active: false, .. } => "deactivate",
        }
    }
}

/// Applies one change to the extension composition of the configured infobase.
pub fn change(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExtensionChangeRequest,
) -> UseCaseResult<ExtensionsResult> {
    debug!(
        command = context.command().as_str(),
        action = request.action(),
        "executing extension composition change"
    );
    let started = Instant::now();
    let connection = IbcmdConnection::from_infobase(&config.infobase)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    let mut utilities = PlatformUtilities::from_config(config);
    let binary = utilities
        .locate(UtilityType::Ibcmd)
        .map(|location| location.path)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    let dsl = IbcmdDsl::new(binary, connection, utilities.runner_for(UtilityType::Ibcmd))
        .with_execution_policy(
            context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
        );

    let platform_result = match request {
        ExtensionChangeRequest::Create {
            name,
            name_prefix,
            synonym,
            purpose,
        } => {
            dsl.infobase_extension_create(name, name_prefix, synonym.as_deref(), purpose.as_deref())
        }
        ExtensionChangeRequest::Delete { name } => dsl.infobase_extension_delete(name),
        ExtensionChangeRequest::SetActive { name, active } => {
            dsl.infobase_extension_set_active(name, *active)
        }
    };

    let step_duration = started.elapsed().as_millis() as u64;
    match platform_result
        .map_err(AppError::from)
        .and_then(|result| validate_success(&result))
    {
        Ok(()) => Ok(ExtensionsResult {
            ok: true,
            steps: vec![ExtensionsStep {
                target: request.target().to_owned(),
                action: request.action().to_owned(),
                ok: true,
                message: None,
                duration_ms: step_duration,
            }],
            duration_ms: started.elapsed().as_millis() as u64,
        }),
        Err(error) => {
            let payload = ExtensionsResult {
                ok: false,
                steps: vec![ExtensionsStep {
                    target: request.target().to_owned(),
                    action: request.action().to_owned(),
                    ok: false,
                    message: Some(error.to_string()),
                    duration_ms: step_duration,
                }],
                duration_ms: started.elapsed().as_millis() as u64,
            };
            Err(UseCaseFailure::with_payload(
                UseCaseError::from(error),
                payload,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_inventory;
    use crate::platform::process::ProcessResult;
    use crate::platform::result::PlatformCommandResult;
    use crate::use_cases::request::{ExtensionInventoryRequest, ExtensionInventoryScope};

    fn platform_result(stdout: &str) -> PlatformCommandResult {
        PlatformCommandResult {
            process: ProcessResult {
                exit_code: 0,
                stdout: stdout.to_owned(),
                stderr: String::new(),
                interruption: None,
            },
            platform_log_path: None,
            platform_log: None,
            platform_log_read_error: None,
        }
    }

    const ONE: &str = "name                         : \"Проба\"\nversion                      : \nactive                       : yes\npurpose                      : add-on\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"9hfFb6YVX2OwLKZaL1L69Eq0Vrg=\"\n";

    #[test]
    fn a_named_read_refuses_a_record_that_is_not_the_requested_one() {
        let request = ExtensionInventoryRequest {
            scope: ExtensionInventoryScope::Named {
                name: "Другая".to_owned(),
            },
        };

        let error = read_inventory(&platform_result(ONE), &request).expect_err("refusal");

        assert!(error.to_string().contains("Другая"), "{error}");
    }

    #[test]
    fn a_named_read_accepts_the_requested_record() {
        let request = ExtensionInventoryRequest {
            scope: ExtensionInventoryScope::Named {
                name: "Проба".to_owned(),
            },
        };

        let extensions = read_inventory(&platform_result(ONE), &request).expect("inventory");

        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "Проба");
    }

    #[test]
    fn an_empty_infobase_reads_as_an_empty_inventory() {
        let request = ExtensionInventoryRequest {
            scope: ExtensionInventoryScope::All,
        };

        let extensions = read_inventory(&platform_result(""), &request).expect("inventory");

        assert!(extensions.is_empty());
    }
}
