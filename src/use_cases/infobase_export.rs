use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::model::{AppConfig, BuilderBackend};
use crate::domain::execution::{ExecutionError, ExecutionOutcome, ExecutionStatus, StepResult};
use crate::domain::infobase_export::{
    ConfigurationState, ConfigurationSubject, ExportConfigurationPackageRequest,
    ExportConfigurationPackageResult, ExportInfobaseSnapshotRequest, ExportInfobaseSnapshotResult,
    ExportPhase, ExportProvider, ExportProviderDecision, ExportTargetState, ProviderCandidate,
    ProviderEvidence, ProviderImplementation, ProviderReadiness,
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
    filesystem_object_identity, hashed_lock_path, nearest_existing_canonical_path,
    stable_path_identity, FilesystemObjectIdentity,
};
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};

use super::interruption::{
    command_interruption_details, deferred_command_interruption_details,
    deferred_process_interruption_details, deferred_process_interruption_warning,
    process_interruption_details,
};
use super::staged_publication::{
    cleanup_owned_orphan_files, interruption_before_publish, PublicationFailureState,
    StagedPublication,
};

const CONFIGURATION_COMMAND: &str = "infobase.configuration.export";
const SNAPSHOT_COMMAND: &str = "infobase.dump";

#[derive(Debug, Clone)]
pub struct PreparedExportProvider {
    selection: ExportProviderDecision,
    provider: ExportProvider,
    executable: PathBuf,
}

