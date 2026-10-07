use std::io::ErrorKind;
mod agent;
mod all;

pub use self::all::execute_configuration_export_all;

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::model::AppConfig;
use crate::domain::capability::{
    database_configuration_exporters, exports_database_configuration, Operation, Provider,
    ProviderPlan, ProviderReceipt, SkippedProvider,
};
use crate::domain::execution::{
    ExecutionError, ExecutionInterruptionPhase, ExecutionOutcome, ExecutionStatus, StepResult,
};
use crate::domain::infobase_export::{
    ConfigurationState, ConfigurationSubject, ExportConfigurationPackageRequest,
    ExportConfigurationPackageResult, ExportInfobaseSnapshotRequest, ExportInfobaseSnapshotResult,
    InfobaseTargetState, InfobaseTransferPhase, RestoreInfobaseSnapshotRequest,
    RestoreInfobaseSnapshotResult, RestoreTargetMode,
};
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessError;
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
    deferred_command_interruption_details, pending_interruption_error, record_cancellation,
    record_deferral, timed_out_record, CommandFailure,
};
use super::staged_publication::{
    cleanup_owned_orphan_files, interruption_before_publish, PublicationFailureState,
    StagedPublication,
};

const CONFIGURATION_COMMAND: &str = "infobase.configuration.export";
const SNAPSHOT_COMMAND: &str = "infobase.dump";

#[derive(Debug, Clone)]
pub struct PreparedTransferProvider {
    receipt: ProviderReceipt,
    provider: Provider,
    /// `None` у исполнителя без утилиты на этой машине — чужого агента.
    executable: Option<PathBuf>,
    /// Состояние конфигурации, под которое выбран исполнитель выгрузки пакета; `None` у
    /// снимка и подъёма базы. Исполнение пакета сверяет с ним свой запрос: исполнителя,
    /// выбранного для рабочего состояния, конфигурацией базы данных не нагружают.
    configuration_state: Option<ConfigurationState>,
}

impl PreparedTransferProvider {
    pub fn receipt(&self) -> &ProviderReceipt {
        &self.receipt
    }
}

/// Квитанция ответа называет точку входа сессии агента, если исполнение шло через неё.
#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<ExportConfigurationPackageResult> {
    crate::use_cases::provider_selection::stamp_session(
        run_configuration_export(context, config, request, prepared),
        context,
    )
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
fn run_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<ExportConfigurationPackageResult> {
    let mut result =
        ExportConfigurationPackageResult::new(request.clone(), Some(prepared.receipt.clone()));
    if let Err(error) = validate_configuration_request(request) {
        return Err(configuration_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }
    if prepared.configuration_state != Some(request.state) {
        let error = AppError::Runtime(format!(
            "the executor was selected for another configuration state than the requested {}",
            request.state.as_str()
        ));
        return Err(configuration_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }
    let provider = prepared.provider;

    let output = resolve_output(config, &request.output).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::ResolveTarget,
        )
    })?;
    result.output = output.target.clone();
    let _target_lock = acquire_target_lock(
        context,
        &output.lock_path,
        CONFIGURATION_COMMAND,
        TARGET_LOCK_WAIT,
    )
    .map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::TargetLock,
        )
    })?;
    let output_observation = observe_locked_output(&output).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::ResolveTarget,
        )
    })?;
    cleanup_export_orphans(&output, &[".infobase-config-stage-"]).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::OrphanCleanup,
        )
    })?;
    let publication = StagedPublication::prepare_file(
        &output.target,
        &output.identity,
        ".infobase-config-stage",
        request.subject.artifact_kind().file_extension(),
    )
    .map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::PrepareStaging,
        )
    })?;

    let provider_started = Instant::now();
    let platform_result = match run_configuration_provider(
        context,
        config,
        provider,
        prepared.executable.as_deref(),
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
                InfobaseTransferPhase::ProviderCommand,
            ))
        }
    };
    if let Err(error) = validate_platform_success(&platform_result) {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            InfobaseTransferPhase::ProviderCommand,
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            InfobaseTransferPhase::ProviderCommand.as_str(),
            InfobaseTransferPhase::ProviderCommand.kind(),
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(publication.staging_path().display().to_string()),
    );
    if let Err(error) = validate_platform_artifact(publication.staging_path()) {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            InfobaseTransferPhase::ValidateProviderOutput,
        ));
    }
    record_deferral(
        ExecutionInterruptionPhase::ProviderCommand,
        "configuration export",
        platform_result.process.interruption,
        &mut result.execution,
        &mut result.warnings,
    );
    if let Some(error) = interruption_before_publish(context, "configuration package publication") {
        return Err(configuration_failure(
            context,
            publication.cleanup_failure(error),
            result,
            InfobaseTransferPhase::BeforePublication,
        ));
    }
    revalidate_before_publish(&output, &output_observation, &publication).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::PublishTargetRevalidation,
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
                InfobaseTransferPhase::Publication,
            )
        })?;
    result.steps.push(
        StepResult::succeeded(
            InfobaseTransferPhase::Publication.as_str(),
            InfobaseTransferPhase::Publication.kind(),
            publication_started.elapsed().as_millis() as u64,
        )
        .with_target(result.output.display().to_string()),
    );
    result.published = true;
    result.target_state = if publication_outcome.previous_target_present {
        InfobaseTargetState::Replaced
    } else {
        InfobaseTargetState::Created
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
                ExecutionInterruptionPhase::Publication,
                message,
            ));
    }
    Ok(result)
}

/// Квитанция ответа называет точку входа сессии агента, если исполнение шло через неё.
#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<ExportInfobaseSnapshotResult> {
    crate::use_cases::provider_selection::stamp_session(
        run_infobase_snapshot(context, config, request, prepared),
        context,
    )
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
fn run_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<ExportInfobaseSnapshotResult> {
    let mut result =
        ExportInfobaseSnapshotResult::new(request.clone(), Some(prepared.receipt.clone()));
    if let Err(error) = validate_snapshot_output(&request.output) {
        return Err(snapshot_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }
    let provider = prepared.provider;

    let output = resolve_output(config, &request.output).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::ResolveTarget,
        )
    })?;
    result.output = output.target.clone();
    let _target_lock = acquire_target_lock(
        context,
        &output.lock_path,
        SNAPSHOT_COMMAND,
        TARGET_LOCK_WAIT,
    )
    .map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::TargetLock,
        )
    })?;
    let output_observation = observe_locked_output(&output).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::ResolveTarget,
        )
    })?;
    cleanup_export_orphans(&output, &[".infobase-dt-stage-"]).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::OrphanCleanup,
        )
    })?;
    let publication = StagedPublication::prepare_file(
        &output.target,
        &output.identity,
        ".infobase-dt-stage",
        "dt",
    )
    .map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::PrepareStaging,
        )
    })?;

    let provider_started = Instant::now();
    let platform_result = match run_snapshot_provider(
        context,
        config,
        provider,
        prepared.executable.as_deref(),
        publication.staging_path(),
    ) {
        Ok(platform_result) => platform_result,
        Err(error) => {
            return Err(snapshot_failure(
                context,
                publication.cleanup_failure(error),
                result,
                InfobaseTransferPhase::ProviderCommand,
            ))
        }
    };
    if let Err(error) = validate_platform_success(&platform_result) {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            InfobaseTransferPhase::ProviderCommand,
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            InfobaseTransferPhase::ProviderCommand.as_str(),
            InfobaseTransferPhase::ProviderCommand.kind(),
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(publication.staging_path().display().to_string()),
    );
    if let Err(error) = validate_platform_artifact(publication.staging_path()) {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            InfobaseTransferPhase::ValidateProviderOutput,
        ));
    }
    record_deferral(
        ExecutionInterruptionPhase::ProviderCommand,
        "infobase DT export",
        platform_result.process.interruption,
        &mut result.execution,
        &mut result.warnings,
    );
    if let Some(error) = interruption_before_publish(context, "infobase DT publication") {
        return Err(snapshot_failure(
            context,
            publication.cleanup_failure(error),
            result,
            InfobaseTransferPhase::BeforePublication,
        ));
    }
    revalidate_before_publish(&output, &output_observation, &publication).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::PublishTargetRevalidation,
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
                InfobaseTransferPhase::Publication,
            )
        })?;
    result.steps.push(
        StepResult::succeeded(
            InfobaseTransferPhase::Publication.as_str(),
            InfobaseTransferPhase::Publication.kind(),
            publication_started.elapsed().as_millis() as u64,
        )
        .with_target(result.output.display().to_string()),
    );
    result.published = true;
    result.target_state = if publication_outcome.previous_target_present {
        InfobaseTargetState::Replaced
    } else {
        InfobaseTargetState::Created
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
                ExecutionInterruptionPhase::Publication,
                message,
            ));
    }
    Ok(result)
}

