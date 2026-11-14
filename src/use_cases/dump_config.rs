use crate::change_detection::analyzer::{
    list_after_dump, list_before_dump, mark_dump_start, observe_dump_writes, watch_before_dump,
    ChangeDetectionError, DumpFact, DumpStart, DumpWatch, DumpWrites, TreeBeforeDump,
};
use crate::change_detection::runner_dirs::{runner_dirs_inside, RunnerDirs};
use crate::domain::progress::ProgressPhase;
use crate::domain::source_set::SourceSetContext;
use crate::use_cases::provider_selection::stamp_session;
use crate::use_cases::source_changes::DumpHashing;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::model::{AppConfig, SourceFormat, SourceSetPurpose};
use crate::domain::dump::{
    DumpMode, DumpModeReason, DumpResult, DumpSelectorResult, ReportedDumpMode,
};
use crate::domain::partial_dump_selector::PartialDumpSelector;
#[cfg(test)]
use crate::domain::partial_dump_selector::{
    PARTIAL_OBJECT_BLANK_ERROR, PARTIAL_OBJECT_CONTROL_ERROR,
};
use crate::platform::designer::DesignerDsl;
use crate::platform::dump_forecast::DumpForecast;
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
use crate::use_cases::destruction_guard::{
    guard_replacement, Destruction, DestructionConsent, Losses, WaysOut,
};
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
mod landing;

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
#[cfg(test)]
use super::staged_publication::cleanup_staging_path;
use super::staged_publication::{interruption_before_publish, BackupRemoval, StagedPublication};
use super::version_file::VERSION_FILE_NAME;
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
/// Приставка промежуточного каталога выгрузки под `workPath/stage`.
const DUMP_STAGE_PREFIX: &str = ".dump-stage";

/// Корень промежуточных каталогов выгрузки: `workPath/stage`.
fn dump_stage_root(config: &AppConfig) -> PathBuf {
    crate::support::path::absolute_from_current_dir(&config.work_path)
        .unwrap_or_else(|_| config.work_path.clone())
        .join("stage")
}

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
    let outcome = if context.external_full_pull().is_some()
        && (config.format != SourceFormat::Designer
            || !matches!(args.mode, DumpModeRequest::Full)
            || !args.discard_uncommitted
            || args.source_set.is_none()
            || !args.objects.is_empty())
    {
        Err(UseCaseFailure::without_payload(AppError::Validation("external publication requires one explicit full DESIGNER source set with replacement consent".into())))
    } else {
        coordinator::run_dump_with_context(context, config, args)
    };
    stamp_dispatch(stamp_session(outcome, context), context.work())
}

type DumpExecutionFailure = UseCaseFailure<DumpResult>;

/// Выгрузка — и полная, и по изменившемуся — отказывает, если правило игнорирования git
/// прячет файл набора, который она пишет: изменения набора берутся у git, и спрятанный файл
/// следующая отправка не увидела бы (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`).
pub(super) fn refuse_hidden_sets(config: &AppConfig, args: &DumpArgs) -> Result<(), AppError> {
    if config.format != SourceFormat::Designer {
        return Ok(());
    }
    let inventory = SourceSetInventory::new(config);
    let contexts: Vec<SourceSetContext> = inventory
        .designer_contexts()
        .iter()
        .filter(|set| {
            args.source_set
                .as_deref()
                .is_none_or(|name| set.name() == name)
        })
        .cloned()
        .collect();
    crate::use_cases::source_changes::refuse_hidden_sets(&contexts, &config.work_path, args.dry_run)
}

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
    /// Где лежат промежуточные каталоги выгрузки: `workPath/stage`
    /// (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
    stage_root: PathBuf,
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

/// Отметка начала выгрузки по изменившемуся: начало по часам файловой системы каталога
/// выгрузки или ошибка зонда — тогда режим по факту не узнать
/// (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
type StartMark = std::io::Result<DumpStart>;

/// Отмечает начало выгрузки по изменившемуся от годного файла версий в каталоге, куда она
/// ляжет (у EDT — в зеркале Конфигуратора). Зонд убран до запуска платформы. `None` —
/// отмечать нечего: выгрузка не по файлу версий, выборка или внешний полный pull.
fn mark_start(
    context: &ExecutionContext,
    resolved: &ResolvedDumpTarget,
    plan: &DumpPlan,
    objects: Option<&[PartialDumpSelector]>,
) -> Option<StartMark> {
    if !by_version_file(context, plan, objects) {
        return None;
    }
    let mark = mark_dump_start(&resolved.platform_target_path);
    if let Err(error) = &mark {
        debug!(
            target = %resolved.platform_target_path.display(),
            %error,
            "the dump start was not marked: the dump mode is unknown and nothing is remembered"
        );
    }
    Some(mark)
}

/// Выгрузка по изменившемуся от файла версий, всего набора и без внешней публикации: только
/// у неё отмечается начало ([`mark_start`]), и только её поколение может пропустить, поэтому
/// сторожа она спрашивает после поколения (`coordinator::generation_then_guard`, #683).
fn by_version_file(
    context: &ExecutionContext,
    plan: &DumpPlan,
    objects: Option<&[PartialDumpSelector]>,
) -> bool {
    matches!(plan, DumpPlan::OverDirectory(OverDirectory::ByVersionFile))
        && objects.is_none()
        && context.external_full_pull().is_none()
}

/// Отметит ли [`mark_start`] начало этой выгрузки, а [`dump_watch`] — дерево до неё: только
/// тогда перечень до выгрузки нужен.
fn watches_the_dump(
    context: &ExecutionContext,
    config: &AppConfig,
    plan: &DumpPlan,
    objects: Option<&[PartialDumpSelector]>,
) -> bool {
    config.format == SourceFormat::Designer && by_version_file(context, plan, objects)
}

/// Начало, если отмечено.
fn marked_start(mark: Option<&StartMark>) -> Option<DumpStart> {
    mark.and_then(|mark| mark.as_ref().ok()).copied()
}

/// Выгрузка по изменившемуся пишет прямо в каталог набора: что в нём было, отмечается перед
/// запуском платформы, чтобы после выгрузки память узнала записанное (#557).
fn dump_watch(
    config: &AppConfig,
    set: Option<&SourceSetContext>,
    start: Option<DumpStart>,
) -> Option<DumpWatch> {
    if config.format != SourceFormat::Designer {
        return None;
    }
    watch_before_dump(set?, &config.work_path, start?)
}

/// Шаг раннера `side` идёт в своём потоке, пока команда делает `step` с платформой (#571, #574).
/// Поток не поднялся — шаг раннера идёт после: медленнее, но с тем же ответом. Паника потока
/// доходит до вызвавшего.
fn beside<T, U>(name: &str, side: impl FnOnce() -> U + Send, step: impl FnOnce() -> T) -> (T, U)
where
    U: Send,
{
    let side = std::sync::Mutex::new(Some(side));
    let run_side = || {
        side.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map(|side| side())
    };
    std::thread::scope(|scope| {
        let span = tracing::Span::current();
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn_scoped(scope, move || span.in_scope(run_side));
        let answer = step();
        let beside = match thread {
            Ok(thread) => thread
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            Err(_) => run_side(),
        };
        (answer, beside.expect("the runner step runs exactly once"))
    })
}

