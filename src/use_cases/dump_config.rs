use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::model::{AppConfig, SourceFormat, SourceSetPurpose};
use crate::domain::dump::{DumpMode, DumpResult, DumpSelectorResult};
use crate::domain::partial_dump_selector::PartialDumpSelector;
#[cfg(test)]
use crate::domain::partial_dump_selector::{
    PARTIAL_OBJECT_BLANK_ERROR, PARTIAL_OBJECT_CONTROL_ERROR,
};
use crate::platform::designer::DesignerDsl;
use crate::platform::dump_format::{known_format, read_recorded, FormatVersion, RecordedFormat};
use crate::platform::edt::EdtDsl;
use crate::platform::edt_session::{EdtSessionHostOptions, EdtSessionManager};
use crate::platform::locator::{PlatformVersion, UtilityType};
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::edt_project::{self, EdtProjectKind};
use crate::support::error::AppError;
use crate::support::fs::{acquire_advisory_lock, ensure_dir, remove_path_if_exists};
use crate::support::path::{
    hashed_lock_path, nearest_existing_canonical_path, stable_path_identity,
};
use crate::support::source_descriptor::{self, ExternalDescriptorParseError};
use crate::use_cases::context::{CommandName, ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::destruction_guard::{DestructionConsent, Losses, WaysOut};
use crate::use_cases::external_artifacts::ExternalArtifactKind;
use crate::use_cases::interruption;
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::request::{DumpModeRequest, DumpRequest as DumpArgs, ForceWayOut};
use crate::use_cases::result::{stamp_dispatch, UseCaseFailure, UseCaseResult};
use tracing::debug;

mod agent;
mod all;
mod coordinator;
pub(crate) mod helpers;

pub use self::all::execute_all;

#[cfg(test)]
use self::helpers::create_dump_object_list_file_with;
use self::helpers::{
    build_designer_dsl, build_ibcmd_dsl, cleanup_orphan_dirs, cleanup_platform_orphan_dirs,
    create_dump_object_list_file, decorate_ibcmd_partial_error, dump_publication_warning,
    empty_result, ensure_platform_success, ibcmd_partial_warning, map_ibcmd_error,
    merge_optional_messages, resolve_dump_edt_base_project_name, validate_dump_objects,
    validate_platform_target, validate_publish_target, validate_supported_matrix,
};
use super::ignored_files::VERSION_FILE_NAME;
#[cfg(test)]
use super::staged_publication::cleanup_staging_path;
use super::staged_publication::{interruption_before_publish, StagedPublication};
#[cfg(test)]
use crate::support::fs::metadata_sidecar_path;
use crate::use_cases::source_inventory::SourceSetInventory;

#[cfg(test)]
const DUMP_COMMAND: &str = crate::use_cases::context::CommandName::Dump.as_str();
const SUPPORTED_DUMP_ERROR: &str =
    "dump currently supports only the Designer, ibcmd or agent provider";
const PARTIAL_OBJECTS_REQUIRED_ERROR: &str = "partial dump requires at least one object";
const NON_PARTIAL_OBJECTS_ERROR: &str = "dump objects are supported only for mode 'partial'";
const ORPHAN_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DUMP_BACKUP_PREFIX: &str = ".dump-backup";

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &DumpArgs,
) -> UseCaseResult<DumpResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        "executing dump use case"
    );
    stamp_dispatch(
        crate::use_cases::provider_selection::stamp_session(
            coordinator::run_dump_with_context(context, config, args),
            context,
        ),
        context.work(),
    )
}

type DumpExecutionFailure = UseCaseFailure<DumpResult>;

#[derive(Debug, Clone)]
struct ResolvedDumpTarget {
    source_set_name: String,
    source_set_purpose: SourceSetPurpose,
    extension: Option<String>,
    target_path: PathBuf,
    canonical_target_path: PathBuf,
    platform_target_path: PathBuf,
    canonical_platform_target_path: PathBuf,
    canonical_base_path: PathBuf,
    canonical_work_path: PathBuf,
    target_identity: String,
    platform_target_identity: String,
    lock_path: PathBuf,
    edt_base_project_name: Option<String>,
    /// Разрешено ли уничтожить незафиксированную работу в каталоге цели.
    consent: DestructionConsent,
}

impl ResolvedDumpTarget {
    /// Согласие для публикации каталога платформы.
    ///
    /// У формата Designer это то же дерево, что видит человек. У EDT — служебный
    /// снимок Конфигуратора (под памятью базы или в `workPath/designer/<имя>`, см.
    /// [`crate::domain::source_set::designer_copy_dir`]), который раннер сам и создаёт: спрашивать
    /// о нём систему контроля версий незачем, а спросив, можно получить отказ на
    /// собственном кеше.
    fn platform_consent(&self) -> &DestructionConsent {
        if self.platform_target_path == self.target_path {
            &self.consent
        } else {
            &DestructionConsent::RunnerOwned
        }
    }
}

/// Что выгрузка сообщает сверх ответа платформы.
#[derive(Debug, Default)]
struct DumpNotes {
    /// Предупреждения и оговорки для ответа.
    message: Option<String>,
    /// Что публикация уничтожила по согласию.
    discarded: Losses,
}

impl DumpNotes {
    fn message(message: Option<String>) -> Self {
        Self {
            message,
            discarded: Losses::default(),
        }
    }

    /// Ставит более раннюю оговорку перед своими.
    fn after(self, earlier: Option<String>) -> Self {
        Self {
            message: merge_optional_messages(earlier, self.message),
            discarded: self.discarded,
        }
    }
}

/// Итог выгрузки до ответа: результат платформы и то, что о нём сказать.
type DumpRun = Result<(PlatformCommandResult, DumpNotes), AppError>;

#[cfg(test)]
fn run_dump(config: &AppConfig, args: &DumpArgs) -> UseCaseResult<DumpResult> {
    let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
    execute(&context, config, args)
}

/// Seal the platform output before publication; only that tree may become the baseline.
/// No storage is opened until publication succeeds, including its Git and cancellation guards.
fn publish_full_dump(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    publication: &StagedPublication,
) -> Result<DumpNotes, AppError> {
    use crate::change_detection::analyzer::{commit_full_snapshot, prepare_full_snapshot};

    validate_platform_target(resolved).map_err(|error| publication.cleanup_failure(error))?;
    validate_full_dump_work_path(config, resolved)
        .map_err(|error| publication.cleanup_failure(error))?;
    // Hashing a large staging tree takes seconds; honour a cancellation that came first.
    if let Some(error) = interruption_before_publish(context, "dump publication") {
        return Err(publication.cleanup_failure(error));
    }

    let snapshot = if config.format == SourceFormat::Designer {
        SourceSetInventory::new(config)
            .designer_context(&resolved.source_set_name)
            .filter(|source| source.persists_snapshot())
            .map(|source| prepare_full_snapshot(source, publication.staging_path()))
    } else {
        None
    };
    // Hashing may take seconds: re-check right before replacing, not only before hashing.
    validate_platform_target(resolved).map_err(|error| publication.cleanup_failure(error))?;
    validate_full_dump_work_path(config, resolved)
        .map_err(|error| publication.cleanup_failure(error))?;

    let published = publication
        .publish_dir(
            context,
            DUMP_BACKUP_PREFIX,
            "failed to publish staged dump",
            resolved.platform_consent(),
            // Платформа пишет опись версий в каждую полную выгрузку.
            &[VERSION_FILE_NAME],
        )
        .map_err(|error| publication.cleanup_failure(error))?;
    debug!(target = %resolved.platform_target_path.display(), "published staged dump");

    // Cancellation observed during publication must not leave a successfully published
    // tree unrecorded. This local commit starts no process and has no cancellation check.
    let memory_warning = snapshot.and_then(|prepared| {
        // Publication can replace a symlink with a real directory. Bind the prepared
        // bytes to the resulting source root, without scanning its contents again.
        let published_inventory = SourceSetInventory::new(config);
        let Some(source) = published_inventory.designer_context(&resolved.source_set_name) else {
            return Some(format!(
                "sources published for '{}', but hash memory was not updated: source context is missing; repeat a full pull to refresh memory",
                resolved.source_set_name
            ));
        };
        prepared
            .and_then(|snapshot| commit_full_snapshot(source, &config.work_path, &snapshot))
            .err()
            .map(|error| format!(
                "sources published for '{}', but hash memory was not updated: {error}; repeat a full pull to refresh memory",
                resolved.source_set_name
            ))
    });
    Ok(DumpNotes {
        message: merge_optional_messages(
            merge_optional_messages(published.cleanup_warning, memory_warning),
            dump_publication_warning(context.command(), published.deferred_interruption),
        ),
        discarded: published.discarded,
    })
}

fn validate_full_dump_work_path(
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
) -> Result<(), AppError> {
    let work = nearest_existing_canonical_path(&config.work_path)
        .map_err(|error| AppError::Runtime(format!("failed to canonicalize workPath: {error}")))?;
    if work.starts_with(&resolved.canonical_target_path)
        || work.starts_with(&resolved.canonical_platform_target_path)
    {
        return Err(AppError::Validation(
            "full pull target must not contain workPath".to_owned(),
        ));
    }
    Ok(())
}

/// Как выгрузка ложится прямо в каталог набора.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OverDirectory {
    /// По изменившемуся от файла версий в каталоге.
    ByVersionFile,
    /// Целиком поверх каталога: файл версий не годится, и выгрузка пишет его заново.
    Whole(WholeReason),
}

/// Почему выгрузка по изменившемуся не может опереться на файл версий в каталоге.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WholeReason {
    /// Файла нет.
    Missing,
    /// Версии формата в корне файла раннер не нашёл: такой файл чужой.
    Unrecognized,
    /// Версия формата не та, что пишет выбранная платформа по таблице замеров.
    Foreign {
        found: FormatVersion,
        platform: PlatformVersion,
        written: FormatVersion,
    },
}

impl WholeReason {
    /// Причина для ответа: что не так с файлом версий в каталоге `dir`.
    fn describe(&self, dir: &Path) -> String {
        let path = dir.join(VERSION_FILE_NAME);
        match self {
            Self::Missing => format!("no version file {VERSION_FILE_NAME} in '{}'", dir.display()),
            Self::Unrecognized => {
                format!(
                    "the format version of '{}' is not recognized",
                    path.display()
                )
            }
            Self::Foreign {
                found,
                platform,
                written,
            } => format!(
                "'{}' is in format {found}, and platform {platform} writes {written}",
                path.display()
            ),
        }
    }
}

/// План выгрузки после сверки файла версий: что запрошено и как ляжет в каталог.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DumpPlan {
    /// Полная выгрузка через ступенчатую публикацию.
    Full,
    /// Выборка объектов.
    Partial,
    /// Выгрузка по изменившемуся прямо в каталог набора.
    OverDirectory(OverDirectory),
}

impl DumpPlan {
    /// Режим, который выгрузка выполнит и назовёт ответ: без годного файла версий
    /// выгрузка по изменившемуся полная.
    pub(super) fn mode(&self) -> DumpMode {
        match self {
            Self::Full | Self::OverDirectory(OverDirectory::Whole(_)) => DumpMode::Full,
            Self::Partial => DumpMode::Partial,
            Self::OverDirectory(OverDirectory::ByVersionFile) => DumpMode::Incremental,
        }
    }

    /// Причина, по которой выгрузка по изменившемуся стала полной, если стала.
    pub(super) fn whole_reason(&self) -> Option<&WholeReason> {
        match self {
            Self::OverDirectory(OverDirectory::Whole(reason)) => Some(reason),
            Self::Full | Self::Partial | Self::OverDirectory(OverDirectory::ByVersionFile) => None,
        }
    }
}

/// План выгрузки до запуска платформы. Выгрузка по изменившемуся держится на файле версий
/// `version_file` (`None` — его нет): нет его, версия формата не распознана или по таблице
/// замеров не та, что пишет платформа, — выгрузка идёт полной поверх каталога.
pub(super) fn plan_dump(
    mode: &DumpMode,
    version_file: Option<&Path>,
    platform: Option<&PlatformVersion>,
) -> Result<DumpPlan, AppError> {
    Ok(match mode {
        DumpMode::Full => DumpPlan::Full,
        DumpMode::Partial => DumpPlan::Partial,
        DumpMode::Incremental => {
            let recorded = match version_file {
                None => RecordedFormat::Missing,
                Some(path) => read_recorded(path).map_err(|error| {
                    AppError::Runtime(format!("failed to read '{}': {error}", path.display()))
                })?,
            };
            DumpPlan::OverDirectory(
                match unusable_version_file(recorded, known_format(platform)) {
                    None => OverDirectory::ByVersionFile,
                    Some(reason) => OverDirectory::Whole(reason),
                },
            )
        }
    })
}

/// Годится ли файл версий для выгрузки по изменившемуся. Версию, которую пишет платформа,
/// раннер знает только по таблице замеров; где не знает, о чужой версии не судит.
fn unusable_version_file(
    recorded: RecordedFormat,
    written: Option<(&PlatformVersion, FormatVersion)>,
) -> Option<WholeReason> {
    match recorded {
        RecordedFormat::Missing => Some(WholeReason::Missing),
        RecordedFormat::Unrecognized => Some(WholeReason::Unrecognized),
        RecordedFormat::Version(found) => {
            written
                .filter(|(_, written)| *written != found)
                .map(|(platform, written)| WholeReason::Foreign {
                    found,
                    platform: platform.clone(),
                    written,
                })
        }
    }
}

/// Выгрузка Конфигуратором прямо в каталог: с файлом версий — по изменившемуся (`-update`),
/// без него — полная поверх каталога, которая файл версий и создаёт. Полная выгрузка без
/// `-update` в существующий каталог ничего в нём не удаляет (замер «Вопросы задачи»).
fn run_dump_over_directory_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    how: &OverDirectory,
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        how = ?how,
        "running dump over the directory"
    );
    ensure_dir(&resolved.platform_target_path)
        .map_err(|error| AppError::Runtime(format!("failed to create target dir: {error}")))?;

    let (stage, label) = match how {
        OverDirectory::ByVersionFile => ("dump: incremental", "incremental"),
        OverDirectory::Whole(_) => ("dump: full", "full"),
    };
    log_live_stage(stage, "[Конфигуратор] exporting configuration files");
    let dsl = build_designer_dsl(
        context,
        config,
        binary,
        runner,
        &resolved.source_set_name,
        label,
    )?;
    let dump_result = match how {
        OverDirectory::ByVersionFile => dsl.dump_config_to_files_incremental(
            &resolved.platform_target_path,
            resolved.extension.as_deref(),
        ),
        OverDirectory::Whole(_) => dsl.dump_config_to_files(
            &resolved.platform_target_path,
            resolved.extension.as_deref(),
        ),
    }
    .map_err(AppError::from)?;
    ensure_platform_success("dump", resolved, &dump_result)?;
    Ok((dump_result, DumpNotes::default()))
}

