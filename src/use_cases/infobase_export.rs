use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::model::{AppConfig, BuilderBackend};
use crate::domain::execution::{
    ExecutionError, ExecutionOutcome, ExecutionStatus, ExecutionStepKind, StepResult,
};
use crate::domain::infobase_export::{
    CapabilityEvidence, ConfigurationState, ConfigurationSubject,
    ExportConfigurationPackageRequest, ExportConfigurationPackageResult,
    ExportInfobaseSnapshotRequest, ExportInfobaseSnapshotResult, ExportProvider,
    ExportProviderDecision,
};
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::process::{ProcessError, ProcessInterruptionReason};
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::try_acquire_advisory_lock;
use crate::support::path::{
    hashed_lock_path, nearest_existing_canonical_path, stable_path_identity,
};
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};

use super::interruption::{
    command_interruption_details, command_interruption_status,
    deferred_command_interruption_details, deferred_process_interruption_details,
    deferred_process_interruption_warning, process_interruption_details,
};
use super::staged_publication::{interruption_before_publish, StagedPublication};

const CONFIGURATION_COMMAND: &str = "infobase.configuration.export";
const SNAPSHOT_COMMAND: &str = "infobase.dump";

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
) -> UseCaseResult<ExportConfigurationPackageResult> {
    let selection = select_configuration_provider(config.builder);
    let mut result = ExportConfigurationPackageResult::new(request.clone(), selection.clone());
    if let Err(error) = validate_configuration_request(request) {
        return Err(configuration_failure(context, error, result, "validation"));
    }
    let provider = match selection.provider() {
        Some(provider) => provider,
        None => {
            return Err(configuration_failure(
                context,
                AppError::CapabilityUnavailable(selection.reason().to_owned()),
                result,
                "provider selection",
            ))
        }
    };
    debug_assert_eq!(selection.evidence(), CapabilityEvidence::Available);

    let output = resolve_output(config, &request.output)
        .map_err(|error| configuration_failure(context, error, result.clone(), "resolve target"))?;
    result.output = output.target.clone();
    let _target_lock = acquire_target_lock(context, &output.lock_path, CONFIGURATION_COMMAND)
        .map_err(|error| configuration_failure(context, error, result.clone(), "target lock"))?;
    let publication = StagedPublication::prepare_file(
        &output.target,
        &output.identity,
        ".infobase-config-stage",
        request.subject.artifact_kind().file_extension(),
    )
    .map_err(|error| configuration_failure(context, error, result.clone(), "prepare staging"))?;

    let provider_started = Instant::now();
    let platform_result = match run_configuration_provider(
        context,
        config,
        provider,
        request.state,
        &request.subject,
        publication.staging_path(),
    ) {
        Ok(platform_result) => platform_result,
        Err(error) => {
            return Err(configuration_failure(
                context,
                publication.cleanup_failure(error),
                result,
                "provider command",
            ))
        }
    };
    if let Err(error) = validate_platform_success(&platform_result) {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            "provider command",
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            "provider command",
            ExecutionStepKind::PlatformCommand,
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(publication.staging_path().display().to_string()),
    );
    if let Err(error) = validate_platform_artifact(publication.staging_path()) {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            "validate provider output",
        ));
    }
    record_deferred_process_interruption(
        &platform_result,
        "provider command",
        "configuration export",
        &mut result.execution,
        &mut result.warnings,
    );
    if let Some(error) = interruption_before_publish(context, "configuration package publication") {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            "before publication",
        ));
    }
    revalidate_output_identity(&output).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            "publish target revalidation",
        )
    })?;
    let publication_started = Instant::now();
    let publication_outcome = publication
        .publish_file(context, "failed to publish configuration package")
        .map_err(|error| configuration_failure(context, error, result.clone(), "publication"))?;
    result.steps.push(
        StepResult::succeeded(
            "publication",
            ExecutionStepKind::Publish,
            publication_started.elapsed().as_millis() as u64,
        )
        .with_target(result.output.display().to_string()),
    );
    result.published = true;
    result.mark_succeeded();
    if let Some(warning) = publication_outcome.cleanup_warning {
        result.warnings.push(warning);
    }
    if let Some(interruption) = publication_outcome.deferred_interruption {
        let message = "interruption was deferred until configuration publication completed";
        result.warnings.push(message.to_owned());
        result
            .execution
            .interruptions
            .push(deferred_command_interruption_details(
                interruption,
                "publication",
                message,
            ));
    }
    Ok(result)
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
) -> UseCaseResult<ExportInfobaseSnapshotResult> {
    let selection = select_snapshot_provider(config.builder);
    let mut result = ExportInfobaseSnapshotResult::new(request.clone(), selection.clone());
    if let Err(error) = validate_snapshot_output(&request.output) {
        return Err(snapshot_failure(context, error, result, "validation"));
    }
    let provider = match selection.provider() {
        Some(provider) => provider,
        None => {
            return Err(snapshot_failure(
                context,
                AppError::CapabilityUnavailable(selection.reason().to_owned()),
                result,
                "provider selection",
            ))
        }
    };
    debug_assert_eq!(selection.evidence(), CapabilityEvidence::Available);

    let output = resolve_output(config, &request.output)
        .map_err(|error| snapshot_failure(context, error, result.clone(), "resolve target"))?;
    result.output = output.target.clone();
    let _target_lock = acquire_target_lock(context, &output.lock_path, SNAPSHOT_COMMAND)
        .map_err(|error| snapshot_failure(context, error, result.clone(), "target lock"))?;
    let publication = StagedPublication::prepare_file(
        &output.target,
        &output.identity,
        ".infobase-dt-stage",
        "dt",
    )
    .map_err(|error| snapshot_failure(context, error, result.clone(), "prepare staging"))?;

    let provider_started = Instant::now();
    let platform_result =
        match run_snapshot_provider(context, config, provider, publication.staging_path()) {
            Ok(platform_result) => platform_result,
            Err(error) => {
                return Err(snapshot_failure(
                    context,
                    publication.cleanup_failure(error),
                    result,
                    "provider command",
                ))
            }
        };
    if let Err(error) = validate_platform_success(&platform_result) {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            "provider command",
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            "provider command",
            ExecutionStepKind::PlatformCommand,
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(publication.staging_path().display().to_string()),
    );
    if let Err(error) = validate_platform_artifact(publication.staging_path()) {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            "validate provider output",
        ));
    }
    record_deferred_process_interruption(
        &platform_result,
        "provider command",
        "infobase DT export",
        &mut result.execution,
        &mut result.warnings,
    );
    if let Some(error) = interruption_before_publish(context, "infobase DT publication") {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            "before publication",
        ));
    }
    revalidate_output_identity(&output).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            "publish target revalidation",
        )
    })?;
    let publication_started = Instant::now();
    let publication_outcome = publication
        .publish_file(context, "failed to publish infobase DT")
        .map_err(|error| snapshot_failure(context, error, result.clone(), "publication"))?;
    result.steps.push(
        StepResult::succeeded(
            "publication",
            ExecutionStepKind::Publish,
            publication_started.elapsed().as_millis() as u64,
        )
        .with_target(result.output.display().to_string()),
    );
    result.published = true;
    result.mark_succeeded();
    if let Some(warning) = publication_outcome.cleanup_warning {
        result.warnings.push(warning);
    }
    if let Some(interruption) = publication_outcome.deferred_interruption {
        let message = "interruption was deferred until DT publication completed";
        result.warnings.push(message.to_owned());
        result
            .execution
            .interruptions
            .push(deferred_command_interruption_details(
                interruption,
                "publication",
                message,
            ));
    }
    Ok(result)
}