/// Перечень дерева до выгрузки по изменившемуся идёт, пока платформа отвечает на вопрос перед
/// ней (`platform_step`), — уже после сторожа и всех отказов до платформы (#574). Он нужен
/// только набору формата Конфигуратора (`wanted`), и только если выгрузка пойдёт: ответ, по
/// которому `goes_on` говорит «не пойдёт», останавливает перечень, и ответ не ждёт его конца.
/// Отмена (`interrupted`) тоже его останавливает; без перечня выгрузка ничего не запоминает.
fn list_before_dump_while<T>(
    set: Option<&SourceSetContext>,
    work_path: &Path,
    wanted: bool,
    interrupted: &(dyn Fn() -> bool + Sync),
    platform_step: impl FnOnce() -> T,
    goes_on: impl FnOnce(&T) -> bool,
) -> (T, Option<TreeBeforeDump>) {
    let Some(set) = set.filter(|_| wanted) else {
        return (platform_step(), None);
    };
    let dropped = std::sync::atomic::AtomicBool::new(false);
    let stopped = || dropped.load(std::sync::atomic::Ordering::Relaxed) || interrupted();
    let (answer, tree) = beside(
        "dump-lister",
        || list_before_dump(set, work_path, &stopped),
        || {
            let answer = platform_step();
            if !goes_on(&answer) {
                dropped.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            answer
        },
    );
    let goes = !dropped.load(std::sync::atomic::Ordering::Relaxed);
    (answer, tree.filter(|_| goes))
}

/// Что выгрузка оставила в каталоге: режим по факту и записанное для памяти.
struct DumpObservation {
    /// `None` — режим по факту не спрашивался или перечень прервала отмена: режим плана.
    fact: Option<DumpFact>,
    /// `None` — записанное не запоминается.
    writes: Option<Result<DumpWrites, ChangeDetectionError>>,
}

/// Режим по факту — у выгрузки с отметкой `fact_mark` — и то, что выгрузка по изменившемуся
/// записала в каталог, найденное и прохешированное, пока платформа отвечала про поколение
/// после выгрузки (#571). Каталог перечисляется один раз, обход читает только файлы и память,
/// платформу не трогает. `hashing` — индекс git, прочитанный, пока шла выгрузка (#574); нет
/// его — индекс читается рядом с перечнем.
fn observe_dump_writes_while<T>(
    set: Option<&SourceSetContext>,
    work_path: &Path,
    fact_mark: Option<&StartMark>,
    watch: Option<&DumpWatch>,
    hashing: Option<DumpHashing>,
    interrupted: &(dyn Fn() -> bool + Sync),
    platform_step: impl FnOnce() -> T,
) -> (T, DumpObservation) {
    let Some(set) = set.filter(|_| fact_mark.is_some() || watch.is_some()) else {
        return (
            platform_step(),
            DumpObservation {
                fact: None,
                writes: None,
            },
        );
    };
    beside(
        "dump-observer",
        move || observe_dump(set, work_path, fact_mark, watch, hashing, interrupted),
        platform_step,
    )
}

/// Перечень каталога после выгрузки отвечает и о режиме, и о записанном: второго обхода нет.
fn observe_dump(
    set: &SourceSetContext,
    work_path: &Path,
    fact_mark: Option<&StartMark>,
    watch: Option<&DumpWatch>,
    hashing: Option<DumpHashing>,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> DumpObservation {
    // Конец выгрузки по часам файловой системы набора, тем же, что отметили её начало: файл,
    // изменённый позже, правили уже после платформы (#170). Не отмечен — граница не ставится.
    let ended = watch.and_then(|_| {
        mark_dump_start(set.path())
            .map_err(|error| {
                tracing::debug!(
                    source_set = %set.name(),
                    %error,
                    "the dump end was not marked; edits after it are not told from its writes"
                );
            })
            .ok()
    });
    let list = || list_after_dump(set, work_path, interrupted);
    let (listed, hashing) = match (watch, hashing) {
        (Some(_), None) => {
            let (hashing, listed) = beside("dump-lister", list, || DumpHashing::of(set));
            (listed, Some(hashing))
        }
        (_, hashing) => (list(), hashing),
    };
    let fact = fact_mark.and_then(|mark| match (mark, &listed) {
        (Ok(start), Ok(listing)) => Some(listing.fact(*start)),
        // Отмена во время перечня оставляет режим плана.
        (Ok(_), Err(_)) if interrupted() => None,
        (Err(_), _) | (Ok(_), Err(_)) => Some(DumpFact::Unlisted),
    });
    let writes = watch.zip(hashing).map(|(watch, hashing)| {
        listed.and_then(|listing| {
            hashing.with(|blobs| {
                observe_dump_writes(set, work_path, watch, ended, listing, blobs, interrupted)
            })
        })
    });
    tracing::debug!(source_set = %set.name(), "the dump writes were observed");
    DumpObservation { fact, writes }
}

/// Выгрузка по изменившемуся над базой, которую во время неё не правили, записывает в память
/// хеши того, что легло в каталог (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`). Базу правили
/// или поколение неизвестно — память не трогается, и следующая отправка грузит изменённое.
fn remember_dump_writes(
    set: &SourceSetContext,
    work_path: &Path,
    observed: Option<Result<DumpWrites, ChangeDetectionError>>,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<String> {
    let observed = observed?;
    if before.is_none() || before != after {
        return None;
    }
    observed
        .and_then(|writes| writes.commit(set, work_path))
        .err()
        .map(|error| {
            format!(
                "the dump of source-set '{}' is in place, but hash memory was not updated: {error}; the next push loads what the dump wrote",
                set.name()
            )
        })
}

/// Что выгрузка сообщает сверх ответа платформы.
#[derive(Debug, Default)]
struct DumpNotes {
    /// Предупреждения и оговорки для ответа.
    message: Option<String>,
    /// Что публикация уничтожила по согласию.
    discarded: Losses,
    /// Что сказало о случившемся режиме; `None` — ничего: режим плана.
    evidence: Option<ModeEvidence>,
}

/// Что говорит о случившемся режиме выгрузки по изменившемуся
/// (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModeEvidence {
    /// Прогноз `ibcmd` перед выгрузкой.
    Forecast(DumpForecast),
    /// Что выгрузка Конфигуратора или агента с `-update` сделала с каталогом.
    Fact(DumpFact),
    /// Выборка `ibcmd` без годной копии описи раннера прошла полной выгрузкой.
    WithoutCopy,
}

impl DumpNotes {
    fn message(message: Option<String>) -> Self {
        Self {
            message,
            ..Self::default()
        }
    }

    /// Ставит более раннюю оговорку перед своими.
    fn after(self, earlier: Option<String>) -> Self {
        Self {
            message: merge_optional_messages(earlier, self.message),
            ..self
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

fn prepare_full_dump(
    context: &ExecutionContext,
    resolved: &ResolvedDumpTarget,
) -> Result<StagedPublication, AppError> {
    if let Some(external) = context.external_full_pull() {
        StagedPublication::externally_owned_dir(
            external.stage(),
            &resolved.platform_target_path,
            &resolved.platform_target_identity,
        )
    } else {
        StagedPublication::prepare_dir_in(
            &resolved.stage_root,
            &resolved.platform_target_path,
            &resolved.platform_target_identity,
            DUMP_STAGE_PREFIX,
        )
    }
}

/// Seal the platform output before publication; only that tree may become the baseline.
/// No storage is opened until publication succeeds, including its Git and cancellation guards.
fn publish_full_dump(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    publication: &StagedPublication,
) -> Result<DumpNotes, AppError> {
    use crate::change_detection::analyzer::commit_full_snapshot;

    validate_platform_target(resolved).map_err(|error| publication.cleanup_failure(error))?;
    validate_full_dump_work_path(config, resolved)
        .map_err(|error| publication.cleanup_failure(error))?;
    // Hashing a large staging tree takes seconds; honour a cancellation that came first.
    if let Some(error) = interruption_before_publish(context, "dump publication") {
        return Err(publication.cleanup_failure(error));
    }
    let external_binding = context
        .external_full_pull()
        .map(|_| {
            super::full_pull_publication::PreparedPublicationBinding::capture(
                publication.staging_path(),
                &resolved.target_path,
            )
        })
        .transpose()?;

    let snapshot = if config.format == SourceFormat::Designer {
        SourceSetInventory::new(config)
            .designer_context(&resolved.source_set_name)
            .filter(|source| source.persists_snapshot())
            .map(|source| {
                crate::use_cases::source_changes::prepare_full_snapshot(
                    source,
                    publication.staging_path(),
                )
            })
    } else {
        None
    };
    // Hashing may take seconds: re-check right before replacing, not only before hashing.
    validate_platform_target(resolved).map_err(|error| publication.cleanup_failure(error))?;
    validate_full_dump_work_path(config, resolved)
        .map_err(|error| publication.cleanup_failure(error))?;

    // Опись платформы в каталог набора не попадает (#646): она уходит из стадии до публикации
    // и становится копией раннера только после удачной публикации. `ConfigDumpInfo.xml`,
    // оставшийся в каталоге человека от прежних версий раннера, переходит в новое дерево как
    // был: раннер его не трогает.
    let version_snapshot = super::version_file::RunnerVersionFile::prepare_from(
        publication.staging_path(),
        &config.work_path,
    )
    .and_then(|prepared| {
        carry_the_left_version_file_over(resolved, publication.staging_path()).map(|()| prepared)
    })
    .map_err(|error| publication.cleanup_failure(error))?;
    if let Some(external) = context.external_full_pull() {
        let binding = external_binding.expect("external publication captured its binding");
        binding.validate_stage(publication.staging_path())?;
        let published_source = crate::change_detection::source_sets::SourceSetsService::new(config)
            .designer_context_for_published_root(
                &resolved.source_set_name,
                binding.canonical_target(),
            );
        let snapshot_prepared = snapshot.as_ref().is_some_and(|prepared| prepared.is_ok());
        let snapshot_warning = snapshot.as_ref().and_then(|prepared| prepared.as_ref().err()).map(|error| format!("hash memory preparation failed: {error}; published sources will need a repeated full pull"));
        // Каталог заменяет вызывающий, но уничтожает замена то же, что и своя: сторож
        // спрашивает до кадра `ready`, пока каталог не тронут, и ответ называет
        // уничтоженное так же (`INV.USE-CASES.FORCE-NAMES-WHAT-IT-DESTROYED`).
        let discarded = guard_replacement(
            context,
            &resolved.platform_target_path,
            resolved.platform_consent(),
            &[VERSION_FILE_NAME],
            Destruction::Replace,
        )?;
        let mut ready = super::full_pull_publication::FullPullReady {
            command: context.command(),
            requested_mode: DumpMode::Full,
            mode: ReportedDumpMode::Full,
            extension: resolved.extension.as_deref(),
            mode_reason: None,
            provider_dispatched: false,
            source_set: &resolved.source_set_name,
            target: &resolved.target_path,
            stage: publication.staging_path(),
            snapshot_prepared,
            snapshot_warning: snapshot_warning.as_deref(),
        };
        super::result::stamp_prepared_dispatch(&mut ready, context.work());
        external.publish(&ready)?;
        // Published is irreversible. Every following action is local and uses
        // prepared stage bytes; pending cancellation cannot abandon this finish.
        if let Err(error) = binding.validate_published(&resolved.target_path) {
            external.memory_finished(super::full_pull_publication::MemoryCommit::NotSaved);
            return Ok(DumpNotes {
                discarded,
                ..DumpNotes::message(merge_optional_messages(
                    Some(format!("publication was acknowledged but baseline was not updated: {error}; repeat a full pull")),
                    super::interruption::deferred_interruption_warning_after(context, "dump publication completed"),
                ))
            });
        }
        let published_source = published_source.as_ref();
        let mut warnings = Vec::new();
        if let Some(prepared) = snapshot {
            let result = prepared.and_then(|prepared| {
                if let Some(source) = published_source {
                    crate::use_cases::source_changes::full_pull_memory(
                        source,
                        &config.work_path,
                        prepared,
                    )
                    .and_then(|memory| commit_full_snapshot(source, &config.work_path, &memory))
                } else {
                    Ok(())
                }
            });
            if let Err(error) = result {
                warnings.push(format!("sources published but hash memory was not updated: {error}; repeat a full pull"));
            }
        }
        if let Some(source) = published_source {
            if let Some(warning) = external.record_generation(config, source) {
                warnings.push(warning);
            }
        }
        let published_version =
            published_source.and_then(super::version_file::RunnerVersionFile::of);
        if let Some(version) = published_version {
            match version.record_prepared(version_snapshot) {
                Ok(()) => {}
                Err(error) => warnings.push(format!(
                    "sources published but the runner version copy was not updated: {error}"
                )),
            }
        }
        external.memory_finished(if snapshot_prepared && warnings.is_empty() {
            super::full_pull_publication::MemoryCommit::Saved
        } else {
            super::full_pull_publication::MemoryCommit::NotSaved
        });
        return Ok(DumpNotes {
            discarded,
            ..DumpNotes::message(merge_optional_messages(
                (!warnings.is_empty()).then(|| warnings.join("; ")),
                super::interruption::deferred_interruption_warning_after(
                    context,
                    "dump publication completed",
                ),
            ))
        });
    }

    let published = publication
        .publish_dir(
            context,
            DUMP_BACKUP_PREFIX,
            "failed to publish staged dump",
            resolved.platform_consent(),
            // Прежний `ConfigDumpInfo.xml` в корне переходит в новое дерево: это не потеря.
            &[VERSION_FILE_NAME],
            // Прежнее дерево полной выгрузки ответу не нужно (#616).
            BackupRemoval::Background,
        )
        .map_err(|error| publication.cleanup_failure(error))?;
    debug!(target = %resolved.platform_target_path.display(), "published staged dump");
    let copy_warning = SourceSetInventory::new(config)
        .designer_context(&resolved.source_set_name)
        .and_then(super::version_file::RunnerVersionFile::of)
        .and_then(|version| version.record_prepared(version_snapshot).err())
        .map(|error| {
            format!("sources published but the runner version copy was not updated: {error}")
        });

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
            .and_then(|snapshot| {
                crate::use_cases::source_changes::full_pull_memory(
                    source,
                    &config.work_path,
                    snapshot,
                )
            })
            .and_then(|snapshot| commit_full_snapshot(source, &config.work_path, &snapshot))
            .err()
            .map(|error| format!(
                "sources published for '{}', but hash memory was not updated: {error}; repeat a full pull to refresh memory",
                resolved.source_set_name
            ))
    });
    Ok(DumpNotes {
        message: merge_optional_messages(
            merge_optional_messages(
                merge_optional_messages(published.cleanup_warning, memory_warning),
                copy_warning,
            ),
            dump_publication_warning(context.command(), published.deferred_interruption),
        ),
        discarded: published.discarded,
        evidence: None,
    })
}

/// `ConfigDumpInfo.xml` в корне каталога человека, оставшийся от прежних версий раннера,
/// переходит в стадию полной выгрузки байт в байт: замена каталога его не теряет. Снимок EDT —
/// каталог раннера, и его прежний файл не переносится.
fn carry_the_left_version_file_over(
    resolved: &ResolvedDumpTarget,
    stage: &Path,
) -> Result<(), AppError> {
    if resolved.platform_target_path != resolved.target_path {
        return Ok(());
    }
    let left = resolved.platform_target_path.join(VERSION_FILE_NAME);
    match std::fs::copy(&left, stage.join(VERSION_FILE_NAME)) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Runtime(format!(
            "failed to keep '{}' in the replaced directory: {error}",
            left.display()
        ))),
    }
}

fn validate_full_dump_work_path(
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
) -> Result<(), AppError> {
    // Полная выгрузка заменяет каталог целиком: каталоги раннера в нём — замок и память
    // команды, стенды и база — пропали бы вместе с ним
    // (`INV.CHANGE-DETECTION.RUNNER-DIRECTORIES-ARE-NOT-SOURCES`).
    let dirs = RunnerDirs::of(config);
    let is_a_runner_dir = |target: &Path| {
        [&config.work_path, &config.local_path]
            .into_iter()
            .any(|dir| nearest_existing_canonical_path(dir).is_ok_and(|dir| dir == target))
    };
    for target in [
        resolved.canonical_target_path.as_path(),
        resolved.canonical_platform_target_path.as_path(),
    ] {
        if is_a_runner_dir(target) || !runner_dirs_inside(target, dirs).is_empty() {
            return Err(AppError::Validation(
                "full pull target must not contain workPath or localPath".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Как выгрузка ложится прямо в каталог набора.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OverDirectory {
    /// По изменившемуся от копии описи раннера.
    ByVersionFile,
    /// Целиком поверх каталога: годной копии описи нет.
    Whole(WholeReason),
}

/// Почему выгрузка по изменившемуся не может опереться на копию описи раннера.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WholeReason {
    /// Годной копии описи у раннера нет.
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
    /// Причина для ответа: что не так с копией описи раннера для каталога `dir`.
    fn describe(&self, dir: &Path) -> String {
        match self {
            Self::Missing => format!(
                "no version file {VERSION_FILE_NAME} for '{}' in the runner's memory",
                dir.display()
            ),
            Self::Unrecognized => format!(
                "the format version of the runner's {VERSION_FILE_NAME} for '{}' is not recognized",
                dir.display()
            ),
            Self::Foreign {
                found,
                platform,
                written,
            } => format!(
                "the runner's {VERSION_FILE_NAME} for '{}' is in format {found}, and platform {platform} writes {written}",
                dir.display()
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

    /// Случившийся режим для ответа и почему он не тот, что просили. Прогноз есть только у
    /// выгрузки `ibcmd` по изменившемуся (и у её выборки, которая идёт ею): «полная» — полная
    /// по прогнозу, нераспознанный — «неизвестен», изменившееся — по изменившемуся. Факт есть
    /// у выгрузки Конфигуратора и агента с `-update`: переписан каждый файл — полная, не
    /// перечислен каталог — «неизвестен», иначе — по изменившемуся. Ни того ни другого —
    /// режим плана.
    pub(super) fn reported(
        &self,
        evidence: Option<ModeEvidence>,
    ) -> (ReportedDumpMode, Option<DumpModeReason>) {
        match (self, evidence) {
            (Self::OverDirectory(OverDirectory::Whole(reason)), _) => (
                ReportedDumpMode::Full,
                Some(match reason {
                    WholeReason::Missing | WholeReason::Unrecognized => DumpModeReason::VersionFile,
                    WholeReason::Foreign { .. } => DumpModeReason::ForeignFormat,
                }),
            ),
            (_, Some(ModeEvidence::WithoutCopy)) => {
                (ReportedDumpMode::Full, Some(DumpModeReason::VersionFile))
            }
            (_, Some(ModeEvidence::Forecast(DumpForecast::Full))) => (
                ReportedDumpMode::Full,
                Some(DumpModeReason::PlatformForecast),
            ),
            (_, Some(ModeEvidence::Fact(DumpFact::RewroteAll))) => (
                ReportedDumpMode::Full,
                Some(DumpModeReason::PlatformRewroteAll),
            ),
            (
                _,
                Some(
                    ModeEvidence::Forecast(DumpForecast::Unknown)
                    | ModeEvidence::Fact(DumpFact::Unlisted),
                ),
            ) => (ReportedDumpMode::Unknown, Some(DumpModeReason::Unknown)),
            (
                _,
                Some(
                    ModeEvidence::Forecast(DumpForecast::Changes)
                    | ModeEvidence::Fact(DumpFact::Partly),
                ),
            ) => (ReportedDumpMode::Incremental, None),
            (plan, None) => (plan.mode().into(), None),
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

/// Выгрузка Конфигуратором поверх каталога набора через стадию
/// ([`landing::dump_over_directory`]): по копии описи раннера — только изменённое
/// (`-update -configDumpInfoForChanges`), без годной копии — полная. В каталог ложится всё,
/// кроме описи; удаления раннер переносит по двум описям.
fn run_dump_over_directory_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    how: &OverDirectory,
    version: Option<&super::version_file::RunnerVersionFile>,
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        how = ?how,
        "running dump over the directory through a stage"
    );
    ensure_dir(&resolved.platform_target_path)
        .map_err(|error| AppError::Runtime(format!("failed to create target dir: {error}")))?;

    let (stage, label) = match how {
        OverDirectory::ByVersionFile => ("dump: incremental", "incremental"),
        OverDirectory::Whole(_) => ("dump: full", "full"),
    };
    let dsl = build_designer_dsl(
        context,
        config,
        binary,
        runner,
        &resolved.source_set_name,
        label,
    )?;
    let (dump_result, message) =
        landing::dump_over_directory(context, config, resolved, how, version, |into| {
            log_live_stage(
                ProgressPhase::Dump,
                Some(resolved.source_set_name.as_str()),
                stage,
                "[Конфигуратор] exporting configuration files",
            );
            // Прогноза нет: режим, который случился, раннер узнаёт по факту после выгрузки.
            let dump_result = match into.description {
                Some(description) => dsl.dump_config_to_files_for_changes(
                    into.output,
                    description,
                    resolved.extension.as_deref(),
                ),
                None => dsl.dump_config_to_files(into.output, resolved.extension.as_deref()),
            }
            .map_err(AppError::from)?;
            ensure_platform_success("dump", resolved, &dump_result)?;
            Ok(dump_result)
        })?;
    Ok((dump_result, DumpNotes::message(message)))
}

/// Прогноз `ibcmd` перед выгрузкой по изменившемуся: `config export status` по копии описи
/// раннера `description`. Отказ и ответ вне словаря — «неизвестен».
fn forecast_ibcmd(
    context: &ExecutionContext,
    resolved: &ResolvedDumpTarget,
    dsl: &crate::platform::ibcmd::IbcmdDsl<'_>,
    description: &Path,
) -> Result<DumpForecast, AppError> {
    log_live_stage(
        ProgressPhase::Forecast,
        Some(resolved.source_set_name.as_str()),
        "dump: forecast",
        "[ibcmd] asking what the dump will do",
    );
    let result = dsl
        .config_export_status(description, resolved.extension.as_deref())
        .map_err(map_ibcmd_error)?;
    ensure_interruption_clear(context, "after the dump forecast")?;
    Ok(match result.process.outcome() {
        Ok(()) => crate::platform::dump_forecast::read_export_status(&result.process.stdout),
        Err(_) => DumpForecast::Unknown,
    })
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
    let publication = prepare_full_dump(context, resolved)?;
    let staging_dir = publication.staging_path().to_path_buf();
    debug!(path = %staging_dir.display(), "created dump staging directory");

    log_live_stage(
        ProgressPhase::Dump,
        Some(resolved.source_set_name.as_str()),
        "dump: full",
        "[Конфигуратор] exporting configuration files",
    );
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

    if let Some(external) = context.external_full_pull() {
        external.generation_after(helpers::read_dump_generation(
            context,
            config,
            crate::domain::capability::Provider::Designer,
            binary,
            runner,
            resolved,
        ));
    }
    let notes = publish_full_dump(context, config, resolved, &publication)?;
    Ok((dump_result, notes))
}

/// Выгрузка `ibcmd` поверх каталога набора через стадию ([`landing::dump_over_directory`]):
/// в непустой каталог `config export` не пишет (замер #403). По копии описи раннера — прогноз
/// (`config export status --base`), затем только изменённое (`config export --base`); прогноз
/// «полная» или отказ «Требуется экспортировать конфигурацию полностью» (замер #646) — полная
/// выгрузка в стадию. Без годной копии — сразу полная
/// (`INV.USE-CASES.AN-IBCMD-FULL-DUMP-LANDS-OVER-THE-DIRECTORY-THROUGH-A-STAGE`).
fn run_dump_over_directory_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    how: &OverDirectory,
    version: Option<&super::version_file::RunnerVersionFile>,
) -> DumpRun {
    debug!(
        source_set = resolved.source_set_name.as_str(),
        target = %resolved.platform_target_path.display(),
        how = ?how,
        "running ibcmd dump over the directory through a stage"
    );
    ensure_dir(&resolved.platform_target_path)
        .map_err(|error| AppError::Runtime(format!("failed to create target dir: {error}")))?;

    let dsl = build_ibcmd_dsl(
        context,
        config,
        binary,
        runner,
        InterruptionSafetyClass::GracefulThenKill,
    )?;
    let extension = resolved.extension.as_deref();
    let full_into = |output: &Path| -> Result<PlatformCommandResult, AppError> {
        log_live_stage(
            ProgressPhase::Dump,
            Some(resolved.source_set_name.as_str()),
            "dump: full",
            "[ibcmd] exporting configuration files",
        );
        // В непустой каталог `config export` не пишет: прежняя попытка его не оставляет.
        remove_path_if_exists(output)
            .and_then(|()| std::fs::create_dir_all(output))
            .map_err(|error| AppError::Runtime(format!("failed to prepare the stage: {error}")))?;
        let result = dsl
            .config_export_full(output, extension)
            .map_err(map_ibcmd_error)?;
        ensure_platform_success("dump", resolved, &result)?;
        Ok(result)
    };
    let ((dump_result, evidence), message) =
        landing::dump_over_directory(context, config, resolved, how, version, |into| {
            let Some(description) = into.description else {
                return full_into(into.output).map(|result| (result, None));
            };
            let forecast = forecast_ibcmd(context, resolved, &dsl, description)?;
            let evidence = Some(ModeEvidence::Forecast(forecast));
            if forecast == DumpForecast::Full {
                return full_into(into.output).map(|result| (result, evidence));
            }
            log_live_stage(
                ProgressPhase::Dump,
                Some(resolved.source_set_name.as_str()),
                "dump: incremental",
                "[ibcmd] exporting configuration files",
            );
            let result = dsl
                .config_export_for_changes(into.output, description, extension)
                .map_err(map_ibcmd_error)?;
            if crate::platform::ibcmd::asks_for_a_full_export(&result.process) {
                ensure_interruption_clear(context, "before the full dump the platform asked for")?;
                let evidence = Some(ModeEvidence::Forecast(DumpForecast::Full));
                let refused = result.process.stderr.trim().to_owned();
                return full_into(into.output)
                    .map(|result| (result, evidence))
                    .map_err(|error| {
                        // Класс отказа полной выгрузки сохраняется; причина первой попытки — в
                        // тексте при любом классе.
                        if refused.is_empty() {
                            error
                        } else {
                            error.with_context(format!(
                                "the dump by changes was refused before the full dump: {refused}"
                            ))
                        }
                    });
            }
            ensure_platform_success("dump", resolved, &result)?;
            Ok((result, evidence))
        })?;
    Ok((
        dump_result,
        DumpNotes {
            evidence,
            ..DumpNotes::message(message)
        },
    ))
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
    let publication = prepare_full_dump(context, resolved)?;
    let staging_dir = publication.staging_path().to_path_buf();
    debug!(path = %staging_dir.display(), "created dump staging directory");

    log_live_stage(
        ProgressPhase::Dump,
        Some(resolved.source_set_name.as_str()),
        "dump: full",
        "[ibcmd] exporting configuration files",
    );
    let dump_result = match build_ibcmd_dsl(
        context,
        config,
        binary,
        runner,
        InterruptionSafetyClass::GracefulThenKill,
    )?
    .config_export_full(&staging_dir, resolved.extension.as_deref())
    .map_err(map_ibcmd_error)
    {
        Ok(result) => result,
        Err(error) => return Err(publication.cleanup_failure(error)),
    };
    ensure_platform_success("dump", resolved, &dump_result)
        .map_err(|error| publication.cleanup_failure(error))?;

    if let Some(external) = context.external_full_pull() {
        external.generation_after(helpers::read_dump_generation(
            context,
            config,
            crate::domain::capability::Provider::Ibcmd,
            binary,
            runner,
            resolved,
        ));
    }
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
        ProgressPhase::Dump,
        Some(resolved.source_set_name.as_str()),
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
        &list_file,
        resolved.extension.as_deref(),
    )
    .map_err(AppError::from)?;
    ensure_platform_success("dump", resolved, &dump_result)?;
    Ok((dump_result, DumpNotes::default()))
}

/// Выборку `ibcmd` не умеет: она идёт выгрузкой по изменившемуся по копии описи раннера, а без
/// годной копии — полной поверх каталога.
fn run_partial_dump_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    runner: &dyn ProcessRunner,
    version: Option<&super::version_file::RunnerVersionFile>,
) -> DumpRun {
    let warning = ibcmd_partial_warning(resolved);
    let how = match version.map(super::version_file::RunnerVersionFile::usable) {
        Some(Err(error)) => return Err(decorate_ibcmd_partial_error(error, &warning)),
        Some(Ok(Some(_))) => OverDirectory::ByVersionFile,
        Some(Ok(None)) | None => OverDirectory::Whole(WholeReason::Missing),
    };
    let whole = matches!(how, OverDirectory::Whole(_));
    match run_dump_over_directory_ibcmd(context, config, resolved, binary, runner, &how, version) {
        Ok((dump_result, notes)) => Ok((
            dump_result,
            DumpNotes {
                message: merge_optional_messages(Some(warning), notes.message),
                evidence: if whole {
                    Some(ModeEvidence::WithoutCopy)
                } else {
                    notes.evidence
                },
                ..notes
            },
        )),
        Err(error) => Err(decorate_ibcmd_partial_error(error, &warning)),
    }
}

fn ensure_interruption_clear(context: &ExecutionContext, phase: &str) -> Result<(), AppError> {
    interruption::pending_interruption_error(context, phase).map_or(Ok(()), Err)
}

#[allow(clippy::too_many_arguments)]
fn run_incremental_dump_edt_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
    version: Option<&super::version_file::RunnerVersionFile>,
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
        version,
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

#[allow(clippy::too_many_arguments)]
fn run_incremental_dump_edt_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
    version: Option<&super::version_file::RunnerVersionFile>,
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
        version,
    )?;
    let evidence = dump_notes.evidence;
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
    .map(|(result, notes)| (result, DumpNotes { evidence, ..notes }))
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

#[allow(clippy::too_many_arguments)]
fn run_partial_dump_edt_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    binary: &Path,
    edt_binary: &Path,
    runner: &dyn ProcessRunner,
    edt_runner: &dyn ProcessRunner,
    version: Option<&super::version_file::RunnerVersionFile>,
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
        run_partial_dump_ibcmd(context, config, resolved, binary, runner, version)?;
    let evidence = dump_notes.evidence;
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
    .map(|(result, notes)| (result, DumpNotes { evidence, ..notes }))
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
    let publication = StagedPublication::prepare_dir_in(
        &resolved.stage_root,
        &resolved.target_path,
        &resolved.target_identity,
        DUMP_STAGE_PREFIX,
    )?;
    let staging_dir = publication.staging_path().to_path_buf();

    let edt_dsl = build_edt_dsl(context, config, edt_binary, edt_runner)
        .map_err(|error| publication.cleanup_failure(error))?;
    log_live_stage(
        ProgressPhase::Convert,
        Some(resolved.source_set_name.as_str()),
        "dump: edt import",
        "[EDT] importing Designer snapshot",
    );
    let import_result = match edt_dsl
        .import_configuration_files(
            &staging_dir,
            &resolved.platform_target_path,
            crate::platform::utilities::edt_import_version(config).as_deref(),
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
            BackupRemoval::Background,
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
            evidence: None,
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
        stage_root: dump_stage_root(config),
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
        PARTIAL_OBJECT_BLANK_ERROR, PARTIAL_OBJECT_CONTROL_ERROR, VERSION_FILE_NAME,
    };
    use crate::config::model::{
        AppConfig, PlatformToolConfig, SourceFormat, SourceSetConfig, SourceSetPurpose,
        TestsConfig, ToolsConfig,
    };
    use crate::domain::dump::{DumpMode, DumpSelectorResult, ReportedDumpMode};
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
    use crate::use_cases::source_inventory::SourceSetInventory;
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

    #[derive(Debug)]
    struct ExternalPublisher {
        cancel_after_publish: Option<CancellationToken>,
        ready: Mutex<Option<(bool, bool)>>,
        saved: Mutex<Option<crate::use_cases::full_pull_publication::MemoryCommit>>,
    }

    impl crate::use_cases::full_pull_publication::FullPullControl for ExternalPublisher {
        fn authorize(
            &self,
            _: &crate::use_cases::full_pull_publication::FullPullExecutor<'_>,
        ) -> Result<(), AppError> {
            Ok(())
        }
        fn publish(
            &self,
            ready: &crate::use_cases::full_pull_publication::FullPullReady<'_>,
        ) -> Result<(), AppError> {
            *self.ready.lock().expect("ready state") =
                Some((ready.snapshot_prepared, ready.snapshot_warning.is_some()));
            let backup = ready.target.with_extension("external-before");
            fs::rename(ready.target, &backup)
                .map_err(|error| AppError::Runtime(error.to_string()))?;
            fs::rename(ready.stage, ready.target)
                .map_err(|error| AppError::Runtime(error.to_string()))?;
            fs::remove_dir_all(backup).map_err(|error| AppError::Runtime(error.to_string()))?;
            if let Some(cancel) = &self.cancel_after_publish {
                cancel.cancel();
            }
            Ok(())
        }
        fn memory_finished(&self, outcome: crate::use_cases::full_pull_publication::MemoryCommit) {
            *self.saved.lock().expect("saved state") = Some(outcome);
        }
    }

    #[test]
    fn an_external_published_decision_finishes_memory_despite_a_late_cancel() {
        use crate::change_detection::analyzer::{analyze_context, AnalysisOutcome};
        use crate::domain::capability::Provider;
        use crate::use_cases::full_pull_publication::{ExternalFullPull, MemoryCommit};
        let (_dir, config, resolved, publication) = publication_fixture();
        fs::write(
            publication.staging_path().join(VERSION_FILE_NAME),
            "prepared version bytes",
        )
        .expect("version file");
        let cancel = CancellationToken::new();
        let publisher = Arc::new(ExternalPublisher {
            cancel_after_publish: Some(cancel.clone()),
            ready: Mutex::new(None),
            saved: Mutex::new(None),
        });
        let external = Arc::new(ExternalFullPull::new(
            publication.staging_path().to_path_buf(),
            publisher.clone(),
        ));
        external.generation_before(Provider::Designer, Some("0".repeat(40)));
        external.generation_after(Some("0".repeat(40)));
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_cancellation(cancel.clone())
            .with_external_full_pull(external);
        let result = super::publish_full_dump(&context, &config, &resolved, &publication)
            .expect("published finish");
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|message| message.contains("cancel")),
            "{:?}",
            result.message
        );
        assert!(cancel.is_cancelled());
        assert_eq!(*publisher.ready.lock().unwrap(), Some((true, false)));
        assert_eq!(*publisher.saved.lock().unwrap(), Some(MemoryCommit::Saved));
        let inventory = super::SourceSetInventory::new(&config);
        let source = inventory.designer_context("main").expect("source");
        assert!(matches!(
            analyze_context(source, &config.work_path, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        let generation =
            fs::read_to_string(config.local_path.join("infobases/origin/generation.json"))
                .expect("generation record");
        let generation: serde_json::Value =
            serde_json::from_str(&generation).expect("generation JSON");
        assert_eq!(
            generation["main"]["identity"].as_str(),
            source.storage_identity()
        );
        assert_eq!(
            fs::read_to_string(
                config
                    .local_path
                    .join("infobases/origin/dump-info/main/ConfigDumpInfo.xml")
            )
            .expect("prepared version copy"),
            "prepared version bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_publication_rebinds_all_memory_after_replacing_a_source_symlink() {
        use crate::change_detection::analyzer::{analyze_context, AnalysisOutcome};
        use crate::domain::capability::Provider;
        use crate::use_cases::full_pull_publication::{ExternalFullPull, MemoryCommit};
        let (_dir, config, resolved, publication) = publication_fixture();
        let original_directory = resolved.target_path.with_extension("original-directory");
        fs::rename(&resolved.target_path, &original_directory).unwrap();
        std::os::unix::fs::symlink(&original_directory, &resolved.target_path).unwrap();
        let old_identity = super::SourceSetInventory::new(&config)
            .designer_context("main")
            .unwrap()
            .storage_identity()
            .unwrap()
            .to_owned();
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                mode: DumpModeRequest::Full,
                source_set: Some("main".into()),
                extension: None,
                objects: vec![],
                discard_uncommitted: true,
                force_way_out: ForceWayOut::PullForce,
                dry_run: false,
            },
        )
        .unwrap();
        fs::write(
            publication.staging_path().join(VERSION_FILE_NAME),
            "prepared version",
        )
        .unwrap();
        let publisher = Arc::new(ExternalPublisher {
            cancel_after_publish: None,
            ready: Mutex::new(None),
            saved: Mutex::new(None),
        });
        let external = Arc::new(ExternalFullPull::new(
            publication.staging_path().to_path_buf(),
            publisher.clone(),
        ));
        external.generation_before(Provider::Designer, Some("0".repeat(40)));
        external.generation_after(Some("0".repeat(40)));
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_external_full_pull(external);
        super::publish_full_dump(&context, &config, &resolved, &publication).unwrap();
        assert_eq!(*publisher.saved.lock().unwrap(), Some(MemoryCommit::Saved));
        assert!(!fs::symlink_metadata(&resolved.target_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(original_directory.join("Module.bsl")).unwrap(),
            "old source"
        );
        let inventory = super::SourceSetInventory::new(&config);
        let source = inventory.designer_context("main").unwrap();
        assert_ne!(source.storage_identity(), Some(old_identity.as_str()));
        assert!(matches!(
            analyze_context(source, &config.work_path, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        let memory = config.local_path.join("infobases/origin");
        let generation: serde_json::Value =
            serde_json::from_slice(&fs::read(memory.join("generation.json")).unwrap()).unwrap();
        assert_eq!(
            generation["main"]["identity"].as_str(),
            source.storage_identity()
        );
        assert_eq!(
            fs::read_to_string(memory.join("dump-info/main/identity")).unwrap(),
            source.storage_identity().unwrap()
        );
        assert_eq!(
            fs::read_to_string(memory.join("dump-info/main/ConfigDumpInfo.xml")).unwrap(),
            "prepared version"
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_publication_never_rebinds_prepared_memory_to_a_retargeted_infobase() {
        use crate::change_detection::analyzer::{snapshot_memory, SnapshotMemory};
        use crate::domain::capability::Provider;
        use crate::use_cases::full_pull_publication::{
            ExternalFullPull, FullPullControl, FullPullExecutor, FullPullReady, MemoryCommit,
        };

        #[derive(Debug)]
        struct RetargetBase {
            publisher: ExternalPublisher,
            link: PathBuf,
            other: PathBuf,
        }
        impl FullPullControl for RetargetBase {
            fn authorize(&self, executor: &FullPullExecutor<'_>) -> Result<(), AppError> {
                self.publisher.authorize(executor)
            }
            fn publish(&self, ready: &FullPullReady<'_>) -> Result<(), AppError> {
                self.publisher.publish(ready)?;
                fs::remove_file(&self.link).unwrap();
                std::os::unix::fs::symlink(&self.other, &self.link).unwrap();
                Ok(())
            }
            fn memory_finished(&self, outcome: MemoryCommit) {
                self.publisher.memory_finished(outcome);
            }
        }

        let (directory, mut config, resolved, publication) = publication_fixture();
        let first = directory.path().join("database-a");
        let second = directory.path().join("database-b");
        let link = directory.path().join("database");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        std::os::unix::fs::symlink(&first, &link).unwrap();
        config.infobase.connection = format!("File={}", link.display());
        let inventory = super::SourceSetInventory::new(&config);
        let source_a = inventory.designer_context("main").unwrap();
        let expected_identity = source_a.storage_identity().unwrap().to_owned();
        fs::write(
            publication.staging_path().join(VERSION_FILE_NAME),
            "prepared version",
        )
        .unwrap();
        let publisher = Arc::new(RetargetBase {
            publisher: ExternalPublisher {
                cancel_after_publish: None,
                ready: Mutex::new(None),
                saved: Mutex::new(None),
            },
            link,
            other: second,
        });
        let external = Arc::new(ExternalFullPull::new(
            publication.staging_path().to_path_buf(),
            publisher.clone(),
        ));
        external.generation_before(Provider::Designer, Some("0".repeat(40)));
        external.generation_after(Some("0".repeat(40)));
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_external_full_pull(external);
        super::publish_full_dump(&context, &config, &resolved, &publication).unwrap();
        assert_eq!(
            *publisher.publisher.saved.lock().unwrap(),
            Some(MemoryCommit::Saved)
        );
        assert!(matches!(
            snapshot_memory(source_a, &config.work_path),
            SnapshotMemory::Own
        ));
        let other_inventory = super::SourceSetInventory::new(&config);
        let source_b = other_inventory.designer_context("main").unwrap();
        assert_ne!(
            source_b.storage_identity(),
            Some(expected_identity.as_str())
        );
        assert!(matches!(
            snapshot_memory(source_b, &config.work_path),
            SnapshotMemory::Foreign
        ));
        let memory = config.local_path.join("infobases/origin");
        let generation: serde_json::Value =
            serde_json::from_slice(&fs::read(memory.join("generation.json")).unwrap()).unwrap();
        assert_eq!(
            generation["main"]["identity"].as_str(),
            Some(expected_identity.as_str())
        );
        assert_eq!(
            fs::read_to_string(memory.join("dump-info/main/identity")).unwrap(),
            expected_identity
        );
    }

    #[test]
    fn external_publication_can_accept_a_failed_snapshot_scan_without_fabricating_memory() {
        use crate::change_detection::analyzer::rescan_and_commit_full;
        use crate::use_cases::full_pull_publication::{ExternalFullPull, MemoryCommit};
        let (_dir, config, resolved, publication) = publication_fixture();
        let inventory = super::SourceSetInventory::new(&config);
        let source = inventory.designer_context("main").expect("source");
        rescan_and_commit_full(source, &config.work_path).expect("old memory");
        let memory_path = source.storage_path(&config.work_path).expect("memory path");
        let before = fs::read(&memory_path).expect("old memory bytes");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(publication.staging_path().join("Module.bsl"))
            .expect("staged file");
        file.set_modified(std::time::SystemTime::UNIX_EPOCH - Duration::from_secs(1))
            .expect("pre-epoch file time");
        assert!(file.metadata().unwrap().modified().unwrap() < std::time::SystemTime::UNIX_EPOCH);
        let publisher = Arc::new(ExternalPublisher {
            cancel_after_publish: None,
            ready: Mutex::new(None),
            saved: Mutex::new(None),
        });
        let external = Arc::new(ExternalFullPull::new(
            publication.staging_path().to_path_buf(),
            publisher.clone(),
        ));
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump)
            .with_external_full_pull(external);
        let warning = super::publish_full_dump(&context, &config, &resolved, &publication)
            .expect("publication succeeds")
            .message
            .expect("memory warning");
        assert!(warning.contains("sources published"), "{warning}");
        assert!(warning.contains("hash memory was not updated"), "{warning}");
        assert_eq!(*publisher.ready.lock().unwrap(), Some((false, true)));
        assert_eq!(
            *publisher.saved.lock().unwrap(),
            Some(MemoryCommit::NotSaved)
        );
        assert_eq!(fs::read(memory_path).expect("memory unchanged"), before);
        assert_eq!(
            fs::read_to_string(resolved.target_path.join("Module.bsl")).expect("published source"),
            "database source"
        );
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
            analyze_context(source, &config.work_path, &mut || false).outcome,
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
            analyze_context(source, &config.work_path, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
        fs::write(source.path().join("Module.bsl"), "user change").expect("edit");
        assert!(matches!(
            analyze_context(source, &config.work_path, &mut || false).outcome,
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

    /// Полная выгрузка заменила бы каталог вместе с `localPath` внутри: стенды и база
    /// пропали бы, поэтому она отказывает так же, как над `workPath`.
    #[test]
    fn only_full_pull_refuses_a_target_containing_local_path() {
        let dir = tempdir().expect("tempdir");
        let mut config = build_config(
            dir.path(),
            &dir.path().join("work"),
            &dir.path().join("1cv8"),
        );
        config.local_path = dir.path().join("main").join("personal");
        fs::create_dir_all(config.local_path.join("db")).expect("local");
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
            .contains("localPath"));
        args.mode = DumpModeRequest::Incremental;
        resolve_target(&config, &args).expect("incremental allowed");
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

    #[derive(Default)]
    struct DumpFixture {
        staged: bool,
        refusal: Option<(i32, &'static str)>,
        forecast: Option<&'static str>,
    }

    impl DumpFixture {
        fn install(self, path: &Path, calls: &Path) -> PathBuf {
            let mut path = path.to_path_buf();
            path.as_mut_os_string().push(std::env::consts::EXE_SUFFIX);
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_platform.rs");
            let compile = std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg(source)
                .arg("-o")
                .arg(&path)
                .output()
                .expect("compile fake platform");
            assert!(
                compile.status.success(),
                "fake platform compilation failed: {}",
                String::from_utf8_lossy(&compile.stderr)
            );

            let scenario = path.with_extension("dump-fixture");
            fs::create_dir(&scenario).expect("dump scenario directory");
            fs::write(
                scenario.join("calls-path"),
                calls.to_str().expect("UTF-8 test path"),
            )
            .expect("record call log path");
            if self.staged {
                fs::write(scenario.join("staged"), "").expect("configure staged export");
            }
            if let Some((code, flag)) = self.refusal {
                fs::write(scenario.join("refusal"), format!("{code}\n{flag}"))
                    .expect("configure dump refusal");
            }
            if let Some(forecast) = self.forecast {
                fs::write(scenario.join("forecast"), forecast).expect("configure dump forecast");
            }
            path
        }
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

    /// Выгрузка Конфигуратором: агент, первый в цепочке умолчаний, в этих тестах не нужен.
    fn build_config(base_path: &Path, work_path: &Path, platform_path: &Path) -> AppConfig {
        build_config_with_builder(
            base_path,
            work_path,
            platform_path,
            [(
                crate::domain::capability::Operation::Dump,
                crate::domain::capability::Provider::Designer,
            )]
            .into(),
        )
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
            local_path: base_path.join(".local"),
            base_path: base_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers,
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            working_copy: None,
            project_file: crate::config::model::default_project_file(),
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
    /// Копия описи раннера для набора `name`, как её оставляет удачная выгрузка (#646): опись,
    /// тождество пары и свидетельство каталога набора. Возвращает путь копии.
    fn seed_runner_copy(config: &AppConfig, name: &str, content: &str) -> PathBuf {
        let inventory = SourceSetInventory::new(config);
        let set = inventory.designer_context(name).expect("designer context");
        fs::create_dir_all(set.path()).expect("set dir");
        let version = crate::use_cases::version_file::RunnerVersionFile::of(set).expect("copy");
        let staged = config.work_path.join("seeded").join(VERSION_FILE_NAME);
        fs::create_dir_all(staged.parent().expect("parent")).expect("seed dir");
        fs::write(&staged, content).expect("seeded description");
        assert_eq!(version.record_from(&staged), None);
        version.path().to_path_buf()
    }

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

    /// Превью, выгрузка расширения и живой агент, выключенный ключом, агента заранее не
    /// поднимают (`INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`, #574).
    #[test]
    fn a_preview_an_extension_and_live_off_raise_no_early_agent() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        create_source_tree(dir.path());
        write_script(&script, "exit 0");
        let mut config = build_config_with_builder(
            dir.path(),
            &dir.path().join("work"),
            &script,
            [(
                crate::domain::capability::Operation::Dump,
                crate::domain::capability::Provider::Agent,
            )]
            .into(),
        );
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
        let args = |dry_run: bool, extension: Option<&str>| DumpArgs {
            dry_run,
            discard_uncommitted: false,
            force_way_out: ForceWayOut::PullForce,
            mode: DumpModeRequest::Incremental,
            source_set: Some(extension.map_or("main", |_| "ext").to_owned()),
            extension: extension.map(str::to_owned),
            objects: vec![],
        };
        let early = |config: &AppConfig, args: &DumpArgs| {
            super::agent::start_early(&context, config, args, &super::DumpMode::Incremental)
                .is_some()
        };
        assert!(!early(&config, &args(true, None)), "a preview");
        assert!(!early(&config, &args(false, Some("ext"))), "an extension");
        config.tools.designer_agent.live = Some(false);
        assert!(!early(&config, &args(false, None)), "live: false");
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
            stage_root: dir.path().join("work").join("stage"),
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
            stage_root: dir.path().join("work").join("stage"),
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

    /// Промежуточный каталог полной выгрузки лежит в `workPath/stage`, а не рядом с набором:
    /// в каталоге-родителе набора во время выгрузки нет `.dump-stage-*`, а публикация кладёт
    /// стадию на место набора переименованием
    /// (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
    #[test]
    fn the_full_dump_stage_lies_in_the_work_path_and_lands_on_the_set() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let context = ExecutionContext::cli(crate::use_cases::context::CommandName::Dump);
        let publication = super::prepare_full_dump(&context, &resolved).expect("stage");

        assert!(publication.staging_path().starts_with(&resolved.stage_root));
        assert!(publication.staging_path().is_dir());
        let beside_the_set = fs::read_dir(dir.path())
            .expect("parent")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".dump-"))
            .count();
        assert_eq!(beside_the_set, 0, "no stage beside the set");

        fs::write(publication.staging_path().join("Configuration.xml"), "new").expect("dumped");
        publication
            .publish_dir(
                &context,
                super::DUMP_BACKUP_PREFIX,
                "publish",
                &DestructionConsent::RunnerOwned,
                &[],
                crate::use_cases::staged_publication::BackupRemoval::Inline,
            )
            .expect("published");
        assert_eq!(
            fs::read_to_string(resolved.target_path.join("Configuration.xml")).expect("landed"),
            "new"
        );
        assert_eq!(
            fs::read_dir(&resolved.stage_root)
                .expect("stage root")
                .count(),
            0,
            "the stage became the set"
        );
    }

    /// Каталог набора и `workPath` на разных томах — отказ `work_path_on_another_volume` до
    /// выгрузки: проверка ничего не пишет. На одном томе отказа нет. Два тома в одном прогоне
    /// не завести, поэтому определитель тома подменён.
    #[test]
    fn a_set_on_another_volume_than_the_work_path_is_refused_before_anything_is_written() {
        use crate::support::path::VolumeId;
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let before: Vec<_> = fs::read_dir(dir.path())
            .expect("dir")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();

        super::helpers::refuse_on_another_volume(&resolved, |_| Ok(VolumeId::new(1)))
            .expect("one volume");

        let stage_root = resolved.stage_root.clone();
        let refused = super::helpers::refuse_on_another_volume(&resolved, |path| {
            Ok(VolumeId::new(if path == stage_root { 2 } else { 1 }))
        })
        .expect_err("another volume");
        assert_eq!(
            UseCaseErrorKind::of(&refused),
            UseCaseErrorKind::WorkPathOnAnotherVolume
        );
        assert_eq!(UseCaseErrorKind::of(&refused).exit_code(), 2);
        assert!(
            refused.to_string().contains("different volumes"),
            "{refused}"
        );
        let after: Vec<_> = fs::read_dir(dir.path())
            .expect("dir")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(before, after, "the check writes nothing");
        assert!(!resolved.stage_root.exists());

        // Том, который не определить, отказом не служит.
        super::helpers::refuse_on_another_volume(&resolved, |_| {
            Err(std::io::Error::other("unknown"))
        })
        .expect("an unknown volume is no refusal");
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
            stage_root: dir.join("work").join("stage"),
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
            TempDirKind::ThrowawayInfobase => unreachable!("a dump stages no infobase"),
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

    /// Опись `version-file-*`, оставленная снятым процессом, убирается только в
    /// `workPath/stage`: одноимённые каталог и файл пользователя рядом с целью остаются (#646).
    #[test]
    fn a_left_version_file_dir_is_removed_only_from_the_stage_root() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let old = std::time::SystemTime::now() - ORPHAN_TTL - Duration::from_secs(60);
        let age = |path: &Path| {
            let mut options = fs::File::options();
            if path.is_dir() {
                options.read(true);
            } else {
                options.write(true);
            }
            // Каталог на Windows открывается только с FILE_FLAG_BACKUP_SEMANTICS.
            #[cfg(windows)]
            std::os::windows::fs::OpenOptionsExt::custom_flags(&mut options, 0x0200_0000);
            #[cfg(windows)]
            options.write(true);
            options
                .open(path)
                .and_then(|file| file.set_modified(old))
                .expect("age");
        };
        let left = resolved.stage_root.join("version-file-run");
        fs::create_dir_all(&left).expect("left dir");
        fs::write(left.join("ConfigDumpInfo.xml"), "<x/>").expect("left file");
        let notes = dir.path().join("version-file-notes");
        fs::create_dir_all(&notes).expect("user dir");
        fs::write(notes.join("note.txt"), "keep").expect("user note");
        let text = dir.path().join("version-file-x.txt");
        fs::write(&text, "keep").expect("user file");
        for path in [&left, &notes, &text] {
            age(path);
        }

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(!left.exists());
        assert!(notes.join("note.txt").is_file());
        assert!(text.is_file());
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

    /// Смерть процесса между двумя переименованиями оставляет резерв без цели: это
    /// единственная копия каталога, и она возвращается раньше, чем что-то создаст цель.
    #[test]
    fn cleanup_orphan_dirs_returns_the_backup_when_the_target_is_gone() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = temp_dir_of(&resolved, TempDirKind::Backup);
        fs::write(backup_dir.join("kept.txt"), "kept").expect("content");
        let stage_dir = stale_temp_dir(&resolved, TempDirKind::Stage, |_| {});
        let _ = fs::remove_dir_all(&resolved.target_path);
        assert!(!resolved.target_path.exists());

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(resolved.target_path.join("kept.txt").exists());
        assert!(!backup_dir.exists());
        assert!(!metadata_sidecar_path(&backup_dir).exists());
        assert!(!stage_dir.exists(), "only the backup is returned");

        // Следующая уборка с уже существующей целью ничего не теряет.
        cleanup_orphan_dirs(&resolved).expect("second cleanup");
        assert!(resolved.target_path.join("kept.txt").exists());
    }

    /// Резерв, оставшийся от завершённой замены, старше удалённой цели: вернуть его значило
    /// бы выдать устаревшее дерево за каталог набора.
    #[test]
    fn cleanup_orphan_dirs_does_not_return_a_superseded_backup() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = stale_temp_dir(&resolved, TempDirKind::Backup, |metadata| {
            metadata.superseded = true;
        });
        let _ = fs::remove_dir_all(&resolved.target_path);

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(!resolved.target_path.exists());
        assert!(
            !backup_dir.exists(),
            "a superseded stale backup is a leftover"
        );
    }

    /// Прежнее дерево завершённой замены, которое не успело или не смогло удалить фоновое
    /// удаление (#616), следующая команда убирает сразу, не дожидаясь срока уборки.
    #[test]
    fn cleanup_orphan_dirs_removes_a_fresh_superseded_backup() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = temp_dir_of(&resolved, TempDirKind::Backup);
        fs::write(backup_dir.join("old.txt"), "old").expect("content");
        let mut metadata = read_temp_dir_metadata(&backup_dir).expect("metadata");
        metadata.superseded = true;
        fs::write(
            metadata_sidecar_path(&backup_dir),
            serde_json::to_vec(&metadata).expect("json"),
        )
        .expect("mark superseded");
        let fresh_unmarked = dir.path().join(".dump-backup-fresh");
        fs::create_dir_all(&fresh_unmarked).expect("unmarked");
        write_temp_dir_metadata(
            &fresh_unmarked,
            TempDirKind::Backup,
            "fresh",
            &resolved.target_path,
            &resolved.target_identity,
        )
        .expect("metadata");

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(
            !backup_dir.exists(),
            "a superseded backup is a leftover at any age"
        );
        assert!(!metadata_sidecar_path(&backup_dir).exists());
        assert!(
            fresh_unmarked.exists(),
            "a fresh unmarked backup waits for its term"
        );
        assert!(resolved.target_path.exists());
    }

    /// Фоновое удаление, оборванное между деревом и его метаданными, оставляет только
    /// метаданные; следующая команда убирает и их.
    #[test]
    fn cleanup_orphan_dirs_removes_the_metadata_of_a_removed_superseded_backup() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = temp_dir_of(&resolved, TempDirKind::Backup);
        let mut metadata = read_temp_dir_metadata(&backup_dir).expect("metadata");
        metadata.superseded = true;
        fs::write(
            metadata_sidecar_path(&backup_dir),
            serde_json::to_vec(&metadata).expect("json"),
        )
        .expect("mark superseded");
        fs::remove_dir_all(&backup_dir).expect("tree removed");
        let foreign = dir.path().join(".dump-backup-foreign.meta.json");
        fs::write(&foreign, "{}").expect("foreign metadata");

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(!metadata_sidecar_path(&backup_dir).exists());
        assert!(foreign.exists(), "unreadable metadata is not ours");
    }

    /// Каталог возвращён раньше, чем выгрузка создаст цель: отказ платформы его не трогает.
    #[cfg(unix)]
    #[test]
    fn a_failed_dump_after_a_killed_replacement_leaves_the_returned_directory() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture {
            staged: true,
            refusal: Some((3, "export")),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let target = base.join("main");
        fs::write(target.join("Configuration.xml"), "old").expect("old");
        commit_sources(&base);
        let identity =
            crate::support::path::stable_path_identity(&target.canonicalize().expect("target"));
        let backup = base.join(".dump-backup-killed");
        fs::rename(&target, &backup).expect("the first rename of a replacement");
        write_temp_dir_metadata(&backup, TempDirKind::Backup, "killed", &target, &identity)
            .expect("metadata");

        let failure = run_dump(&config, &incremental_main()).expect_err("the platform refused");
        assert_eq!(
            fs::read_to_string(target.join("Configuration.xml")).expect("returned"),
            "old"
        );
        assert!(!backup.exists());
        assert!(
            failure.error.message().contains("exit code 3"),
            "{failure:?}"
        );
        let calls = fs::read_to_string(calls).expect("platform calls");
        assert!(
            calls
                .lines()
                .any(|call| call.contains(" config export ") && call.contains(".dump-stage-")),
            "{calls}"
        );
    }

    #[test]
    fn cleanup_orphan_dirs_does_not_return_a_foreign_backup() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = stale_temp_dir(&resolved, TempDirKind::Backup, |metadata| {
            metadata.target_identity = "another target".to_owned();
        });
        let _ = fs::remove_dir_all(&resolved.target_path);

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(backup_dir.exists());
        assert!(!resolved.target_path.exists());
    }

    #[test]
    fn cleanup_orphan_dirs_removes_a_stale_backup_beside_its_target() {
        let dir = tempdir().expect("tempdir");
        let resolved = resolved_dump_target(dir.path());
        let backup_dir = stale_temp_dir(&resolved, TempDirKind::Backup, |_| {});
        fs::create_dir_all(&resolved.target_path).expect("target");

        cleanup_orphan_dirs(&resolved).expect("cleanup");

        assert!(!backup_dir.exists());
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
        let script = DumpFixture::default().install(&script, &calls);
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
        assert_eq!(result.mode, ReportedDumpMode::Full);
        let message = result.message.expect("the reason is named");
        assert!(
            message.contains("no version file ConfigDumpInfo.xml"),
            "{message}"
        );
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(calls.contains("/DumpConfigToFiles"));
        assert!(!calls.contains("-update"));
        assert!(calls.contains(".dump-stage-"), "{calls}");
    }

    /// Файл без распознанной версии формата чужой, и выгрузка по изменившемуся становится
    /// полной. Прочитанную версию чужой делает только таблица замеров (#403): для платформы
    /// вне таблицы никакая версия чужой не считается.
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
        let write_version = |version: &str| {
            fs::write(
                &file,
                format!("<ConfigDumpInfo format=\"Hierarchical\" version=\"{version}\">"),
            )
            .expect("version file");
        };
        write_version("2.20");
        assert_eq!(
            plan(Some(&file)),
            DumpPlan::OverDirectory(OverDirectory::ByVersionFile)
        );
        for (version, found) in [
            ("2.17", FormatVersion::new(2, 17)),
            ("9.99", FormatVersion::new(9, 99)),
        ] {
            write_version(version);
            assert_eq!(
                plan(Some(&file)),
                whole(WholeReason::Foreign {
                    found,
                    platform: platform.clone(),
                    written: FormatVersion::new(2, 20),
                }),
                "{version}: 8.3.27 writes 2.20 by measurement"
            );
        }
        let unmeasured = crate::platform::locator::PlatformVersion {
            minor: 5,
            patch: 2,
            ..platform.clone()
        };
        assert_eq!(
            plan_dump(&DumpMode::Incremental, Some(&file), Some(&unmeasured)).expect("plan"),
            DumpPlan::OverDirectory(OverDirectory::ByVersionFile),
            "a platform outside the table has no foreign version"
        );
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
        let script = DumpFixture::default().install(&script, &calls);
        let config = build_config(&base, &work, &script);
        let copy = seed_runner_copy(&config, "ext", "<ConfigDumpInfo version=\"2.17\"/>");

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
        assert!(
            calls.contains(&format!(
                "-update -configDumpInfoForChanges {}",
                copy.display()
            )),
            "{calls}"
        );
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
        assert_eq!(result.mode, ReportedDumpMode::Partial);
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

    /// Значение ключа в запросе к Конфигуратору.
    #[cfg(windows)]
    fn designer_option(request: &ProcessRequest, key: &str) -> Option<PathBuf> {
        let position = request.args.iter().position(|arg| arg == key)?;
        request.args.get(position + 1).map(PathBuf::from)
    }

    /// Открывает файл так, как замеренный Конфигуратор: чужой дескриптор на чтение ему не
    /// мешает, дескриптор на запись даёт отказ
    /// (`references/1c/confirmed-runtime-measurements.md`, «Открытый файл у `-listFile` и
    /// `-getChanges`»).
    #[cfg(windows)]
    fn open_as_the_designer(path: &Path, write: bool) -> std::io::Result<fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .write(write)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(path)
    }

    #[cfg(windows)]
    fn designer_dump_fixture(
        objects: &[&str],
    ) -> (tempfile::TempDir, AppConfig, super::ResolvedDumpTarget) {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        create_source_tree(&base);
        let config = build_config(&base, &dir.path().join("work"), &dir.path().join("1cv8"));
        let resolved = resolve_target(
            &config,
            &DumpArgs {
                dry_run: false,
                discard_uncommitted: false,
                force_way_out: ForceWayOut::PullForce,
                mode: DumpModeRequest::Partial,
                source_set: Some("main".to_owned()),
                extension: None,
                objects: objects.iter().map(|&object| object.to_owned()).collect(),
            },
        )
        .expect("target");
        (dir, config, resolved)
    }

    #[cfg(windows)]
    #[test]
    fn a_partial_dump_hands_the_designer_a_closed_object_list() {
        let (dir, config, resolved) = designer_dump_fixture(&["Catalog.Items"]);
        let read = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&read);
        let runner = TestProcessRunner::with_on_run(move |request| {
            use std::io::Read;
            let list = designer_option(request, "-listFile").expect("-listFile");
            let mut text = String::new();
            open_as_the_designer(&list, false)
                .expect("the list holds no writer handle")
                .read_to_string(&mut text)
                .expect("read list");
            *seen.lock().expect("seen") = Some(text);
        });

        super::run_partial_dump_designer(
            &ExecutionContext::cli(crate::use_cases::context::CommandName::Dump),
            &config,
            &resolved,
            &dir.path().join("1cv8"),
            &runner,
            &[PartialDumpSelector::parse("Catalog.Items").expect("selector")],
        )
        .expect("partial dump");

        assert_eq!(
            read.lock().expect("read").as_deref(),
            Some("Catalog.Items\n")
        );
    }

    #[test]
    fn dump_partial_designer_extension_uses_extension_flag() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture::default().install(&script, &calls);
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
        let script = DumpFixture {
            refusal: Some((17, "-partial")),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
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
        assert_eq!(
            failure.payload.expect("payload").mode,
            ReportedDumpMode::Partial
        );
        assert!(partial_list_paths(&work).is_empty());
    }

    #[test]
    fn dump_partial_ibcmd_goes_by_the_runner_copy_and_returns_warning() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture::default().install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let copy = seed_runner_copy(&config, "main", "<ConfigDumpInfo version=\"2.17\"/>");

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
        // `ibcmd` выборку не умеет: идёт выгрузка по изменившемуся, и ответ называет её.
        assert_eq!(result.requested_mode, DumpMode::Partial);
        assert_eq!(result.mode, ReportedDumpMode::Incremental);
        assert!(result
            .message
            .as_deref()
            .expect("warning")
            .contains("IBCMD does not support object-scoped partial dump"));
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(
            calls.contains(&format!(" export --base={} ", copy.display())),
            "{calls}"
        );
        assert!(!calls.contains("--sync"), "{calls}");
    }

    #[test]
    fn dump_partial_ibcmd_accepts_future_root_type_and_degrades_to_incremental() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture::default().install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        seed_runner_copy(&config, "main", "<ConfigDumpInfo version=\"2.17\"/>");

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
        // `ibcmd` выборку не умеет: идёт выгрузка по изменившемуся, и ответ называет её.
        assert_eq!(result.requested_mode, DumpMode::Partial);
        assert_eq!(result.mode, ReportedDumpMode::Incremental);
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
        assert!(calls.contains("--base="), "{calls}");
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
        let script = DumpFixture::default().install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let copy = seed_runner_copy(&config, "ext", "<ConfigDumpInfo version=\"2.17\"/>");

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
        // `ibcmd` выборку не умеет: идёт выгрузка по изменившемуся, и ответ называет её.
        assert_eq!(result.requested_mode, DumpMode::Partial);
        assert_eq!(result.mode, ReportedDumpMode::Incremental);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(
            calls.contains(&format!(
                " export --extension ext --base={} ",
                copy.display()
            )),
            "{calls}"
        );
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
        let script = DumpFixture {
            refusal: Some((17, "--force")),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
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
        assert_eq!(payload.mode, ReportedDumpMode::Partial);
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
        let script = DumpFixture {
            refusal: Some((17, "/DumpConfigToFiles")),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
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

        assert!(
            failure.error.message().contains("exit code 17"),
            "{failure:?}"
        );
        let calls = fs::read_to_string(calls).expect("platform calls");
        assert!(
            calls
                .lines()
                .any(|call| call.contains("/DumpConfigToFiles") && call.contains(".dump-stage-")),
            "{calls}"
        );
    }

    #[test]
    fn dump_full_success_replaces_old_target() {
        let dir = tempfile::Builder::new()
            .prefix("designer staged dump ")
            .tempdir()
            .expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture {
            staged: true,
            ..DumpFixture::default()
        }
        .install(&script, &calls);
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
        assert_eq!(
            fs::read_to_string(base.join("main/Configuration.xml")).expect("published dump"),
            "new\n"
        );
        let calls = fs::read_to_string(calls).expect("Designer calls");
        assert!(
            calls
                .lines()
                .any(|call| call.contains("/DumpConfigToFiles") && call.contains(".dump-stage-")),
            "{calls}"
        );
    }

    #[test]
    fn ibcmd_dump_full_uses_staging_dir_and_atomic_publish() {
        let dir = tempfile::Builder::new()
            .prefix("ibcmd staged dump ")
            .tempdir()
            .expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture {
            staged: true,
            ..DumpFixture::default()
        }
        .install(&script, &calls);
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
        assert_eq!(
            fs::read_to_string(base.join("main/Configuration.xml")).expect("published dump"),
            "new\n"
        );
    }

    /// `ibcmd` в непустой каталог не выгружает (замер #403). Без файла версий выгрузка идёт в
    /// пустой промежуточный каталог и ложится поверх: файлы выгрузки переписаны, посторонние
    /// файлы и скрытые каталоги целы, промежуточного каталога после выгрузки нет.
    #[test]
    fn an_ibcmd_full_dump_lands_over_a_non_empty_directory_through_a_stage() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        // Как настоящий `ibcmd`: непустой каталог — отказ, пустой — выгрузка с файлом версий.
        let script = DumpFixture {
            staged: true,
            ..DumpFixture::default()
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let target = base.join("main");
        fs::create_dir_all(target.join(".idea")).expect("hidden dir");
        fs::write(target.join(".idea").join("workspace.xml"), "ide").expect("hidden file");
        fs::write(target.join("stray.txt"), "keep").expect("stray");
        fs::write(target.join("Configuration.xml"), "old").expect("old dump file");
        commit_sources(&base);

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

        assert!(result.ok, "{result:?}");
        assert_eq!(result.mode, ReportedDumpMode::Full);
        assert_eq!(
            fs::read_to_string(target.join("Configuration.xml")).expect("dumped"),
            "new\n"
        );
        assert!(
            !target.join("ConfigDumpInfo.xml").exists(),
            "the description stays out of the directory"
        );
        let copy = crate::use_cases::version_file::RunnerVersionFile::of(
            SourceSetInventory::new(&config)
                .designer_context("main")
                .expect("set"),
        )
        .expect("copy")
        .path()
        .to_path_buf();
        assert_eq!(
            fs::read_to_string(copy).expect("the description became the runner copy"),
            "<ConfigDumpInfo version=\"2.20\"/>\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("stray.txt")).expect("stray"),
            "keep"
        );
        assert_eq!(
            fs::read_to_string(target.join(".idea/workspace.xml")).expect("hidden"),
            "ide"
        );
        assert_eq!(stage_leftovers(&base), 0, "the stage is removed");
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(!calls.contains("--sync"), "{calls}");
    }

    /// Промежуточные каталоги выгрузки рядом с набором и в `workPath/stage` проекта `base`:
    /// рабочий каталог тестов — `work` рядом с `base`.
    fn stage_leftovers(base: &Path) -> usize {
        let stage = base.parent().expect("parent").join("work").join("stage");
        [base.to_path_buf(), stage]
            .iter()
            .filter(|dir| dir.exists())
            .flat_map(|dir| fs::read_dir(dir).expect("dir").flatten())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".dump-stage"))
            .count()
    }

    fn incremental_main() -> DumpArgs {
        DumpArgs {
            dry_run: false,
            discard_uncommitted: false,
            force_way_out: ForceWayOut::PullForce,
            mode: DumpModeRequest::Incremental,
            source_set: Some("main".to_owned()),
            extension: None,
            objects: vec![],
        }
    }

    /// Отказ выгрузки в промежуточный каталог набор не трогает, а промежуточный убирает.
    #[test]
    fn a_failed_ibcmd_stage_dump_leaves_the_directory_untouched() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture {
            staged: true,
            refusal: Some((3, "export")),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let target = base.join("main");
        fs::write(target.join("Configuration.xml"), "old").expect("old");
        commit_sources(&base);

        let failure = run_dump(&config, &incremental_main()).expect_err("the platform refused");
        assert_eq!(
            fs::read_to_string(target.join("Configuration.xml")).expect("old"),
            "old"
        );
        assert_eq!(stage_leftovers(&base), 0, "the stage is removed");
        assert!(
            failure.error.message().contains("exit code 3"),
            "{failure:?}"
        );
        let calls = fs::read_to_string(calls).expect("platform calls");
        assert!(
            calls
                .lines()
                .any(|call| call.contains(" config export ") && call.contains(".dump-stage-")),
            "{calls}"
        );
    }

    /// Оборванный перенос копию раннера не трогает: прежняя опись старше дерева, и следующая
    /// выгрузка по ней выгрузит больше, а не меньше (#646). Стадия убрана, описи в каталоге нет.
    #[test]
    fn a_broken_overlay_keeps_the_runner_copy_as_it_was() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture {
            staged: true,
            ..DumpFixture::default()
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let target = base.join("main");
        // Файл на месте каталога, куда должен лечь `Catalogs/Items.xml`: перенос обрывается.
        fs::write(target.join("Catalogs"), "a file, not a directory").expect("blocker");
        let copy = seed_runner_copy(&config, "main", "<ConfigDumpInfo/>");
        commit_sources(&base);

        let failed = run_dump(&config, &incremental_main());

        let error = format!("{:?}", failed.expect_err("the overlay broke"));
        assert!(error.contains("lay the staged dump"), "{error}");
        assert_eq!(
            fs::read_to_string(&copy).expect("copy"),
            "<ConfigDumpInfo/>"
        );
        assert!(!target.join("ConfigDumpInfo.xml").exists());
        assert_eq!(stage_leftovers(&base), 0, "the stage is removed");
    }

    /// Набор с годной копией описи раннера и закоммиченными исходниками: выгрузка идёт по
    /// изменившемуся.
    fn project_with_a_version_file(config: &AppConfig) {
        create_source_tree(&config.base_path);
        seed_runner_copy(config, "main", "<ConfigDumpInfo version=\"2.20\"/>");
        commit_sources(&config.base_path);
    }

    /// Конфигуратор: выгрузка переписывает в каталоге файлы `rewrites`, остальное не трогает;
    /// поколение называет `token`, а без него — не называет.
    fn write_rewriting_designer_script(
        script: &Path,
        calls: &Path,
        token: Option<&str>,
        rewrites: &[&str],
    ) {
        let writes = rewrites
            .iter()
            .map(|file| {
                format!(
                    "  mkdir -p \"$(dirname \"$target/{file}\")\"\n  printf 'from the base\\n' > \"$target/{file}\"\n"
                )
            })
            .collect::<String>();
        write_script(
            script,
            &format!(
                "args=\"$*\"\nprintf '%s\\n' \"$args\" >> \"{}\"\nprev=\"\"\ntarget=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then printf '%s' '{token}' > \"$a\"; fi\n  if [ \"$prev\" = \"/DumpConfigToFiles\" ]; then target=\"$a\"; fi\n  prev=\"$a\"\ndone\nif [ -n \"$target\" ]; then\n{writes}  :\nfi\nexit 0",
                calls.display(),
                token = token.unwrap_or_default(),
            ),
        );
    }

    /// Зонд начала выгрузки убран: в каталоге его нет.
    fn assert_no_clock_probe(dir: &Path) {
        let probes: Vec<_> = fs::read_dir(dir)
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .filter(|name| {
                name.to_string_lossy()
                    .starts_with(crate::change_detection::scanner::DUMP_START_PROBE)
            })
            .collect();
        assert!(probes.is_empty(), "{probes:?}");
    }

    /// Конфигуратор прогноза не спрашивает: одна выгрузка с `-update`, без `-getChanges`;
    /// файл, которого она не переписала, оставляет режим `INCREMENTAL`
    /// (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`). Агента проверяет
    /// `tests/cli_dump_agent.rs::the_agent_asks_no_forecast`.
    #[test]
    fn the_designer_asks_no_forecast() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        let token = format!("{:040}", 7);
        write_rewriting_designer_script(&script, &calls, Some(&token), &[]);
        let config = build_config(&base, &dir.path().join("work"), &script);
        project_with_a_version_file(&config);

        let result = run_dump(&config, &incremental_main()).expect("dump");

        assert_eq!(result.mode, ReportedDumpMode::Incremental);
        assert_eq!(result.mode_reason, None);
        // Ровно три вызова платформы: поколение, выгрузка с `-update`, поколение. Вопрос о
        // прогнозе в любом виде был бы лишним вызовом.
        let calls = fs::read_to_string(calls).expect("calls");
        let steps: Vec<&str> = calls
            .lines()
            .map(|line| {
                if line.contains("/GetConfigGenerationID") {
                    "generation"
                } else if line.contains("/DumpConfigToFiles")
                    && line.contains("-update")
                    && !line.contains("-getChanges")
                {
                    "dump -update"
                } else {
                    line
                }
            })
            .collect();
        assert_eq!(
            steps,
            ["generation", "dump -update", "generation"],
            "{calls}"
        );
        assert_no_clock_probe(&base.join("main"));
    }

    /// Набор, выгрузка которого ничего не запоминает (поколение не прочитано, списка дерева
    /// до выгрузки нет), режим всё равно узнаёт по факту: выгрузка переписала каждый файл —
    /// `FULL` с причиной `platform_rewrote_all`. Файл версий в область не входит.
    #[test]
    fn a_set_without_memory_still_decides_by_fact() {
        use crate::domain::dump::DumpModeReason;
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        write_rewriting_designer_script(
            &script,
            &calls,
            None,
            &["Catalogs.Items/ObjectModule.bsl"],
        );
        let config = build_config(&base, &dir.path().join("work"), &script);
        project_with_a_version_file(&config);

        let result = run_dump(&config, &incremental_main()).expect("dump");

        assert_eq!(result.requested_mode, DumpMode::Incremental);
        assert_eq!(result.mode, ReportedDumpMode::Full);
        assert_eq!(result.mode_reason, Some(DumpModeReason::PlatformRewroteAll));
        assert!(fs::read_to_string(calls)
            .expect("calls")
            .lines()
            .any(|line| line.contains("/DumpConfigToFiles") && line.contains("-update")));
        assert_no_clock_probe(&base.join("main"));
    }

    /// У EDT выгрузка ложится в зеркало Конфигуратора: начало отмечается и каталог
    /// перечисляется там, а не в проекте EDT, файлы которого выгрузка не пишет.
    #[test]
    fn an_edt_dump_decides_its_mode_on_the_designer_mirror() {
        use crate::domain::dump::DumpModeReason;
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
        let mirror = designer_snapshot(&config, "main");
        fs::create_dir_all(&mirror).expect("mirror");
        fs::write(mirror.join("Configuration.xml"), "<Configuration/>").expect("mirror root");
        seed_runner_copy(&config, "main", "<ConfigDumpInfo version=\"2.20\"/>");

        let result = run_dump(&config, &incremental_main()).expect("dump");

        let designer_calls = fs::read_to_string(designer_calls).expect("designer calls");
        let dumps: Vec<&str> = designer_calls
            .lines()
            .filter(|line| line.contains("/DumpConfigToFiles"))
            .collect();
        assert_eq!(dumps.len(), 1, "{designer_calls}");
        assert!(dumps[0].contains("-update"), "{designer_calls}");
        // Зеркало держит только `Configuration.xml`, и выгрузка его переписала; в проекте
        // EDT остались файлы, которых она не писала, — по нему режим был бы `INCREMENTAL`.
        assert_eq!(result.mode, ReportedDumpMode::Full);
        assert_eq!(result.mode_reason, Some(DumpModeReason::PlatformRewroteAll));
        assert_no_clock_probe(&mirror);
    }

    /// Каталог набора с файлами `files`, помеченными часом раньше: выгрузка их не писала.
    fn a_tree_before_the_dump(root: &Path, files: &[&str]) {
        fs::create_dir_all(root).expect("root");
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            fs::write(&path, "before").expect("file");
            fs::File::options()
                .write(true)
                .open(&path)
                .expect("open")
                .set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
                .expect("old mtime");
        }
    }

    /// Режим выгрузки по изменившемуся, которая в каталоге с файлами `before` переписала
    /// `rewritten`: отмечено начало, перечень после выгрузки — тот же шаг, что у команды.
    fn mode_by_fact(
        before: &[&str],
        rewritten: &[&str],
    ) -> (
        ReportedDumpMode,
        Option<crate::domain::dump::DumpModeReason>,
    ) {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        a_tree_before_the_dump(&root, before);
        let set =
            crate::domain::source_set::SourceSetContext::new("main", root.clone(), "designer-main")
                .without_memory();
        let start = crate::change_detection::analyzer::mark_dump_start(&root).expect("start");
        for file in rewritten {
            fs::write(root.join(file), "from the base").expect("rewrite");
        }
        let ((), observed) = super::observe_dump_writes_while(
            Some(&set),
            &dir.path().join("work"),
            Some(&Ok(start)),
            None,
            None,
            &|| false,
            || (),
        );
        assert!(observed.writes.is_none());
        super::DumpPlan::OverDirectory(super::OverDirectory::ByVersionFile)
            .reported(observed.fact.map(super::ModeEvidence::Fact))
    }

    /// Выгрузка, переписавшая часть файлов, — по изменившемуся.
    #[test]
    fn a_partly_rewritten_dump_is_incremental() {
        assert_eq!(
            mode_by_fact(&["A.bsl", "B.bsl"], &["A.bsl"]),
            (ReportedDumpMode::Incremental, None)
        );
    }

    /// Посторонний файл, которого платформа не трогает, не даёт назвать выгрузку полной,
    /// даже когда каждый её файл переписан.
    #[test]
    fn a_stray_file_keeps_the_dump_incremental() {
        assert_eq!(
            mode_by_fact(
                &["Configuration.xml", "Catalogs/Items.xml", "Notes.txt"],
                &["Configuration.xml", "Catalogs/Items.xml"]
            ),
            (ReportedDumpMode::Incremental, None)
        );
    }

    /// Пустая область — не полная перезапись: `ConfigDumpInfo.xml` выгрузка переписывает
    /// всегда, и в область он не входит.
    #[test]
    fn an_empty_scope_is_not_full() {
        assert_eq!(
            mode_by_fact(&["ConfigDumpInfo.xml"], &["ConfigDumpInfo.xml"]),
            (ReportedDumpMode::Incremental, None)
        );
        assert_eq!(
            mode_by_fact(&[], &[]),
            (ReportedDumpMode::Incremental, None)
        );
    }

    /// Каталог после выгрузки не перечислен или начало не отмечено — режим «неизвестен»;
    /// отмена во время перечня оставляет режим плана.
    #[test]
    fn a_failed_listing_reports_the_mode_as_unknown() {
        use crate::domain::dump::DumpModeReason;
        let plan = super::DumpPlan::OverDirectory(super::OverDirectory::ByVersionFile);
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        fs::create_dir_all(&root).expect("root");
        let set =
            crate::domain::source_set::SourceSetContext::new("main", root.clone(), "designer-main")
                .without_memory();
        let start = crate::change_detection::analyzer::mark_dump_start(&root).expect("start");
        fs::remove_dir(&root).expect("the directory is gone before the listing");
        let observe = |mark: super::StartMark, interrupted: bool| {
            super::observe_dump_writes_while(
                Some(&set),
                &dir.path().join("work"),
                Some(&mark),
                None,
                None,
                &|| interrupted,
                || (),
            )
            .1
            .fact
            .map(super::ModeEvidence::Fact)
        };

        let unlisted = observe(Ok(start), false);
        assert_eq!(
            plan.reported(unlisted),
            (ReportedDumpMode::Unknown, Some(DumpModeReason::Unknown))
        );
        let cancelled = observe(Ok(start), true);
        assert_eq!(cancelled, None);
        assert_eq!(
            plan.reported(cancelled),
            (ReportedDumpMode::Incremental, None)
        );
        fs::create_dir_all(&root).expect("root again");
        let unmarked = observe(Err(std::io::Error::other("no probe")), false);
        assert_eq!(
            plan.reported(unmarked),
            (ReportedDumpMode::Unknown, Some(DumpModeReason::Unknown))
        );
    }

    /// Команда отмечает конец выгрузки перед перечнем: файл, изменённый после выгрузки, память
    /// не принимает за записанный платформой (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`).
    #[test]
    fn the_command_keeps_an_edit_after_the_dump_as_a_change() {
        use crate::change_detection::analyzer::{mark_dump_start, rescan_and_commit_full};
        use crate::change_detection::hash_storage::HashStorage;
        use crate::domain::source_set::SourceSetContext;

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        a_tree_before_the_dump(&root, &["A.bsl", "Late.bsl"]);
        let set = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory(
                &dir.path().join(".local"),
                "origin",
                "base-identity".to_owned(),
            );
        rescan_and_commit_full(&set, &work).expect("prime");
        let start = mark_dump_start(&root).expect("start");
        let watch = super::dump_watch(
            &build_config(dir.path(), &work, &dir.path().join("1cv8")),
            Some(&set),
            Some(start),
        )
        .expect("memory of this base");

        fs::write(root.join("A.bsl"), "from the base").expect("A");
        // Правка после выгрузки: её время позже отметки конца, которую ставит команда.
        fs::write(root.join("Late.bsl"), "edited later").expect("late");
        fs::File::options()
            .write(true)
            .open(root.join("Late.bsl"))
            .expect("open")
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
            .expect("later mtime");
        let (answer, observed) = super::observe_dump_writes_while(
            Some(&set),
            &work,
            Some(&Ok(start)),
            Some(&watch),
            None,
            &|| false,
            || "g1",
        );
        let note =
            super::remember_dump_writes(&set, &work, observed.writes, Some("g1"), Some(answer));
        assert!(note.is_none(), "{note:?}");
        let snapshot = HashStorage::new(set.storage_path(&work).expect("memory path"))
            .load_snapshot()
            .expect("snapshot");
        let hash = |name: &str| {
            crate::change_detection::scanner::hash_file(&root.join(name), &mut || false)
                .expect("hash")
        };
        assert_eq!(snapshot.entries["A.bsl"].hash, hash("A.bsl"));
        assert_ne!(snapshot.entries["Late.bsl"].hash, hash("Late.bsl"));
    }

    /// Выгрузка, переписавшая каждый файл набора, — полная по факту; тот же перечень
    /// запоминает записанное, второго обхода нет.
    #[test]
    fn a_dump_that_rewrote_every_file_is_full_by_fact() {
        use crate::change_detection::analyzer::{mark_dump_start, rescan_and_commit_full};
        use crate::change_detection::hash_storage::HashStorage;
        use crate::domain::dump::DumpModeReason;
        use crate::domain::source_set::SourceSetContext;

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        a_tree_before_the_dump(&root, &["A.bsl", "B.bsl", "ConfigDumpInfo.xml"]);
        let set = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory(
                &dir.path().join(".local"),
                "origin",
                "base-identity".to_owned(),
            );
        rescan_and_commit_full(&set, &work).expect("prime");
        let start = mark_dump_start(&root).expect("start");
        let watch = super::dump_watch(
            &build_config(dir.path(), &work, &dir.path().join("1cv8")),
            Some(&set),
            Some(start),
        )
        .expect("memory of this base");

        fs::write(root.join("A.bsl"), "from the base").expect("A");
        fs::write(root.join("B.bsl"), "from the base too").expect("B");
        let (answer, observed) = super::observe_dump_writes_while(
            Some(&set),
            &work,
            Some(&Ok(start)),
            Some(&watch),
            None,
            &|| false,
            || "g1",
        );
        let plan = super::DumpPlan::OverDirectory(super::OverDirectory::ByVersionFile);
        assert_eq!(
            plan.reported(observed.fact.map(super::ModeEvidence::Fact)),
            (
                ReportedDumpMode::Full,
                Some(DumpModeReason::PlatformRewroteAll)
            )
        );
        let note =
            super::remember_dump_writes(&set, &work, observed.writes, Some("g1"), Some(answer));
        assert!(note.is_none(), "{note:?}");
        let snapshot = HashStorage::new(set.storage_path(&work).expect("memory path"))
            .load_snapshot()
            .expect("snapshot");
        let written =
            crate::change_detection::scanner::hash_file(&root.join("B.bsl"), &mut || false)
                .expect("hash");
        assert_eq!(
            snapshot
                .entries
                .get("B.bsl")
                .map(|entry| entry.hash.as_str()),
            Some(written.as_str())
        );
    }

    /// Без файла версий выгрузка полная по плану, и прогноз не спрашивается.
    #[test]
    fn a_dump_without_a_version_file_asks_for_no_forecast() {
        use crate::domain::dump::DumpModeReason;
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        write_rewriting_designer_script(&script, &calls, None, &[]);
        let config = build_config(&base, &dir.path().join("work"), &script);

        let result = run_dump(&config, &incremental_main()).expect("dump");

        assert_eq!(result.mode, ReportedDumpMode::Full);
        assert_eq!(result.mode_reason, Some(DumpModeReason::VersionFile));
        assert!(!fs::read_to_string(calls)
            .expect("calls")
            .contains("-getChanges"));
    }

    /// `ibcmd` предсказал полную выгрузку: `--sync` её не сделает (#424), поэтому выгрузка
    /// идёт через промежуточный каталог, и ответ называет причину.
    #[test]
    fn an_ibcmd_full_forecast_dumps_through_the_stage_without_sync() {
        use crate::domain::dump::DumpModeReason;
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        let script = DumpFixture {
            staged: true,
            forecast: Some("modified: all\n"),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        project_with_a_version_file(&config);

        let result = run_dump(&config, &incremental_main()).expect("dump");

        assert_eq!(result.mode, ReportedDumpMode::Full);
        assert_eq!(result.mode_reason, Some(DumpModeReason::PlatformForecast));
        let calls = fs::read_to_string(calls).expect("calls");
        let forecast = calls
            .lines()
            .find(|call| call.contains(" export status "))
            .expect("forecast call");
        assert!(!forecast.contains("--threads"), "{forecast}");
        assert!(!calls.contains("--sync"), "{calls}");
        assert!(
            calls.contains(
                &dir.path()
                    .join("work")
                    .join("stage")
                    .join(".dump-stage-")
                    .display()
                    .to_string()
            ),
            "the stage lies in workPath/stage: {calls}"
        );
        assert_eq!(
            fs::read_to_string(base.join("main/Configuration.xml")).expect("dumped"),
            "new\n"
        );
    }

    /// Выборка `ibcmd` идёт `--sync` и тоже спрашивает прогноз: после удаления объекта она
    /// уходит в промежуточный каталог, а не падает на `--sync` (#424).
    #[test]
    fn an_ibcmd_partial_dump_follows_a_full_forecast_too() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        let script = DumpFixture {
            staged: true,
            forecast: Some("modified: all\n"),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        project_with_a_version_file(&config);

        let result = run_dump(
            &config,
            &DumpArgs {
                mode: DumpModeRequest::Partial,
                objects: vec!["Catalog.Items".to_owned()],
                ..incremental_main()
            },
        )
        .expect("dump");

        assert_eq!(result.requested_mode, DumpMode::Partial);
        assert_eq!(result.mode, ReportedDumpMode::Full);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(!calls.contains("--sync"), "{calls}");
    }

    /// Выгрузка `ibcmd` по копии описи отказывает кодом `-1` («Требуется экспортировать
    /// конфигурацию полностью», удаление объекта, замер #646): раннер выгружает полностью в
    /// чистую стадию, и ответ называет режим `FULL`.
    #[test]
    fn an_ibcmd_refusal_to_export_by_changes_falls_back_to_a_full_export() {
        use crate::domain::dump::DumpModeReason;
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        let script = DumpFixture {
            staged: true,
            refusal: Some((-1, "--base=")),
            forecast: Some("modified: CommonModule.A\n"),
        }
        .install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &dir.path().join("work"),
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        project_with_a_version_file(&config);

        let result = run_dump(&config, &incremental_main()).expect("dump");

        assert_eq!(result.mode, ReportedDumpMode::Full);
        assert_eq!(result.mode_reason, Some(DumpModeReason::PlatformForecast));
        let calls = fs::read_to_string(calls).expect("calls");
        let staged_exports: Vec<_> = calls
            .lines()
            .filter(|call| call.contains(" config export ") && call.contains(".dump-stage-"))
            .collect();
        let [by_changes, full] = staged_exports.as_slice() else {
            panic!("expected two staged exports: {calls}");
        };
        assert!(
            by_changes.contains("--base=") && !by_changes.contains("--force"),
            "{calls}"
        );
        assert!(
            full.contains("--force") && !full.contains("--base="),
            "{calls}"
        );
        assert_eq!(
            fs::read_to_string(base.join("main/Configuration.xml")).expect("dumped"),
            "new\n"
        );
        assert!(!base.join("main/ConfigDumpInfo.xml").exists());
    }

    /// Выгрузка Конфигуратора по изменившемуся идёт в стадию по копии описи раннера
    /// (`-update -configDumpInfoForChanges`): написанное ложится поверх каталога, опись в
    /// каталог не попадает, а неразобранная новая опись копией не становится; стадия убрана (#646).
    #[cfg(unix)]
    #[test]
    fn a_dump_by_changes_lands_from_the_stage_and_keeps_the_description_in_memory() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let script = dir.path().join("1cv8");
        let calls = dir.path().join("calls.log");
        write_rewriting_designer_script(
            &script,
            &calls,
            None,
            &["Catalogs.Items/ObjectModule.bsl", "ConfigDumpInfo.xml"],
        );
        let config = build_config(&base, &dir.path().join("work"), &script);
        project_with_a_version_file(&config);
        fs::write(base.join("main/Kept.bsl"), "kept").expect("a file the dump leaves");
        commit_sources(&base);
        let copy = crate::use_cases::version_file::RunnerVersionFile::of(
            SourceSetInventory::new(&config)
                .designer_context("main")
                .expect("set"),
        )
        .expect("copy")
        .path()
        .to_path_buf();

        let result = run_dump(&config, &incremental_main()).expect("dump");

        assert_eq!(result.mode, ReportedDumpMode::Incremental);
        let calls = fs::read_to_string(calls).expect("calls");
        assert!(
            calls.contains(&format!(
                "-update -configDumpInfoForChanges {}",
                copy.display()
            )),
            "{calls}"
        );
        assert_eq!(
            fs::read_to_string(base.join("main/Catalogs.Items/ObjectModule.bsl")).expect("laid"),
            "from the base\n"
        );
        assert_eq!(
            fs::read_to_string(base.join("main/Kept.bsl")).expect("kept"),
            "kept"
        );
        assert!(!base.join("main/ConfigDumpInfo.xml").exists());
        // Опись, которую поддельный Конфигуратор пишет текстом, не разобрать: какие элементы
        // пропали, не узнать, и копия остаётся прежней, а ответ это называет.
        assert_eq!(
            fs::read_to_string(&copy).expect("copy"),
            "<ConfigDumpInfo version=\"2.20\"/>"
        );
        assert!(
            result
                .message
                .as_deref()
                .is_some_and(|message| message.contains("removals of the dump")),
            "{:?}",
            result.message
        );
        assert_eq!(stage_leftovers(&base), 0, "the stage is removed");
    }

    #[test]
    fn ibcmd_dump_with_server_infobase_passes_dbms_and_infobase_credentials() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture::default().install(&script, &calls);
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
        let script = DumpFixture {
            refusal: Some((17, "--force")),
            ..DumpFixture::default()
        }
        .install(&script, &calls);
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
        assert!(
            failure.error.message().contains("exit code 17"),
            "{failure:?}"
        );
        let calls = fs::read_to_string(calls).expect("platform calls");
        assert!(
            calls
                .lines()
                .any(|call| call.contains(" config export ") && call.contains(".dump-stage-")),
            "{calls}"
        );
    }

    #[test]
    fn ibcmd_dump_incremental_goes_by_the_runner_copy_through_a_stage() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let script = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        create_source_tree(&base);
        let script = DumpFixture::default().install(&script, &calls);
        let config = build_config_with_builder(
            &base,
            &work,
            &script,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        let copy = seed_runner_copy(&config, "main", "<ConfigDumpInfo version=\"2.17\"/>");
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
        assert_eq!(result.mode, ReportedDumpMode::Incremental);
        let by_copy = fs::read_to_string(&calls).expect("calls");
        assert!(
            by_copy.contains(&format!("--base={}", copy.display())),
            "{by_copy}"
        );
        assert!(!by_copy.contains("--sync"), "{by_copy}");
        assert!(by_copy.contains(".dump-stage-"), "{by_copy}");

        // Каталог набора создан заново: копия его не подтверждает, и `ibcmd` выгружает
        // полностью в промежуточный каталог.
        fs::remove_dir_all(base.join("main")).expect("remove target");
        fs::remove_file(&calls).expect("reset calls");
        let result = run_dump(&config, &args).expect("dump");
        assert!(result.ok);
        assert_eq!(result.mode, ReportedDumpMode::Full);
        let full = fs::read_to_string(&calls).expect("calls");
        assert!(!full.contains("--sync"), "{full}");
        assert!(full.contains(".dump-stage-"), "{full}");
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
        // Полная выгрузка снимка идёт в промежуточный каталог `workPath/stage`
        // (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
        assert!(
            designer_calls.contains(
                work.join("stage")
                    .join(".dump-stage-")
                    .display()
                    .to_string()
                    .as_str()
            ),
            "{designer_calls}"
        );
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
        assert_eq!(result.mode, ReportedDumpMode::Full);

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
        // Полная выгрузка снимка идёт в промежуточный каталог `workPath/stage`
        // (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
        assert!(
            ibcmd_calls.contains(
                work.join("stage")
                    .join(".dump-stage-")
                    .display()
                    .to_string()
                    .as_str()
            ),
            "{ibcmd_calls}"
        );
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
            None,
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
            None,
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
        let error = super::run_partial_dump_edt_ibcmd(
            &context,
            &config,
            &resolved,
            &ibcmd,
            &edt,
            &dump_runner,
            &edt_runner,
            None,
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
        let leftover_stage_dirs = stage_leftovers(&base);
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
        let leftover_stage_dirs = stage_leftovers(&base);
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
            requested_mode: DumpMode::Incremental,
            mode: ReportedDumpMode::Incremental,
            mode_reason: None,
            target_path: PathBuf::from("/tmp/main"),
            platform_log_path: Some(PathBuf::from("/tmp/platform.log")),
            duration_ms: 5,
            message: Some("ok".to_owned()),
            losses: None,
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
        let script = DumpFixture::default().install(&script, &calls);
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
        let script = DumpFixture::default().install(&script, &calls);
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

    /// A set with memory of its base and one remembered file, for the listing tests.
    fn remembered_set(
        dir: &Path,
    ) -> (
        crate::domain::source_set::SourceSetContext,
        PathBuf,
        PathBuf,
    ) {
        let root = dir.join("src");
        let work = dir.join("work");
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("Kept.bsl"), "before").expect("kept");
        let set =
            crate::domain::source_set::SourceSetContext::new("main", root.clone(), "designer-main")
                .with_infobase_memory(&dir.join(".local"), "origin", "base-identity".to_owned());
        crate::change_detection::analyzer::rescan_and_commit_full(&set, &work).expect("prime");
        (set, root, work)
    }

    /// The tree is listed while the platform answers before the dump (#574): an answer by
    /// which the dump goes on gets the listing, any other answer drops it, and so does a set
    /// that needs none.
    #[test]
    fn the_tree_is_listed_while_the_platform_answers_only_for_a_dump_that_goes_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (set, _, work) = remembered_set(dir.path());
        for (wanted, answer, listed) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
        ] {
            let (got, tree) = super::list_before_dump_while(
                Some(&set),
                &work,
                wanted,
                &|| false,
                || answer,
                |answer| *answer,
            );
            assert_eq!(got, answer);
            assert_eq!(tree.is_some(), listed, "wanted {wanted}, goes on {answer}");
        }
        let (_, tree) =
            super::list_before_dump_while(Some(&set), &work, true, &|| true, || (), |()| true);
        assert!(tree.is_none(), "a cancelled listing remembers nothing");
    }

    /// The listing taken before the start is marked keeps the rule's split: a file the dump
    /// writes after the start is remembered, an edit made between the listing and the start
    /// stays a change (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`, #574).
    #[test]
    fn a_listing_taken_before_the_start_remembers_only_what_the_dump_wrote() {
        use crate::change_detection::analyzer::{list_before_dump, mark_dump_start};
        use crate::change_detection::hash_storage::HashStorage;

        let dir = tempfile::tempdir().expect("tempdir");
        let (set, root, work) = remembered_set(dir.path());
        let tree = list_before_dump(&set, &work, &|| false).expect("memory of this base");
        fs::write(root.join("Edited.bsl"), "a person's edit").expect("edit");
        // The file system stamps by its own clock: let it move past the edit.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let start = mark_dump_start(&root).expect("start");
        let watch = tree.watch(start);
        fs::write(root.join("Written.bsl"), "from the base").expect("written");
        let (answer, observed) = super::observe_dump_writes_while(
            Some(&set),
            &work,
            Some(&Ok(start)),
            Some(&watch),
            None,
            &|| false,
            || "g1",
        );
        let note =
            super::remember_dump_writes(&set, &work, observed.writes, Some("g1"), Some(answer));
        assert!(note.is_none(), "{note:?}");
        let snapshot = HashStorage::new(set.storage_path(&work).expect("memory path"))
            .load_snapshot()
            .expect("snapshot");
        assert!(snapshot.entries.contains_key("Written.bsl"));
        assert!(!snapshot.entries.contains_key("Edited.bsl"));
    }

    /// The walk of what the dump wrote now runs before the platform answers about the
    /// generation; a changed generation must still leave the memory as it was
    /// (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`, #571).
    #[test]
    fn writes_observed_during_the_generation_answer_are_remembered_only_for_the_same_generation() {
        use crate::change_detection::analyzer::{
            mark_dump_start, rescan_and_commit_full, watch_before_dump,
        };
        use crate::change_detection::hash_storage::HashStorage;
        use crate::domain::source_set::SourceSetContext;

        for (after, remembered) in [("g1", true), ("g2", false)] {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().join("src");
            let work = dir.path().join("work");
            fs::create_dir_all(&root).expect("root");
            fs::write(root.join("Kept.bsl"), "before").expect("kept");
            let set = SourceSetContext::new("main", root.clone(), "designer-main")
                .with_infobase_memory(
                    &dir.path().join(".local"),
                    "origin",
                    "base-identity".to_owned(),
                );
            rescan_and_commit_full(&set, &work).expect("prime");
            let storage = HashStorage::new(set.storage_path(&work).expect("memory path"));
            let primed = storage.current_generation().expect("generation");
            let start = mark_dump_start(&root).expect("start");
            let watch = watch_before_dump(&set, &work, start).expect("memory of this base");

            fs::write(root.join("Written.bsl"), "from the base").expect("written");
            let (answer, observed) = super::observe_dump_writes_while(
                Some(&set),
                &work,
                Some(&Ok(start)),
                Some(&watch),
                None,
                &|| false,
                || after,
            );
            let note =
                super::remember_dump_writes(&set, &work, observed.writes, Some("g1"), Some(answer));

            assert!(note.is_none(), "{note:?}");
            let snapshot = storage.load_snapshot().expect("snapshot");
            assert_eq!(
                snapshot.entries.contains_key("Written.bsl"),
                remembered,
                "generation after: {after}"
            );
            assert_eq!(
                snapshot.generation,
                if remembered { primed + 1 } else { primed },
                "generation after: {after}"
            );
        }
    }
}