fn run_full_dump_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        "running full dump via staging directory"
    );
    let publication = StagedPublication::prepare_dir(
        &resolved.platform_target_path,
        &resolved.platform_target_identity,
        ".dump-stage",
    )?;
    let staging_dir = publication.staging_path().to_path_buf();
    debug!(path = %staging_dir.display(), "created dump staging directory");

    log_live_stage("dump: full", "[Конфигуратор] exporting configuration files");
    let dump_result = match build_designer_dsl(
        context,
        config,
        binary,
        runner,
        &resolved.source_set_name,
        "full",
    )?
    .dump_config_to_files(&staging_dir, resolved.extension.as_deref())
    .map_err(AppError::from)
    {
        Ok(result) => result,
        Err(error) => return Err(publication.cleanup_failure(error)),
    };
    ensure_platform_success("dump", resolved, &dump_result)
        .map_err(|error| publication.cleanup_failure(error))?;

    let notes = publish_full_dump(context, config, resolved, &publication)?;
    Ok((dump_result, notes))
}

/// Выгрузка `ibcmd` прямо в каталог: с файлом версий — `--sync`, без него — полная
/// поверх каталога без `--force`. Как `config export` без `--sync` и `--force` ведёт себя в
/// непустом каталоге и пишет ли файл версий, не замерено (#403).
fn run_dump_over_directory_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    how: &OverDirectory,
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        how = ?how,
        "running ibcmd dump over the directory"
    );
    ensure_dir(&resolved.platform_target_path)
        .map_err(|error| AppError::Runtime(format!("failed to create target dir: {error}")))?;

    let dsl = build_ibcmd_dsl(context, config, binary, runner)?;
    let dump_result = match how {
        OverDirectory::ByVersionFile => {
            log_live_stage("dump: incremental", "[ibcmd] exporting configuration files");
            dsl.config_export_incremental(
                &resolved.platform_target_path,
                resolved.extension.as_deref(),
            )
        }
        OverDirectory::Whole(_) => {
            log_live_stage("dump: full", "[ibcmd] exporting configuration files");
            dsl.config_export_over(
                &resolved.platform_target_path,
                resolved.extension.as_deref(),
            )
        }
    }
    .map_err(map_ibcmd_error)?;
    ensure_platform_success("dump", resolved, &dump_result)?;
    Ok((dump_result, DumpNotes::default()))
}

fn run_full_dump_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        "running full ibcmd dump via staging directory"
    );
    let publication = StagedPublication::prepare_dir(
        &resolved.platform_target_path,
        &resolved.platform_target_identity,
        ".dump-stage",
    )?;
    let staging_dir = publication.staging_path().to_path_buf();
    debug!(path = %staging_dir.display(), "created dump staging directory");

    log_live_stage("dump: full", "[ibcmd] exporting configuration files");
    let dump_result = match build_ibcmd_dsl(context, config, binary, runner)?
        .config_export_full(&staging_dir, resolved.extension.as_deref())
        .map_err(map_ibcmd_error)
    {
        Ok(result) => result,
        Err(error) => return Err(publication.cleanup_failure(error)),
    };
    ensure_platform_success("dump", resolved, &dump_result)
        .map_err(|error| publication.cleanup_failure(error))?;

    let notes = publish_full_dump(context, config, resolved, &publication)?;
    Ok((dump_result, notes))
}

fn run_partial_dump_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    objects: &[PartialDumpSelector],
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        object_count = objects.len(),
        "running partial designer dump"
    );
    ensure_dir(&resolved.platform_target_path)
        .map_err(|error| AppError::Runtime(format!("failed to create target dir: {error}")))?;

    let list_file = create_dump_object_list_file(&config.work_path, objects)?;
    log_live_stage(
        "dump: partial",
        "[Конфигуратор] exporting selected configuration objects",
    );
    let dump_result = build_designer_dsl(
        context,
        config,
        binary,
        runner,
        &resolved.source_set_name,
        "partial",
    )?
    .dump_config_to_files_partial(
        &resolved.platform_target_path,
        list_file.path(),
        resolved.extension.as_deref(),
    )
    .map_err(AppError::from)?;
    ensure_platform_success("dump", resolved, &dump_result)?;
    Ok((dump_result, DumpNotes::default()))
}

fn run_partial_dump_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    _objects: &[PartialDumpSelector],
) -> DumpRun {
    let warning = ibcmd_partial_warning(resolved);
    match run_dump_over_directory_ibcmd(
        context,
        config,
        resolved,
        binary,
        runner,
        &OverDirectory::ByVersionFile,
    ) {
        Ok((dump_result, _)) => Ok((dump_result, DumpNotes::message(Some(warning)))),
        Err(error) => Err(decorate_ibcmd_partial_error(error, &warning)),
    }
}

fn ensure_interruption_clear(context: &ExecutionContext, phase: &str) -> Result<(), AppError> {
    interruption::pending_interruption_error(context, phase).map_or(Ok(()), Err)
}

fn run_incremental_dump_edt_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
) -> DumpRun {
    let bootstrap_message = ensure_edt_platform_target_seeded(
        context,
        config,
        resolved,
        binary,
        runner,
        run_full_dump_designer,
    )?;
    ensure_interruption_clear(
        context,
        "before starting EDT follow-up dump after bootstrap publication",
    )?;
    let (dump_result, dump_notes) = run_dump_over_directory_designer(
        context,
        config,
        resolved,
        binary,
        runner,
        &OverDirectory::ByVersionFile,
    )?;
    finalize_edt_dump(
        context,
        config,
        resolved,
        edt_binary,
        edt_runner,
        dump_result,
        // Снимок Конфигуратора — каталог раннера: уничтоженного по согласию у него нет.
        merge_optional_messages(bootstrap_message, dump_notes.message),
    )
}

fn run_full_dump_edt_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
) -> DumpRun {
    let (dump_result, dump_notes) =
        run_full_dump_designer(context, config, resolved, binary, runner)?;
    finalize_edt_dump(
        context,
        config,
        resolved,
        edt_binary,
        edt_runner,
        dump_result,
        dump_notes.message,
    )
}

fn run_partial_dump_edt_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
    objects: &[PartialDumpSelector],
) -> DumpRun {
    let bootstrap_message = ensure_edt_platform_target_seeded(
        context,
        config,
        resolved,
        binary,
        runner,
        run_full_dump_designer,
    )?;
    ensure_interruption_clear(
        context,
        "before starting EDT follow-up dump after bootstrap publication",
    )?;
    let (dump_result, dump_notes) =
        run_partial_dump_designer(context, config, resolved, binary, runner, objects)?;
    finalize_edt_dump(
        context,
        config,
        resolved,
        edt_binary,
        edt_runner,
        dump_result,
        // Снимок Конфигуратора — каталог раннера: уничтоженного по согласию у него нет.
        merge_optional_messages(bootstrap_message, dump_notes.message),
    )
}

fn run_incremental_dump_edt_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
) -> DumpRun {
    let bootstrap_message = ensure_edt_platform_target_seeded(
        context,
        config,
        resolved,
        binary,
        runner,
        run_full_dump_ibcmd,
    )?;
    ensure_interruption_clear(
        context,
        "before starting EDT follow-up dump after bootstrap publication",
    )?;
    let (dump_result, dump_notes) = run_dump_over_directory_ibcmd(
        context,
        config,
        resolved,
        binary,
        runner,
        &OverDirectory::ByVersionFile,
    )?;
    finalize_edt_dump(
        context,
        config,
        resolved,
        edt_binary,
        edt_runner,
        dump_result,
        // Снимок Конфигуратора — каталог раннера: уничтоженного по согласию у него нет.
        merge_optional_messages(bootstrap_message, dump_notes.message),
    )
}

fn run_full_dump_edt_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
) -> DumpRun {
    let (dump_result, dump_notes) = run_full_dump_ibcmd(context, config, resolved, binary, runner)?;
    finalize_edt_dump(
        context,
        config,
        resolved,
        edt_binary,
        edt_runner,
        dump_result,
        dump_notes.message,
    )
}

fn run_partial_dump_edt_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
    objects: &[PartialDumpSelector],
) -> DumpRun {
    let bootstrap_message = ensure_edt_platform_target_seeded(
        context,
        config,
        resolved,
        binary,
        runner,
        run_full_dump_ibcmd,
    )?;
    ensure_interruption_clear(
        context,
        "before starting EDT follow-up dump after bootstrap publication",
    )?;
    let (dump_result, dump_notes) =
        run_partial_dump_ibcmd(context, config, resolved, binary, runner, objects)?;
    finalize_edt_dump(
        context,
        config,
        resolved,
        edt_binary,
        edt_runner,
        dump_result,
        // Снимок Конфигуратора — каталог раннера: уничтоженного по согласию у него нет.
        merge_optional_messages(bootstrap_message, dump_notes.message),
    )
}

/// Полная выгрузка, переданная как значение: обратная синхронизация EDT сеет снимок
/// Конфигуратора тем же кодом, которым идёт обычная выгрузка.
type FullDumpRunner =
    fn(&ExecutionContext, &AppConfig, &ResolvedDumpTarget, &Path, &dyn ProcessRunner) -> DumpRun;

fn ensure_edt_platform_target_seeded(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    full_dump_runner: FullDumpRunner,
) -> Result<Option<String>, AppError> {
    if designer_snapshot_is_ready(&resolved.platform_target_path)? {
        return Ok(None);
    }

    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        "bootstrapping missing designer dump snapshot for EDT reverse sync"
    );
    let (_, notes) = full_dump_runner(context, config, resolved, binary, runner)?;
    Ok(notes.message)
}

fn designer_snapshot_is_ready(path: &Path) -> Result<bool, AppError> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_dir() {
        return Ok(false);
    }
    Ok(path.join("Configuration.xml").is_file())
}

fn finalize_edt_dump(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    edt_binary: &Path,
    edt_runner: &dyn ProcessRunner,
    platform_result: PlatformCommandResult,
    inherited_message: Option<String>,
) -> DumpRun {
    ensure_interruption_clear(
        context,
        "before starting EDT reverse-sync import after designer snapshot publication",
    )?;
    let publication = StagedPublication::prepare_dir(
        &resolved.target_path,
        &resolved.target_identity,
        ".dump-stage",
    )?;
    let staging_dir = publication.staging_path().to_path_buf();

    let edt_dsl = build_edt_dsl(context, config, edt_binary, edt_runner)
        .map_err(|error| publication.cleanup_failure(error))?;
    log_live_stage("dump: edt import", "[EDT] importing Designer snapshot");
    let import_result = match edt_dsl
        .import_configuration_files(
            &staging_dir,
            &resolved.platform_target_path,
            normalize_config_hint(config.tools.platform.version.as_deref()),
            resolved.edt_base_project_name.as_deref(),
            false,
        )
        .map_err(AppError::from)
    {
        Ok(result) => result,
        Err(error) => return Err(publication.cleanup_failure(error)),
    };
    ensure_import_success(resolved, &import_result)
        .map_err(|error| publication.cleanup_failure(error))?;
    validate_edt_dump_staging_output(
        &staging_dir,
        resolved.source_set_purpose,
        resolved.edt_base_project_name.as_deref(),
    )
    .map_err(|error| publication.cleanup_failure(error))?;
    validate_publish_target(resolved).map_err(|error| publication.cleanup_failure(error))?;

    if let Some(error) = interruption_before_publish(context, "dump publication") {
        return Err(publication.cleanup_failure(error));
    }

    let publish_phase = publication
        .publish_dir(
            context,
            DUMP_BACKUP_PREFIX,
            "failed to publish staged dump",
            // Здесь публикуется дерево человека, а не служебный снимок.
            &resolved.consent,
            // Импорт EDT описи версий не пишет: исключений нет.
            &[],
        )
        .map_err(|error| publication.cleanup_failure(error))?;

    Ok((
        platform_result,
        DumpNotes {
            message: merge_optional_messages(
                publish_phase.cleanup_warning,
                dump_publication_warning(context.command(), publish_phase.deferred_interruption),
            ),
            discarded: publish_phase.discarded,
        }
        .after(inherited_message),
    ))
}

fn validate_edt_dump_staging_output(
    staging_dir: &Path,
    expected_purpose: SourceSetPurpose,
    expected_base_project: Option<&str>,
) -> Result<(), AppError> {
    let expected_kind = match expected_purpose {
        SourceSetPurpose::Configuration => EdtProjectKind::Configuration,
        SourceSetPurpose::Extension => EdtProjectKind::Extension,
        _ => {
            return Err(AppError::Validation(format!(
                "EDT dump output validation supports only ordinary source-sets: {}",
                staging_dir.display()
            )));
        }
    };
    edt_project::validate_native_ordinary_project(staging_dir, expected_kind, expected_base_project)
        .map(|_| ())
        .map_err(|error| {
            AppError::Validation(format!(
                "EDT dump output is not a valid native EDT project: {error}"
            ))
        })
}

fn ensure_import_success(
    resolved: &ResolvedDumpTarget,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    let Err(code) = result.process.outcome() else {
        return Ok(());
    };

    let mut details = vec![format!(
        "dump EDT import failed for source-set '{}' with exit code {code}",
        resolved.source_set_name
    )];
    if !result.process.stdout.trim().is_empty() {
        details.push(format!("stdout: {}", result.process.stdout.trim()));
    }
    if !result.process.stderr.trim().is_empty() {
        details.push(format!("stderr: {}", result.process.stderr.trim()));
    }
    if let Some(log) = result
        .platform_log
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        details.push(format!("platform log: {}", log.trim()));
    }
    if let Some(path) = result.platform_log_path.as_ref() {
        details.push(format!("platform log path: {}", path.display()));
    }
    Err(AppError::Platform(details.join("; ")))
}