impl PreparedExportProvider {
    pub fn selection(&self) -> &ExportProviderDecision {
        &self.selection
    }
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
    prepared: &PreparedExportProvider,
) -> UseCaseResult<ExportConfigurationPackageResult> {
    let mut result =
        ExportConfigurationPackageResult::new(request.clone(), prepared.selection.clone());
    if let Err(error) = validate_configuration_request(request) {
        return Err(configuration_failure(
            context,
            error,
            result,
            ExportPhase::Validation,
        ));
    }
    let provider = prepared.provider;

    let output = resolve_output(config, &request.output).map_err(|error| {
        configuration_failure(context, error, result.clone(), ExportPhase::ResolveTarget)
    })?;
    result.output = output.target.clone();
    let _target_lock = acquire_target_lock(context, &output.lock_path, CONFIGURATION_COMMAND)
        .map_err(|error| {
            configuration_failure(context, error, result.clone(), ExportPhase::TargetLock)
        })?;
    let output_observation = observe_locked_output(&output).map_err(|error| {
        configuration_failure(context, error, result.clone(), ExportPhase::ResolveTarget)
    })?;
    cleanup_export_orphans(&output, &[".infobase-config-stage-"]).map_err(|error| {
        configuration_failure(context, error, result.clone(), ExportPhase::OrphanCleanup)
    })?;
    let publication = StagedPublication::prepare_file(
        &output.target,
        &output.identity,
        ".infobase-config-stage",
        request.subject.artifact_kind().file_extension(),
    )
    .map_err(|error| {
        configuration_failure(context, error, result.clone(), ExportPhase::PrepareStaging)
    })?;

    let provider_started = Instant::now();
    let platform_result = match run_configuration_provider(
        context,
        config,
        provider,
        &prepared.executable,
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
                ExportPhase::ProviderCommand,
            ))
        }
    };
    if let Err(error) = validate_platform_success(&platform_result) {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            ExportPhase::ProviderCommand,
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            ExportPhase::ProviderCommand.as_str(),
            ExportPhase::ProviderCommand.kind(),
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(publication.staging_path().display().to_string()),
    );
    if let Err(error) = validate_platform_artifact(publication.staging_path()) {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            ExportPhase::ValidateProviderOutput,
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
            ExportPhase::BeforePublication,
        ));
    }
    revalidate_before_publish(&output, &output_observation, &publication).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            ExportPhase::PublishTargetRevalidation,
        )
    })?;
    let publication_started = Instant::now();
    let publication_outcome = publication
        .publish_file_with_state(context, "failed to publish configuration package")
        .map_err(|mut failure| {
            failure.error = publication.cleanup_failure(failure.error);
            result.target_state = export_failure_state(failure.target_state);
            record_uncertain_target_warning(&mut result.warnings, result.target_state);
            configuration_failure(
                context,
                failure.error,
                result.clone(),
                ExportPhase::Publication,
            )
        })?;
    result.steps.push(
        StepResult::succeeded(
            ExportPhase::Publication.as_str(),
            ExportPhase::Publication.kind(),
            publication_started.elapsed().as_millis() as u64,
        )
        .with_target(result.output.display().to_string()),
    );
    result.published = true;
    result.target_state = if publication_outcome.previous_target_present {
        ExportTargetState::Replaced
    } else {
        ExportTargetState::Created
    };
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
    prepared: &PreparedExportProvider,
) -> UseCaseResult<ExportInfobaseSnapshotResult> {
    let mut result = ExportInfobaseSnapshotResult::new(request.clone(), prepared.selection.clone());
    if let Err(error) = validate_snapshot_output(&request.output) {
        return Err(snapshot_failure(
            context,
            error,
            result,
            ExportPhase::Validation,
        ));
    }
    let provider = prepared.provider;

    let output = resolve_output(config, &request.output).map_err(|error| {
        snapshot_failure(context, error, result.clone(), ExportPhase::ResolveTarget)
    })?;
    result.output = output.target.clone();
    let _target_lock =
        acquire_target_lock(context, &output.lock_path, SNAPSHOT_COMMAND).map_err(|error| {
            snapshot_failure(context, error, result.clone(), ExportPhase::TargetLock)
        })?;
    let output_observation = observe_locked_output(&output).map_err(|error| {
        snapshot_failure(context, error, result.clone(), ExportPhase::ResolveTarget)
    })?;
    cleanup_export_orphans(&output, &[".infobase-dt-stage-"]).map_err(|error| {
        snapshot_failure(context, error, result.clone(), ExportPhase::OrphanCleanup)
    })?;
    let publication = StagedPublication::prepare_file(
        &output.target,
        &output.identity,
        ".infobase-dt-stage",
        "dt",
    )
    .map_err(|error| {
        snapshot_failure(context, error, result.clone(), ExportPhase::PrepareStaging)
    })?;

    let provider_started = Instant::now();
    let platform_result = match run_snapshot_provider(
        context,
        config,
        provider,
        &prepared.executable,
        publication.staging_path(),
    ) {
        Ok(platform_result) => platform_result,
        Err(error) => {
            return Err(snapshot_failure(
                context,
                publication.cleanup_failure(error),
                result,
                ExportPhase::ProviderCommand,
            ))
        }
    };
    if let Err(error) = validate_platform_success(&platform_result) {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            ExportPhase::ProviderCommand,
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            ExportPhase::ProviderCommand.as_str(),
            ExportPhase::ProviderCommand.kind(),
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(publication.staging_path().display().to_string()),
    );
    if let Err(error) = validate_platform_artifact(publication.staging_path()) {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            ExportPhase::ValidateProviderOutput,
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
            ExportPhase::BeforePublication,
        ));
    }
    revalidate_before_publish(&output, &output_observation, &publication).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            ExportPhase::PublishTargetRevalidation,
        )
    })?;
    let publication_started = Instant::now();
    let publication_outcome = publication
        .publish_file_with_state(context, "failed to publish infobase DT")
        .map_err(|mut failure| {
            failure.error = publication.cleanup_failure(failure.error);
            result.target_state = export_failure_state(failure.target_state);
            record_uncertain_target_warning(&mut result.warnings, result.target_state);
            snapshot_failure(
                context,
                failure.error,
                result.clone(),
                ExportPhase::Publication,
            )
        })?;
    result.steps.push(
        StepResult::succeeded(
            ExportPhase::Publication.as_str(),
            ExportPhase::Publication.kind(),
            publication_started.elapsed().as_millis() as u64,
        )
        .with_target(result.output.display().to_string()),
    );
    result.published = true;
    result.target_state = if publication_outcome.previous_target_present {
        ExportTargetState::Replaced
    } else {
        ExportTargetState::Created
    };
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

fn export_failure_state(state: PublicationFailureState) -> ExportTargetState {
    match state {
        PublicationFailureState::Unchanged => ExportTargetState::Unchanged,
        PublicationFailureState::Restored => ExportTargetState::Restored,
        PublicationFailureState::Uncertain => ExportTargetState::Uncertain,
    }
}

