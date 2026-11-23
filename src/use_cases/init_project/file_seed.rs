use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{
    create_file_infobase, create_from_dt_plan, designer_log_file, ensure_created,
    existing_file_infobase, infobase_marker_path, infobase_secrets, init_result,
    interruption_step_outcome, missing_infobase_marker_error, prepare_infobase_parent, recreate,
    InitExecutionFailure, StepOutcome, INFOBASE_CREATE,
};
use crate::change_detection::source_sets::SourceSetsService;
use crate::config::model::AppConfig;
use crate::domain::capability::{Operation, Provider, ProviderReceipt, TargetKind};
use crate::domain::infobase_export::RestoreTargetMode;
use crate::domain::init::InitResult;
use crate::platform::designer::DesignerDsl;
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::path::{
    nearest_existing_canonical_path, normalize_windows_verbatim_path, stable_path_identity,
};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::dump_config::helpers::build_ibcmd_dsl;
use crate::use_cases::exchange_guard::{forget_base_memory, remember_created_base};
use crate::use_cases::infobase_export::{run_restore_provider, validate_transfer_input};
use crate::use_cases::infobase_lock::{settle_live_agents, LiveAgents};
use crate::use_cases::interruption::{collecting_deferrals, Deferrals};
use crate::use_cases::provider_selection::{locate_from, SelectedProvider};
use crate::use_cases::result::UseCaseResult;
use crate::use_cases::transport::record_created_file_owner;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seed {
    Empty,
    Cf(PathBuf),
    Dt(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileCreation {
    target: PathBuf,
    seed: Seed,
}

impl FileCreation {
    pub(crate) fn prepare(
        config: &AppConfig,
        input: Option<&Path>,
        clean_before_execution: bool,
    ) -> Result<Self, AppError> {
        let target = config
            .v8_connection()
            .file_infobase_dir(&config.base_path)
            .filter(|_| config.target_kind() == TargetKind::File)
            .ok_or_else(|| {
                AppError::Validation(
                    "infobase create --empty and --input require a file infobase".into(),
                )
            })?;
        crate::config::validate::validate_providers(config, &[Operation::Init])
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let target_root = resolved_path(&target)?;
        let memory = SourceSetsService::new(config)
            .base_memory_dir()
            .as_deref()
            .map(resolved_path)
            .transpose()?;
        let logs = resolved_path(&config.work_path.join("logs/platform"))?;
        for directory in memory.iter().chain(std::iter::once(&logs)) {
            if is_within(&target_root, directory) || is_within(directory, &target_root) {
                return Err(AppError::Validation(format!(
                    "file infobase '{}' overlaps runner state at '{}'",
                    target.display(),
                    directory.display()
                )));
            }
        }
        let seed = match input {
            None => Seed::Empty,
            Some(path) => {
                let kind = path.extension().and_then(std::ffi::OsStr::to_str);
                let cf = kind.is_some_and(|suffix| suffix.eq_ignore_ascii_case("cf"));
                let dt = kind.is_some_and(|suffix| suffix.eq_ignore_ascii_case("dt"));
                if !cf && !dt {
                    return Err(AppError::Validation(
                        "--input must have a .cf or .dt suffix".into(),
                    ));
                }
                validate_transfer_input(path)?;
                std::fs::File::open(path).map_err(|error| {
                    AppError::Validation(format!(
                        "--input '{}' is not readable: {error}",
                        path.display()
                    ))
                })?;
                let path = std::fs::canonicalize(path).map_err(|error| {
                    AppError::Validation(format!(
                        "cannot resolve input '{}': {error}",
                        path.display()
                    ))
                })?;
                for directory in std::iter::once(target_root.as_path())
                    .chain(memory.as_deref())
                    .chain(clean_before_execution.then_some(logs.as_path()))
                {
                    if is_within(&path, directory) {
                        return Err(AppError::Validation(format!(
                            "--input '{}' is inside '{}' which this command may change",
                            path.display(),
                            directory.display()
                        )));
                    }
                }
                let path = normalize_windows_verbatim_path(&path);
                if path.to_str().is_none() {
                    return Err(AppError::Validation(
                        "--input path cannot be represented in the platform command line".into(),
                    ));
                }
                if cf {
                    Seed::Cf(path)
                } else {
                    Seed::Dt(path)
                }
            }
        };
        let creation = Self { target, seed };
        creation.require_absent_target()?;
        Ok(creation)
    }

    fn require_absent_target(&self) -> Result<(), AppError> {
        if let Some(aside) = recreate::leftover_of(&self.target) {
            return Err(AppError::Validation(format!(
                "'{}' holds the files of an infobase from an earlier recreation; resolve that first",
                aside.display()
            )));
        }
        match std::fs::symlink_metadata(infobase_marker_path(&self.target)) {
            Ok(_) => Err(existing_file_infobase(&self.target)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::Runtime(format!(
                "cannot inspect file infobase '{}': {error}",
                self.target.display()
            ))),
        }
    }

    pub(crate) fn execute(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        dry_run: bool,
    ) -> UseCaseResult<InitResult> {
        let started = Instant::now();
        let mut receipt = None;
        let step = self
            .run(context, config, dry_run, started, &mut receipt)
            .unwrap_or_else(|error| StepOutcome::failed("infobase", "create", started, error));
        let mut result = init_result(started, vec![step.step], step.error.is_none());
        result.provider = receipt;
        match step.error {
            Some(error) => Err(InitExecutionFailure::with_payload(error, result)),
            None => Ok(result),
        }
    }

    fn run(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        dry_run: bool,
        started: Instant,
        receipt: &mut Option<ProviderReceipt>,
    ) -> Result<StepOutcome, AppError> {
        // The CLI holds the workspace, memory and file-base locks here during execution.
        self.require_absent_target()?;
        let mut utilities = PlatformUtilities::from_config(config);
        let plan = config.provider_plan(Operation::Init);
        let (plan, mut skipped) = if matches!(self.seed, Seed::Dt(_)) {
            create_from_dt_plan(plan, "--input").map_err(|(error, refused)| {
                *receipt = Some(refused);
                error
            })?
        } else {
            (plan, Vec::new())
        };
        let selected = locate_from(config, &mut utilities, Operation::Init, plan).map_err(
            |(error, mut refused)| {
                skipped.append(&mut refused.skipped);
                *receipt = Some(refused.with_skipped(std::mem::take(&mut skipped)));
                error
            },
        )?;
        skipped.extend(selected.receipt.skipped.iter().cloned());
        *receipt = Some(selected.receipt.clone().with_skipped(skipped));
        let contents = match &self.seed {
            Seed::Empty => "empty".to_owned(),
            Seed::Cf(path) => format!("with applied configuration from '{}'", path.display()),
            Seed::Dt(path) => format!("from DT '{}'", path.display()),
        };
        if dry_run {
            return Ok(StepOutcome::planned(
                "infobase",
                "create",
                started,
                format!(
                    "would create a file infobase at '{}' {contents} via {}",
                    self.target.display(),
                    selected.provider.as_str()
                ),
            ));
        }
        if let Some(outcome) =
            interruption_step_outcome(context, "infobase", "create", started, INFOBASE_CREATE)
        {
            return Ok(outcome);
        }
        prepare_infobase_parent(&self.target)?;
        settle_live_agents(config, INFOBASE_CREATE, LiveAgents::Remove)?;
        let marker = infobase_marker_path(&self.target);
        let (step, deferrals) = collecting_deferrals(|deferrals| {
            let result = self.create(
                context,
                config,
                &mut utilities,
                &selected,
                &marker,
                deferrals,
            )?;
            if !marker.is_file() {
                return Err(missing_infobase_marker_error(
                    "infobase creation did not produce marker file",
                    &marker,
                    &result,
                ));
            }
            let mut warnings = match record_created_file_owner(config) {
                Ok(warnings) => warnings
                    .into_iter()
                    .map(|warning| warning.message().to_owned())
                    .collect(),
                Err(error) => vec![format!(
                    "the infobase was created but ownership was not recorded: {error}"
                )],
            };
            let forgotten = forget_base_memory(config);
            if let Some(failure) = forgotten {
                warnings.push(failure);
            } else if matches!(self.seed, Seed::Empty) {
                warnings.extend(remember_created_base(config, None));
            }
            Ok(StepOutcome::ok(
                "infobase",
                "create",
                started,
                format!("infobase created {contents}: {}", marker.display()),
            )
            .with_warnings(&warnings))
        })
        .map_err(|error| {
            if context.work().given() && marker.exists() {
                let details = format!(
                    "the target '{}' contains files from this incomplete creation",
                    self.target.display()
                );
                let details = match forget_base_memory(config) {
                    Some(warning) => format!("{details}. {warning}"),
                    None => details,
                };
                error.with_context(details)
            } else {
                error
            }
        })?;
        Ok(step.with_warnings(&deferrals))
    }

    fn create(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        utilities: &mut PlatformUtilities,
        selected: &SelectedProvider,
        marker: &Path,
        deferrals: &mut Deferrals,
    ) -> Result<PlatformCommandResult, AppError> {
        if matches!(self.seed, Seed::Empty) {
            return create_file_infobase(
                context,
                config,
                utilities,
                selected.provider,
                None,
                marker,
                deferrals,
            );
        }
        let binary = selected
            .location
            .as_ref()
            .map(|location| location.path.as_path());
        let result = match (&self.seed, selected.provider) {
            (Seed::Dt(input), provider) => run_restore_provider(
                context,
                config,
                provider,
                binary,
                input,
                RestoreTargetMode::Create,
            )
            .map_err(|failure| failure.into_error(INFOBASE_CREATE))?,
            (Seed::Cf(input), Provider::Designer) => {
                let binary = binary.ok_or_else(|| {
                    AppError::Runtime("no Designer executable for CF creation".into())
                })?;
                DesignerDsl::new(
                    binary.to_path_buf(),
                    config.v8_connection(),
                    utilities.runner_for(UtilityType::V8),
                    Some(designer_log_file(config)?),
                    context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
                )
                .create_infobase_from_cf(input)?
            }
            (Seed::Cf(input), Provider::Ibcmd) => {
                let binary = binary.ok_or_else(|| {
                    AppError::Runtime("no ibcmd executable for CF creation".into())
                })?;
                build_ibcmd_dsl(
                    context,
                    config,
                    binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    InterruptionSafetyClass::CriticalNonAbortable,
                )?
                .infobase_create_from_cf(input)?
            }
            (_, provider) => {
                return Err(crate::use_cases::unimplemented_provider(
                    Operation::Init,
                    provider,
                ))
            }
        };
        deferrals.note_result(INFOBASE_CREATE, &result);
        ensure_created(&result, marker, &infobase_secrets(config))?;
        Ok(result)
    }
}

fn resolved_path(path: &Path) -> Result<PathBuf, AppError> {
    nearest_existing_canonical_path(path).map_err(|error| {
        AppError::Validation(format!("cannot resolve '{}': {error}", path.display()))
    })
}

fn is_within(path: &Path, directory: &Path) -> bool {
    let directory = stable_path_identity(directory);
    path.ancestors()
        .any(|ancestor| stable_path_identity(ancestor) == directory)
}