fn build_edt_dsl<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    runner: &'a dyn ProcessRunner,
) -> Result<EdtDsl<'a>, AppError> {
    let workspace = config.work_path.join("edt-workspace");
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
    if config.tools.edt_cli.interactive_mode {
        let manager =
            EdtSessionManager::for_config(config, EdtSessionHostOptions::for_cli_command(config))
                .map_err(AppError::from)?;
        EdtDsl::new_shared_session(
            binary.to_path_buf(),
            workspace,
            Arc::new(manager),
            Duration::from_millis(config.tools.edt_cli.startup_timeout_ms),
            Duration::from_millis(config.tools.edt_cli.command_timeout_ms),
            policy,
        )
        .map_err(AppError::from)
        .map(|dsl| dsl.with_timeout(context.edt_timeout()))
    } else {
        Ok(EdtDsl::new(binary.to_path_buf(), workspace, runner, policy)
            .with_timeout(context.edt_timeout()))
    }
}

fn normalize_config_hint(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn run_external_dump_designer(
    dsl: &DesignerDsl<'_>,
    binary_path: &Path,
    root_xml_path: &Path,
    expected_kind: ExternalArtifactKind,
    expected_logical_name: &str,
) -> Result<(PlatformCommandResult, PathBuf), (AppError, Option<PathBuf>)> {
    let parent = root_xml_path.parent().ok_or_else(|| {
        (
            AppError::Runtime(format!(
                "external dump target has no parent: {}",
                root_xml_path.display()
            )),
            None,
        )
    })?;
    ensure_dir(parent).map_err(|error| {
        (
            AppError::Runtime(format!("failed to create external dump dir: {error}")),
            None,
        )
    })?;
    remove_path_if_exists(root_xml_path).map_err(|error| {
        (
            AppError::Runtime(format!("failed to clean external root xml: {error}")),
            None,
        )
    })?;
    let result = dsl
        .dump_external_data_processor_or_report_to_files(binary_path, root_xml_path)
        .map_err(|error| (AppError::from(error), None))?;
    if let Err(code) = result.process.outcome() {
        return Err((
            AppError::Platform(format!("external dump failed with exit code {code}")),
            result.platform_log_path.clone(),
        ));
    }
    verify_external_dump_descriptor(root_xml_path, expected_kind, expected_logical_name)
        .map_err(|error| (error, result.platform_log_path.clone()))?;
    Ok((result, root_xml_path.to_path_buf()))
}

/// Выгруженный обратно описатель должен быть того вида и с тем именем, что собирали:
/// иначе платформа собрала не то, что просили, и файл нельзя публиковать.
pub(crate) fn verify_external_dump_descriptor(
    root_xml_path: &Path,
    expected_kind: ExternalArtifactKind,
    expected_logical_name: &str,
) -> Result<(), AppError> {
    let contents = match std::fs::read_to_string(root_xml_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::Validation(format!(
                "external dump '{}' did not produce descriptor xml",
                root_xml_path.display()
            )));
        }
        Err(error) => {
            return Err(AppError::Runtime(format!(
                "failed to read external dump root xml '{}': {error}",
                root_xml_path.display()
            )));
        }
    };
    let parsed = parse_external_dump_descriptor(&contents, root_xml_path)?;
    if parsed.purpose.external_root_tag() != Some(expected_kind.root_tag()) {
        return Err(AppError::Validation(format!(
            "external dump '{}' has unexpected root element",
            root_xml_path.display()
        )));
    }
    if parsed.logical_name != expected_logical_name {
        return Err(AppError::Validation(format!(
            "external dump '{}' has unexpected logical name",
            root_xml_path.display()
        )));
    }
    Ok(())
}

fn parse_external_dump_descriptor(
    contents: &str,
    path: &Path,
) -> Result<source_descriptor::ParsedExternalDescriptor, AppError> {
    source_descriptor::parse_external_descriptor(contents).map_err(|error| match error {
        ExternalDescriptorParseError::Xml(error) => AppError::Validation(format!(
            "failed to parse external dump xml '{}': {error}",
            path.display()
        )),
        ExternalDescriptorParseError::DecodeLogicalName(error) => AppError::Validation(format!(
            "failed to decode external dump logical name in '{}': {error}",
            path.display()
        )),
        ExternalDescriptorParseError::MissingRootElement => {
            AppError::Validation(format!("missing root XML element in '{}'", path.display()))
        }
        ExternalDescriptorParseError::UnsupportedRootElement(root) => {
            AppError::Validation(format!(
                "unsupported root XML element '{root}' in '{}'",
                path.display()
            ))
        }
        ExternalDescriptorParseError::MissingLogicalName => AppError::Validation(format!(
            "external dump '{}' must contain Properties/Name",
            path.display()
        )),
    })
}

#[cfg(test)]
fn cleanup_staging_on_platform_failure(staging_dir: &Path, error: AppError) -> AppError {
    cleanup_staging_path(staging_dir, error)
}

#[cfg(test)]
fn cleanup_staging_on_interruption(staging_dir: &Path, error: AppError) -> AppError {
    cleanup_staging_on_platform_failure(staging_dir, error)
}