fn record_uncertain_target_warning(warnings: &mut Vec<String>, state: ExportTargetState) {
    if state == ExportTargetState::Uncertain {
        warnings.push(
            "publication rollback failed; the output target requires manual inspection".to_owned(),
        );
    }
}

fn valid_platform_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

pub fn prepare_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
) -> Result<PreparedExportProvider, UseCaseFailure<ExportConfigurationPackageResult>> {
    if let Err(error) = validate_configuration_request(request) {
        let decision = ExportProviderDecision::unavailable(
            "provider selection was not attempted because the request is invalid",
            Vec::new(),
        );
        let result = ExportConfigurationPackageResult::new(request.clone(), decision);
        return Err(configuration_failure(
            context,
            error,
            result,
            ExportPhase::Validation,
        ));
    }

    match select_provider(context, config, ExportIntent::Configuration) {
        Ok(prepared) => Ok(prepared),
        Err((error, decision)) => {
            let result = ExportConfigurationPackageResult::new(request.clone(), decision);
            Err(configuration_failure(
                context,
                error,
                result,
                ExportPhase::ProviderSelection,
            ))
        }
    }
}

pub fn prepare_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
) -> Result<PreparedExportProvider, UseCaseFailure<ExportInfobaseSnapshotResult>> {
    if let Err(error) = validate_snapshot_output(&request.output) {
        let decision = ExportProviderDecision::unavailable(
            "provider selection was not attempted because the request is invalid",
            Vec::new(),
        );
        let result = ExportInfobaseSnapshotResult::new(request.clone(), decision);
        return Err(snapshot_failure(
            context,
            error,
            result,
            ExportPhase::Validation,
        ));
    }

    match select_provider(context, config, ExportIntent::Snapshot) {
        Ok(prepared) => Ok(prepared),
        Err((error, decision)) => {
            let result = ExportInfobaseSnapshotResult::new(request.clone(), decision);
            Err(snapshot_failure(
                context,
                error,
                result,
                ExportPhase::ProviderSelection,
            ))
        }
    }
}

pub fn preview_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
    prepared: &PreparedExportProvider,
) -> UseCaseResult<ExportConfigurationPackageResult> {
    let mut result =
        ExportConfigurationPackageResult::new(request.clone(), prepared.selection().clone());
    result.mark_preview_failure();
    let output = resolve_output(config, &request.output).map_err(|error| {
        configuration_failure(context, error, result.clone(), ExportPhase::ResolveTarget)
    })?;
    result.output = output.target;
    result.mark_preview();
    Ok(result)
}