pub(crate) fn validate_configuration_output(
    subject: &ConfigurationSubject,
    output: &Path,
) -> Result<(), AppError> {
    validate_output_suffix(output, subject.artifact_kind().file_extension())
}

pub(crate) fn validate_snapshot_output(output: &Path) -> Result<(), AppError> {
    validate_output_suffix(output, "dt")
}

pub(crate) fn validate_configuration_request(
    request: &ExportConfigurationPackageRequest,
) -> Result<(), AppError> {
    if let ConfigurationSubject::Extension { name } = &request.subject {
        if !valid_platform_identifier(name) {
            return Err(AppError::Validation(
                "--extension must be a non-empty 1C identifier".to_owned(),
            ));
        }
    }
    validate_configuration_output(&request.subject, &request.output)
}

fn validate_output_suffix(output: &Path, expected: &str) -> Result<(), AppError> {
    let actual = output.extension().and_then(|value| value.to_str());
    if actual.is_some_and(|value| value.eq_ignore_ascii_case(expected)) {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "output '{}' must have .{expected} suffix",
        output.display()
    )))
}

fn valid_platform_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

pub(crate) fn select_snapshot_provider(builder: BuilderBackend) -> ExportProviderDecision {
    match builder {
        BuilderBackend::Designer => ExportProviderDecision::available(
            ExportProvider::DesignerBatch,
            "configured builder selects verified Designer batch /DumpIB",
        ),
        BuilderBackend::Ibcmd => ExportProviderDecision::unavailable(
            CapabilityEvidence::Unverified,
            "IBCMD DT export requires a verified no-active-connections preflight",
        )
        .expect("unverified provider decision"),
    }
}

