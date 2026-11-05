//! Queries the working configuration without exporting sources or recording synchronization.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::capability::{
    Operation, Provider, ProviderPlan, ProviderReceipt, SkippedProvider,
};
use crate::domain::diff::{DiffAgainst, DiffResult, MetadataChange};
use crate::platform::designer::DesignerDsl;
use crate::platform::dump_forecast::{parse_changes_list, parse_export_status, ChangeList};
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::process::ProcessExecutor;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::temp::{platform_logs_dir, temp_root};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::ignored_files::VERSION_FILE_NAME;
use crate::use_cases::infobase_export::validate_platform_artifact;
use crate::use_cases::interruption::pending_interruption_error;
use crate::use_cases::provider_selection::{self, SelectedProvider};
use crate::use_cases::result::{stamp_dispatch, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;
use crate::use_cases::version_file::RunnerVersionFile;

/// Caller holds the workspace and has passed the file infobase owner boundary.
pub(crate) fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    against: Option<&DiffAgainst>,
) -> UseCaseResult<DiffResult> {
    stamp_dispatch(run(context, config, against), context.work())
}

fn run(
    context: &ExecutionContext,
    config: &AppConfig,
    against: Option<&DiffAgainst>,
) -> UseCaseResult<DiffResult> {
    let started = Instant::now();
    let input = match against {
        Some(DiffAgainst::File(file)) if !file.is_file() => {
            return Err(UseCaseFailure::without_payload(AppError::Validation(
                format!(
                    "comparison package is not a readable file: '{}'",
                    file.display()
                ),
            )));
        }
        Some(against) => DiffInput::Against(against),
        None => {
            DiffInput::VersionFile(version_file(config).map_err(UseCaseFailure::without_payload)?)
        }
    };
    let selected = select_provider(config, against);
    let (receipt, outcome) = match selected {
        Ok(selected) => {
            let outcome = run_provider(context, config, &selected, &input);
            (selected.receipt, outcome)
        }
        Err((error, receipt)) => (receipt, Err(error)),
    };
    let mut result = DiffResult {
        provider: receipt,
        provider_dispatched: false,
        changes: Vec::new(),
        full_dump: false,
        report: None,
        duration_ms: started.elapsed().as_millis() as u64,
    };
    match outcome {
        Ok(DiffOutput::Changes(changes)) => result.changes = changes,
        Ok(DiffOutput::FullDump) => result.full_dump = true,
        Ok(DiffOutput::Report(path)) => result.report = Some(path),
        Err(error) => return Err(UseCaseFailure::with_payload(error, result)),
    }
    Ok(result)
}

fn version_file(config: &AppConfig) -> Result<PathBuf, AppError> {
    let inventory = SourceSetInventory::new(config);
    let main = inventory.main_configuration().ok_or_else(|| {
        AppError::Validation("diff requires a main configuration source-set".to_owned())
    })?;
    let file = inventory
        .designer_context(&main.name)
        .and_then(|context| RunnerVersionFile::of(config, context));
    if let Some(file) = file
        .as_ref()
        .map(RunnerVersionFile::file_after_restore)
        .transpose()?
        .flatten()
    {
        return Ok(file.to_owned());
    }
    Err(AppError::Validation(format!(
        "no {VERSION_FILE_NAME} for source-set '{}'; synchronize it before running diff",
        main.name
    )))
}

fn select_provider(
    config: &AppConfig,
    against: Option<&DiffAgainst>,
) -> Result<SelectedProvider, (AppError, ProviderReceipt)> {
    let mut utilities = PlatformUtilities::from_config(config);
    let mut plan = config.provider_plan(Operation::Diff);
    if against.is_none() {
        return provider_selection::select_from(config, &mut utilities, Operation::Diff, plan);
    }
    let skip = |provider| SkippedProvider {
        provider,
        reason: format!(
            "{provider} cannot produce a comparison report; diff --against requires designer"
        ),
    };
    let mut excluded = Vec::new();
    match &mut plan {
        ProviderPlan::Default { chain } => chain.retain(|provider| {
            if *provider == Provider::Designer {
                true
            } else {
                excluded.push(skip(*provider));
                false
            }
        }),
        ProviderPlan::Override { provider, file } if *provider != Provider::Designer => {
            let provider = *provider;
            return Err((
                AppError::capability(format!(
                    "diff --against requires designer; providers.diff in {file} assigns {provider}: remove the key or assign designer"
                )),
                plan.receipt_for_nobody(vec![skip(provider)]),
            ));
        }
        ProviderPlan::Override { .. } => {}
    }
    let mut selection =
        provider_selection::select_from(config, &mut utilities, Operation::Diff, plan);
    let receipt = match &mut selection {
        Ok(selected) => &mut selected.receipt,
        Err((_, receipt)) => receipt,
    };
    receipt.skipped.splice(..0, excluded);
    selection
}

enum DiffInput<'a> {
    VersionFile(PathBuf),
    Against(&'a DiffAgainst),
}