fn resolve_target(config: &AppConfig, args: &DumpArgs) -> Result<ResolvedDumpTarget, AppError> {
    let inventory = SourceSetInventory::new(config);

    let (source_set, extension) = match (args.source_set.as_deref(), args.extension.as_deref()) {
        // Набор называет и предмет: набор расширения — это расширение с именем набора, как
        // если бы его назвали `--extension`.
        (Some(source_set_name), None) => {
            let (source_set, extension) =
                inventory.configuration_package(source_set_name, CommandName::Dump)?;
            (source_set, extension.map(str::to_owned))
        }
        (None, Some(extension_name)) => {
            let source_set = inventory.source_set(extension_name).ok_or_else(|| {
                AppError::Validation(format!("unknown extension '{extension_name}'"))
            })?;
            if source_set.purpose != SourceSetPurpose::Extension {
                return Err(AppError::Validation(format!(
                    "source-set '{extension_name}' is not an extension source-set"
                )));
            }
            (source_set, Some(extension_name.to_owned()))
        }
        (Some(source_set_name), Some(extension_name)) => {
            if source_set_name != extension_name {
                return Err(AppError::Validation(format!(
                    "<SET> '{source_set_name}' does not match --extension '{extension_name}'"
                )));
            }
            let source_set = inventory.source_set(source_set_name).ok_or_else(|| {
                AppError::Validation(format!("unknown extension '{extension_name}'"))
            })?;
            if source_set.purpose != SourceSetPurpose::Extension {
                return Err(AppError::Validation(format!(
                    "source-set '{source_set_name}' is not an extension source-set"
                )));
            }
            (source_set, Some(extension_name.to_owned()))
        }
        (None, None) => {
            let configuration_source_sets =
                inventory.source_sets_with_purpose(SourceSetPurpose::Configuration);
            if configuration_source_sets.len() != 1 {
                let candidates = configuration_source_sets
                    .iter()
                    .map(|source_set| source_set.name.as_str())
                    .collect::<Vec<_>>();
                return Err(AppError::Validation(format!(
                    "dump requires exactly one configuration source-set when <SET> is omitted; found [{}]",
                    candidates.join(", ")
                )));
            }
            (configuration_source_sets[0], None)
        }
    };

    let target_path = inventory.source_path(source_set);
    let platform_target_path = if config.format == SourceFormat::Edt {
        inventory
            .designer_context(&source_set.name)
            .ok_or_else(|| {
                AppError::Runtime(format!(
                    "missing designer runtime context for source-set '{}'",
                    source_set.name
                ))
            })?
            .path()
            .to_path_buf()
    } else {
        target_path.clone()
    };
    let canonical_target_path = nearest_existing_canonical_path(&target_path).map_err(|error| {
        AppError::Runtime(format!("failed to canonicalize target path: {error}"))
    })?;
    let canonical_platform_target_path = nearest_existing_canonical_path(&platform_target_path)
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to canonicalize platform target path: {error}"
            ))
        })?;
    let canonical_base_path =
        nearest_existing_canonical_path(&config.base_path).map_err(|error| {
            AppError::Runtime(format!("failed to canonicalize project base path: {error}"))
        })?;
    let canonical_work_path = nearest_existing_canonical_path(&config.work_path)
        .map_err(|error| AppError::Runtime(format!("failed to canonicalize workPath: {error}")))?;
    let target_identity = stable_path_identity(&canonical_target_path);
    let platform_target_identity = stable_path_identity(&canonical_platform_target_path);
    if target_path.parent().is_none() {
        return Err(AppError::Runtime(format!(
            "target path has no parent: {}",
            target_path.display()
        )));
    }
    let edt_base_project_name = if config.format == SourceFormat::Edt
        && source_set.purpose == SourceSetPurpose::Extension
    {
        Some(resolve_dump_edt_base_project_name(&inventory)?)
    } else {
        None
    };
    let lock_path = hashed_lock_path(&canonical_target_path, "dump")
        .map_err(|error| AppError::Runtime(format!("failed to resolve dump lock path: {error}")))?;

    let resolved = ResolvedDumpTarget {
        source_set_name: source_set.name.clone(),
        source_set_purpose: source_set.purpose,
        extension,
        target_path,
        canonical_target_path,
        platform_target_path,
        canonical_platform_target_path,
        canonical_base_path,
        canonical_work_path,
        target_identity,
        platform_target_identity,
        lock_path,
        edt_base_project_name,
        consent: if args.discard_uncommitted {
            DestructionConsent::Granted
        } else {
            DestructionConsent::AskFirst(match args.force_way_out {
                ForceWayOut::Withheld => WaysOut::SaveWork,
                // Набор назван по разрешённой цели, а не по тому, как его назвал вызов:
                // `pull --extension ext` и `pull` без набора приходят сюда с именем.
                ForceWayOut::PullForce => WaysOut::PullForce {
                    source_set: source_set.name.clone(),
                },
                ForceWayOut::Undeclared => WaysOut::Undeclared {
                    source_set: source_set.name.clone(),
                    path: source_set.path.display().to_string(),
                },
            })
        },
    };
    if matches!(args.mode, DumpModeRequest::Full) {
        validate_full_dump_work_path(config, &resolved)?;
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::{
        build_designer_dsl, cleanup_orphan_dirs, cleanup_staging_on_interruption,
        create_dump_object_list_file_with, finalize_edt_dump, metadata_sidecar_path,
        parse_external_dump_descriptor, resolve_target, run_dump, run_external_dump_designer,
        validate_publish_target, validate_supported_matrix, DUMP_BACKUP_PREFIX, DUMP_COMMAND,
        NON_PARTIAL_OBJECTS_ERROR, ORPHAN_TTL, PARTIAL_OBJECTS_REQUIRED_ERROR,
        PARTIAL_OBJECT_BLANK_ERROR, PARTIAL_OBJECT_CONTROL_ERROR,
    };
    use crate::config::model::{
        AppConfig, PlatformToolConfig, SourceFormat, SourceSetConfig, SourceSetPurpose,
        TestsConfig, ToolsConfig,
    };
    use crate::domain::dump::{DumpMode, DumpSelectorResult};
    use crate::domain::partial_dump_selector::PartialDumpSelector;
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessResult, ProcessRunner,
        SpawnResult,
    };
    use crate::platform::result::PlatformCommandResult;
    use crate::support::error::AppError;
    use crate::support::fs::{
        acquire_advisory_lock, read_temp_dir_metadata, write_temp_dir_metadata, TempDirKind,
        TempDirMetadata,
    };
    use crate::support::path::{nearest_existing_canonical_path, stable_path_identity};
    use crate::use_cases::context::ExecutionContext;
    use crate::use_cases::destruction_guard::DestructionConsent;
    use crate::use_cases::external_artifacts::ExternalArtifactKind;
    use crate::use_cases::request::{DumpModeRequest, DumpRequest as DumpArgs, ForceWayOut};
    use crate::use_cases::result::UseCaseErrorKind;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn publication_fixture() -> (
        tempfile::TempDir,
        AppConfig,
        super::ResolvedDumpTarget,
        super::StagedPublication,
    ) {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        fs::create_dir_all(base.join("main")).expect("source");
        fs::write(base.join("main/Module.bsl"), "old source").expect("old source");
        let mut config = build_config(&base, &dir.path().join("work"), &dir.path().join("1cv8"));
        config.infobase_name = Some("origin".to_owned());
        let args = DumpArgs {
            mode: DumpModeRequest::Full,
            source_set: Some("main".to_owned()),
            extension: None,
            objects: vec![],
            discard_uncommitted: true,
            force_way_out: ForceWayOut::PullForce,
            dry_run: false,
        };
        let resolved = resolve_target(&config, &args).expect("target");
        let publication = super::StagedPublication::prepare_dir(
            &resolved.platform_target_path,
            &resolved.platform_target_identity,
            ".dump-stage",
        )
        .expect("staging");
        fs::write(
            publication.staging_path().join("Module.bsl"),
            "database source",
        )
        .expect("stage");
        (dir, config, resolved, publication)
    }

    #[test]
    fn cancelled_full_publication_leaves_source_and_memory_unchanged_and_can_retry() {
        use crate::change_detection::analyzer::{
            analyze_context, rescan_and_commit_full, AnalysisOutcome,
        };
        let (_dir, config, resolved, publication) = publication_fixture();
        let inventory = super::SourceSetInventory::new(&config);
        let source = inventory.designer_context("main").expect("context");
        rescan_and_commit_full(source, &config.work_path).expect("old snapshot");
        let before = fs::read(source.storage_path(&config.work_path).expect("memory path"))
            .expect("old memory");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancel);
        super::publish_full_dump(&context, &config, &resolved, &publication).expect_err("cancel");
        assert_eq!(
            fs::read_to_string(source.path().join("Module.bsl")).expect("source"),
            "old source"
        );
        assert_eq!(
            fs::read(source.storage_path(&config.work_path).expect("memory path")).expect("memory"),
            before
        );
        assert!(matches!(
            analyze_context(source, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        let retry = super::StagedPublication::prepare_dir(
            &resolved.platform_target_path,
            &resolved.platform_target_identity,
            ".dump-stage",
        )
        .expect("retry stage");
        fs::write(retry.staging_path().join("Module.bsl"), "database source")
            .expect("retry source");
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
        assert!(
            super::publish_full_dump(&context, &config, &resolved, &retry)
                .expect("retry")
                .message
                .is_none()
        );
        assert_eq!(
            fs::read_to_string(source.path().join("Module.bsl")).expect("source"),
            "database source"
        );
        assert!(matches!(
            analyze_context(source, &config.work_path).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        fs::write(source.path().join("Module.bsl"), "user change").expect("edit");
        assert!(matches!(
            analyze_context(source, &config.work_path).outcome,
            Ok(AnalysisOutcome::Changes { .. })
        ));
    }

    #[test]
    fn full_publication_reports_memory_write_failure_after_publishing() {
        let (_dir, config, resolved, publication) = publication_fixture();
        let inventory = super::SourceSetInventory::new(&config);
        let source = inventory.designer_context("main").expect("context");
        let storage = source.storage_path(&config.work_path).expect("memory path");
        fs::create_dir_all(storage.parent().expect("parent")).expect("parent dir");
        fs::create_dir(&storage).expect("block storage with a directory");
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
        let warning = super::publish_full_dump(&context, &config, &resolved, &publication)
            .expect("publication succeeds")
            .message
            .expect("memory warning");
        assert!(warning.contains("sources published"), "{warning}");
        assert!(warning.contains("hash memory was not updated"), "{warning}");
        assert!(warning.contains("full pull"), "{warning}");
        assert_eq!(
            fs::read_to_string(source.path().join("Module.bsl")).expect("source"),
            "database source"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_staging_scan_failure_does_not_discard_the_successful_full_dump() {
        use crate::change_detection::analyzer::rescan_and_commit_full;
        let (_dir, config, resolved, publication) = publication_fixture();
        let inventory = super::SourceSetInventory::new(&config);
        let source = inventory.designer_context("main").expect("context");
        rescan_and_commit_full(source, &config.work_path).expect("old memory");
        let before =
            fs::read(source.storage_path(&config.work_path).expect("memory path")).expect("memory");
        fs::set_permissions(
            publication.staging_path().join("Module.bsl"),
            fs::Permissions::from_mode(0o000),
        )
        .expect("unreadable");
        if fs::read(publication.staging_path().join("Module.bsl")).is_ok() {
            // Running as root overrides the mode: the scan cannot be made to fail this way.
            eprintln!("skipped: file mode does not deny reading to this user (root)");
            return;
        }
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
        let result = super::publish_full_dump(&context, &config, &resolved, &publication);
        fs::set_permissions(
            source.path().join("Module.bsl"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("restore access");
        let warning = result
            .expect("publication succeeds")
            .message
            .expect("scan warning");
        assert!(warning.contains("sources published"), "{warning}");
        assert_eq!(
            fs::read_to_string(source.path().join("Module.bsl")).expect("source"),
            "database source"
        );
        assert_eq!(
            fs::read(source.storage_path(&config.work_path).expect("memory path")).expect("memory"),
            before
        );
    }

    #[test]
    fn full_publication_rechecks_that_the_target_does_not_contain_work_path() {
        let (_dir, mut config, resolved, publication) = publication_fixture();
        // workPath moved under the target after the target was resolved.
        config.work_path = resolved.platform_target_path.join("work");
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
        let error = super::publish_full_dump(&context, &config, &resolved, &publication)
            .expect_err("refused before publication");
        assert!(error.to_string().contains("contain workPath"), "{error}");
        assert_eq!(
            fs::read_to_string(resolved.platform_target_path.join("Module.bsl")).expect("source"),
            "old source"
        );
    }

    #[test]
    fn only_full_pull_refuses_a_target_containing_work_path() {
        let dir = tempdir().expect("tempdir");
        let config = build_config(
            dir.path(),
            &dir.path().join("main/work"),
            &dir.path().join("1cv8"),
        );
        fs::create_dir_all(dir.path().join("main")).expect("source");
        let mut args = DumpArgs {
            mode: DumpModeRequest::Full,
            source_set: Some("main".to_owned()),
            extension: None,
            objects: vec![],
            discard_uncommitted: true,
            force_way_out: ForceWayOut::PullForce,
            dry_run: false,
        };
        assert!(resolve_target(&config, &args)
            .expect_err("full refused")
            .to_string()
            .contains("contain workPath"));
        args.mode = DumpModeRequest::Incremental;
        resolve_target(&config, &args).expect("incremental allowed");
        args.mode = DumpModeRequest::Partial;
        resolve_target(&config, &args).expect("partial allowed");
    }

    #[cfg(unix)]
    #[test]
    fn full_pull_refuses_an_aliased_work_path_but_allows_a_mirror_beneath_work_path() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("main/work")).expect("nested work");
        symlink(dir.path().join("main/work"), dir.path().join("work-alias")).expect("alias");
        let mut config = build_config(
            dir.path(),
            &dir.path().join("work-alias"),
            &dir.path().join("1cv8"),
        );
        let args = DumpArgs {
            mode: DumpModeRequest::Full,
            source_set: Some("main".to_owned()),
            extension: None,
            objects: vec![],
            discard_uncommitted: true,
            force_way_out: ForceWayOut::PullForce,
            dry_run: false,
        };
        assert!(resolve_target(&config, &args)
            .expect_err("alias refused")
            .to_string()
            .contains("contain workPath"));
        config.work_path = dir.path().join("work");
        config.source_sets[0].path = PathBuf::from("work/designer/main");
        resolve_target(&config, &args).expect("reverse nesting allowed");
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write");
        make_executable(path);
    }

    fn write_dump_script(path: &Path, calls_log: &Path, fail_pattern: Option<&str>, sleep_ms: u64) {
        let pattern_branch = fail_pattern
            .map(|pattern| {
                format!(
                    "if printf '%s' \"$args\" | grep -F -q -- '{}'; then exit 17; fi",
                    pattern
                )
            })
            .unwrap_or_default();
        let sleep_branch = if sleep_ms == 0 {
            String::new()
        } else {
            format!("sleep {}", sleep_ms as f64 / 1000.0)
        };
        let body = format!(
            "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log for %s\\n' \"$args\" > \"$out\"; fi\nprintf '%s\\n' \"$args\" >> \"{}\"\ncase \" $args \" in *\" /GetConfigGenerationID\"*) exit 0;; esac\n{}\n{}\nmkdir -p \"$(printf '%s' \"$args\" | awk '{{print $NF}}')\"\nexit 0",
            calls_log.display(),
            sleep_branch,
            pattern_branch
        );
        write_script(path, &body);
    }

    fn write_ibcmd_dump_script(
        path: &Path,
        calls_log: &Path,
        fail_pattern: Option<&str>,
        sleep_ms: u64,
    ) {
        let pattern_branch = fail_pattern
            .map(|pattern| {
                format!(
                    "if printf '%s' \"$args\" | grep -F -q -- '{}'; then exit 17; fi",
                    pattern
                )
            })
            .unwrap_or_default();
        let sleep_branch = if sleep_ms == 0 {
            String::new()
        } else {
            format!("sleep {}", sleep_ms as f64 / 1000.0)
        };
        let body = format!(
            "args=\"$*\"\nprintf '%s\\n' \"$args\" >> \"{}\"\ncase \" $args \" in *\" generation-id \"*) exit 0;; esac\n{}\n{}\nmkdir -p \"$(printf '%s' \"$args\" | awk '{{print $NF}}')\"\nexit 0",
            calls_log.display(),
            sleep_branch,
            pattern_branch
        );
        write_script(path, &body);
    }

    fn write_designer_dump_script_for_edt(
        path: &Path,
        calls_log: &Path,
        fail_pattern: Option<&str>,
    ) {
        let pattern_branch = fail_pattern
            .map(|pattern| {
                format!(
                    "if printf '%s' \"$args\" | grep -F -q -- '{}'; then exit 17; fi",
                    pattern
                )
            })
            .unwrap_or_default();
        let body = format!(
            "args=\"$*\"\nout=\"\"\ntarget=\"\"\nextension_name=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  if [ \"$prev\" = \"/DumpConfigToFiles\" ]; then target=\"$arg\"; fi\n  if [ \"$prev\" = \"-Extension\" ]; then extension_name=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log for %s\\n' \"$args\" > \"$out\"; fi\nprintf '%s\\n' \"$args\" >> \"{}\"\nif [ -z \"$target\" ]; then exit 0; fi\n{}\nmkdir -p \"$target\"\nif [ -n \"$extension_name\" ]; then\n  config_xml='<Configuration><Properties><Name>ExtensionProject</Name></Properties><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>'\nelse\n  config_xml='<Configuration><Properties><Name>BaseProject</Name></Properties></Configuration>'\nfi\nprintf '%s\\n' \"$config_xml\" > \"$target/Configuration.xml\"\nif printf '%s' \"$args\" | grep -F -q -- '-partial'; then\n  printf '<Partial />\\n' > \"$target/PartialOnly.xml\"\nfi\nexit 0",
            calls_log.display(),
            pattern_branch
        );
        write_script(path, &body);
    }

    fn write_ibcmd_dump_script_for_edt(path: &Path, calls_log: &Path, fail_pattern: Option<&str>) {
        let pattern_branch = fail_pattern
            .map(|pattern| {
                format!(
                    "if printf '%s' \"$args\" | grep -F -q -- '{}'; then exit 17; fi",
                    pattern
                )
            })
            .unwrap_or_default();
        let body = format!(
            "args=\"$*\"\ncase \" $args \" in *\" generation-id \"*) printf '%s\\n' \"$args\" >> \"{}\"; exit 0;; esac\ntarget=\"$(printf '%s' \"$args\" | awk '{{print $NF}}')\"\nextension_name=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"-Extension\" ]; then extension_name=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"{}\"\n{}\nmkdir -p \"$target\"\nif [ -n \"$extension_name\" ]; then\n  printf '<Configuration><Properties><Name>ExtensionProject</Name></Properties><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>\\n' > \"$target/Configuration.xml\"\nelse\n  printf '<Configuration><Properties><Name>BaseProject</Name></Properties></Configuration>\\n' > \"$target/Configuration.xml\"\nfi\nexit 0",
            calls_log.display(),
            calls_log.display(),
            pattern_branch
        );
        write_script(path, &body);
    }

    fn write_edt_import_script(path: &Path, calls_log: &Path) {
        let body = format!(
            r#"args="$*"
printf '%s\n' "$args" >> "{}"
project=""
config_files=""
base_project_name=""
prev=""
read_configuration_name() {{
  config_file="$1/Configuration.xml"
  if [ ! -f "$config_file" ]; then
    printf 'ImportedProject'
    return
  fi
  name=$(sed -n 's:.*<Name>\([^<][^<]*\)</Name>.*:\1:p' "$config_file" | head -n 1)
  if [ -n "$name" ]; then
    printf '%s' "$name"
  else
    printf 'ImportedProject'
  fi
}}
configuration_is_extension() {{
  config_file="$1/Configuration.xml"
  [ -f "$config_file" ] && grep -q 'ConfigurationExtensionPurpose\|ObjectBelonging' "$config_file"
}}
for arg in "$@"; do
  if [ "$prev" = "--project" ]; then project="$arg"; fi
  if [ "$prev" = "--configuration-files" ]; then config_files="$arg"; fi
  if [ "$prev" = "--base-project-name" ]; then base_project_name="$arg"; fi
  prev="$arg"
done
imported_name=$(read_configuration_name "$config_files")
mkdir -p "$project/DT-INF" "$project/src/Configuration"
if configuration_is_extension "$config_files"; then
  if [ "$base_project_name" != "BaseProject" ]; then
    printf 'unexpected base project: %s\n' "$base_project_name" >&2
    exit 23
  fi
  imported_nature="{}"
  imported_base="BaseProject"
else
  imported_nature="{}"
  imported_base=""
fi
cat > "$project/.project" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<projectDescription>
  <name>$imported_name</name>
  <natures>
    <nature>$imported_nature</nature>
  </natures>
</projectDescription>
EOF
{{
  if [ -n "$imported_base" ]; then printf 'Base-Project: %s\n' "$imported_base"; fi
  printf 'Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n'
}} > "$project/DT-INF/PROJECT.PMF"
printf '<Configuration />\n' > "$project/src/Configuration/Configuration.mdo"
printf 'Procedure Test()\nEndProcedure\n' > "$project/src/Configuration/Module.bsl"
exit 0"#,
            calls_log.display(),
            crate::support::edt_project::V8_EXTENSION_NATURE,
            crate::support::edt_project::V8_CONFIGURATION_NATURE
        );
        write_script(path, &body);
    }

    /// Наблюдатель за каждым запросом к процессу: тест подсматривает аргументы,
    /// не подменяя самого исполнителя.
    type RunObserver = Arc<dyn Fn(&ProcessRequest) + Send + Sync>;

    #[derive(Clone, Default)]
    struct TestProcessRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        cancel_after_call: Option<(usize, CancellationToken)>,
        on_run: Option<RunObserver>,
    }

    impl TestProcessRunner {
        fn with_cancellation_on_call(call_index: usize, cancellation: CancellationToken) -> Self {
            Self {
                cancel_after_call: Some((call_index, cancellation)),
                ..Self::default()
            }
        }

        fn with_on_run(on_run: impl Fn(&ProcessRequest) + Send + Sync + 'static) -> Self {
            Self {
                on_run: Some(Arc::new(on_run)),
                ..Self::default()
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().expect("calls").len()
        }

        fn run_request(&self, request: &ProcessRequest) -> Result<ProcessResult, ProcessError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("calls");
                calls.push(request.args.clone());
                calls.len()
            };
            if let Some(on_run) = &self.on_run {
                on_run(request);
            }
            if let Some((cancel_on_call, cancellation)) = &self.cancel_after_call {
                if call_index == *cancel_on_call {
                    cancellation.cancel();
                }
            }
            Ok(ProcessResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            })
        }
    }

    impl ProcessRunner for TestProcessRunner {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            // Как настоящий исполнитель, двойник отмечает работу, едва «запустил» процесс.
            policy.mark_started_for_test();
            self.run_request(request)
        }

        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &crate::platform::process::WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            panic!("spawn must not be used in dump_config tests")
        }
    }

    fn build_config(base_path: &Path, work_path: &Path, platform_path: &Path) -> AppConfig {
        build_config_with_builder(base_path, work_path, platform_path, Default::default())
    }

    fn build_config_with_builder(
        base_path: &Path,
        work_path: &Path,
        platform_path: &Path,
        providers: std::collections::BTreeMap<
            crate::domain::capability::Operation,
            crate::domain::capability::Provider,
        >,
    ) -> AppConfig {
        AppConfig {
            base_path: base_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers,
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: PathBuf::from("main"),
                },
                SourceSetConfig {
                    name: "ext".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: PathBuf::from("ext"),
                },
            ],
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(platform_path.to_path_buf()),
                    strict: false,
                    version: None,
                },
                ..ToolsConfig::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    /// Снимок Конфигуратора набора EDT: под памятью выбранной базы.
    fn designer_snapshot(config: &AppConfig, name: &str) -> PathBuf {
        crate::use_cases::source_inventory::SourceSetInventory::new(config)
            .designer_context(name)
            .expect("designer context")
            .path()
            .to_path_buf()
    }

    fn build_edt_config(
        base_path: &Path,
        work_path: &Path,
        platform_path: &Path,
        edt_path: &Path,
        providers: std::collections::BTreeMap<
            crate::domain::capability::Operation,
            crate::domain::capability::Provider,
        >,
    ) -> AppConfig {
        let mut config = build_config_with_builder(base_path, work_path, platform_path, providers);
        config.format = SourceFormat::Edt;
        config.tools.edt_cli.path = Some(edt_path.to_path_buf());
        config
    }

    /// Фиксирует всё дерево исходников в репозитории под `base_path`: каталог набора без
    /// незафиксированного сторож пропускает и без согласия.
    fn commit_sources(base_path: &Path) {
        if !base_path.join(".git").exists() {
            crate::platform::test_git::init_git_repo(base_path);
        }
        crate::platform::test_git::run_git(base_path, &["add", "-A"]);
        crate::platform::test_git::run_git(base_path, &["commit", "-qm", "sources"]);
    }

    fn create_source_tree(base_path: &Path) {
        fs::create_dir_all(base_path.join("main").join("Catalogs.Items")).expect("main");
        fs::create_dir_all(base_path.join("ext").join("CommonModules")).expect("ext");
        fs::write(
            base_path
                .join("main")
                .join("Catalogs.Items")
                .join("ObjectModule.bsl"),
            "module",
        )
        .expect("main bsl");
        fs::write(
            base_path
                .join("ext")
                .join("CommonModules")
                .join("Module.bsl"),
            "module",
        )
        .expect("ext bsl");
        commit_sources(base_path);
    }

    fn write_native_edt_project(path: &Path, project_name: &str, nature: &str, base: Option<&str>) {
        fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
        fs::create_dir_all(path.join("src").join("Configuration")).expect("src");
        let base_line = base
            .map(|value| format!("Base-Project: {value}\n"))
            .unwrap_or_default();
        fs::write(
            path.join(".project"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{project_name}</name>\n  <natures>\n    <nature>{nature}</nature>\n  </natures>\n</projectDescription>\n"
            ),
        )
        .expect("project");
        fs::write(
            path.join("DT-INF").join("PROJECT.PMF"),
            format!("{base_line}Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n"),
        )
        .expect("manifest");
        fs::write(
            path.join("src")
                .join("Configuration")
                .join("Configuration.mdo"),
            "<Configuration />\n",
        )
        .expect("configuration marker");
        fs::write(
            path.join("src").join("Configuration").join("Module.bsl"),
            "Procedure Test()\nEndProcedure\n",
        )
        .expect("module marker");
    }

    fn create_edt_source_tree(base_path: &Path) {
        write_native_edt_project(
            &base_path.join("main"),
            "BaseProject",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );
        write_native_edt_project(
            &base_path.join("ext"),
            "ExtensionProject",
            crate::support::edt_project::V8_EXTENSION_NATURE,
            Some("BaseProject"),
        );
        commit_sources(base_path);
    }

    fn assert_native_edt_project(path: &Path) {
        assert!(path.join(".project").exists());
        assert!(path.join("DT-INF").join("PROJECT.PMF").exists());
        assert!(path.join("src/Configuration/Configuration.mdo").exists());
    }

    fn partial_list_paths(work_path: &Path) -> Vec<PathBuf> {
        let partial_dir = work_path.join("temp").join("partial-lists");
        if !partial_dir.is_dir() {
            return Vec::new();
        }

        let mut paths = fs::read_dir(partial_dir)
            .expect("partial lists dir")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    #[test]
    fn edt_dump_support_matrix_accepts_designer_backend() {
        let dir = tempdir().expect("tempdir");
        let config = AppConfig {
            format: SourceFormat::Edt,
            providers: Default::default(),
            provider_origins: Default::default(),
            ..build_config(dir.path(), dir.path(), dir.path())
        };

        let error = validate_supported_matrix(&config);

        assert!(error.is_none());
    }

    #[test]
    fn ibcmd_dump_support_matrix_accepts_designer_format_with_ibcmd_builder() {
        let dir = tempdir().expect("tempdir");
        let config = build_config_with_builder(
            dir.path(),
            dir.path(),
            dir.path(),
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let error = validate_supported_matrix(&config);

        assert!(error.is_none());
    }

    #[test]
    fn partial_requires_at_least_one_object() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: None,
                extension: None,
                objects: vec![],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(failure.error.message(), PARTIAL_OBJECTS_REQUIRED_ERROR);
    }

    #[test]
    fn partial_rejects_blank_objects() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: None,
                extension: None,
                objects: vec!["   ".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(failure.error.message(), PARTIAL_OBJECT_BLANK_ERROR);
    }

    #[test]
    fn partial_rejects_control_characters_in_objects() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: None,
                extension: None,
                objects: vec!["Catalog.Items\nLine".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(failure.error.message(), PARTIAL_OBJECT_CONTROL_ERROR);
    }

    #[test]
    fn partial_accepts_future_root_type_before_running_designer() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(dir.path());
        write_script(&script, &format!("touch '{}'", calls.display()));
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: None,
                extension: None,
                objects: vec!["Unknown:Items".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        let expected_selectors = [DumpSelectorResult {
            requested: "Unknown:Items".to_owned(),
            normalized: "Unknown.Items".to_owned(),
        }];
        assert_eq!(
            result.selectors.as_deref(),
            Some(expected_selectors.as_slice())
        );
        assert!(calls.exists());
    }

    #[test]
    fn partial_rejects_leading_or_trailing_control_characters_after_trim() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        for object in ["\nCatalog.Items".to_owned(), "Catalog.Items\t".to_owned()] {
            let failure = run_dump(
                &config,
                &DumpArgs {
                    dry_run: false,
                    discard_uncommitted: false,
                    force_way_out: ForceWayOut::PullForce,
                    mode: DumpModeRequest::Partial,
                    source_set: None,
                    extension: None,
                    objects: vec![object],
                },
            )
            .expect_err("failure");

            assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
            assert_eq!(failure.error.message(), PARTIAL_OBJECT_CONTROL_ERROR);
        }
    }

    #[test]
    fn rejects_objects_for_incremental() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Incremental,
                source_set: None,
                extension: None,
                objects: vec!["Catalog:Items".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(failure.error.message(), NON_PARTIAL_OBJECTS_ERROR);
    }

    #[test]
    fn rejects_objects_for_full() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: None,
                extension: None,
                objects: vec!["Catalog:Items".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(failure.error.message(), NON_PARTIAL_OBJECTS_ERROR);
    }

    #[test]
    fn resolve_target_requires_explicit_source_set_when_multiple_configurations_exist() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let mut config = build_config(dir.path(), &dir.path().join("work"), &script);
        config.source_sets.push(SourceSetConfig {
            name: "main2".to_owned(),
            purpose: SourceSetPurpose::Configuration,
            path: PathBuf::from("main"),
        });

        let error = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: None,
                extension: None,
                objects: vec![],
            },
        )
        .expect_err("expected ambiguity");

        assert!(matches!(error, AppError::Validation(_)));
        let message = error.to_string();
        assert!(message.contains("when <SET> is omitted"), "{message}");
        assert!(!message.contains("--source-set"), "{message}");
    }

    #[test]
    fn resolve_target_requires_extension_source_set_to_match_extension_name() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config(dir.path(), &dir.path().join("work"), &script);

        let error = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: Some("ext".to_owned()),
                objects: vec![],
            },
        )
        .expect_err("expected mismatch");

        assert!(matches!(error, AppError::Validation(_)));
        let message = error.to_string();
        assert!(
            message.contains("<SET> 'main' does not match --extension 'ext'"),
            "{message}"
        );
        assert!(!message.contains("--source-set"), "{message}");
    }

    #[test]
    fn validate_publish_target_allows_absolute_source_set_outside_base_path() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let external = dir.path().join("external-main");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::create_dir_all(&external).expect("external");
        fs::create_dir_all(&work).expect("work");
        write_script(&script, "exit 0");

        let mut config = build_config(&base, &work, &script);
        config.source_sets[0].path = external.clone();

        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved");

        validate_publish_target(&resolved).expect("absolute source-set should be allowed");
    }

    #[test]
    fn validate_publish_target_rejects_base_path() {
        let dir = tempdir().expect("tempdir");
        let resolved = super::ResolvedDumpTarget {
            source_set_name: "main".to_owned(),
            source_set_purpose: SourceSetPurpose::Configuration,
            extension: None,
            target_path: dir.path().to_path_buf(),
            canonical_target_path: std::fs::canonicalize(dir.path()).expect("canonical"),
            platform_target_path: dir.path().to_path_buf(),
            canonical_platform_target_path: std::fs::canonicalize(dir.path()).expect("canonical"),
            canonical_base_path: std::fs::canonicalize(dir.path()).expect("canonical"),
            canonical_work_path: std::fs::canonicalize(dir.path().join("work").as_path())
                .unwrap_or_else(|_| dir.path().join("work")),
            target_identity: "id".to_owned(),
            platform_target_identity: "id".to_owned(),
            lock_path: dir.path().join(".lock"),
            edt_base_project_name: None,
            consent: DestructionConsent::RunnerOwned,
        };

        let error = validate_publish_target(&resolved).expect_err("expected invalid");
        assert!(matches!(error, AppError::Validation(_)));
    }

    /// Служебный снимок Конфигуратора у проекта EDT раннер заводит сам: его публикация
    /// сторожа не спрашивает, а каталог проекта человека спрашивает.
    #[test]
    fn the_edt_designer_snapshot_is_runner_owned() {
        let dir = tempdir().expect("tempdir");
        let project = dir.path().join("project");
        let snapshot = dir.path().join("work").join("designer");
        let resolved = super::ResolvedDumpTarget {
            source_set_name: "main".to_owned(),
            source_set_purpose: SourceSetPurpose::Configuration,
            extension: None,
            target_path: project.clone(),
            canonical_target_path: project.clone(),
            platform_target_path: snapshot.clone(),
            canonical_platform_target_path: snapshot,
            canonical_base_path: dir.path().to_path_buf(),
            canonical_work_path: dir.path().join("work"),
            target_identity: "id".to_owned(),
            platform_target_identity: "snapshot".to_owned(),
            lock_path: dir.path().join(".lock"),
            edt_base_project_name: None,
            consent: DestructionConsent::AskFirst(
                crate::use_cases::destruction_guard::WaysOut::SaveWork,
            ),
        };

        assert_eq!(
            resolved.platform_consent(),
            &DestructionConsent::RunnerOwned
        );

        let designer = super::ResolvedDumpTarget {
            platform_target_path: project.clone(),
            canonical_platform_target_path: project,
            ..resolved
        };
        assert_eq!(
            designer.platform_consent(),
            &DestructionConsent::AskFirst(crate::use_cases::destruction_guard::WaysOut::SaveWork)
        );
    }

    #[test]
    fn nearest_existing_canonical_path_uses_existing_ancestor() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).expect("root");

        let resolved =
            nearest_existing_canonical_path(&root.join("nested").join("target")).expect("resolved");

        assert_eq!(
            resolved,
            std::fs::canonicalize(&root)
                .expect("canonical")
                .join("nested/target")
        );
    }

    /// Цель `main` в `dir` такой, какой её видит уборка: те же канонический путь и
    /// опознание, что своя выгрузка пишет в метаданные следа.
    fn resolved_dump_target(dir: &Path) -> super::ResolvedDumpTarget {
        let target = dir.join("main");
        fs::create_dir_all(&target).expect("target");
        let canonical = std::fs::canonicalize(&target).expect("canonical");
        let identity = stable_path_identity(&canonical);
        let canonical_dir = std::fs::canonicalize(dir).expect("canonical dir");
        super::ResolvedDumpTarget {
            source_set_name: "main".to_owned(),
            source_set_purpose: SourceSetPurpose::Configuration,
            extension: None,
            target_path: target.clone(),
            canonical_target_path: canonical.clone(),
            platform_target_path: target,
            canonical_platform_target_path: canonical,
            canonical_base_path: canonical_dir.clone(),
            canonical_work_path: canonical_dir,
            target_identity: identity.clone(),
            platform_target_identity: identity,
            lock_path: dir.join(".lock"),
            edt_base_project_name: None,
            consent: DestructionConsent::RunnerOwned,
        }
    }

    /// Промежуточный или резервный каталог выгрузки в `resolved` рядом с целью — с именем
    /// `<префикс>-<запуск>` и метаданными, какими их пишет сама выгрузка.
    fn temp_dir_of(resolved: &super::ResolvedDumpTarget, kind: TempDirKind) -> PathBuf {
        let prefix = match kind {
            TempDirKind::Stage => ".dump-stage",
            TempDirKind::Backup => DUMP_BACKUP_PREFIX,
        };
        let path = resolved
            .target_path
            .parent()
            .expect("parent")
            .join(format!("{prefix}-run"));
        fs::create_dir_all(&path).expect("temp dir");
        write_temp_dir_metadata(
            &path,
            kind,
            "run",
            &resolved.target_path,
            &resolved.target_identity,
        )
        .expect("write metadata");
        path
    }

    /// Тот же каталог, но старше срока уборки; `edit` правит его метаданные.
    fn stale_temp_dir(
        resolved: &super::ResolvedDumpTarget,
        kind: TempDirKind,
        edit: impl FnOnce(&mut TempDirMetadata),
    ) -> PathBuf {
        let path = temp_dir_of(resolved, kind);
        let mut metadata = read_temp_dir_metadata(&path).expect("read metadata");
        metadata.created_at = chrono::Utc::now()
            - chrono::Duration::from_std(ORPHAN_TTL + Duration::from_secs(1)).expect("duration");
        edit(&mut metadata);
        fs::write(
            metadata_sidecar_path(&path),
            serde_json::to_vec(&metadata).expect("json"),
        )
        .expect("rewrite metadata");
        path
    }

    #[test]
    fn cleanup_orphan_dirs_ignores_malformed_metadata() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let stage_dir = dir.path().join(".dump-stage-run");
        fs::create_dir_all(&stage_dir).expect("stage");
        fs::write(metadata_sidecar_path(&stage_dir), b"not json").expect("metadata");

        cleanup_orphan_dirs(&resolved).expect("cleanup");
        assert!(stage_dir.exists());
    }

    #[test]
    fn cleanup_orphan_dirs_removes_old_valid_metadata() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let stage_dir = stale_temp_dir(&resolved, TempDirKind::Stage, |_| {});
        let meta_path = metadata_sidecar_path(&stage_dir);

        cleanup_orphan_dirs(&resolved).expect("cleanup");
        assert!(!stage_dir.exists());
        assert!(!meta_path.exists());
    }

    #[test]
    fn cleanup_orphan_dirs_ignores_recent_metadata() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = temp_dir_of(&resolved, TempDirKind::Backup);

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(backup_dir.exists());
        assert!(metadata_sidecar_path(&backup_dir).exists());
    }

    #[test]
    fn cleanup_orphan_dirs_ignores_foreign_metadata() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let stage_dir = stale_temp_dir(&resolved, TempDirKind::Stage, |metadata| {
            metadata.tool = "foreign-tool".to_owned();
        });
        let meta_path = metadata_sidecar_path(&stage_dir);

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(stage_dir.exists());
        assert!(meta_path.exists());
    }

    /// След своей выгрузки, но другой цели в том же каталоге, — не свой для этой цели.
    #[test]
    fn cleanup_orphan_dirs_ignores_metadata_of_another_target() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = stale_temp_dir(&resolved, TempDirKind::Backup, |metadata| {
            metadata.target_identity = "another target".to_owned();
        });

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(backup_dir.exists());
        assert!(metadata_sidecar_path(&backup_dir).exists());
    }

    /// Свой устаревший каталог с именем вне договора уборка не трогает: своё она опознаёт
    /// и по метаданным, и по имени.
    #[test]
    fn cleanup_orphan_dirs_ignores_a_directory_named_outside_the_contract() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let stale = stale_temp_dir(&resolved, TempDirKind::Backup, |_| {});
        let renamed = dir.path().join("backup-copy");
        fs::rename(&stale, &renamed).expect("rename dir");
        fs::rename(
            metadata_sidecar_path(&stale),
            metadata_sidecar_path(&renamed),
        )
        .expect("rename sidecar");

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(renamed.exists());
        assert!(metadata_sidecar_path(&renamed).exists());
    }

    #[test]
    fn cleanup_staging_on_interruption_removes_stage_dir_and_sidecar() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let stage_dir = temp_dir_of(&resolved, TempDirKind::Stage);
        let meta_path = metadata_sidecar_path(&stage_dir);

        let error = cleanup_staging_on_interruption(
            &stage_dir,
            AppError::Runtime("interrupted before publish".to_owned()),
        );

        assert_eq!(
            error.to_string(),
            "runtime error: interrupted before publish"
        );
        assert!(!stage_dir.exists());
        assert!(!meta_path.exists());
    }

    /// Выгрузка по изменившемуся без файла версий становится полной до запуска платформы:
    /// `-update` в аргументах нет, каталог создан, ответ называет полный режим и причину.
    #[test]
    fn an_incremental_dump_without_a_version_file_runs_full_before_the_platform_starts() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_dump_script(&script, &calls, None, 0);
        let config = build_config(&base, &work, &script);
        fs::remove_dir_all(base.join("main")).expect("remove target");

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Incremental,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert!(base.join("main").exists());
        assert_eq!(result.mode, DumpMode::Full);
        let message = result.message.expect("the reason is named");
        assert!(
            message.contains("no version file ConfigDumpInfo.xml"),
            "{message}"
        );
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("/DumpConfigToFiles"));
        assert!(!calls.contains("-update"));
        assert!(calls.contains(base.join("main").display().to_string().as_str()));
    }

    /// Файл без распознанной версии формата чужой, и выгрузка по изменившемуся становится
    /// полной. Прочитанную версию чужой делает только таблица замеров, а она пуста (#403):
    /// без неё никакая версия чужой не считается. Механизм сверки — на версии из теста.
    #[test]
    fn an_unrecognized_format_turns_the_dump_full_and_a_version_is_foreign_only_by_measurement() {
        use super::{
            plan_dump, unusable_version_file, DumpPlan, FormatVersion, OverDirectory,
            RecordedFormat, WholeReason, VERSION_FILE_NAME,
        };
        let dir = tempdir().expect("tempdir");
        let platform = crate::platform::locator::PlatformVersion {
            major: 8,
            minor: 3,
            patch: 27,
            build: 2074,
        };
        let file = dir.path().join(VERSION_FILE_NAME);
        let plan = |file: Option<&Path>| {
            plan_dump(&DumpMode::Incremental, file, Some(&platform)).expect("plan")
        };
        let whole = |reason| DumpPlan::OverDirectory(OverDirectory::Whole(reason));

        assert_eq!(plan(None), whole(WholeReason::Missing));
        assert_eq!(plan(Some(&file)), whole(WholeReason::Missing));
        fs::write(&file, "<ConfigDumpInfo format=\"Hierarchical\">").expect("no version");
        assert_eq!(plan(Some(&file)), whole(WholeReason::Unrecognized));
        assert_eq!(plan(Some(&file)).mode(), DumpMode::Full);
        for version in ["2.17", "2.20", "9.99"] {
            fs::write(
                &file,
                format!("<ConfigDumpInfo format=\"Hierarchical\" version=\"{version}\">"),
            )
            .expect("version file");
            assert_eq!(
                plan(Some(&file)),
                DumpPlan::OverDirectory(OverDirectory::ByVersionFile),
                "{version}"
            );
        }
        assert_eq!(
            plan_dump(&DumpMode::Full, None, None).expect("full"),
            DumpPlan::Full
        );

        let measured = Some((&platform, FormatVersion::new(2, 20)));
        let reason =
            unusable_version_file(RecordedFormat::Version(FormatVersion::new(2, 17)), measured)
                .expect("a foreign format");
        let described = reason.describe(dir.path());
        assert!(described.contains("format 2.17"), "{described}");
        assert!(described.contains("8.3.27.2074 writes 2.20"), "{described}");
        assert_eq!(
            unusable_version_file(RecordedFormat::Version(FormatVersion::new(2, 20)), measured),
            None
        );
        assert_eq!(
            unusable_version_file(RecordedFormat::Version(FormatVersion::new(2, 17)), None),
            None
        );
    }

    #[test]
    fn dump_incremental_designer_extension_uses_update_and_extension_flag() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        fs::write(
            base.join("ext/ConfigDumpInfo.xml"),
            "<ConfigDumpInfo version=\"2.17\"/>",
        )
        .expect("version file");
        write_dump_script(&script, &calls, None, 0);
        let config = build_config(&base, &work, &script);

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Incremental,
                source_set: Some("ext".to_owned()),
                extension: Some("ext".to_owned()),
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("/DumpConfigToFiles"));
        assert!(calls.contains("-update"));
        assert!(!calls.contains("-updateConfigDumpInfo"));
        assert!(calls.contains("-Extension"));
        assert!(calls.contains("ext"));
    }

    #[test]
    fn partial_validation_is_shared_with_ibcmd() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("ibcmd");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let config = build_config_with_builder(
            dir.path(),
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalog\nItem".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(failure.error.message(), PARTIAL_OBJECT_CONTROL_ERROR);
    }

    #[test]
    fn partial_dump_write_failure_cleans_up_temp_file() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        let partial_root = work.join("temp");
        fs::create_dir_all(&partial_root).expect("temp");
        fs::write(partial_root.join("partial-lists"), "not a dir").expect("sentinel");

        let objects = [PartialDumpSelector::parse("Catalog.Items").expect("selector")];
        let error = create_dump_object_list_file_with(&work, &objects, |_file, _objects| Ok(()))
            .expect_err("expected failure");

        assert!(matches!(error, AppError::Runtime(_)));
        assert!(partial_list_paths(&work).is_empty());
    }

    #[test]
    fn partial_dump_writer_failure_does_not_leave_temp_file() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");

        let objects = [PartialDumpSelector::parse("Catalog.Items").expect("selector")];
        let error = create_dump_object_list_file_with(&work, &objects, |_file, _objects| {
            Err(std::io::Error::other("boom"))
        })
        .expect_err("expected failure");

        assert!(matches!(error, AppError::Runtime(_)));
        assert!(partial_list_paths(&work).is_empty());
    }

    #[test]
    fn dump_partial_designer_creates_missing_target_dir_and_writes_normalized_list() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        let captured_list = dir.path().join("captured-list.txt");
        create_source_tree(&base);
        write_script(
            &script,
            &format!(
                "args=\"$*\"\nout=\"\"\nlist=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  if [ \"$prev\" = \"-listFile\" ]; then list=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log for %s\\n' \"$args\" > \"$out\"; fi\nif [ -n \"$list\" ]; then cp \"$list\" \"{}\"; fi\nprintf '%s\\n' \"$args\" >> \"{}\"\nexit 0",
                captured_list.display(),
                calls.display(),
            ),
        );
        let config = build_config(&base, &work, &script);
        fs::remove_dir_all(base.join("main")).expect("remove target");

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalog:Items".to_owned(), "Document:Order".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_eq!(result.mode, DumpMode::Partial);
        let expected_selectors = [
            DumpSelectorResult {
                requested: "Catalog:Items".to_owned(),
                normalized: "Catalog.Items".to_owned(),
            },
            DumpSelectorResult {
                requested: "Document:Order".to_owned(),
                normalized: "Document.Order".to_owned(),
            },
        ];
        assert_eq!(
            result.selectors.as_deref(),
            Some(expected_selectors.as_slice())
        );
        assert!(base.join("main").exists());
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("/DumpConfigToFiles"));
        assert!(calls.contains("-partial"));
        assert!(calls.contains("-listFile"));
        assert!(!calls.contains("-updateConfigDumpInfo"));
        assert_eq!(
            fs::read_to_string(captured_list).expect("captured list"),
            "Catalog.Items\nDocument.Order\n"
        );
        assert!(partial_list_paths(&work).is_empty());
    }

    #[test]
    fn dump_partial_designer_extension_uses_extension_flag() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_dump_script(&script, &calls, None, 0);
        let config = build_config(&base, &work, &script);

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("ext".to_owned()),
                extension: Some("ext".to_owned()),
                objects: vec!["CommonModule.Module".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("-Extension"));
        assert!(calls.contains("ext"));
        assert!(partial_list_paths(&work).is_empty());
    }

    #[test]
    fn dump_partial_designer_failure_cleans_up_temp_file_and_keeps_partial_mode() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_dump_script(&script, &calls, Some("-partial"), 0);
        let config = build_config(&base, &work, &script);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalog.Items".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        assert_eq!(failure.payload.expect("payload").mode, DumpMode::Partial);
        assert!(partial_list_paths(&work).is_empty());
    }

    #[test]
    fn dump_partial_ibcmd_uses_sync_and_returns_warning() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, None, 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalog.Items".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_eq!(result.mode, DumpMode::Partial);
        assert!(result
            .message
            .as_deref()
            .expect("warning")
            .contains("IBCMD does not support object-scoped partial dump"));
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("--sync"));
    }

    #[test]
    fn dump_partial_ibcmd_accepts_future_root_type_and_degrades_to_incremental() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, None, 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["FutureRoot:Items".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_eq!(result.mode, DumpMode::Partial);
        assert!(result
            .message
            .as_deref()
            .expect("warning")
            .contains("IBCMD does not support object-scoped partial dump"));
        let expected_selectors = [DumpSelectorResult {
            requested: "FutureRoot:Items".to_owned(),
            normalized: "FutureRoot.Items".to_owned(),
        }];
        assert_eq!(
            result.selectors.as_deref(),
            Some(expected_selectors.as_slice())
        );
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("--sync"));
        assert!(!calls.contains("FutureRoot.Items"));
    }

    #[test]
    fn dump_partial_ibcmd_extension_uses_extension_flag() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, None, 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("ext".to_owned()),
                extension: Some("ext".to_owned()),
                objects: vec!["CommonModule.Module".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_eq!(result.mode, DumpMode::Partial);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("--sync"));
        assert!(calls.contains("--extension ext"));
        assert!(result
            .message
            .as_deref()
            .expect("warning")
            .contains("extension 'ext'"));
    }

    #[test]
    fn dump_partial_ibcmd_failure_keeps_partial_mode_and_warning() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, Some("--sync"), 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalog.Items".to_owned()],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        assert!(failure
            .error
            .message()
            .contains("IBCMD does not support object-scoped partial dump"));
        let payload = failure.payload.expect("payload");
        assert_eq!(payload.mode, DumpMode::Partial);
        assert!(payload
            .message
            .as_deref()
            .expect("message")
            .contains("IBCMD does not support object-scoped partial dump"));
    }

    #[test]
    fn dump_full_preserves_old_dump_on_platform_failure() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_dump_script(&script, &calls, Some("/DumpConfigToFiles"), 0);
        let config = build_config(&base, &work, &script);
        fs::write(base.join("main").join("old.txt"), "keep me").expect("old");
        commit_sources(&base);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        assert_eq!(
            fs::read_to_string(base.join("main").join("old.txt")).expect("old"),
            "keep me"
        );
        // Конфигуратору назван промежуточный каталог, а не цель.
        let calls = fs::read_to_string(&calls).expect("calls");
        assert!(calls.contains(".dump-stage-"), "{calls}");
    }

    #[test]
    fn dump_full_success_replaces_old_target() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_dump_script(&script, &calls, None, 0);
        let config = build_config(&base, &work, &script);
        fs::write(base.join("main").join("old.txt"), "old").expect("old");
        commit_sources(&base);

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert!(!base.join("main").join("old.txt").exists());
    }

    #[test]
    fn ibcmd_dump_full_uses_staging_dir_and_atomic_publish() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, None, 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        fs::write(base.join("main").join("old.txt"), "old").expect("old");
        commit_sources(&base);

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        let calls = fs::read_to_string(calls).expect("calls");
        assert!(result.ok);
        assert!(calls.contains("--force"));
        assert!(calls.contains(".dump-stage-"));
        assert!(!base.join("main").join("old.txt").exists());
    }

    #[test]
    fn ibcmd_dump_with_server_infobase_passes_dbms_and_infobase_credentials() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, None, 0);
        let mut config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        config.infobase = crate::config::model::InfobaseConfig::server(
            "Srvr=cluster:1541;Ref=demo",
            crate::config::model::InfobaseDbmsConfig::new("PostgreSQL", "localhost", "demo")
                .with_credentials(Some("postgres".to_owned()), Some("pg-secret".to_owned())),
        )
        .with_credentials(Some("Admin".to_owned()), Some("secret".to_owned()));

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("--dbms PostgreSQL"));
        assert!(calls.contains("--database-server localhost"));
        assert!(calls.contains("--database-name demo"));
        assert!(calls.contains("--database-user postgres"));
        assert!(calls.contains("--database-password pg-secret"));
        assert!(calls.contains("--user Admin"));
        assert!(calls.contains("--password secret"));
    }

    #[test]
    fn ibcmd_dump_full_preserves_old_target_on_platform_failure() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, Some("--force"), 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        fs::write(base.join("main").join("old.txt"), "keep me").expect("old");
        commit_sources(&base);

        let failure = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        assert_eq!(
            fs::read_to_string(base.join("main").join("old.txt")).expect("old"),
            "keep me"
        );
    }

    #[test]
    fn ibcmd_dump_incremental_uses_sync_against_resolved_target() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_ibcmd_dump_script(&script, &calls, None, 0);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        fs::write(
            base.join("main/ConfigDumpInfo.xml"),
            "<ConfigDumpInfo version=\"2.17\"/>",
        )
        .expect("version file");
        let args = DumpArgs {
            dry_run: false,
            discard_uncommitted: false,
            force_way_out: ForceWayOut::PullForce,
            mode: DumpModeRequest::Incremental,
            source_set: Some("main".to_owned()),
            extension: None,
            objects: vec![],
        };

        let result = run_dump(&config, &args).expect("dump");

        assert!(result.ok);
        assert_eq!(result.mode, DumpMode::Incremental);
        let synced = fs::read_to_string(&calls).expect("calls");
        assert!(synced.contains("--sync"));
        assert!(synced.contains(base.join("main").display().to_string().as_str()));

        // Без файла версий `ibcmd` выгружает полностью поверх каталога: без `--sync` и без
        // `--force`, который заменил бы каталог.
        fs::remove_dir_all(base.join("main")).expect("remove target");
        fs::remove_file(&calls).expect("reset calls");
        let result = run_dump(&config, &args).expect("dump");
        assert!(result.ok);
        assert_eq!(result.mode, DumpMode::Full);
        let full = fs::read_to_string(&calls).expect("calls");
        assert!(!full.contains("--sync"), "{full}");
        assert!(!full.contains("--force"), "{full}");
        assert!(full.contains(base.join("main").display().to_string().as_str()));
    }

    #[test]
    fn dump_full_edt_designer_updates_designer_mirror_and_publishes_edt_target() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("edt").join("1cedtcli");
        let designer_calls = dir.path().join("designer-calls.log");
        let edt_calls = dir.path().join("edt-calls.log");
        create_edt_source_tree(&base);
        write_designer_dump_script_for_edt(&designer, &designer_calls, None);
        write_edt_import_script(&edt, &edt_calls);
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        fs::write(base.join("main").join("stale.txt"), "stale").expect("stale");
        commit_sources(&base);

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_eq!(result.target_path, base.join("main"));
        assert_native_edt_project(&base.join("main"));
        assert!(!base.join("main").join("stale.txt").exists());
        assert!(designer_snapshot(&config, "main")
            .join("Configuration.xml")
            .exists());

        let designer_calls = fs::read_to_string(designer_calls).expect("designer calls");
        let edt_calls = fs::read_to_string(edt_calls).expect("edt calls");
        assert!(designer_calls.contains(
            designer_snapshot(&config, "main")
                .parent()
                .expect("snapshot parent")
                .display()
                .to_string()
                .as_str()
        ));
        assert!(edt_calls.contains(
            designer_snapshot(&config, "main")
                .display()
                .to_string()
                .as_str()
        ));
        assert!(edt_calls.contains(work.join("edt-workspace").display().to_string().as_str()));
    }

    #[test]
    fn dump_partial_edt_designer_bootstraps_missing_or_invalid_designer_snapshot() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("edt").join("1cedtcli");
        let designer_calls = dir.path().join("designer-calls.log");
        let edt_calls = dir.path().join("edt-calls.log");
        create_edt_source_tree(&base);
        write_designer_dump_script_for_edt(&designer, &designer_calls, None);
        write_edt_import_script(&edt, &edt_calls);
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        fs::create_dir_all(designer_snapshot(&config, "main")).expect("empty designer snapshot");
        fs::write(
            designer_snapshot(&config, "main").join("BrokenMirror.xml"),
            "<Broken />\n",
        )
        .expect("broken snapshot marker");

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalog.Items".to_owned()],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_native_edt_project(&base.join("main"));
        assert!(designer_snapshot(&config, "main")
            .join("Configuration.xml")
            .exists());
        assert!(designer_snapshot(&config, "main")
            .join("PartialOnly.xml")
            .exists());

        let designer_calls = fs::read_to_string(designer_calls).expect("designer calls");
        let edt_calls = fs::read_to_string(edt_calls).expect("edt calls");
        assert_eq!(designer_calls.matches("/DumpConfigToFiles").count(), 2);
        assert!(designer_calls.contains("-partial"));
        assert_eq!(edt_calls.matches("-command import").count(), 1);
    }

    /// Снимок без файла версий: выгрузка по изменившемуся формата EDT сразу полная — снимок
    /// заменяется целиком, а не дополняется поверх.
    #[test]
    fn dump_incremental_edt_without_a_version_file_in_the_snapshot_is_full() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("edt").join("1cedtcli");
        let designer_calls = dir.path().join("designer-calls.log");
        let edt_calls = dir.path().join("edt-calls.log");
        create_edt_source_tree(&base);
        write_designer_dump_script_for_edt(&designer, &designer_calls, None);
        write_edt_import_script(&edt, &edt_calls);
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Incremental,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_native_edt_project(&base.join("main"));
        let designer_calls = fs::read_to_string(designer_calls).expect("designer calls");
        let dump_calls = designer_calls
            .lines()
            .filter(|line| line.contains("/DumpConfigToFiles"))
            .collect::<Vec<_>>();
        assert_eq!(dump_calls.len(), 1);
        assert!(!dump_calls[0].contains("-update"));
        assert_eq!(result.mode, DumpMode::Full);

        let edt_calls = fs::read_to_string(edt_calls).expect("edt calls");
        assert_eq!(edt_calls.matches("-command import").count(), 1);
    }

    #[test]
    fn dump_full_edt_extension_infers_base_project_name_from_configuration_source_set() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("edt").join("1cedtcli");
        let designer_calls = dir.path().join("designer-calls.log");
        let edt_calls = dir.path().join("edt-calls.log");
        create_edt_source_tree(&base);
        write_designer_dump_script_for_edt(&designer, &designer_calls, None);
        write_edt_import_script(&edt, &edt_calls);
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("ext".to_owned()),
                extension: Some("ext".to_owned()),
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert_native_edt_project(&base.join("ext"));
        let designer_calls = fs::read_to_string(designer_calls).expect("designer calls");
        let edt_calls = fs::read_to_string(edt_calls).expect("edt calls");
        assert!(designer_calls.contains("-Extension"));
        assert!(designer_calls.contains("ext"));
        assert!(edt_calls.contains("--base-project-name BaseProject"));
    }

    /// Проект EDT в репозитории, где у расширения есть работа вне учёта: любая выгрузка
    /// расширения заменяет его каталог, и сторож отказывает.
    fn edt_extension_with_uncommitted_work(dir: &Path) -> AppConfig {
        let base = dir.join("base");
        let work = dir.join("work");
        let designer = dir.join("1cv8");
        let edt = dir.join("edt").join("1cedtcli");
        create_edt_source_tree(&base);
        fs::write(base.join("ext").join("hand-written.xml"), "mine\n").expect("hand-written");
        write_designer_dump_script_for_edt(&designer, &dir.join("designer-calls.log"), None);
        write_edt_import_script(&edt, &dir.join("edt-calls.log"));
        build_edt_config(&base, &work, &designer, &edt, Default::default())
    }

    fn incremental_extension_dump(
        source_set: Option<&str>,
        extension: Option<&str>,
        force_way_out: ForceWayOut,
    ) -> DumpArgs {
        DumpArgs {
            dry_run: false,
            discard_uncommitted: false,
            force_way_out,
            mode: DumpModeRequest::Incremental,
            source_set: source_set.map(str::to_owned),
            extension: extension.map(str::to_owned),
            objects: vec![],
        }
    }

    /// `pull ext` и `pull --extension ext` в проекте EDT: совет — точная `pull ext --force`
    /// с набором разрешённой цели, а не голый `pull --force`, который без набора выгрузил
    /// бы основную конфигурацию в её каталог.
    #[test]
    fn an_edt_extension_refusal_does_not_offer_a_bare_pull_force() {
        let dir = tempdir().expect("tempdir");
        let config = edt_extension_with_uncommitted_work(dir.path());

        for args in [
            incremental_extension_dump(Some("ext"), None, ForceWayOut::PullForce),
            incremental_extension_dump(None, Some("ext"), ForceWayOut::PullForce),
        ] {
            let failure = run_dump(&config, &args).expect_err("refused");
            let message = failure.error.message();
            assert_eq!(
                failure.error.kind(),
                UseCaseErrorKind::Validation,
                "{message}"
            );
            assert!(message.contains("hand-written.xml"), "{message}");
            assert!(!message.contains("`pull --force`"), "{message}");
            assert!(
                message.contains("`v8-runner pull ext --force`"),
                "{message}"
            );
        }
    }

    /// MCP `dump_config` с расширением: ключа у MCP нет, и отказ называет точную команду
    /// строки для той же цели — с именем набора.
    #[test]
    fn an_mcp_extension_refusal_names_the_exact_pull_for_the_same_source_set() {
        let dir = tempdir().expect("tempdir");
        let config = edt_extension_with_uncommitted_work(dir.path());
        let context = ExecutionContext::mcp_stdio(crate::use_cases::context::CommandName::Dump);

        let failure = super::execute(
            &context,
            &config,
            &incremental_extension_dump(None, Some("ext"), ForceWayOut::PullForce),
        )
        .expect_err("refused");
        let message = failure.error.message();
        assert!(
            message.contains("run `v8-runner pull ext --force` from the command line"),
            "{message}"
        );
        assert!(!message.contains("`v8-runner pull --force`"), "{message}");
    }

    /// Вызывающий без ключа согласия (клонирование) не получает совета повторить с ключом,
    /// который у него ничего не делает.
    #[test]
    fn a_caller_without_a_force_way_out_is_not_told_to_force() {
        let dir = tempdir().expect("tempdir");
        let config = edt_extension_with_uncommitted_work(dir.path());

        let failure = run_dump(
            &config,
            &incremental_extension_dump(Some("ext"), None, ForceWayOut::Withheld),
        )
        .expect_err("refused");
        let message = failure.error.message();
        assert!(message.contains("commit or stash them"), "{message}");
        assert!(!message.contains("--force"), "{message}");
    }

    #[test]
    fn dump_full_edt_ibcmd_exports_to_designer_mirror_before_edt_import() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let ibcmd = dir.path().join("ibcmd");
        let edt = dir.path().join("edt").join("1cedtcli");
        let ibcmd_calls = dir.path().join("ibcmd-calls.log");
        let edt_calls = dir.path().join("edt-calls.log");
        create_edt_source_tree(&base);
        write_ibcmd_dump_script_for_edt(&ibcmd, &ibcmd_calls, None);
        write_edt_import_script(&edt, &edt_calls);
        let config = build_edt_config(
            &base,
            &work,
            &ibcmd,
            &edt,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

        let result = run_dump(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("dump");

        assert!(result.ok);
        assert!(base.join("main").join(".project").exists());
        assert!(designer_snapshot(&config, "main")
            .join("Configuration.xml")
            .exists());

        let ibcmd_calls = fs::read_to_string(ibcmd_calls).expect("ibcmd calls");
        let edt_calls = fs::read_to_string(edt_calls).expect("edt calls");
        assert!(ibcmd_calls.contains("--force"));
        assert!(ibcmd_calls.contains(
            designer_snapshot(&config, "main")
                .parent()
                .expect("snapshot parent")
                .display()
                .to_string()
                .as_str()
        ));
        assert!(edt_calls.contains(
            designer_snapshot(&config, "main")
                .display()
                .to_string()
                .as_str()
        ));
    }

    #[test]
    fn dump_incremental_edt_designer_stops_after_bootstrap_when_interruption_becomes_pending() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("1cedtcli");
        create_edt_source_tree(&base);
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Incremental,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved");
        let cancellation = CancellationToken::new();
        let dump_runner = TestProcessRunner::with_cancellation_on_call(1, cancellation.clone());
        let edt_runner = TestProcessRunner::default();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancellation);

        let error = super::run_incremental_dump_edt_designer(
            &context,
            &config,
            &resolved,
            &designer,
            &edt,
            &dump_runner,
            &edt_runner,
        )
        .expect_err("interrupted after bootstrap");

        assert_eq!(
            error.cancellation(),
            Some(crate::support::error::CancelledAt::Boundary),
            "a safe point is a cancellation at the boundary: {error}"
        );
        assert_eq!(dump_runner.call_count(), 1);
        assert_eq!(edt_runner.call_count(), 0);
    }

    #[test]
    fn dump_partial_edt_designer_stops_after_bootstrap_when_interruption_becomes_pending() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("1cedtcli");
        create_edt_source_tree(&base);
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalogs.Items".to_owned()],
            },
        )
        .expect("resolved");
        let cancellation = CancellationToken::new();
        let dump_runner = TestProcessRunner::with_cancellation_on_call(1, cancellation.clone());
        let edt_runner = TestProcessRunner::default();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancellation);
        let objects = [PartialDumpSelector::parse("Catalog.Items").expect("selector")];

        let error = super::run_partial_dump_edt_designer(
            &context,
            &config,
            &resolved,
            &designer,
            &edt,
            &dump_runner,
            &edt_runner,
            &objects,
        )
        .expect_err("interrupted after bootstrap");

        assert_eq!(
            error.cancellation(),
            Some(crate::support::error::CancelledAt::Boundary),
            "a safe point is a cancellation at the boundary: {error}"
        );
        assert_eq!(dump_runner.call_count(), 1);
        assert_eq!(edt_runner.call_count(), 0);
    }

    #[test]
    fn dump_incremental_edt_ibcmd_stops_after_bootstrap_when_interruption_becomes_pending() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let ibcmd = dir.path().join("ibcmd");
        let edt = dir.path().join("1cedtcli");
        create_edt_source_tree(&base);
        let config = build_edt_config(
            &base,
            &work,
            &ibcmd,
            &edt,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Incremental,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved");
        let cancellation = CancellationToken::new();
        let dump_runner = TestProcessRunner::with_cancellation_on_call(1, cancellation.clone());
        let edt_runner = TestProcessRunner::default();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancellation);

        let error = super::run_incremental_dump_edt_ibcmd(
            &context,
            &config,
            &resolved,
            &ibcmd,
            &edt,
            &dump_runner,
            &edt_runner,
        )
        .expect_err("interrupted after bootstrap");

        assert_eq!(
            error.cancellation(),
            Some(crate::support::error::CancelledAt::Boundary),
            "a safe point is a cancellation at the boundary: {error}"
        );
        assert_eq!(dump_runner.call_count(), 1);
        assert_eq!(edt_runner.call_count(), 0);
    }

    #[test]
    fn dump_partial_edt_ibcmd_stops_after_bootstrap_when_interruption_becomes_pending() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let ibcmd = dir.path().join("ibcmd");
        let edt = dir.path().join("1cedtcli");
        create_edt_source_tree(&base);
        let config = build_edt_config(
            &base,
            &work,
            &ibcmd,
            &edt,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec!["Catalogs.Items".to_owned()],
            },
        )
        .expect("resolved");
        let cancellation = CancellationToken::new();
        let dump_runner = TestProcessRunner::with_cancellation_on_call(1, cancellation.clone());
        let edt_runner = TestProcessRunner::default();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancellation);
        let objects = [PartialDumpSelector::parse("Catalog.Items").expect("selector")];

        let error = super::run_partial_dump_edt_ibcmd(
            &context,
            &config,
            &resolved,
            &ibcmd,
            &edt,
            &dump_runner,
            &edt_runner,
            &objects,
        )
        .expect_err("interrupted after bootstrap");

        assert_eq!(
            error.cancellation(),
            Some(crate::support::error::CancelledAt::Boundary),
            "a safe point is a cancellation at the boundary: {error}"
        );
        assert_eq!(dump_runner.call_count(), 1);
        assert_eq!(edt_runner.call_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn finalize_edt_dump_revalidates_publish_target_after_import() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("1cedtcli");
        let drift = dir.path().join("drift-target");
        create_edt_source_tree(&base);
        fs::create_dir_all(&drift).expect("drift target");
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved");
        let target_path = resolved.target_path.clone();
        let drift_path = drift.clone();
        let edt_runner = TestProcessRunner::with_on_run(move |request| {
            let project = request
                .args
                .windows(2)
                .find_map(|window| (window[0] == "--project").then(|| PathBuf::from(&window[1])))
                .expect("project arg");
            write_native_edt_project(
                &project,
                "ImportedProject",
                crate::support::edt_project::V8_CONFIGURATION_NATURE,
                None,
            );
            fs::remove_dir_all(&target_path).expect("remove target");
            symlink(&drift_path, &target_path).expect("retarget");
        });
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);

        let error = finalize_edt_dump(
            &context,
            &config,
            &resolved,
            &edt,
            &edt_runner,
            PlatformCommandResult {
                process: ProcessResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    interruption: None,
                },
                platform_log_path: None,
                platform_log: None,
                platform_log_read_error: None,
            },
            None,
        )
        .expect_err("expected publish target re-validation failure");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("target path changed during dump resolution"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
        assert_eq!(edt_runner.call_count(), 1);
        assert!(fs::symlink_metadata(resolved.target_path.as_path())
            .expect("target metadata")
            .file_type()
            .is_symlink());
        assert!(!drift.join(".project").exists());
        let leftover_stage_dirs = fs::read_dir(&base)
            .expect("base dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".dump-stage-"))
            .count();
        assert_eq!(leftover_stage_dirs, 0);
    }

    #[test]
    fn validate_edt_dump_staging_output_rejects_wrong_ordinary_kind() {
        let dir = tempdir().expect("tempdir");
        write_native_edt_project(
            dir.path(),
            "ImportedProject",
            crate::support::edt_project::V8_CONFIGURATION_NATURE,
            None,
        );

        let error =
            super::validate_edt_dump_staging_output(dir.path(), SourceSetPurpose::Extension, None)
                .expect_err("expected wrong ordinary kind");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("expected Extension"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn validate_edt_dump_staging_output_rejects_extension_without_base_project() {
        let dir = tempdir().expect("tempdir");
        write_native_edt_project(
            dir.path(),
            "ImportedExtension",
            crate::support::edt_project::V8_EXTENSION_NATURE,
            None,
        );

        let error = super::validate_edt_dump_staging_output(
            dir.path(),
            SourceSetPurpose::Extension,
            Some("BaseProject"),
        )
        .expect_err("expected missing Base-Project validation error");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("Base-Project"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn validate_edt_dump_staging_output_rejects_unexpected_extension_base_project() {
        let dir = tempdir().expect("tempdir");
        write_native_edt_project(
            dir.path(),
            "ImportedExtension",
            crate::support::edt_project::V8_EXTENSION_NATURE,
            Some("WrongBase"),
        );

        let error = super::validate_edt_dump_staging_output(
            dir.path(),
            SourceSetPurpose::Extension,
            Some("BaseProject"),
        )
        .expect_err("expected mismatched Base-Project validation error");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("BaseProject"));
                assert!(message.contains("WrongBase"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn finalize_edt_dump_cleans_staging_dir_when_edt_dsl_initialization_fails() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("edt").join("1cedtcli");
        create_edt_source_tree(&base);
        fs::create_dir_all(&work).expect("work");
        fs::write(work.join("edt-workspace"), "not a directory").expect("workspace file");
        let mut config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        config.tools.edt_cli.interactive_mode = true;
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved");
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);

        let error = finalize_edt_dump(
            &context,
            &config,
            &resolved,
            &edt,
            &TestProcessRunner::default(),
            PlatformCommandResult {
                process: ProcessResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    interruption: None,
                },
                platform_log_path: None,
                platform_log: None,
                platform_log_read_error: None,
            },
            None,
        )
        .expect_err("expected EDT DSL initialization failure");

        assert!(matches!(error, AppError::PlatformEdt(_)));
        let leftover_stage_dirs = fs::read_dir(&base)
            .expect("base dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".dump-stage-"))
            .count();
        assert_eq!(leftover_stage_dirs, 0);
    }

    #[test]
    fn finalize_edt_dump_stops_before_import_when_interruption_is_already_pending() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let designer = dir.path().join("1cv8");
        let edt = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        create_edt_source_tree(&base);
        write_edt_import_script(&edt, &edt_calls);
        write_script(&designer, "exit 0");
        let config = build_edt_config(&base, &work, &designer, &edt, Default::default());
        fs::create_dir_all(designer_snapshot(&config, "main")).expect("designer snapshot");
        fs::write(
            designer_snapshot(&config, "main").join("Configuration.xml"),
            "<Configuration />\n",
        )
        .expect("configuration xml");
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancellation);

        let error = finalize_edt_dump(
            &context,
            &config,
            &resolved,
            &edt,
            &crate::platform::process::ProcessExecutor,
            PlatformCommandResult {
                process: ProcessResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    interruption: None,
                },
                platform_log_path: None,
                platform_log: None,
                platform_log_read_error: None,
            },
            None,
        )
        .expect_err("interrupted");

        assert_eq!(
            error.cancellation(),
            Some(crate::support::error::CancelledAt::Boundary),
            "a safe point is a cancellation at the boundary: {error}"
        );
        assert!(
            !edt_calls.exists()
                || fs::read_to_string(edt_calls)
                    .expect("edt calls")
                    .trim()
                    .is_empty()
        );
        assert!(!base.join("main").join("stale.txt").exists());
    }

    #[test]
    fn advisory_lock_serializes_access() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("test.lock");
        let guard = acquire_advisory_lock(&lock_path).expect("lock");
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let lock_path_clone = lock_path.clone();

        let handle = thread::spawn(move || {
            started_tx.send(()).expect("send started");
            let _guard = acquire_advisory_lock(&lock_path_clone).expect("second lock");
            done_tx.send(()).expect("send done");
        });

        started_rx.recv().expect("started");
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(guard);
        done_rx.recv_timeout(Duration::from_secs(1)).expect("done");
        handle.join().expect("join");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_target_uses_same_lock_path_for_canonical_and_symlinked_base_path() {
        let dir = tempdir().expect("tempdir");
        let real_base = dir.path().join("real-base");
        let base_link = dir.path().join("base-link");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        create_source_tree(&real_base);
        fs::create_dir_all(&work).expect("work");
        write_script(&script, "exit 0");
        symlink(&real_base, &base_link).expect("symlink");

        let config_real = build_config(&real_base, &work, &script);
        let config_link = build_config(&base_link, &work, &script);

        let resolved_real = resolve_target(
            &config_real,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved real");
        let resolved_link = resolve_target(
            &config_link,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Full,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: vec![],
            },
        )
        .expect("resolved link");

        assert_eq!(resolved_real.lock_path, resolved_link.lock_path);
    }

    #[cfg(unix)]
    #[test]
    fn lock_identity_is_based_on_canonical_target() {
        let dir = tempdir().expect("tempdir");
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        fs::create_dir_all(&real).expect("real");
        symlink(&real, &link).expect("symlink");

        let hash_real = stable_path_identity(&std::fs::canonicalize(&real).expect("canonical"));
        let hash_link = stable_path_identity(&std::fs::canonicalize(&link).expect("canonical"));

        assert_eq!(hash_real, hash_link);
    }

    #[test]
    fn dump_result_json_contains_new_fields() {
        let result = crate::domain::dump::DumpResult {
            provider: None,
            provider_dispatched: true,
            up_to_date: false,
            ok: true,
            source_set: Some("main".to_owned()),
            extension: Some("ext".to_owned()),
            selectors: None,
            mode: DumpMode::Incremental,
            target_path: PathBuf::from("/tmp/main"),
            platform_log_path: Some(PathBuf::from("/tmp/platform.log")),
            duration_ms: 5,
            message: Some("ok".to_owned()),
            losses: Vec::new(),
        };

        let json = serde_json::to_value(result).expect("json");

        assert_eq!(DUMP_COMMAND, "pull");
        assert_eq!(json["source_set"], "main");
        assert_eq!(json["extension"], "ext");
        assert_eq!(json["platform_log_path"], "/tmp/platform.log");
    }

    #[test]
    fn build_designer_dsl_requests_platform_log() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(dir.path());
        write_dump_script(&script, &calls, None, 0);
        let config = build_config(dir.path(), &dir.path().join("work"), &script);
        let runner = crate::platform::process::ProcessExecutor;
        let context = crate::use_cases::context::ExecutionContext::cli(
            crate::use_cases::context::CommandName::Dump,
        );
        let dsl = build_designer_dsl(&context, &config, &script, &runner, "main", "incremental")
            .expect("dsl");

        let result = dsl
            .dump_config_to_files(dir.path().join("out").as_path(), None)
            .expect("dump");

        assert!(result.platform_log_path.is_some());
        assert!(result
            .platform_log
            .as_deref()
            .unwrap_or_default()
            .contains("designer log"));
    }

    #[test]
    fn parse_external_dump_descriptor_decodes_escaped_name() {
        let xml = "<ExternalDataProcessor><Properties><Name>Foo &amp; Bar</Name></Properties></ExternalDataProcessor>";
        let parsed =
            parse_external_dump_descriptor(xml, Path::new("/tmp/dump.xml")).expect("parse");

        assert_eq!(
            parsed.purpose.external_root_tag(),
            Some("ExternalDataProcessor")
        );
        assert_eq!(parsed.logical_name, "Foo & Bar");
    }

    #[test]
    fn parse_external_dump_descriptor_accepts_metadataobject_wrapper() {
        let xml = "<MetaDataObject><ExternalDataProcessor><Properties><Name>Foo</Name></Properties></ExternalDataProcessor></MetaDataObject>";
        let parsed =
            parse_external_dump_descriptor(xml, Path::new("/tmp/dump.xml")).expect("parse");

        assert_eq!(
            parsed.purpose.external_root_tag(),
            Some("ExternalDataProcessor")
        );
        assert_eq!(parsed.logical_name, "Foo");
    }

    #[test]
    fn run_external_dump_designer_rejects_missing_descriptor() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        let work = dir.path().join("work");
        let root_xml = dir.path().join("out").join("root.xml");
        create_source_tree(dir.path());
        write_dump_script(&script, &calls, None, 0);
        let config = build_config(dir.path(), &work, &script);
        let runner = crate::platform::process::ProcessExecutor;
        let context = crate::use_cases::context::ExecutionContext::cli(
            crate::use_cases::context::CommandName::Dump,
        );
        let dsl = build_designer_dsl(&context, &config, &script, &runner, "main", "incremental")
            .expect("dsl");

        let error = run_external_dump_designer(
            &dsl,
            &script,
            &root_xml,
            ExternalArtifactKind::DataProcessor,
            "Foo",
        )
        .expect_err("missing descriptor");

        assert!(matches!(error.0, AppError::Validation(_)));
    }
}