fn select_configuration_provider(builder: BuilderBackend) -> ExportProviderDecision {
    match builder {
        BuilderBackend::Designer => ExportProviderDecision::available(
            ExportProvider::DesignerBatch,
            "configured builder selects verified Designer batch configuration export",
        ),
        BuilderBackend::Ibcmd => ExportProviderDecision::available(
            ExportProvider::IbcmdProcess,
            "configured builder selects verified ibcmd config save",
        ),
    }
}

#[derive(Debug)]
struct ResolvedOutput {
    requested: PathBuf,
    target: PathBuf,
    identity: String,
    lock_path: PathBuf,
}

fn configuration_failure(
    context: &ExecutionContext,
    error: AppError,
    mut result: ExportConfigurationPackageResult,
    phase: &str,
) -> UseCaseFailure<ExportConfigurationPackageResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(error, result)
}

fn snapshot_failure(
    context: &ExecutionContext,
    error: AppError,
    mut result: ExportInfobaseSnapshotResult,
    phase: &str,
) -> UseCaseFailure<ExportInfobaseSnapshotResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(error, result)
}

fn failed_step(phase: &str, error: &AppError) -> StepResult {
    StepResult::failed(phase, step_kind(phase), 0).with_message(error.to_string())
}

fn step_kind(phase: &str) -> ExecutionStepKind {
    match phase {
        "validation" | "validate provider output" => ExecutionStepKind::Validation,
        "resolve target" | "target lock" | "publish target revalidation" => {
            ExecutionStepKind::ResolveTarget
        }
        "prepare staging" => ExecutionStepKind::PrepareWorkspace,
        "provider selection" => ExecutionStepKind::Other,
        "provider command" => ExecutionStepKind::PlatformCommand,
        "before publication" | "publication" => ExecutionStepKind::Publish,
        _ => ExecutionStepKind::Other,
    }
}

fn record_execution_failure(
    context: &ExecutionContext,
    error: &AppError,
    phase: &str,
    execution: &mut ExecutionOutcome<()>,
) {
    let message = error.to_string();
    let mut interruption_details = None;
    let (status, code) = match process_error(error) {
        Some(ProcessError::Cancelled { .. }) => {
            interruption_details = Some(process_interruption_details(
                ProcessInterruptionReason::Cancelled,
                phase,
                false,
                &message,
            ));
            (ExecutionStatus::Cancelled, "cancelled")
        }
        Some(ProcessError::TimedOut { .. }) => {
            interruption_details = Some(process_interruption_details(
                ProcessInterruptionReason::TimedOut,
                phase,
                false,
                &message,
            ));
            (ExecutionStatus::TimedOut, "timed_out")
        }
        _ => match error {
            AppError::CapabilityUnavailable(_) => {
                (ExecutionStatus::Failed, "capability_unavailable")
            }
            AppError::InvalidOutput(_) => (ExecutionStatus::InvalidOutput, "invalid_output"),
            _ => match context.interruption() {
                Some(interruption) => {
                    interruption_details =
                        Some(command_interruption_details(interruption, phase, &message));
                    (
                        command_interruption_status(interruption),
                        match interruption {
                            crate::use_cases::context::ExecutionInterruption::Cancelled => {
                                "cancelled"
                            }
                            crate::use_cases::context::ExecutionInterruption::TimedOut => {
                                "timed_out"
                            }
                        },
                    )
                }
                None => (ExecutionStatus::Failed, execution_error_code(error)),
            },
        },
    };
    execution.status = status;
    execution.errors.push(ExecutionError::new(code, message));
    if let Some(details) = interruption_details {
        execution.interruptions.push(details);
    }
}

fn process_error(error: &AppError) -> Option<&ProcessError> {
    match error {
        AppError::PlatformProcess(error)
        | AppError::PlatformProcessContext { source: error, .. } => Some(error),
        _ => None,
    }
}

