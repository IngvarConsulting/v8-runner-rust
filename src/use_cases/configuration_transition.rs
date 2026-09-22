use crate::config::model::AppConfig;
use crate::domain::capability::{capability_of, Operation, SkippedProvider};
use crate::domain::configuration_transition::ConfigurationTransitionResult;
use crate::domain::execution::ExecutionStatus;
use crate::platform::designer::{DesignerDsl, DesignerError};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessError;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{
    CommandName, ExecutionContext, ExecutionInterruption, InterruptionSafetyClass,
};
use crate::use_cases::interruption::{
    command_interruption_details, command_interruption_status,
    deferred_process_interruption_details, deferred_process_interruption_warning,
};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum Transition {
    Apply,
    Reset,
}
impl Transition {
    pub const fn command(self) -> CommandName {
        match self {
            Self::Apply => CommandName::Apply,
            Self::Reset => CommandName::Reset,
        }
    }
    const fn operation(self) -> Operation {
        match self {
            Self::Apply => Operation::Apply,
            Self::Reset => Operation::Reset,
        }
    }
}

pub fn validate(extension: Option<&str>) -> Result<(), AppError> {
    if extension.is_some_and(|name| name.trim().is_empty() || name.chars().any(char::is_control)) {
        return Err(AppError::Validation(
            "extension must be a nonempty installed extension name without control characters"
                .into(),
        ));
    }
    Ok(())
}

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    transition: Transition,
    extension: Option<&str>,
    dry_run: bool,
) -> UseCaseResult<ConfigurationTransitionResult> {
    let started = Instant::now();
    let mut result = ConfigurationTransitionResult {
        duration_ms: 0,
        dry_run,
        extension: extension.map(str::to_owned),
        provider_dispatched: Some(false),
        completed: false,
        status: ExecutionStatus::Failed,
        provider: None,
        platform_log_path: None,
        interruption: None,
        warnings: Vec::new(),
    };
    let outcome = run(context, config, transition, extension, dry_run, &mut result);
    result.duration_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(()) => {
            result.status = ExecutionStatus::Succeeded;
            Ok(result)
        }
        Err(error) => Err(UseCaseFailure::with_payload(error, result)),
    }
}