/// Validates the restore request without touching the infobase.
///
/// The DT suffix and the readable input are checked here, before provider selection,
/// so a typo never reaches a process that would replace an infobase.
pub(crate) fn validate_restore_request(
    request: &RestoreInfobaseSnapshotRequest,
) -> Result<(), AppError> {
    validate_output_suffix(&request.input, "dt")?;
    let metadata = std::fs::symlink_metadata(&request.input).map_err(|error| {
        AppError::Validation(format!(
            "--input '{}' is not readable: {error}",
            request.input.display()
        ))
    })?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(AppError::Validation(format!(
            "--input '{}' is not a non-empty regular file",
            request.input.display()
        )));
    }
    Ok(())
}

/// Refuses a restore whose requested target mode does not match the observed infobase.
///
/// This runs before provider selection and again under the workspace lock, because the
/// provider replaces the infobase in place and no staging step can undo it.
pub(crate) fn validate_restore_target(
    config: &AppConfig,
    mode: RestoreTargetMode,
) -> Result<bool, AppError> {
    let target_present = observe_target_infobase(config)?;
    match (mode, target_present) {
        (RestoreTargetMode::Create, true) => Err(AppError::Validation(
            "--create was requested but the target infobase already exists; pass --replace to discard its data"
                .to_owned(),
        )),
        (RestoreTargetMode::Replace, false) => Err(AppError::Validation(
            "--replace was requested but the target infobase does not exist; pass --create to create it"
                .to_owned(),
        )),
        _ => Ok(target_present),
    }
}