fn execution_error_code(error: &AppError) -> &'static str {
    match error {
        AppError::Validation(_)
        | AppError::ValidationIbcmd(_)
        | AppError::ValidationIbcmdContext { .. }
        | AppError::Config(_)
        | AppError::ConfigContext { .. } => "invalid_argument",
        AppError::CapabilityUnavailable(_) => "capability_unavailable",
        AppError::InvalidOutput(_) => "invalid_output",
        AppError::Runtime(_) => "runtime_failure",
        _ => "platform_failure",
    }
}

fn record_deferred_process_interruption(
    platform_result: &PlatformCommandResult,
    phase: &str,
    completed_action: &str,
    execution: &mut ExecutionOutcome<()>,
    warnings: &mut Vec<String>,
) {
    if let Some(details) =
        deferred_process_interruption_details(phase, completed_action, platform_result)
    {
        execution.interruptions.push(details);
    }
    if let Some(warning) = deferred_process_interruption_warning(completed_action, platform_result)
    {
        warnings.push(warning);
    }
}

fn resolve_output(config: &AppConfig, requested: &Path) -> Result<ResolvedOutput, AppError> {
    let requested = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        config.base_path.join(requested)
    };
    let canonical = nearest_existing_canonical_path(&requested).map_err(|error| {
        AppError::Runtime(format!(
            "failed to canonicalize output '{}': {error}",
            requested.display()
        ))
    })?;
    if canonical.is_dir() {
        return Err(AppError::Validation(format!(
            "output '{}' is a directory",
            requested.display()
        )));
    }
    let identity = stable_path_identity(&canonical);
    let lock_path = hashed_lock_path(&canonical, "infobase-export").map_err(|error| {
        AppError::Runtime(format!("failed to resolve output lock path: {error}"))
    })?;
    Ok(ResolvedOutput {
        requested,
        target: canonical,
        identity,
        lock_path,
    })
}

fn revalidate_output_identity(output: &ResolvedOutput) -> Result<(), AppError> {
    let canonical = nearest_existing_canonical_path(&output.requested).map_err(|error| {
        AppError::Runtime(format!(
            "failed to revalidate output '{}': {error}",
            output.requested.display()
        ))
    })?;
    let current_identity = stable_path_identity(&canonical);
    if current_identity != output.identity {
        return Err(AppError::Runtime(format!(
            "output identity changed before publication: expected '{}', resolved '{}'",
            output.identity, current_identity
        )));
    }
    Ok(())
}

fn acquire_target_lock(
    context: &ExecutionContext,
    lock_path: &Path,
    command: &str,
) -> Result<crate::support::fs::AdvisoryLockGuard, AppError> {
    loop {
        match try_acquire_advisory_lock(lock_path) {
            Ok(guard) => return Ok(guard),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if let Some(interruption) = context.interruption() {
                    return Err(AppError::Runtime(format!(
                        "{} while waiting for {command} output lock '{}'",
                        interruption.message(context.command()),
                        lock_path.display()
                    )));
                }
                let delay = context
                    .remaining_budget()
                    .map(|remaining| remaining.min(Duration::from_millis(25)))
                    .unwrap_or(Duration::from_millis(25));
                if delay.is_zero() {
                    continue;
                }
                thread::sleep(delay);
            }
            Err(error) => {
                return Err(AppError::Runtime(format!(
                    "failed to acquire {command} output lock '{}': {error}",
                    lock_path.display()
                )))
            }
        }
    }
}