fn run(
    context: &ExecutionContext,
    config: &AppConfig,
    transition: Transition,
    extension: Option<&str>,
    dry_run: bool,
    result: &mut ConfigurationTransitionResult,
) -> Result<(), AppError> {
    validate(extension)?;
    if let Some(interruption) = context.interruption() {
        result.status = command_interruption_status(interruption);
        result.interruption = Some(command_interruption_details(
            interruption,
            "before_dispatch",
            interruption.message(context.command()),
        ));
        return Err(match interruption {
            ExecutionInterruption::Cancelled => {
                AppError::Cancelled(interruption.message(context.command()).into())
            }
            ExecutionInterruption::TimedOut => {
                AppError::TimedOut(interruption.message(context.command()).into())
            }
        });
    }
    let plan = config.provider_plan(transition.operation());
    let unsupported: Vec<_> = plan
        .candidates()
        .into_iter()
        .filter(|provider| {
            capability_of(transition.operation(), config.target_kind(), *provider).is_none()
        })
        .map(|provider| SkippedProvider {
            provider,
            reason: format!(
                "{provider} does not implement {} on {}",
                transition.operation(),
                config.target_kind().as_str()
            ),
        })
        .collect();
    if !unsupported.is_empty() {
        result.provider = Some(plan.receipt_for_nobody(unsupported));
        return Err(AppError::CapabilityUnavailable(
            "configured provider does not implement this configuration transition".into(),
        ));
    }
    let mut utilities = PlatformUtilities::from_config(config);
    let selected = crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        transition.operation(),
    )
    .map_err(|(error, receipt)| {
        result.provider = Some(receipt);
        error
    })?;
    result.provider = Some(selected.receipt);
    let location = selected.location.ok_or_else(|| {
        AppError::CapabilityUnavailable("configuration transition requires Designer".into())
    })?;
    if !config.v8_connection().has_supported_shape() {
        return Err(AppError::Validation(
            "expected File=..., Srvr=...;Ref=..., or /S server\\ref connection".into(),
        ));
    }
    if dry_run {
        return Ok(());
    }
    let log_file = platform_logs_dir(&config.work_path)
        .map_err(|error| {
            AppError::Runtime(format!("failed to create platform log directory: {error}"))
        })?
        .join(format!("{}.log", transition.command().as_str()));
    result.platform_log_path = Some(log_file.clone());
    let dsl = DesignerDsl::new(
        location.path,
        config.v8_connection(),
        utilities.runner_for(UtilityType::V8),
        Some(log_file),
    )
    .with_execution_policy(
        context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
    );
    let executed = match transition {
        Transition::Apply => dsl.update_db_cfg(extension),
        Transition::Reset => dsl.rollback_cfg(extension),
    };
    let platform = executed.map_err(|error| {
        result.provider_dispatched = match &error {
            DesignerError::Spawn(ProcessError::SpawnFailed { .. })
            | DesignerError::StaleLogCleanup { .. }
            | DesignerError::UtilityNotFound(_) => Some(false),
            _ => None,
        };
        match &error {
            DesignerError::Spawn(ProcessError::Cancelled { .. })
            | DesignerError::Spawn(ProcessError::TimedOut { .. }) => {
                // A critical process is never killed; these errors occur only before spawn.
                let interruption =
                    if matches!(&error, DesignerError::Spawn(ProcessError::Cancelled { .. })) {
                        ExecutionInterruption::Cancelled
                    } else {
                        ExecutionInterruption::TimedOut
                    };
                result.provider_dispatched = Some(false);
                result.status = command_interruption_status(interruption);
                result.interruption = Some(command_interruption_details(
                    interruption,
                    "before_dispatch",
                    interruption.message(context.command()),
                ));
            }
            _ => {}
        }
        AppError::from(error)
    })?;
    result.provider_dispatched = Some(true);
    result.interruption = deferred_process_interruption_details(
        transition.command().as_str(),
        transition.command().as_str(),
        &platform,
    );
    if let Some(warning) =
        deferred_process_interruption_warning(transition.command().as_str(), &platform)
    {
        result.warnings.push(warning);
    }
    if platform.process.exit_code != 0 {
        return Err(AppError::Platform(format!(
            "{} failed with platform exit code {}; see platform_log_path",
            transition.command().as_str(),
            platform.process.exit_code
        )));
    }
    result.completed = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn cancelled_transition_keeps_receipt_and_does_not_create_work_path() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir
            .path()
            .join(if cfg!(windows) { "1cv8.exe" } else { "1cv8" });
        std::fs::write(&binary, "not executed").unwrap();
        let config_path = dir.path().join("v8project.yaml");
        std::fs::write(&config_path, format!("format: DESIGNER\nworkPath: work\ninfobase:\n  connection: 'File=base'\ntools:\n  platform:\n    path: '{}'\nsource-set: []\n", binary.display())).unwrap();
        let config =
            crate::config::loader::load_config_for_infobase_export(config_path.to_str(), None)
                .unwrap();
        for transition in [Transition::Apply, Transition::Reset] {
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            let context =
                ExecutionContext::cli(transition.command()).with_cancellation(cancellation);
            let failure = execute(&context, &config, transition, None, false).unwrap_err();
            assert_eq!(
                failure.error.kind(),
                crate::use_cases::result::UseCaseErrorKind::Cancelled
            );
            let result = failure.payload.unwrap();
            assert_eq!(result.status, ExecutionStatus::Cancelled);
            assert_eq!(result.provider_dispatched, Some(false));
            assert!(!result.completed);
            assert!(!result.interruption.unwrap().deferred);
            assert!(!config.work_path.exists());
        }
    }
}