enum DiffOutput {
    Changes(Vec<MetadataChange>),
    FullDump,
    Report(PathBuf),
}

fn run_provider(
    context: &ExecutionContext,
    config: &AppConfig,
    selected: &SelectedProvider,
    input: &DiffInput<'_>,
) -> Result<DiffOutput, AppError> {
    let binary = selected
        .location
        .as_ref()
        .ok_or_else(|| {
            AppError::EnvironmentUnavailable(format!(
                "{} has no executable for diff",
                selected.provider
            ))
        })?
        .path
        .clone();
    let runner = ProcessExecutor;
    let policy = context.process_policy(InterruptionSafetyClass::Interruptible, None);
    match (selected.provider, input) {
        (Provider::Ibcmd, DiffInput::VersionFile(version_file)) => {
            let connection = IbcmdConnection::from_infobase(&config.infobase)?;
            let result = IbcmdDsl::new(binary, connection, &runner, policy)
                .config_export_status(version_file, None)?;
            check_result(context, config, &result)?;
            metadata_changes(parse_export_status(&result.process.stdout))
        }
        (Provider::Designer, _) => {
            let log = platform_logs_dir(&config.work_path)
                .map_err(io_error)?
                .join("diff.log");
            let designer =
                DesignerDsl::new(binary, config.v8_connection(), &runner, Some(log), policy);
            match input {
                DiffInput::Against(against) => compare(context, config, &designer, against),
                DiffInput::VersionFile(version_file) => {
                    designer_changes(context, config, &designer, version_file)
                }
            }
        }
        (provider, _) => Err(crate::use_cases::unimplemented_provider(
            Operation::Diff,
            provider,
        )),
    }
}

fn designer_changes(
    context: &ExecutionContext,
    config: &AppConfig,
    designer: &DesignerDsl<'_>,
    version_file: &Path,
) -> Result<DiffOutput, AppError> {
    let root = temp_root(&config.work_path).map_err(io_error)?;
    let stage = tempfile::tempdir_in(root).map_err(io_error)?;
    // Designer takes a directory, while the version-file owner may have selected its own copy.
    fs::copy(version_file, stage.path().join(VERSION_FILE_NAME)).map_err(io_error)?;
    let list = stage.path().join("changes.txt");
    let result = designer.dump_changes_list(stage.path(), &list, None)?;
    check_result(context, config, &result)?;
    validate_platform_artifact(&list)?;
    let text = fs::read_to_string(&list).map_err(|error| {
        AppError::InvalidOutput(format!(
            "cannot read the Designer change list as UTF-8: {error}"
        ))
    })?;
    if text != "\u{feff}" && text.trim_matches(['\u{feff}', '\r', '\n']).is_empty() {
        return Err(AppError::InvalidOutput(
            "empty Designer change list must contain only a UTF-8 BOM".to_owned(),
        ));
    }
    metadata_changes(parse_changes_list(&text))
}

fn metadata_changes(list: Option<ChangeList<'_>>) -> Result<DiffOutput, AppError> {
    match list {
        Some(ChangeList::FullDump) => Ok(DiffOutput::FullDump),
        Some(ChangeList::Objects(objects))
            if objects.iter().all(|(_, object)| !object.trim().is_empty()) =>
        {
            Ok(DiffOutput::Changes(
                objects
                    .into_iter()
                    .map(|(kind, object)| MetadataChange {
                        kind,
                        object: object.to_owned(),
                    })
                    .collect(),
            ))
        }
        Some(ChangeList::Objects(_)) | None => Err(AppError::InvalidOutput(
            "unrecognized metadata change list".to_owned(),
        )),
    }
}

fn compare(
    context: &ExecutionContext,
    config: &AppConfig,
    designer: &DesignerDsl<'_>,
    against: &DiffAgainst,
) -> Result<DiffOutput, AppError> {
    let root = config.work_path.join("reports").join("diff");
    fs::create_dir_all(&root).map_err(io_error)?;
    let stage = tempfile::tempdir_in(root).map_err(io_error)?;
    let report = stage.path().join("report.txt");
    let result = designer.compare_full_report(against, &report)?;
    check_result(context, config, &result)?;
    validate_platform_artifact(&report)?;
    let _ = stage.keep();
    Ok(DiffOutput::Report(report))
}

fn check_result(
    context: &ExecutionContext,
    config: &AppConfig,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    result.process.outcome().map_err(|code| {
        let evidence = crate::use_cases::ibcmd_diagnostics::format_failure_evidence(
            format!("diff platform command exited with {code}"),
            &result.process.stdout,
            &result.process.stderr,
            result.platform_log.as_deref(),
            result.platform_log_path.as_deref(),
        );
        AppError::Platform(crate::platform::secrets::mask_text(
            &evidence,
            config.infobase.password.as_deref().as_slice(),
        ))
    })?;
    pending_interruption_error(context, "after the diff platform command").map_or(Ok(()), Err)
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::Runtime(format!("diff workspace I/O: {error}"))
}