fn run_configuration_provider(
    context: &ExecutionContext,
    config: &AppConfig,
    provider: ExportProvider,
    state: ConfigurationState,
    subject: &ConfigurationSubject,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    let extension = match subject {
        ConfigurationSubject::Main => None,
        ConfigurationSubject::Extension { name } => Some(name.as_str()),
    };
    let mut utilities = PlatformUtilities::from_config(config);
    let utility = match provider {
        ExportProvider::DesignerBatch => UtilityType::V8,
        ExportProvider::IbcmdProcess => UtilityType::Ibcmd,
    };
    let location = utilities.locate(utility).map_err(AppError::from)?;
    let runner = utilities.runner_for(utility);
    let result = match provider {
        ExportProvider::DesignerBatch => {
            let log = provider_log_path(config, "configuration-export")?;
            let dsl = DesignerDsl::new(location.path, config.v8_connection(), runner, Some(log))
                .with_execution_policy(
                    context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
                );
            match state {
                ConfigurationState::Working => dsl.dump_cfg(staging_path, extension),
                ConfigurationState::Database => dsl.dump_db_cfg(staging_path, extension),
            }
            .map_err(AppError::from)?
        }
        ExportProvider::IbcmdProcess => {
            let connection =
                IbcmdConnection::from_infobase(&config.infobase).map_err(AppError::from)?;
            let data_path = config.work_path.join("ibcmd-data");
            std::fs::create_dir_all(&data_path).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to create IBCMD data directory '{}': {error}",
                    data_path.display()
                ))
            })?;
            IbcmdDsl::new(location.path, connection, runner)
                .with_data_path(data_path)
                .with_execution_policy(
                    context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
                )
                .config_save(
                    staging_path,
                    state == ConfigurationState::Database,
                    extension,
                )
                .map_err(AppError::from)?
        }
    };
    Ok(result)
}

fn run_snapshot_provider(
    context: &ExecutionContext,
    config: &AppConfig,
    provider: ExportProvider,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    match provider {
        ExportProvider::DesignerBatch => {
            let mut utilities = PlatformUtilities::from_config(config);
            let location = utilities.locate(UtilityType::V8).map_err(AppError::from)?;
            let log = provider_log_path(config, "infobase-dump")?;
            DesignerDsl::new(
                location.path,
                config.v8_connection(),
                utilities.runner_for(UtilityType::V8),
                Some(log),
            )
            .with_execution_policy(
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            )
            .dump_infobase(staging_path)
            .map_err(AppError::from)
        }
        ExportProvider::IbcmdProcess => Err(AppError::Validation(
            "IBCMD DT export is unverified and cannot be dispatched".to_owned(),
        )),
    }
}

fn provider_log_path(config: &AppConfig, stem: &str) -> Result<PathBuf, AppError> {
    platform_logs_dir(&config.work_path)
        .map(|dir| dir.join(format!("{stem}.log")))
        .map_err(|error| AppError::Runtime(format!("failed to create platform logs dir: {error}")))
}

fn validate_platform_success(result: &PlatformCommandResult) -> Result<(), AppError> {
    if result.process.exit_code != 0 {
        let mut details = vec![format!(
            "platform export failed with exit code {}",
            result.process.exit_code
        )];
        append_platform_diagnostic(&mut details, "stdout", &result.process.stdout);
        append_platform_diagnostic(&mut details, "stderr", &result.process.stderr);
        if let Some(log) = result.platform_log.as_deref() {
            append_platform_diagnostic(&mut details, "platform log", log);
        }
        if let Some(error) = result.platform_log_read_error.as_deref() {
            append_platform_diagnostic(&mut details, "platform log read error", error);
        }
        if let Some(path) = result.platform_log_path.as_deref() {
            details.push(format!("platform log path: {}", path.display()));
        }
        return Err(AppError::Platform(details.join("; ")));
    }
    Ok(())
}

fn append_platform_diagnostic(details: &mut Vec<String>, label: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        details.push(format!("{label}: {value}"));
    }
}