pub fn preview_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
    prepared: &PreparedExportProvider,
) -> UseCaseResult<ExportInfobaseSnapshotResult> {
    let mut result =
        ExportInfobaseSnapshotResult::new(request.clone(), prepared.selection().clone());
    result.mark_preview_failure();
    let output = resolve_output(config, &request.output).map_err(|error| {
        snapshot_failure(context, error, result.clone(), ExportPhase::ResolveTarget)
    })?;
    result.output = output.target;
    result.mark_preview();
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
enum ExportIntent {
    Configuration,
    Snapshot,
}

fn select_provider(
    context: &ExecutionContext,
    config: &AppConfig,
    intent: ExportIntent,
) -> Result<PreparedExportProvider, (AppError, ExportProviderDecision)> {
    let providers = match config.builder {
        BuilderBackend::Designer => [ExportProvider::DesignerBatch, ExportProvider::IbcmdProcess],
        BuilderBackend::Ibcmd => [ExportProvider::IbcmdProcess, ExportProvider::DesignerBatch],
    };
    let mut utilities = PlatformUtilities::from_config(config);
    let mut candidates = Vec::new();
    let mut selected = None;

    for provider in providers {
        if let Some(interruption) = context.interruption() {
            let reason = format!(
                "{} during provider selection",
                interruption.message(context.command())
            );
            let decision = ExportProviderDecision::unavailable(reason.clone(), candidates);
            let error = match interruption {
                crate::use_cases::context::ExecutionInterruption::Cancelled => {
                    AppError::Cancelled(reason)
                }
                crate::use_cases::context::ExecutionInterruption::TimedOut => {
                    AppError::TimedOut(reason)
                }
            };
            return Err((error, decision));
        }
        let (implementation, evidence, implementation_reason) = capability(intent, provider);
        if selected.is_some() || implementation != ProviderImplementation::Implemented {
            candidates.push(ProviderCandidate::new(
                provider,
                implementation,
                ProviderReadiness::NotChecked,
                evidence,
                implementation_reason,
            ));
            continue;
        }

        let utility = provider_utility(provider);
        match readiness(config, &mut utilities, provider, utility) {
            Ok(executable) => {
                candidates.push(ProviderCandidate::new(
                    provider,
                    implementation,
                    ProviderReadiness::Ready,
                    evidence,
                    format!(
                        "{}; '{}' resolved without starting a provider process",
                        implementation_reason,
                        executable.display()
                    ),
                ));
                selected = Some((provider, executable));
            }
            Err(reason) => candidates.push(ProviderCandidate::new(
                provider,
                implementation,
                ProviderReadiness::Unavailable,
                evidence,
                format!("{implementation_reason}; {reason}"),
            )),
        }
    }

    if let Some((provider, executable)) = selected {
        let reason = format!(
            "selected {} before dispatch from the operation-specific candidate order",
            provider.as_str()
        );
        let selection = ExportProviderDecision::selected(provider, reason, candidates);
        return Ok(PreparedExportProvider {
            selection,
            provider,
            executable,
        });
    }

    let has_implemented = candidates
        .iter()
        .any(|candidate| candidate.implementation == ProviderImplementation::Implemented);
    let reason = candidates
        .iter()
        .map(|candidate| format!("{}: {}", candidate.provider.as_str(), candidate.reason))
        .collect::<Vec<_>>()
        .join("; ");
    let decision = ExportProviderDecision::unavailable(reason.clone(), candidates);
    let error = if has_implemented {
        AppError::EnvironmentUnavailable(reason)
    } else {
        AppError::CapabilityUnavailable(reason)
    };
    Err((error, decision))
}

fn capability(
    intent: ExportIntent,
    provider: ExportProvider,
) -> (ProviderImplementation, ProviderEvidence, &'static str) {
    match (intent, provider) {
        (ExportIntent::Configuration, ExportProvider::DesignerBatch) => (
            ProviderImplementation::Implemented,
            ProviderEvidence::ArgvTested,
            "Designer CF/CFE adapter is implemented from the documented batch contract",
        ),
        (ExportIntent::Configuration, ExportProvider::IbcmdProcess) => (
            ProviderImplementation::Implemented,
            ProviderEvidence::ArgvTested,
            "IBCMD CF/CFE adapter is implemented from the documented config-save contract",
        ),
        (ExportIntent::Snapshot, ExportProvider::DesignerBatch) => (
            ProviderImplementation::Implemented,
            ProviderEvidence::ArgvTested,
            "Designer DT adapter is implemented from the documented batch contract",
        ),
        (ExportIntent::Snapshot, ExportProvider::IbcmdProcess) => (
            ProviderImplementation::Experimental,
            ProviderEvidence::Documented,
            "IBCMD DT export is disabled until an exclusive-access preflight is implemented",
        ),
    }
}

fn provider_utility(provider: ExportProvider) -> UtilityType {
    match provider {
        ExportProvider::DesignerBatch => UtilityType::V8,
        ExportProvider::IbcmdProcess => UtilityType::Ibcmd,
    }
}

fn readiness(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: ExportProvider,
    utility: UtilityType,
) -> Result<PathBuf, String> {
    validate_file_infobase_readiness(config)?;
    if provider == ExportProvider::IbcmdProcess {
        IbcmdConnection::from_infobase(&config.infobase)
            .map_err(|error| format!("connection is not ready for IBCMD: {error}"))?;
    }
    utilities
        .locate(utility)
        .map(|location| location.path)
        .map_err(|error| format!("environment is not ready: {error}"))
}

fn validate_file_infobase_readiness(config: &AppConfig) -> Result<(), String> {
    let connection = config.v8_connection();
    if !connection.has_supported_shape() {
        return Err(
            "infobase connection is not ready: expected non-empty File=..., Srvr=...;Ref=..., or /S server\\ref"
                .to_owned(),
        );
    }
    let Some(file_path) = connection.file_path() else {
        return Ok(());
    };
    let database_file = Path::new(file_path).join("1Cv8.1CD");
    if database_file.is_file() {
        return Ok(());
    }
    Err(format!(
        "file infobase is not ready: '{}' is missing or is not a file",
        database_file.display()
    ))
}

#[derive(Debug)]
struct ResolvedOutput {
    requested: PathBuf,
    target: PathBuf,
    identity: String,
    lock_path: PathBuf,
}

#[derive(Debug)]
struct OutputObservation {
    parent: FilesystemObjectIdentity,
    target: Option<TargetObjectObservation>,
}

#[derive(Debug, PartialEq, Eq)]
struct TargetObjectObservation {
    identity: FilesystemObjectIdentity,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

fn configuration_failure(
    context: &ExecutionContext,
    error: AppError,
    mut result: ExportConfigurationPackageResult,
    phase: ExportPhase,
) -> UseCaseFailure<ExportConfigurationPackageResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(infobase_use_case_error(error), result)
}

fn snapshot_failure(
    context: &ExecutionContext,
    error: AppError,
    mut result: ExportInfobaseSnapshotResult,
    phase: ExportPhase,
) -> UseCaseFailure<ExportInfobaseSnapshotResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(infobase_use_case_error(error), result)
}