pub fn prepare_infobase_restore(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &RestoreInfobaseSnapshotRequest,
) -> Result<PreparedTransferProvider, UseCaseFailure<RestoreInfobaseSnapshotResult>> {
    if let Err(error) = validate_restore_request(request) {
        let result = RestoreInfobaseSnapshotResult::new(request.clone(), None);
        return Err(restore_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }

    if let Err(error) = validate_restore_target(config, request.target_mode) {
        let result = RestoreInfobaseSnapshotResult::new(request.clone(), None);
        return Err(restore_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }

    let intent = InfobaseTransferIntent::SnapshotRestore {
        expects_absent_target: request.target_mode == RestoreTargetMode::Create,
    };
    match select_provider(context, config, intent) {
        Ok(prepared) => Ok(prepared),
        Err((error, receipt)) => {
            let result = RestoreInfobaseSnapshotResult::new(request.clone(), Some(receipt));
            Err(restore_failure(
                context,
                error,
                result,
                InfobaseTransferPhase::ProviderSelection,
            ))
        }
    }
}

pub fn preview_infobase_restore(
    _context: &ExecutionContext,
    _config: &AppConfig,
    request: &RestoreInfobaseSnapshotRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<RestoreInfobaseSnapshotResult> {
    let mut result =
        RestoreInfobaseSnapshotResult::new(request.clone(), Some(prepared.receipt().clone()));
    result.mark_preview();
    Ok(result)
}

/// Loads the infobase from a DT file.
///
/// There is no staging step here, unlike an export: the provider writes straight into
/// the infobase, so the target mode checked during provider selection is the only
/// protection the caller gets, and it is checked again after the workspace lock.
///
/// Квитанция ответа называет точку входа сессии агента, если исполнение шло через неё.
#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
pub fn execute_infobase_restore(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &RestoreInfobaseSnapshotRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<RestoreInfobaseSnapshotResult> {
    crate::use_cases::provider_selection::stamp_session(
        run_infobase_restore(context, config, request, prepared),
        context,
    )
}

#[allow(clippy::result_large_err)] // Failure payload preserves the typed AI-facing result.
fn run_infobase_restore(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &RestoreInfobaseSnapshotRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<RestoreInfobaseSnapshotResult> {
    let mut result =
        RestoreInfobaseSnapshotResult::new(request.clone(), Some(prepared.receipt.clone()));
    if let Err(error) = validate_restore_request(request) {
        return Err(restore_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }
    let target_present = match validate_restore_target(config, request.target_mode) {
        Ok(present) => present,
        Err(error) => {
            return Err(restore_failure(
                context,
                error,
                result,
                InfobaseTransferPhase::ResolveTarget,
            ))
        }
    };
    if let Some(error) = interruption_before_publish(context, "infobase DT restore") {
        return Err(restore_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::BeforePublication,
        ));
    }

    let provider_started = Instant::now();
    let platform_result = match run_restore_provider(
        context,
        config,
        prepared.provider,
        prepared.executable.as_deref(),
        &request.input,
    ) {
        Ok(platform_result) => platform_result,
        Err(failure) => {
            // Базу мог тронуть только исполнитель, получивший работу. Отказ до неё — отмена
            // до запуска, исполнитель, которого не собрать, — оставляет цель как была.
            if context.work().given() {
                result.target_state = InfobaseTargetState::Uncertain;
                record_uncertain_target_warning(&mut result.warnings, result.target_state);
            }
            // Отмена, отложенная до конца критической фазы, названа и у неудачи — как на
            // пути Конфигуратора ниже, в том же порядке предупреждений.
            let error = failure.record_into(
                ExecutionInterruptionPhase::ProviderCommand,
                "infobase DT restore",
                &mut result.execution,
                &mut result.warnings,
            );
            return Err(restore_failure(
                context,
                error,
                result,
                InfobaseTransferPhase::ProviderCommand,
            ));
        }
    };
    if let Err(error) = validate_platform_success(&platform_result) {
        // The provider may have replaced part of the data before failing, and nothing
        // here can tell how much, so the target state is reported as uncertain.
        result.target_state = InfobaseTargetState::Uncertain;
        record_uncertain_target_warning(&mut result.warnings, result.target_state);
        // Отмена, отложенная до конца критической фазы, названа и у неудачной загрузки:
        // оператор просил остановить, и ответ говорит, почему его не послушали.
        record_deferral(
            ExecutionInterruptionPhase::ProviderCommand,
            "infobase DT restore",
            platform_result.process.interruption,
            &mut result.execution,
            &mut result.warnings,
        );
        return Err(restore_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::ProviderCommand,
        ));
    }
    result.steps.push(
        StepResult::succeeded(
            InfobaseTransferPhase::ProviderCommand.as_str(),
            InfobaseTransferPhase::ProviderCommand.kind(),
            provider_started.elapsed().as_millis() as u64,
        )
        .with_target(request.input.display().to_string()),
    );
    record_deferral(
        ExecutionInterruptionPhase::ProviderCommand,
        "infobase DT restore",
        platform_result.process.interruption,
        &mut result.execution,
        &mut result.warnings,
    );
    result.restored = true;
    result.target_state = if target_present {
        InfobaseTargetState::Replaced
    } else {
        InfobaseTargetState::Created
    };
    result.mark_succeeded();
    Ok(result)
}

/// Reports whether the configured target infobase already holds data.
///
/// Only a file infobase can be observed without a process; for a server infobase the
/// restore mode is taken on the caller's word and the platform has the final say.
fn observe_target_infobase(config: &AppConfig) -> Result<bool, AppError> {
    let connection = config.v8_connection();
    let Some(file_path) = connection.file_path() else {
        return Ok(true);
    };
    Ok(Path::new(file_path).join("1Cv8.1CD").is_file())
}

fn run_restore_provider(
    context: &ExecutionContext,
    config: &AppConfig,
    provider: Provider,
    executable: Option<&Path>,
    source_file: &Path,
) -> Result<PlatformCommandResult, CommandFailure> {
    match provider {
        // Исполнитель без адаптера: отказ, а не паника — строка матрицы опередила код.
        other @ (Provider::IbcmdRs | Provider::Webinst) => Err(CommandFailure::without_deferral(
            crate::use_cases::unimplemented_provider(
                crate::domain::capability::Operation::InfobaseRestore,
                other,
            ),
        )),
        Provider::Agent => agent::restore_snapshot(context, config, executable, source_file),
        Provider::Designer => {
            let executable = executable_of(executable).map_err(CommandFailure::without_deferral)?;
            let runner = crate::platform::process::ProcessExecutor;
            let log = provider_log_path(config, "infobase-restore")
                .map_err(CommandFailure::without_deferral)?;
            // Загрузка снимка подменяет базу целиком: фаза критическая, как у `restore-ib`
            // агента. Снятый посреди записи Конфигуратор оставил бы базу в состоянии,
            // которое не назовёт никто.
            DesignerDsl::new(
                executable.to_path_buf(),
                config.v8_connection(),
                &runner,
                Some(log),
                context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
            )
            .restore_infobase(source_file)
            // Отказ раннера фактом отсрочки не владеет: процесс, отложивший отмену,
            // отвечает результатом, а не ошибкой.
            .map_err(|error| CommandFailure::without_deferral(AppError::from(error)))
        }
        Provider::Ibcmd => Err(CommandFailure::without_deferral(AppError::capability(
            "IBCMD DT restore is experimental and cannot be dispatched".to_owned(),
        ))),
    }
}

fn restore_failure(
    context: &ExecutionContext,
    error: AppError,
    mut result: RestoreInfobaseSnapshotResult,
    phase: InfobaseTransferPhase,
) -> UseCaseFailure<RestoreInfobaseSnapshotResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(infobase_use_case_error(error), result)
}

pub(crate) fn validate_configuration_output(
    subject: &ConfigurationSubject,
    output: &Path,
) -> Result<(), AppError> {
    validate_output_suffix(output, subject.artifact_kind().file_extension())
}

/// Снимок базы пишется только в `.dt`. Пакет конфигурации — работа соседней команды, и
/// отказ называет её, а не одно лишь ожидаемое расширение.
pub(crate) fn validate_snapshot_output(output: &Path) -> Result<(), AppError> {
    let package = output
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| value.eq_ignore_ascii_case("cf") || value.eq_ignore_ascii_case("cfe"));
    if let Some(suffix) = package {
        return Err(AppError::Validation(format!(
            "output '{}' must have .dt suffix: infobase dump writes a transfer file of the whole infobase, a .{suffix} configuration package is taken by `download`",
            output.display()
        )));
    }
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

fn export_failure_state(state: PublicationFailureState) -> InfobaseTargetState {
    match state {
        PublicationFailureState::Unchanged => InfobaseTargetState::Unchanged,
        PublicationFailureState::Restored => InfobaseTargetState::Restored,
        PublicationFailureState::Uncertain => InfobaseTargetState::Uncertain,
    }
}

fn record_uncertain_target_warning(warnings: &mut Vec<String>, state: InfobaseTargetState) {
    if state == InfobaseTargetState::Uncertain {
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
) -> Result<PreparedTransferProvider, UseCaseFailure<ExportConfigurationPackageResult>> {
    if let Err(error) = validate_configuration_request(request) {
        let result = ExportConfigurationPackageResult::new(request.clone(), None);
        return Err(configuration_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }

    let intent = InfobaseTransferIntent::Configuration {
        state: request.state,
    };
    match select_provider(context, config, intent) {
        Ok(prepared) => Ok(prepared),
        Err((error, receipt)) => {
            let result = ExportConfigurationPackageResult::new(request.clone(), Some(receipt));
            Err(configuration_failure(
                context,
                error,
                result,
                InfobaseTransferPhase::ProviderSelection,
            ))
        }
    }
}

pub fn prepare_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
) -> Result<PreparedTransferProvider, UseCaseFailure<ExportInfobaseSnapshotResult>> {
    if let Err(error) = validate_snapshot_output(&request.output) {
        let result = ExportInfobaseSnapshotResult::new(request.clone(), None);
        return Err(snapshot_failure(
            context,
            error,
            result,
            InfobaseTransferPhase::Validation,
        ));
    }

    match select_provider(context, config, InfobaseTransferIntent::Snapshot) {
        Ok(prepared) => Ok(prepared),
        Err((error, receipt)) => {
            let result = ExportInfobaseSnapshotResult::new(request.clone(), Some(receipt));
            Err(snapshot_failure(
                context,
                error,
                result,
                InfobaseTransferPhase::ProviderSelection,
            ))
        }
    }
}

pub fn preview_configuration_export(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportConfigurationPackageRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<ExportConfigurationPackageResult> {
    let mut result =
        ExportConfigurationPackageResult::new(request.clone(), Some(prepared.receipt().clone()));
    result.mark_preview_failure();
    let output = resolve_output(config, &request.output).map_err(|error| {
        configuration_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::ResolveTarget,
        )
    })?;
    result.output = output.target;
    result.mark_preview();
    Ok(result)
}

pub fn preview_infobase_snapshot(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExportInfobaseSnapshotRequest,
    prepared: &PreparedTransferProvider,
) -> UseCaseResult<ExportInfobaseSnapshotResult> {
    let mut result =
        ExportInfobaseSnapshotResult::new(request.clone(), Some(prepared.receipt().clone()));
    result.mark_preview_failure();
    let output = resolve_output(config, &request.output).map_err(|error| {
        snapshot_failure(
            context,
            error,
            result.clone(),
            InfobaseTransferPhase::ResolveTarget,
        )
    })?;
    result.output = output.target;
    result.mark_preview();
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
enum InfobaseTransferIntent {
    Configuration { state: ConfigurationState },
    Snapshot,
    SnapshotRestore { expects_absent_target: bool },
}

impl InfobaseTransferIntent {
    const fn operation(self) -> Operation {
        match self {
            Self::Configuration { .. } => Operation::ConfigurationExport,
            Self::Snapshot => Operation::InfobaseDump,
            Self::SnapshotRestore { .. } => Operation::InfobaseRestore,
        }
    }

    const fn configuration_state(self) -> Option<ConfigurationState> {
        match self {
            Self::Configuration { state } => Some(state),
            Self::Snapshot | Self::SnapshotRestore { .. } => None,
        }
    }
}

fn select_provider(
    context: &ExecutionContext,
    config: &AppConfig,
    intent: InfobaseTransferIntent,
) -> Result<PreparedTransferProvider, (AppError, ProviderReceipt)> {
    use crate::use_cases::provider_selection::{
        no_adapter, no_executor, nobody_ready, utilities_of, without_a_way,
    };

    // Кандидаты приходят из матрицы: переопределение — один исполнитель без отката,
    // умолчание — цепочка, из которой берётся первый готовый. Исполнителя вне матрицы
    // сюда не пускает проверка настроек, а реализованность каждого держит
    // `domain::capability`: второго мнения о ней здесь нет.
    let operation = intent.operation();
    let plan = config.provider_plan(operation);
    let (plan, mut skipped) = match intent {
        InfobaseTransferIntent::Configuration {
            state: ConfigurationState::Database,
        } => database_configuration_plan(config, plan)?,
        InfobaseTransferIntent::Configuration {
            state: ConfigurationState::Working,
        }
        | InfobaseTransferIntent::Snapshot
        | InfobaseTransferIntent::SnapshotRestore { .. } => (plan, Vec::new()),
    };
    if plan.candidates().is_empty() {
        return Err((
            no_executor(config, operation),
            plan.receipt_for_nobody(skipped),
        ));
    }
    let mut utilities = PlatformUtilities::from_config(config);

    for provider in plan.candidates() {
        if let Some(error) = pending_interruption_error(context, "during provider selection") {
            let receipt = plan.receipt_for_nobody(skipped);
            return Err((error, receipt));
        }
        if let Some(skip) = without_a_way(config, operation, provider) {
            skipped.push(skip);
            continue;
        }
        let Some(needed) = utilities_of(provider, config) else {
            skipped.push(no_adapter(provider, operation));
            continue;
        };
        // Исполнителю переноса нужна не больше чем одна утилита: первая из его списка.
        debug_assert!(
            needed.len() <= 1,
            "a transfer executor needs at most one utility, {provider} needs {needed:?}"
        );
        let utility = needed.first().copied();
        match readiness(config, &mut utilities, intent, provider, utility) {
            Ok(executable) => {
                let receipt = plan.receipt_for(provider, skipped);
                return Ok(PreparedTransferProvider {
                    receipt,
                    provider,
                    executable,
                    configuration_state: intent.configuration_state(),
                });
            }
            Err(reason) => skipped.push(SkippedProvider { provider, reason }),
        }
    }

    let error = nobody_ready(config, &plan, &skipped);
    Err((error, plan.receipt_for_nobody(skipped)))
}

/// План `download --state db`: из цепочки умолчаний остаются те, кто выгружает
/// конфигурацию базы данных (`domain::capability::exports_database_configuration`), в её
/// порядке; прочие сразу попадают в пропущенные с причиной. Ключ `providers.download`,
/// назначивший того, кто её не выгружает, и цель, у которой в цепочке таких нет,
/// отказывают до выбора — до запуска платформы и до сессии агента.
fn database_configuration_plan(
    config: &AppConfig,
    plan: ProviderPlan,
) -> Result<(ProviderPlan, Vec<SkippedProvider>), (AppError, ProviderReceipt)> {
    let operation = Operation::ConfigurationExport;
    let (exporters, unable): (Vec<Provider>, Vec<Provider>) = plan
        .candidates()
        .into_iter()
        .partition(|provider| exports_database_configuration(*provider));
    let skipped = unable
        .into_iter()
        .map(|provider| SkippedProvider {
            provider,
            reason: format!(
                "{provider} has no command for the database configuration that {operation} --state db takes"
            ),
        })
        .collect::<Vec<_>>();
    if !exporters.is_empty() {
        let plan = match plan {
            ProviderPlan::Override { .. } => plan,
            ProviderPlan::Default { .. } => ProviderPlan::Default { chain: exporters },
        };
        return Ok((plan, skipped));
    }
    let unable = skipped
        .iter()
        .map(|entry| entry.provider.as_str())
        .collect::<Vec<_>>()
        .join(" and ");
    let named = database_configuration_exporters()
        .map(Provider::as_str)
        .collect::<Vec<_>>()
        .join(" or ");
    let reason = format!(
        "{operation} --state db takes the database configuration, which only {named} exports: {unable} has no command for it"
    );
    let refusal = match &plan {
        ProviderPlan::Override { provider, file } => format!(
            "{reason}; providers.{operation} in {file} assigns {provider}: remove the key or assign {named}"
        ),
        // Цепочка без экспортёров бывает только у автономного сервера, которому прямой шлюз
        // не объявлен: выход — объявить его или выгрузить рабочую конфигурацию.
        ProviderPlan::Default { .. } => {
            let declare = database_configuration_exporters()
                .find_map(|provider| {
                    config
                        .missing_way(Operation::ConfigurationExport, provider)
                        .map(|way| format!("{}, or ", way.undeclared(provider)))
                })
                .unwrap_or_default();
            format!(
                "{reason}; a {} target as declared serves {operation} only through the agent: {declare}omit --state db to export the working configuration",
                config.target_kind().as_str()
            )
        }
    };
    let receipt = plan.receipt_for_nobody(skipped);
    Err((AppError::capability(refusal), receipt))
}

fn readiness(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    intent: InfobaseTransferIntent,
    provider: Provider,
    utility: Option<UtilityType>,
) -> Result<Option<PathBuf>, String> {
    // Автономный сервер обслуживает существующую базу: файловых проверок нет, а
    // `--create` ему не адресовать.
    if config.infobase.standalone.is_some() {
        if matches!(
            intent,
            InfobaseTransferIntent::SnapshotRestore {
                expects_absent_target: true
            }
        ) {
            return Err(
                "a standalone server serves an existing infobase: restore it with --replace, not --create"
                    .to_owned(),
            );
        }
    } else {
        match intent {
            InfobaseTransferIntent::SnapshotRestore {
                expects_absent_target: true,
            } => validate_restore_target_connection(config)?,
            _ => validate_file_infobase_readiness(config)?,
        }
    }
    if provider == Provider::Ibcmd {
        IbcmdConnection::from_infobase(&config.infobase)
            .map_err(|error| format!("connection is not ready for IBCMD: {error}"))?;
    }
    match utility {
        Some(utility) => utilities
            .locate(utility)
            .map(|location| Some(location.path))
            .map_err(|error| format!("environment is not ready: {error}")),
        None => Ok(None),
    }
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

/// Readiness for a restore that creates its target: only the connection shape can be
/// judged here, because the infobase is expected not to exist yet. Whether the observed
/// target matches the requested mode is a request question, refused during validation.
fn validate_restore_target_connection(config: &AppConfig) -> Result<(), String> {
    if config.v8_connection().has_supported_shape() {
        return Ok(());
    }
    Err(
        "infobase connection is not ready: expected non-empty File=..., Srvr=...;Ref=..., or /S server\\ref"
            .to_owned(),
    )
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
    phase: InfobaseTransferPhase,
) -> UseCaseFailure<ExportConfigurationPackageResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(infobase_use_case_error(error), result)
}

fn snapshot_failure(
    context: &ExecutionContext,
    error: AppError,
    mut result: ExportInfobaseSnapshotResult,
    phase: InfobaseTransferPhase,
) -> UseCaseFailure<ExportInfobaseSnapshotResult> {
    record_execution_failure(context, &error, phase, &mut result.execution);
    result.steps.push(failed_step(phase, &error));
    UseCaseFailure::with_payload(infobase_use_case_error(error), result)
}

/// Истёкший срок процесса перенос называет сроком, а не отказом платформы. Отмену называет
/// `From` — одинаково для всех команд.
fn infobase_use_case_error(error: AppError) -> UseCaseError {
    match process_error(&error) {
        Some(ProcessError::TimedOut { .. }) => {
            UseCaseError::new(UseCaseErrorKind::TimedOut, error.to_string())
        }
        _ => error.into(),
    }
}

fn failed_step(phase: InfobaseTransferPhase, error: &AppError) -> StepResult {
    StepResult::failed(phase.as_str(), phase.kind(), 0).with_message(error.to_string())
}

fn record_execution_failure(
    _context: &ExecutionContext,
    error: &AppError,
    phase: InfobaseTransferPhase,
    execution: &mut ExecutionOutcome<()>,
) {
    let message = error.to_string();
    // Отмену и её место называет ошибка: безопасная точка — `command_boundary`, где бы её
    // ни проверили, оборванная работа исполнителя — фаза шага.
    if let Some(at) = error.cancellation() {
        record_cancellation(execution, at, phase.interruption_phase(), message);
        return;
    }
    let mut interruption_details = None;
    let (status, code) = match process_error(error) {
        Some(ProcessError::TimedOut { .. }) => {
            interruption_details = Some(timed_out_record(phase.interruption_phase(), &message));
            (ExecutionStatus::TimedOut, "timed_out")
        }
        // Сюда `timed_out` вне процесса приходит только от шага — например от завершения
        // агентской сессии, у которого предел свой. Срок команды его дать не может, поэтому
        // улика записывается как процессная, а не командная. Код шага и статус прочих
        // отказов выводятся из рода отказа — единственного отображения `AppError` в код.
        _ => {
            let kind = UseCaseErrorKind::of(error);
            if kind == UseCaseErrorKind::TimedOut {
                interruption_details = Some(timed_out_record(phase.interruption_phase(), &message));
            }
            (kind.execution_status(), kind.execution_step_code())
        }
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

fn resolve_output(config: &AppConfig, requested: &Path) -> Result<ResolvedOutput, AppError> {
    let requested = crate::support::path::resolve_from(&config.base_path, requested);
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

/// How long an export waits for another run to release the same output target.
///
/// This is a step bound, not a command deadline: the lock guards a file we own, and a
/// conflict that has not cleared in this window is a second run writing the same target,
/// not a slow platform operation. Waiting is a courtesy for back-to-back commands that
/// briefly overlap; past it the honest answer is that the target is busy.
const TARGET_LOCK_WAIT: Duration = Duration::from_secs(300);

const TARGET_LOCK_POLL: Duration = Duration::from_millis(25);

fn acquire_target_lock(
    context: &ExecutionContext,
    lock_path: &Path,
    command: &str,
    wait: Duration,
) -> Result<crate::support::fs::AdvisoryLockGuard, AppError> {
    let waiting_since = Instant::now();
    loop {
        match try_acquire_advisory_lock(lock_path) {
            Ok(guard) => return Ok(guard),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if let Some(error) = pending_interruption_error(
                    context,
                    format!(
                        "while waiting for {command} output lock '{}'",
                        lock_path.display()
                    ),
                ) {
                    return Err(error);
                }
                if waiting_since.elapsed() >= wait {
                    return Err(AppError::WorkspaceBusy(format!(
                        "another run still holds the {command} output lock '{}' after {}ms; finish or stop it, or send this run to a different output",
                        lock_path.display(),
                        wait.as_millis()
                    )));
                }
                thread::sleep(TARGET_LOCK_POLL);
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
    provider: Provider,
    executable: Option<&Path>,
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
        // Исполнитель без адаптера: отказ, а не паника — строка матрицы опередила код.
        other @ (Provider::IbcmdRs | Provider::Webinst) => {
            return Err(crate::use_cases::unimplemented_provider(
                crate::domain::capability::Operation::ConfigurationExport,
                other,
            ));
        }
        Provider::Agent => {
            return agent::export_configuration(
                context,
                config,
                executable,
                extension,
                staging_path,
            )
            // Выгрузка критических команд не ведёт, отложенной отмены у её сессии не бывает.
            .map_err(|failure| failure.into_error("configuration export"));
        }
        Provider::Designer => {
            let executable = executable_of(executable)?;
            let log = provider_log_path(config, "configuration-export")?;
            let dsl = DesignerDsl::new(
                executable.to_path_buf(),
                config.v8_connection(),
                &runner,
                Some(log),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            );
            match state {
                ConfigurationState::Working => dsl.dump_cfg(staging_path, extension),
                ConfigurationState::Database => dsl.dump_db_cfg(staging_path, extension),
            }
            .map_err(AppError::from)?
        }
        Provider::Ibcmd => {
            let executable = executable_of(executable)?;
            let connection =
                IbcmdConnection::from_infobase(&config.infobase).map_err(AppError::from)?;
            let data_path = config.work_path.join("ibcmd-data");
            std::fs::create_dir_all(&data_path).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to create IBCMD data directory '{}': {error}",
                    data_path.display()
                ))
            })?;
            IbcmdDsl::new(
                executable.to_path_buf(),
                connection,
                &runner,
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            )
            .with_data_path(data_path)
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
    provider: Provider,
    executable: Option<&Path>,
    staging_path: &Path,
) -> Result<PlatformCommandResult, AppError> {
    match provider {
        // Исполнитель без адаптера: отказ, а не паника — строка матрицы опередила код.
        other @ (Provider::IbcmdRs | Provider::Webinst) => {
            Err(crate::use_cases::unimplemented_provider(
                crate::domain::capability::Operation::InfobaseDump,
                other,
            ))
        }
        // Выгрузка критических команд не ведёт, отложенной отмены у её сессии не бывает.
        Provider::Agent => agent::export_snapshot(context, config, executable, staging_path)
            .map_err(|failure| failure.into_error("infobase DT export")),
        Provider::Designer => {
            let executable = executable_of(executable)?;
            let runner = crate::platform::process::ProcessExecutor;
            let log = provider_log_path(config, "infobase-dump")?;
            DesignerDsl::new(
                executable.to_path_buf(),
                config.v8_connection(),
                &runner,
                Some(log),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            )
            .dump_infobase(staging_path)
            .map_err(AppError::from)
        }
        Provider::Ibcmd => Err(AppError::capability(
            "IBCMD DT export is experimental and cannot be dispatched".to_owned(),
        )),
    }
}

/// Утилита, без которой пакетному исполнителю не работать; её отсутствие после
/// выбора — ошибка раннера, а не среды.
fn executable_of(executable: Option<&Path>) -> Result<&Path, AppError> {
    executable
        .ok_or_else(|| AppError::Runtime("executor was selected without its utility".to_owned()))
}

fn provider_log_path(config: &AppConfig, stem: &str) -> Result<PathBuf, AppError> {
    platform_logs_dir(&config.work_path)
        .map(|dir| dir.join(format!("{stem}.log")))
        .map_err(|error| AppError::Runtime(format!("failed to create platform logs dir: {error}")))
}

fn validate_platform_success(result: &PlatformCommandResult) -> Result<(), AppError> {
    if let Err(code) = result.process.outcome() {
        let mut details = vec![format!("platform export failed with exit code {code}")];
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
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use crate::config::model::{
        AppConfig, InfobaseConfig, McpConfig, SourceFormat, TestsConfig, ToolsConfig,
    };
    use crate::domain::capability::Provider;
    use crate::domain::execution::{
        ExecutionInterruptionKind, ExecutionInterruptionPhase, ExecutionOutcome, ExecutionStatus,
    };
    use crate::domain::infobase_export::{
        ConfigurationSubject, ExportInfobaseSnapshotRequest, ExportInfobaseSnapshotResult,
    };
    use crate::platform::locator::{LocatorError, UtilityType};
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;
    use crate::use_cases::context::ExecutionContext;
    use crate::use_cases::result::UseCaseErrorKind;

    use super::{
        acquire_target_lock, cleanup_export_orphans, observe_locked_output,
        record_execution_failure, resolve_output, revalidate_before_publish,
        revalidate_output_observation, snapshot_failure, validate_configuration_output,
        validate_snapshot_output, InfobaseTransferPhase, SNAPSHOT_COMMAND, TARGET_LOCK_WAIT,
    };

    fn config(base: &Path, work: &Path) -> AppConfig {
        AppConfig {
            base_path: base.to_path_buf(),
            work_path: work.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: Vec::new(),
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
    fn snapshot_output_is_dt() {
        assert!(validate_snapshot_output(Path::new("dist/base.dt")).is_ok());
        assert!(validate_snapshot_output(Path::new("dist/base.backup")).is_err());
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

    /// Ожидание чужой блокировки — шаг со своим пределом, а не остаток срока команды.
    ///
    /// Срока у команды нет (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE), и без этого предела
    /// цикл крутился бы до Ctrl+C. Отказ обязан называться занятостью, а не таймаутом:
    /// держит цель другой прогон, и ждать дальше бессмысленно.
    #[test]
    fn target_lock_wait_gives_up_and_names_the_run_that_holds_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock_path = dir.path().join("target.lock");
        let _guard = crate::support::fs::acquire_advisory_lock(&lock_path).expect("held lock");
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump);
        let started = Instant::now();

        let error = acquire_target_lock(
            &context,
            &lock_path,
            SNAPSHOT_COMMAND,
            Duration::from_millis(40),
        )
        .expect_err("the wait window must end the lock wait");

        assert!(
            matches!(error, AppError::WorkspaceBusy(_)),
            "a held target is busy, not timed out: {error:?}"
        );
        assert!(
            error.to_string().contains(&lock_path.display().to_string()),
            "the refusal must name the lock it waited on: {error}"
        );
        assert!(started.elapsed() < Duration::from_millis(400));
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

        // Окно ожидания нарочно полное: пройти этот тест можно только через прерывание.
        let error = acquire_target_lock(&context, &lock_path, SNAPSHOT_COMMAND, TARGET_LOCK_WAIT)
            .expect_err("cancellation must stop lock wait");

        assert_eq!(
            error.cancellation(),
            Some(crate::support::error::CancelledAt::Boundary)
        );
    }

    #[test]
    fn provider_selection_observes_the_operators_interrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&base).expect("base");
        let config = config(&base, &work);
        let request = crate::domain::infobase_export::ExportInfobaseSnapshotRequest {
            output: base.join("base.dt"),
        };
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump)
            .with_cancellation(cancellation);

        let failure = super::prepare_infobase_snapshot(&context, &config, &request)
            .expect_err("an interrupted run must not pick a provider");

        assert_eq!(
            failure.error.kind(),
            UseCaseErrorKind::Cancelled(crate::support::error::CancelledAt::Boundary)
        );
        let result = failure.payload.expect("typed payload");
        assert_eq!(result.execution.status, ExecutionStatus::Cancelled);
        crate::use_cases::interruption::assert_stopped_at_a_safe_point(&result.execution);
        // Прерывание замечено на безопасной точке команды; шаг выбора называет `steps[]`.
        let [interruption] = result.execution.interruptions.as_slice() else {
            panic!(
                "one interruption expected: {:?}",
                result.execution.interruptions
            );
        };
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::CommandBoundary)
        );
        let failed = result.steps.last().expect("the failed step");
        assert_eq!(
            failed.name,
            InfobaseTransferPhase::ProviderSelection.as_str()
        );
    }

    /// `download`, остановленный на безопасной точке выбора исполнителя, пишет остановку как
    /// всякая форма: ошибка `cancelled` рядом с записью `command_boundary` (#319).
    #[test]
    fn a_download_stopped_at_provider_selection_names_the_cancellation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&base).expect("base");
        let config = config(&base, &work);
        let request = crate::domain::infobase_export::ExportConfigurationPackageRequest {
            state: crate::domain::infobase_export::ConfigurationState::Working,
            subject: crate::domain::infobase_export::ConfigurationSubject::Main,
            output: base.join("main.cf"),
        };
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(
            crate::use_cases::context::CommandName::InfobaseConfigurationExport,
        )
        .with_cancellation(cancellation);

        let failure = super::prepare_configuration_export(&context, &config, &request)
            .expect_err("an interrupted run must not pick a provider");

        assert_eq!(
            failure.error.kind(),
            UseCaseErrorKind::Cancelled(crate::support::error::CancelledAt::Boundary)
        );
        let result = failure.payload.expect("typed payload");
        crate::use_cases::interruption::assert_stopped_at_a_safe_point(&result.execution);
    }

    /// Исполнитель, выбранный под рабочее состояние, конфигурацию базы данных не исполняет:
    /// несовпадение запросов отвергается до цели и до исполнителя, а отказ «у агента нет
    /// такой команды» остаётся только у выбора.
    #[test]
    fn a_provider_prepared_for_another_state_is_not_dispatched() {
        use crate::domain::capability::{ProviderOrigin, ProviderReceipt};
        use crate::domain::infobase_export::ConfigurationState;

        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&base).expect("base");
        let config = config(&base, &work);
        let prepared = super::PreparedTransferProvider {
            receipt: ProviderReceipt::new(Provider::Agent, ProviderOrigin::Default),
            provider: Provider::Agent,
            executable: None,
            configuration_state: Some(ConfigurationState::Working),
        };
        let request = crate::domain::infobase_export::ExportConfigurationPackageRequest {
            state: ConfigurationState::Database,
            subject: ConfigurationSubject::Main,
            output: base.join("main.cf"),
        };
        let context = ExecutionContext::cli(
            crate::use_cases::context::CommandName::InfobaseConfigurationExport,
        );

        let failure = super::execute_configuration_export(&context, &config, &request, &prepared)
            .expect_err("a mismatched state must not reach the executor");

        assert!(
            failure
                .error
                .to_string()
                .contains("another configuration state"),
            "{}",
            failure.error
        );
        assert!(!base.join("main.cf").exists());
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

    /// Снятый процесс исполнителя — отмена его работы, а отказ запустить процесс по отмене —
    /// безопасная точка: фазу записи называет сама ошибка, а не место вызова.
    #[test]
    fn cancelled_process_is_not_collapsed_into_generic_failure() {
        for (delivered, phase) in [
            (true, ExecutionInterruptionPhase::ProviderCommand),
            (false, ExecutionInterruptionPhase::CommandBoundary),
        ] {
            let context = ExecutionContext::cli(
                crate::use_cases::context::CommandName::InfobaseConfigurationExport,
            );
            if delivered {
                context.work().mark_work_given();
            }
            let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed);
            let error = AppError::PlatformProcess(ProcessError::Cancelled {
                cmd: "1cv8 DESIGNER".to_owned(),
                delivered,
            });

            record_execution_failure(
                &context,
                &error,
                InfobaseTransferPhase::ProviderCommand,
                &mut execution,
            );

            assert_eq!(execution.status, ExecutionStatus::Cancelled);
            assert_eq!(execution.errors[0].code, "cancelled");
            assert!(!execution.errors[0].retryable);
            let [interruption] = execution.interruptions.as_slice() else {
                panic!("one interruption expected: {:?}", execution.interruptions);
            };
            assert!(!interruption.deferred);
            assert_eq!(interruption.phase, Some(phase), "delivered: {delivered}");
        }
    }

    #[test]
    fn unrelated_failure_is_not_reclassified_by_an_interrupted_context() {
        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(
            crate::use_cases::context::CommandName::InfobaseConfigurationExport,
        )
        .with_cancellation(cancellation);
        let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed);
        let error = AppError::Runtime("publication failed".to_owned());

        record_execution_failure(
            &context,
            &error,
            InfobaseTransferPhase::Publication,
            &mut execution,
        );

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert_eq!(execution.errors[0].code, "runtime_failure");
        assert!(execution.interruptions.is_empty());
    }

    /// `infobase dump` без утилиты: в `data.execution.errors[]` — `environment_unavailable`,
    /// как и род конверта (`INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE`), а не
    /// `platform_failure` прежнего запасного отображения.
    #[test]
    fn a_dump_without_its_utility_records_an_environment_step_code() {
        let missing = || LocatorError::NotFound {
            utility: UtilityType::V8,
            detail: None,
        };
        for error in [
            AppError::from(missing()),
            AppError::from(missing()).with_context("failed to locate 1cv8"),
        ] {
            let context =
                ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump);
            let request = ExportInfobaseSnapshotRequest {
                output: PathBuf::from("/tmp/main.dt"),
            };
            let failure = snapshot_failure(
                &context,
                error,
                ExportInfobaseSnapshotResult::new(request, None),
                InfobaseTransferPhase::ProviderCommand,
            );

            assert_eq!(failure.error.kind(), UseCaseErrorKind::Environment);
            let data = serde_json::to_value(failure.payload.expect("dump payload"))
                .expect("serialize dump data");
            assert_eq!(data["execution"]["status"], "failed", "{data}");
            assert_eq!(
                data["execution"]["errors"][0]["code"], "environment_unavailable",
                "{data}"
            );
        }
    }

    /// Код шага не расходится с родом конверта: оба выводятся из одного отображения
    /// `AppError` в род. Страж против второго владельца под любым именем.
    #[test]
    fn the_step_code_follows_the_envelope_kind_for_every_error() {
        let cases = || {
            vec![
                AppError::capability("not here".to_owned()),
                AppError::EnvironmentUnavailable("no infobase".to_owned()),
                AppError::WorkspaceBusy("held".to_owned()),
                AppError::TimedOut("agent session".to_owned()),
                AppError::InvalidOutput("garbled".to_owned()),
                AppError::Validation("bad".to_owned()),
                AppError::Runtime("io".to_owned()),
                AppError::Platform("designer said no".to_owned()),
                AppError::from(LocatorError::NotFound {
                    utility: UtilityType::Ibcmd,
                    detail: None,
                }),
                AppError::from(LocatorError::NotFound {
                    utility: UtilityType::Ibcmd,
                    detail: None,
                })
                .with_context("failed to locate ibcmd"),
                AppError::PlatformProcess(ProcessError::ExitedEarly {
                    cmd: "1cv8 DESIGNER".to_owned(),
                    exit_code: 1,
                }),
            ]
        };
        for (error, expected) in cases().into_iter().zip(cases()) {
            let context =
                ExecutionContext::cli(crate::use_cases::context::CommandName::InfobaseDump);
            let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed);
            record_execution_failure(
                &context,
                &error,
                InfobaseTransferPhase::ProviderCommand,
                &mut execution,
            );
            let kind = crate::use_cases::result::UseCaseError::from(expected).kind();

            assert_eq!(
                execution.errors[0].code,
                kind.execution_step_code(),
                "{error}"
            );
            assert_eq!(execution.status, kind.execution_status(), "{error}");
        }
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

    /// Двойник `1cv8` для `/RestoreIB`, который пишет базу, пока его не отпустят.
    ///
    /// Он кладёт `started`, ждёт файла `release` (не дольше 30 с) и кладёт `finished`;
    /// сигнал снятия он записывает в `terminated`. Оператор отменяет команду, когда запись
    /// уже идёт, и отпускает двойника, когда раннер уже отложил отмену: мягкое снятие дошло
    /// бы до процесса раньше, а критическая фаза его не посылает.
    #[cfg(unix)]
    #[track_caller]
    fn restore_cancelled_while_the_platform_writes(
        exit_code: i32,
    ) -> (
        tempfile::TempDir,
        crate::use_cases::result::UseCaseResult<
            crate::domain::infobase_export::RestoreInfobaseSnapshotResult,
        >,
    ) {
        use std::os::unix::fs::PermissionsExt;

        use crate::domain::capability::{ProviderOrigin, ProviderReceipt};
        use crate::domain::infobase_export::{RestoreInfobaseSnapshotRequest, RestoreTargetMode};
        use crate::use_cases::context::CommandName;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let base = root.join("base");
        let infobase = root.join("ib");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::create_dir_all(&infobase).expect("infobase");
        std::fs::write(infobase.join("1Cv8.1CD"), "data").expect("existing infobase");
        let input = root.join("snapshot.dt");
        std::fs::write(&input, "snapshot").expect("snapshot");
        let designer = root.join("1cv8");
        std::fs::write(
            &designer,
            format!(
                "#!/bin/sh\n\
                 trap \"printf terminated > '{root}/terminated'; exit 143\" TERM INT\n\
                 printf started > '{root}/started'\n\
                 waited=0\n\
                 while [ ! -e '{root}/release' ] && [ \"$waited\" -lt 300 ]; do\n\
                   sleep 0.1\n\
                   waited=$((waited + 1))\n\
                 done\n\
                 printf finished > '{root}/finished'\n\
                 exit {exit_code}\n",
                root = root.display(),
            ),
        )
        .expect("fake designer");
        std::fs::set_permissions(&designer, std::fs::Permissions::from_mode(0o755))
            .expect("executable");

        let mut config = config(&base, &root.join("work"));
        config.infobase = InfobaseConfig::file(format!("File={}", infobase.display()));
        let request = RestoreInfobaseSnapshotRequest {
            input,
            target_mode: RestoreTargetMode::Replace,
        };
        let prepared = super::PreparedTransferProvider {
            receipt: ProviderReceipt::new(Provider::Designer, ProviderOrigin::Default),
            provider: Provider::Designer,
            executable: Some(designer),
            configuration_state: None,
        };
        let cancellation = tokio_util::sync::CancellationToken::new();
        let context = ExecutionContext::cli(CommandName::InfobaseRestore)
            .with_cancellation(cancellation.clone());
        // Конфигуратор отпускают, когда раннер уже отложил отмену, а не через отсчёт времени.
        let held = crate::platform::process::HeldCommand::with_markers(
            root.join("started"),
            root.join("release"),
        );
        let outcome = held.interrupt_during(cancellation, || {
            super::execute_infobase_restore(&context, &config, &request, &prepared)
        });
        (dir, outcome)
    }

    /// Запись в базу — критическая фаза: отмена посреди `/RestoreIB` не снимает
    /// Конфигуратор, раннер дожидается конца загрузки и называет отмену отложенной.
    #[cfg(unix)]
    #[test]
    fn a_cancelled_designer_restore_runs_to_its_end_and_names_the_deferral() {
        let (dir, outcome) = restore_cancelled_while_the_platform_writes(0);

        let result = outcome.expect("the restore finishes despite the cancellation");
        assert!(
            !dir.path().join("terminated").exists(),
            "the platform must not be signalled during a critical phase"
        );
        assert!(dir.path().join("finished").exists());
        assert!(result.restored);
        assert_eq!(result.execution.status, ExecutionStatus::Succeeded);
        assert_eq!(result.execution.interruptions.len(), 1, "{result:?}");
        let interruption = &result.execution.interruptions[0];
        assert_eq!(interruption.kind, ExecutionInterruptionKind::Cancelled);
        assert!(interruption.deferred);
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::ProviderCommand)
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("unsafe interruption was not performed")),
            "{:?}",
            result.warnings
        );
    }

    /// Загрузка, неудачная уже после отложенной отмены, называет и отмену: оператор
    /// просил остановить, и отказ говорит, почему его не послушали.
    #[cfg(unix)]
    #[test]
    fn a_failed_restore_after_a_deferred_cancellation_still_names_it() {
        let (dir, outcome) = restore_cancelled_while_the_platform_writes(1);

        let failure = outcome.expect_err("the platform reported a failure");
        assert!(!dir.path().join("terminated").exists());
        assert!(dir.path().join("finished").exists());
        let result = failure.payload.expect("typed payload");
        assert!(!result.restored);
        assert_eq!(
            result.execution.interruptions.len(),
            1,
            "{:?}",
            result.execution.interruptions
        );
        let interruption = &result.execution.interruptions[0];
        assert_eq!(interruption.kind, ExecutionInterruptionKind::Cancelled);
        assert!(interruption.deferred);
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::ProviderCommand)
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("unsafe interruption was not performed")),
            "{:?}",
            result.warnings
        );
    }

    /// Подъём снимка для двух случаев без работы исполнителя: отмена до запуска и
    /// исполнитель, которого не собрать. Цель оба раза не тронута.
    #[cfg(unix)]
    fn restore_without_work(
        provider: Provider,
        cancelled: bool,
    ) -> (
        tempfile::TempDir,
        crate::use_cases::result::UseCaseResult<
            crate::domain::infobase_export::RestoreInfobaseSnapshotResult,
        >,
    ) {
        use std::os::unix::fs::PermissionsExt;

        use crate::domain::capability::{ProviderOrigin, ProviderReceipt};
        use crate::domain::infobase_export::{RestoreInfobaseSnapshotRequest, RestoreTargetMode};
        use crate::use_cases::context::CommandName;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let base = root.join("base");
        let infobase = root.join("ib");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::create_dir_all(&infobase).expect("infobase");
        std::fs::write(infobase.join("1Cv8.1CD"), "data").expect("existing infobase");
        let input = root.join("snapshot.dt");
        std::fs::write(&input, "snapshot").expect("snapshot");
        let designer = root.join("1cv8");
        std::fs::write(
            &designer,
            format!("#!/bin/sh\nprintf ran > '{}/ran'\nexit 0\n", root.display()),
        )
        .expect("fake designer");
        std::fs::set_permissions(&designer, std::fs::Permissions::from_mode(0o755))
            .expect("executable");
        let mut config = config(&base, &root.join("work"));
        config.infobase = InfobaseConfig::file(format!("File={}", infobase.display()));
        let request = RestoreInfobaseSnapshotRequest {
            input,
            target_mode: RestoreTargetMode::Replace,
        };
        let prepared = super::PreparedTransferProvider {
            receipt: ProviderReceipt::new(provider, ProviderOrigin::Default),
            provider,
            executable: Some(designer),
            configuration_state: None,
        };
        let cancellation = tokio_util::sync::CancellationToken::new();
        if cancelled {
            cancellation.cancel();
        }
        let context =
            ExecutionContext::cli(CommandName::InfobaseRestore).with_cancellation(cancellation);
        let outcome = super::execute_infobase_restore(&context, &config, &request, &prepared);
        (dir, outcome)
    }

    /// Отмена перед подъёмом снимка — безопасная точка: род отказа — отмена, запись —
    /// `command_boundary`, цель не тронута, и ответ не пугает неудавшимся откатом (#308).
    #[cfg(unix)]
    #[test]
    fn a_restore_cancelled_before_the_provider_stops_at_the_boundary() {
        let (dir, outcome) = restore_without_work(Provider::Designer, true);

        let failure = outcome.expect_err("the restore was cancelled");
        assert_eq!(
            failure.error.kind(),
            UseCaseErrorKind::Cancelled(crate::support::error::CancelledAt::Boundary)
        );
        assert!(!dir.path().join("ran").exists(), "the platform never ran");
        let result = failure.payload.expect("typed payload");
        assert_eq!(result.execution.status, ExecutionStatus::Cancelled);
        crate::use_cases::interruption::assert_stopped_at_a_safe_point(&result.execution);
        let [interruption] = result.execution.interruptions.as_slice() else {
            panic!(
                "one interruption expected: {:?}",
                result.execution.interruptions
            );
        };
        assert!(!interruption.deferred);
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::CommandBoundary)
        );
        assert_eq!(
            result.target_state,
            crate::domain::infobase_export::InfobaseTargetState::Unchanged
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    /// Исполнитель, которого не собрать, базу не трогал: цель остаётся `unchanged`, и
    /// предупреждения о неудавшемся откате нет — его давала бы только работа исполнителя.
    #[cfg(unix)]
    #[test]
    fn a_restore_refused_before_any_work_leaves_the_target_unchanged() {
        let (dir, outcome) = restore_without_work(Provider::Webinst, false);

        let failure = outcome.expect_err("the provider has no adapter");
        assert!(!dir.path().join("ran").exists(), "the platform never ran");
        let result = failure.payload.expect("typed payload");
        assert!(!result.restored);
        assert_eq!(
            result.target_state,
            crate::domain::infobase_export::InfobaseTargetState::Unchanged
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }
}