fn validate_platform_artifact(staging_path: &Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(staging_path).map_err(|error| {
        AppError::InvalidOutput(format!(
            "provider did not produce export file '{}': {error}",
            staging_path.display()
        ))
    })?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(AppError::InvalidOutput(format!(
            "provider export '{}' is not a non-empty regular file",
            staging_path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use crate::config::model::{
        AppConfig, BuildConfig, BuilderBackend, InfobaseConfig, McpConfig, SourceFormat,
        TestsConfig, ToolsConfig,
    };
    use crate::domain::execution::{ExecutionOutcome, ExecutionStatus};
    use crate::domain::infobase_export::{
        CapabilityEvidence, ConfigurationSubject, ExportProvider,
    };
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;
    use crate::use_cases::context::ExecutionContext;

    use super::{
        acquire_target_lock, record_execution_failure, resolve_output, revalidate_output_identity,
        select_snapshot_provider, validate_configuration_output, validate_snapshot_output,
        SNAPSHOT_COMMAND,
    };

    fn config(base: &Path, work: &Path) -> AppConfig {
        AppConfig {
            base_path: base.to_path_buf(),
            work_path: work.to_path_buf(),
            execution_timeout: 300_000,
            format: SourceFormat::Designer,
            builder: BuilderBackend::Designer,
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            source_sets: Vec::new(),
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: McpConfig::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn configuration_output_suffix_is_derived_from_subject() {
        assert!(validate_configuration_output(
            &ConfigurationSubject::Main,
            Path::new("dist/main.cf")
        )
        .is_ok());
        assert!(validate_configuration_output(
            &ConfigurationSubject::Extension {
                name: "Sales".to_owned(),
            },
            Path::new("dist/sales.cfe")
        )
        .is_ok());
        assert!(validate_configuration_output(
            &ConfigurationSubject::Main,
            Path::new("dist/main.cfe")
        )
        .is_err());
    }

    #[test]
    fn snapshot_output_is_dt_and_ibcmd_remains_unverified() {
        assert!(validate_snapshot_output(Path::new("dist/base.dt")).is_ok());
        assert!(validate_snapshot_output(Path::new("dist/base.backup")).is_err());

        let designer = select_snapshot_provider(BuilderBackend::Designer);
        assert_eq!(designer.provider(), Some(ExportProvider::DesignerBatch));
        assert_eq!(designer.evidence(), CapabilityEvidence::Available);

        let ibcmd = select_snapshot_provider(BuilderBackend::Ibcmd);
        assert_eq!(ibcmd.provider(), None);
        assert_eq!(ibcmd.evidence(), CapabilityEvidence::Unverified);
    }

    #[test]
    fn different_workspaces_resolve_one_output_to_one_serializing_target_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("shared/base.dt");
        std::fs::create_dir_all(output.parent().expect("parent")).expect("output parent");
        let first = config(&dir.path().join("one"), &dir.path().join("work-one"));
        let second = config(&dir.path().join("two"), &dir.path().join("work-two"));
        let first_target = resolve_output(&first, &output).expect("first target");
        let second_target = resolve_output(&second, &output).expect("second target");
        assert_eq!(first_target.lock_path, second_target.lock_path);

        let first_guard = crate::support::fs::acquire_advisory_lock(&first_target.lock_path)
            .expect("first target lock");
        let lock_path = second_target.lock_path.clone();
        let (tx, rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _guard =
                crate::support::fs::acquire_advisory_lock(&lock_path).expect("second target lock");
            tx.send(()).expect("send acquired");
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first_guard);
        rx.recv_timeout(Duration::from_secs(2))
            .expect("second workspace acquires after release");
        waiter.join().expect("waiter");
    }

    #[test]
    fn target_lock_wait_observes_the_command_deadline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_path = dir.path().join("target.lock");
        let _guard = crate::support::fs::acquire_advisory_lock(&lock_path).expect("held lock");
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump)
            .with_deadline(Some(Instant::now() + Duration::from_millis(30)));
        let started = Instant::now();

        let error = acquire_target_lock(&context, &lock_path, SNAPSHOT_COMMAND)
            .expect_err("deadline must stop lock wait");

        assert!(error.to_string().contains("timeout"));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn cancelled_process_is_not_collapsed_into_generic_failure() {
        let context = ExecutionContext::cli(
            crate::use_cases::context::CommandName::InfobaseConfigurationExport,
        );
        let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed);
        let error = AppError::PlatformProcess(ProcessError::Cancelled {
            cmd: "1cv8 DESIGNER".to_owned(),
        });

        record_execution_failure(&context, &error, "provider command", &mut execution);

        assert_eq!(execution.status, ExecutionStatus::Cancelled);
        assert_eq!(execution.errors[0].code, "cancelled");
        assert!(!execution.errors[0].retryable);
        assert_eq!(execution.interruptions.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn publication_rejects_target_identity_change_after_provider_execution() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let original_parent = dir.path().join("real-a");
        let replacement_parent = dir.path().join("real-b");
        let alias = dir.path().join("output");
        std::fs::create_dir_all(&original_parent).expect("original parent");
        std::fs::create_dir_all(&replacement_parent).expect("replacement parent");
        symlink(&original_parent, &alias).expect("initial alias");
        let config = config(dir.path(), &dir.path().join("work"));
        let output = resolve_output(&config, &alias.join("main.cf")).expect("output");

        std::fs::remove_file(&alias).expect("remove old alias");
        symlink(&replacement_parent, &alias).expect("retarget alias");

        let error = revalidate_output_identity(&output).expect_err("identity change");
        assert!(error.to_string().contains("identity changed"));
    }
}