fn infobase_use_case_error(error: AppError) -> UseCaseError {
    let kind = match process_error(&error) {
        Some(ProcessError::Cancelled { .. }) => Some(UseCaseErrorKind::Cancelled),
        Some(ProcessError::TimedOut { .. }) => Some(UseCaseErrorKind::TimedOut),
        _ => None,
    };
    match kind {
        Some(kind) => UseCaseError::new(kind, error.to_string()),
        None => error.into(),
    }
}

fn failed_step(phase: ExportPhase, error: &AppError) -> StepResult {
    StepResult::failed(phase.as_str(), phase.kind(), 0).with_message(error.to_string())
}

fn record_execution_failure(
    _context: &ExecutionContext,
    error: &AppError,
    phase: ExportPhase,
    execution: &mut ExecutionOutcome<()>,
) {
    let message = error.to_string();
    let mut interruption_details = None;
    let (status, code) = match process_error(error) {
        Some(ProcessError::Cancelled { .. }) => {
            interruption_details = Some(process_interruption_details(
                ProcessInterruptionReason::Cancelled,
                phase.as_str(),
                false,
                &message,
            ));
            (ExecutionStatus::Cancelled, "cancelled")
        }
        Some(ProcessError::TimedOut { .. }) => {
            interruption_details = Some(process_interruption_details(
                ProcessInterruptionReason::TimedOut,
                phase.as_str(),
                false,
                &message,
            ));
            (ExecutionStatus::TimedOut, "timed_out")
        }
        _ => match error {
            AppError::Cancelled(_) => {
                interruption_details = Some(command_interruption_details(
                    crate::use_cases::context::ExecutionInterruption::Cancelled,
                    phase.as_str(),
                    &message,
                ));
                (ExecutionStatus::Cancelled, "cancelled")
            }
            AppError::TimedOut(_) => {
                interruption_details = Some(command_interruption_details(
                    crate::use_cases::context::ExecutionInterruption::TimedOut,
                    phase.as_str(),
                    &message,
                ));
                (ExecutionStatus::TimedOut, "timed_out")
            }
            AppError::CapabilityUnavailable(_) => {
                (ExecutionStatus::Failed, "capability_unavailable")
            }
            AppError::EnvironmentUnavailable(_) => {
                (ExecutionStatus::Failed, "environment_unavailable")
            }
            AppError::WorkspaceBusy(_) => (ExecutionStatus::Failed, "workspace_busy"),
            AppError::InvalidOutput(_) => (ExecutionStatus::InvalidOutput, "invalid_output"),
            _ => (ExecutionStatus::Failed, execution_error_code(error)),
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
        AppError::EnvironmentUnavailable(_) => "environment_unavailable",
        AppError::WorkspaceBusy(_) => "workspace_busy",
        AppError::Cancelled(_) => "cancelled",
        AppError::TimedOut(_) => "timed_out",
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

fn observe_locked_output(output: &ResolvedOutput) -> Result<OutputObservation, AppError> {
    revalidate_output_identity(output)?;
    let parent = output.target.parent().ok_or_else(|| {
        AppError::Runtime(format!(
            "output path has no parent: {}",
            output.target.display()
        ))
    })?;
    let parent = filesystem_object_identity(parent).map_err(|error| {
        AppError::Runtime(format!(
            "failed to observe output parent '{}': {error}",
            parent.display()
        ))
    })?;
    let target = match observe_target_object(&output.target) {
        Ok(identity) => Some(identity),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AppError::Runtime(format!(
                "failed to observe output '{}': {error}",
                output.target.display()
            )))
        }
    };
    Ok(OutputObservation { parent, target })
}

fn observe_target_object(path: &Path) -> std::io::Result<TargetObjectObservation> {
    let metadata = std::fs::metadata(path)?;
    Ok(TargetObjectObservation {
        identity: filesystem_object_identity(path)?,
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn revalidate_output_observation(
    output: &ResolvedOutput,
    observation: &OutputObservation,
) -> Result<(), AppError> {
    revalidate_output_identity(output)?;
    let parent = output.target.parent().ok_or_else(|| {
        AppError::Runtime(format!(
            "output path has no parent: {}",
            output.target.display()
        ))
    })?;
    let current_parent = filesystem_object_identity(parent).map_err(|error| {
        AppError::Runtime(format!(
            "failed to revalidate output parent '{}': {error}",
            parent.display()
        ))
    })?;
    if current_parent != observation.parent {
        return Err(AppError::Runtime(format!(
            "output parent changed before publication: '{}'",
            parent.display()
        )));
    }
    let current_target = match observe_target_object(&output.target) {
        Ok(identity) => Some(identity),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AppError::Runtime(format!(
                "failed to revalidate output '{}': {error}",
                output.target.display()
            )))
        }
    };
    if current_target != observation.target {
        return Err(AppError::Runtime(format!(
            "output target changed before publication: '{}'",
            output.target.display()
        )));
    }
    Ok(())
}

fn revalidate_before_publish(
    output: &ResolvedOutput,
    observation: &OutputObservation,
    publication: &StagedPublication,
) -> Result<(), AppError> {
    revalidate_output_observation(output, observation)
        .map_err(|error| publication.cleanup_failure(error))
}

fn cleanup_export_orphans(
    output: &ResolvedOutput,
    stage_prefixes: &[&str],
) -> Result<(), AppError> {
    let roots = output
        .target
        .parent()
        .map(|parent| vec![parent.to_path_buf()])
        .unwrap_or_default();
    cleanup_owned_orphan_files(
        &roots,
        &output.target,
        &output.identity,
        stage_prefixes,
        &[],
        false,
    )
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
                    let message = format!(
                        "{} while waiting for {command} output lock '{}'",
                        interruption.message(context.command()),
                        lock_path.display()
                    );
                    return Err(match interruption {
                        crate::use_cases::context::ExecutionInterruption::Cancelled => {
                            AppError::Cancelled(message)
                        }
                        crate::use_cases::context::ExecutionInterruption::TimedOut => {
                            AppError::TimedOut(message)
                        }
                    });
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
    executable: &Path,
    state: ConfigurationState,
    subject: &ConfigurationSubject,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    let extension = match subject {
        ConfigurationSubject::Main => None,
        ConfigurationSubject::Extension { name } => Some(name.as_str()),
    };
    let runner = crate::platform::process::ProcessExecutor;
    let result = match provider {
        ExportProvider::DesignerBatch => {
            let log = provider_log_path(config, "configuration-export")?;
            let dsl = DesignerDsl::new(
                executable.to_path_buf(),
                config.v8_connection(),
                &runner,
                Some(log),
            )
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
            IbcmdDsl::new(executable.to_path_buf(), connection, &runner)
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
    executable: &Path,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    match provider {
        ExportProvider::DesignerBatch => {
            let runner = crate::platform::process::ProcessExecutor;
            let log = provider_log_path(config, "infobase-dump")?;
            DesignerDsl::new(
                executable.to_path_buf(),
                config.v8_connection(),
                &runner,
                Some(log),
            )
            .with_execution_policy(
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            )
            .dump_infobase(staging_path)
            .map_err(AppError::from)
        }
        ExportProvider::IbcmdProcess => Err(AppError::CapabilityUnavailable(
            "IBCMD DT export is experimental and cannot be dispatched".to_owned(),
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
        ConfigurationSubject, ExportProvider, ProviderImplementation,
    };
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;
    use crate::use_cases::context::ExecutionContext;
    use crate::use_cases::result::UseCaseErrorKind;

    use super::{
        acquire_target_lock, capability, cleanup_export_orphans, observe_locked_output,
        record_execution_failure, resolve_output, revalidate_before_publish,
        revalidate_output_observation, validate_configuration_output, validate_snapshot_output,
        ExportIntent, ExportPhase, SNAPSHOT_COMMAND,
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
    fn snapshot_output_is_dt_and_ibcmd_remains_experimental() {
        assert!(validate_snapshot_output(Path::new("dist/base.dt")).is_ok());
        assert!(validate_snapshot_output(Path::new("dist/base.backup")).is_err());

        let (designer, _, _) = capability(ExportIntent::Snapshot, ExportProvider::DesignerBatch);
        assert_eq!(designer, ProviderImplementation::Implemented);
        let (ibcmd, _, _) = capability(ExportIntent::Snapshot, ExportProvider::IbcmdProcess);
        assert_eq!(ibcmd, ProviderImplementation::Experimental);
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

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn case_aliases_for_absent_output_share_one_target_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output_dir = dir.path().join("shared");
        std::fs::create_dir_all(&output_dir).expect("output parent");
        let config = config(&dir.path().join("base"), &dir.path().join("work"));

        let lower = resolve_output(&config, &output_dir.join("base.dt")).expect("lower target");
        let upper = resolve_output(&config, &output_dir.join("BASE.DT")).expect("upper target");

        assert_eq!(lower.lock_path, upper.lock_path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unicode_normalization_aliases_for_absent_output_share_one_target_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output_dir = dir.path().join("shared");
        std::fs::create_dir_all(&output_dir).expect("output parent");
        let config = config(&dir.path().join("base"), &dir.path().join("work"));

        let nfc = resolve_output(&config, &output_dir.join("caf\u{e9}.dt")).expect("NFC target");
        let nfd = resolve_output(&config, &output_dir.join("cafe\u{301}.dt")).expect("NFD target");

        assert_eq!(nfc.lock_path, nfd.lock_path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn full_unicode_casefold_aliases_for_absent_output_share_one_target_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output_dir = dir.path().join("shared");
        std::fs::create_dir_all(&output_dir).expect("output parent");
        let config = config(&dir.path().join("base"), &dir.path().join("work"));

        for (first, second) in [
            ("Stra\u{df}e.dt", "STRASSE.DT"),
            ("\u{fb00}.dt", "ff.dt"),
            ("\u{3c2}.dt", "\u{3c3}.dt"),
        ] {
            let first = resolve_output(&config, &output_dir.join(first)).expect("first target");
            let second = resolve_output(&config, &output_dir.join(second)).expect("second target");
            assert_eq!(first.lock_path, second.lock_path);
        }
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

        assert!(matches!(error, AppError::TimedOut(_)));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn target_lock_wait_reports_typed_cancellation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_path = dir.path().join("target.lock");
        let _guard = crate::support::fs::acquire_advisory_lock(&lock_path).expect("held lock");
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump)
            .with_cancellation(cancellation);

        let error = acquire_target_lock(&context, &lock_path, SNAPSHOT_COMMAND)
            .expect_err("cancellation must stop lock wait");

        assert!(matches!(error, AppError::Cancelled(_)));
    }

    #[test]
    fn provider_selection_observes_the_shared_command_deadline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&base).expect("base");
        let config = config(&base, &work);
        let request = crate::domain::infobase_export::ExportInfobaseSnapshotRequest {
            output: base.join("base.dt"),
        };
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump)
            .with_deadline(Some(Instant::now() - Duration::from_millis(1)));

        let failure = super::prepare_infobase_snapshot(&context, &config, &request)
            .expect_err("expired deadline");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::TimedOut);
        let result = failure.payload.expect("typed payload");
        assert_eq!(result.execution.status, ExecutionStatus::TimedOut);
        assert_eq!(result.execution.errors[0].code, "timed_out");
    }

    #[test]
    fn orphan_cleanup_removes_only_owned_stale_export_files() {
        use crate::support::fs::{
            metadata_sidecar_path, read_temp_dir_metadata, write_temp_dir_metadata, TempDirKind,
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&base).expect("base");
        let config = config(&base, &work);
        let output = base.join("dist/main.cf");
        std::fs::create_dir_all(output.parent().expect("parent")).expect("output parent");
        let resolved = resolve_output(&config, &output).expect("resolved");
        let stage = output
            .parent()
            .expect("parent")
            .join(".infobase-config-stage-old-run.cf");
        std::fs::write(&stage, "payload").expect("stage");
        write_temp_dir_metadata(
            &stage,
            TempDirKind::Stage,
            "old-run",
            &resolved.target,
            &resolved.identity,
        )
        .expect("metadata");
        let metadata_path = metadata_sidecar_path(&stage);
        let mut metadata = read_temp_dir_metadata(&stage).expect("read metadata");
        metadata.created_at -= chrono::Duration::days(2);
        std::fs::write(&metadata_path, serde_json::to_vec(&metadata).expect("json"))
            .expect("rewrite metadata");

        cleanup_export_orphans(&resolved, &[".infobase-config-stage-"]).expect("cleanup");

        assert!(!stage.exists());
        assert!(!metadata_path.exists());
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

        record_execution_failure(
            &context,
            &error,
            ExportPhase::ProviderCommand,
            &mut execution,
        );

        assert_eq!(execution.status, ExecutionStatus::Cancelled);
        assert_eq!(execution.errors[0].code, "cancelled");
        assert!(!execution.errors[0].retryable);
        assert_eq!(execution.interruptions.len(), 1);
    }

    #[test]
    fn unrelated_failure_is_not_reclassified_by_an_expired_deadline() {
        let context = ExecutionContext::cli(
            crate::use_cases::context::CommandName::InfobaseConfigurationExport,
        )
        .with_deadline(Some(Instant::now() - Duration::from_millis(1)));
        let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed);
        let error = AppError::Runtime("publication failed".to_owned());

        record_execution_failure(&context, &error, ExportPhase::Publication, &mut execution);

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert_eq!(execution.errors[0].code, "runtime_failure");
        assert!(execution.interruptions.is_empty());
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
        let observation = observe_locked_output(&output).expect("observation");
        let publication = crate::use_cases::staged_publication::StagedPublication::prepare_file(
            &output.target,
            &output.identity,
            ".infobase-config-stage",
            "cf",
        )
        .expect("publication");
        std::fs::write(publication.staging_path(), "payload").expect("stage");
        let stage = publication.staging_path().to_path_buf();
        let sidecar = crate::support::fs::metadata_sidecar_path(&stage);

        std::fs::remove_file(&alias).expect("remove old alias");
        symlink(&replacement_parent, &alias).expect("retarget alias");

        let error = revalidate_before_publish(&output, &observation, &publication)
            .expect_err("identity change");
        assert!(error.to_string().contains("identity changed"));
        assert!(!stage.exists());
        assert!(!sidecar.exists());
    }

    #[test]
    fn publication_rejects_target_created_after_locked_observation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = config(dir.path(), &dir.path().join("work"));
        let output = resolve_output(&config, &dir.path().join("main.cf")).expect("output");
        let observation = observe_locked_output(&output).expect("observation");
        let publication = crate::use_cases::staged_publication::StagedPublication::prepare_file(
            &output.target,
            &output.identity,
            ".infobase-config-stage",
            "cf",
        )
        .expect("publication");
        std::fs::write(publication.staging_path(), "payload").expect("stage");
        let stage = publication.staging_path().to_path_buf();
        let sidecar = crate::support::fs::metadata_sidecar_path(&stage);
        std::fs::write(&output.target, "external target").expect("external target");

        let error = revalidate_before_publish(&output, &observation, &publication)
            .expect_err("target appearance");

        assert!(error.to_string().contains("target changed"));
        assert_eq!(
            std::fs::read_to_string(&output.target).expect("target"),
            "external target"
        );
        assert!(!stage.exists());
        assert!(!sidecar.exists());
    }

    #[cfg(unix)]
    #[test]
    fn output_observation_detects_parent_replacement_at_the_same_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("output");
        std::fs::create_dir(&parent).expect("parent");
        let config = config(dir.path(), &dir.path().join("work"));
        let output = resolve_output(&config, &parent.join("main.cf")).expect("output");
        let observation = observe_locked_output(&output).expect("observation");

        std::fs::rename(&parent, dir.path().join("moved-output")).expect("move parent");
        std::fs::create_dir(&parent).expect("replacement parent");

        let error =
            revalidate_output_observation(&output, &observation).expect_err("parent replacement");
        assert!(error.to_string().contains("output parent changed"));
    }
}
