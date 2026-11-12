use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tempfile::{NamedTempFile, TempPath};

use crate::config::model::{AppConfig, SourceSetPurpose};
use crate::domain::capability::{Operation, Provider};
use crate::domain::dump::{DumpMode, DumpResult, DumpSelectorResult};
use crate::domain::partial_dump_selector::PartialDumpSelector;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl, IbcmdError};
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::support::edt_project;
use crate::support::error::AppError;
use crate::support::fs::{
    is_known_tool_name, metadata_sidecar_path, read_temp_dir_metadata, remove_path_if_exists,
    remove_tree_in_parallel, TempDirKind,
};
use crate::support::path::{is_filesystem_root, nearest_existing_canonical_path};
use crate::support::temp::{dump_object_list_file, platform_logs_dir};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::ibcmd_diagnostics::format_ibcmd_failure_details;
use crate::use_cases::interruption;
use crate::use_cases::source_inventory::SourceSetInventory;

use super::ResolvedDumpTarget;

pub(super) fn validate_supported_matrix(config: &AppConfig) -> Option<AppError> {
    if matches!(
        config.selected_provider(Operation::Dump),
        Provider::Designer | Provider::Ibcmd | Provider::Agent
    ) {
        None
    } else {
        Some(AppError::Validation(super::SUPPORTED_DUMP_ERROR.to_owned()))
    }
}

pub(super) fn validate_publish_target(resolved: &ResolvedDumpTarget) -> Result<(), AppError> {
    validate_publish_target_path(
        &resolved.target_path,
        &resolved.canonical_target_path,
        &resolved.canonical_base_path,
        &resolved.canonical_work_path,
    )
}

pub(super) fn cleanup_orphan_dirs(resolved: &ResolvedDumpTarget) -> Result<(), AppError> {
    cleanup_orphan_dirs_for(
        &resolved.target_path,
        &resolved.target_identity,
        &resolved.stage_root,
    )
}

/// Каталог набора и `workPath` на разных томах — отказ до выгрузки: промежуточный каталог
/// лежит в `workPath/stage`, и публикация переименованием через границу тома не прошла бы.
/// Проверка ничего не пишет; у EDT сверяется и снимок Конфигуратора
/// (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
pub(super) fn refuse_a_work_path_on_another_volume(
    resolved: &ResolvedDumpTarget,
) -> Result<(), AppError> {
    refuse_on_another_volume(resolved, crate::support::path::volume_of)
}

/// То же с определителем тома, который подменяет тест: два тома в одном прогоне не завести.
pub(super) fn refuse_on_another_volume(
    resolved: &ResolvedDumpTarget,
    volume_of: impl Fn(&Path) -> std::io::Result<crate::support::path::VolumeId>,
) -> Result<(), AppError> {
    // Том, который не определить, отказом не служит: публикация назовёт свою ошибку сама.
    let Ok(stage) = volume_of(&resolved.stage_root) else {
        return Ok(());
    };
    for target in [&resolved.target_path, &resolved.platform_target_path] {
        if volume_of(target).is_ok_and(|volume| volume != stage) {
            return Err(AppError::Refused(Box::new(
                crate::use_cases::result::UseCaseError::new(
                    crate::use_cases::result::UseCaseErrorKind::WorkPathOnAnotherVolume,
                    format!(
                        "source-set '{}' at '{}' and workPath '{}' are on different volumes: the dump stage lies in workPath and is published by a rename; move the source set or workPath onto one volume",
                        resolved.source_set_name,
                        target.display(),
                        resolved
                            .stage_root
                            .parent()
                            .unwrap_or(&resolved.stage_root)
                            .display()
                    ),
                ),
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_platform_target(resolved: &ResolvedDumpTarget) -> Result<(), AppError> {
    validate_publish_target_path(
        &resolved.platform_target_path,
        &resolved.canonical_platform_target_path,
        &resolved.canonical_base_path,
        &resolved.canonical_work_path,
    )
}

fn validate_publish_target_path(
    target_path: &Path,
    canonical_target_path: &Path,
    canonical_base_path: &Path,
    canonical_work_path: &Path,
) -> Result<(), AppError> {
    if canonical_target_path
        != nearest_existing_canonical_path(target_path).map_err(|error| {
            AppError::Runtime(format!("failed to re-canonicalize target path: {error}"))
        })?
    {
        return Err(AppError::Validation(format!(
            "target path changed during dump resolution: {}",
            target_path.display()
        )));
    }

    if canonical_target_path == canonical_base_path {
        return Err(AppError::Validation(
            "dump target must not equal project base path".to_owned(),
        ));
    }
    if canonical_target_path == canonical_work_path {
        return Err(AppError::Validation(
            "dump target must not equal workPath".to_owned(),
        ));
    }
    if is_filesystem_root(canonical_target_path) {
        return Err(AppError::Validation(
            "dump target must not equal filesystem root".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn cleanup_platform_orphan_dirs(resolved: &ResolvedDumpTarget) -> Result<(), AppError> {
    cleanup_orphan_dirs_for(
        &resolved.platform_target_path,
        &resolved.platform_target_identity,
        &resolved.stage_root,
    )
}

/// A process that died between the two renames of a replacement leaves a backup and no
/// target. The backup is then the only copy of the directory, and it goes back before anything
/// creates an empty target: an existing target would make it look like a leftover.
/// The caller holds the target lock, so no replacement is in progress.
fn return_backup_to_a_missing_target(
    target_path: &Path,
    target_parent: &Path,
    target_identity: &str,
) -> Result<(), AppError> {
    let read_failure =
        |error: std::io::Error| AppError::Runtime(format!("failed to read target parent: {error}"));
    match std::fs::symlink_metadata(target_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return Ok(()),
        Err(error) => {
            return Err(AppError::Runtime(format!(
                "failed to inspect dump target '{}': {error}",
                target_path.display()
            )))
        }
    }
    let mut newest = None;
    for entry in std::fs::read_dir(target_parent).map_err(read_failure)? {
        let path = entry.map_err(read_failure)?.path();
        let is_backup = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".dump-backup-"));
        if !is_backup || !path.is_dir() {
            continue;
        }
        let Ok(metadata) = read_temp_dir_metadata(&path) else {
            continue;
        };
        if matches!(metadata.kind, TempDirKind::Backup)
            && !metadata.superseded
            && is_known_tool_name(&metadata.tool)
            && metadata.target_identity == target_identity
            && newest
                .as_ref()
                .is_none_or(|(created_at, _)| metadata.created_at > *created_at)
        {
            newest = Some((metadata.created_at, path));
        }
    }
    let Some((_, backup)) = newest else {
        return Ok(());
    };
    std::fs::rename(&backup, target_path).map_err(|error| {
        AppError::Runtime(format!(
            "the target '{}' is missing and its backup '{}' could not be returned: {error}",
            target_path.display(),
            backup.display()
        ))
    })?;
    tracing::warn!(
        target = %target_path.display(),
        backup = %backup.display(),
        "returned the backup of an interrupted replacement to its missing target"
    );
    if let Err(error) = remove_path_if_exists(&metadata_sidecar_path(&backup)) {
        tracing::warn!(%error, "could not remove the metadata of a returned backup");
    }
    Ok(())
}

/// Убирает устаревшие промежуточные каталоги и копии выгрузки цели: копии лежат рядом с
/// целью, промежуточные каталоги — в `stage_root`, а прежние, до #624, — тоже рядом с целью.
fn cleanup_orphan_dirs_for(
    target_path: &Path,
    target_identity: &str,
    stage_root: &Path,
) -> Result<(), AppError> {
    let target_parent = target_path.parent().ok_or_else(|| {
        AppError::Runtime(format!(
            "target path has no parent: {}",
            target_path.display()
        ))
    })?;
    if target_parent.exists() {
        return_backup_to_a_missing_target(target_path, target_parent, target_identity)?;
        remove_stale_temp_dirs(target_parent, target_identity, TempDirPlace::BesideTarget)?;
    }
    if stage_root.exists() {
        remove_stale_temp_dirs(stage_root, target_identity, TempDirPlace::StageRoot)?;
    }
    Ok(())
}

/// Где лежит каталог, который убирает [`remove_stale_temp_dirs`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum TempDirPlace {
    /// Рядом с целью — в каталоге пользователя: только `.dump-stage-*` и `.dump-backup-*` с
    /// метаданными этой цели.
    BesideTarget,
    /// `workPath/stage` — каталог раннера: там же и описи `version-file-*`, оставленные снятым
    /// процессом (#646).
    StageRoot,
}

/// Устаревшие `.dump-stage-*` и `.dump-backup-*` цели `target_identity` в каталоге `dir`, а в
/// `workPath/stage` — и каталоги `version-file-*`.
fn remove_stale_temp_dirs(
    dir: &Path,
    target_identity: &str,
    place: TempDirPlace,
) -> Result<(), AppError> {
    for entry in std::fs::read_dir(dir)
        .map_err(|error| AppError::Runtime(format!("failed to read target parent: {error}")))?
    {
        let entry = entry
            .map_err(|error| AppError::Runtime(format!("failed to read dir entry: {error}")))?;
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map_err(|error| AppError::Runtime(format!("failed to stat dir entry: {error}")))?
            .is_dir();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !is_dir {
            remove_metadata_of_a_removed_backup(&path, file_name, target_identity);
            continue;
        }
        if place == TempDirPlace::StageRoot
            && file_name.starts_with(crate::use_cases::version_file::STALE_VERSION_FILE_DIR_PREFIX)
        {
            remove_left_version_file_dir(&path, &entry)?;
            continue;
        }
        if !file_name.starts_with(".dump-stage-") && !file_name.starts_with(".dump-backup-") {
            continue;
        }

        let Ok(metadata) = read_temp_dir_metadata(&path) else {
            continue;
        };
        if !is_known_tool_name(&metadata.tool) || metadata.target_identity != target_identity {
            continue;
        }
        // Прежнее дерево завершённой замены — остаток в любом возрасте: его не успело или не
        // смогло удалить фоновое удаление (#616), или оно удаляет его прямо сейчас.
        if !metadata.is_superseded_backup()
            && (chrono::Utc::now() - metadata.created_at)
                .to_std()
                .unwrap_or_default()
                < super::ORPHAN_TTL
        {
            continue;
        }

        let removed = remove_tree_in_parallel(&path).or_else(already_gone);
        if metadata.is_superseded_backup() {
            // Прежнее дерево ни для чего не нужно: не удалось сейчас — удалится в следующий раз,
            // а выгрузку это не останавливает. Его может удалять и фоновый процесс.
            match removed.and_then(|()| remove_path_if_exists(&metadata_sidecar_path(&path))) {
                Ok(()) => {}
                Err(error) => tracing::warn!(
                    path = %path.display(),
                    %error,
                    "could not remove the replaced tree of a previous pull yet"
                ),
            }
            continue;
        }
        removed.map_err(|error| {
            AppError::Runtime(format!(
                "failed to remove stale temp dir '{}': {error}",
                path.display()
            ))
        })?;
        remove_path_if_exists(&metadata_sidecar_path(&path)).map_err(|error| {
            AppError::Runtime(format!(
                "failed to remove stale temp metadata '{}': {error}",
                metadata_sidecar_path(&path).display()
            ))
        })?;
    }

    Ok(())
}

/// Временный каталог свежей описи после загрузки (#646), оставленный снятым процессом:
/// убирается, когда он старше [`super::ORPHAN_TTL`], — моложе его может писать идущая отправка.
fn remove_left_version_file_dir(path: &Path, entry: &std::fs::DirEntry) -> Result<(), AppError> {
    let age = entry
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok());
    if age.is_none_or(|age| age < super::ORPHAN_TTL) {
        return Ok(());
    }
    std::fs::remove_dir_all(path)
        .or_else(already_gone)
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to remove stale temp dir '{}': {error}",
                path.display()
            ))
        })
}

/// Фоновое удаление уже убрало дерево, или его убирает тот, кто удаляет его одновременно.
fn already_gone(error: std::io::Error) -> std::io::Result<()> {
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error)
    }
}

/// Метаданные прежнего дерева, которое удалили, а метаданные — нет: процесс фонового удаления
/// оборвался между ними (#616).
fn remove_metadata_of_a_removed_backup(path: &Path, file_name: &str, target_identity: &str) {
    let Some(backup_name) = file_name
        .strip_suffix(".meta.json")
        .filter(|name| name.starts_with(".dump-backup-"))
    else {
        return;
    };
    let backup = path.with_file_name(backup_name);
    if !matches!(std::fs::symlink_metadata(&backup), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return;
    }
    let Ok(metadata) = read_temp_dir_metadata(&backup) else {
        return;
    };
    if !is_known_tool_name(&metadata.tool)
        || metadata.target_identity != target_identity
        || !matches!(metadata.kind, TempDirKind::Backup)
        || backup_name.strip_prefix(".dump-backup-") != Some(metadata.run_id.as_str())
        || metadata.target_path.parent() != backup.parent()
    {
        return;
    }
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(%error, path = %path.display(), "failed to remove the backup metadata");
        }
    }
}

pub(super) fn resolve_dump_edt_base_project_name(
    inventory: &SourceSetInventory<'_>,
) -> Result<String, AppError> {
    let configuration_source_sets =
        inventory.source_sets_with_purpose(SourceSetPurpose::Configuration);
    if configuration_source_sets.len() != 1 {
        let candidates = configuration_source_sets
            .iter()
            .map(|source_set| source_set.name.as_str())
            .collect::<Vec<_>>();
        return Err(AppError::Validation(format!(
            "dump EDT extension reverse sync requires exactly one configuration source-set to infer EDT base project name; found [{}]",
            candidates.join(", ")
        )));
    }

    read_edt_project_name(
        &inventory.source_path(configuration_source_sets[0]),
        &format!(
            "configuration source-set '{}'",
            configuration_source_sets[0].name
        ),
    )
}

fn read_edt_project_name(path: &Path, label: &str) -> Result<String, AppError> {
    edt_project::read_project_name_from_dir(path).map_err(|error| {
        AppError::Validation(format!(
            "{label} must contain a valid EDT project name in '.project': {error}"
        ))
    })
}

pub(crate) fn build_designer_dsl<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    runner: &'a dyn ProcessRunner,
    source_set_name: &str,
    action: &str,
) -> Result<DesignerDsl<'a>, AppError> {
    let log_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        AppError::Runtime(format!("failed to create platform logs dir: {error}"))
    })?;
    let log_file = log_dir.join(format!("dump-{source_set_name}-{action}.log"));

    Ok(DesignerDsl::new(
        binary.to_path_buf(),
        config.v8_connection(),
        runner,
        Some(log_file),
        context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
    ))
}

/// `ibcmd` к базе проекта: единственное место, где он строится
/// (`INV.USE-CASES.AN-IBCMD-RUN-HOLDS-THE-DATA-DIRECTORY-OF-THE-PROJECT`). Класс прерывания задаёт
/// шаг: запись в базу — критическая фаза, чтение снимается.
pub(crate) fn build_ibcmd_dsl<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    runner: &'a dyn ProcessRunner,
    safety: InterruptionSafetyClass,
) -> Result<IbcmdDsl<'a>, AppError> {
    let connection = IbcmdConnection::from_infobase(&config.infobase).map_err(map_ibcmd_error)?;
    let data_path = ibcmd_data_path(config)?;

    Ok(IbcmdDsl::new(
        binary.to_path_buf(),
        connection,
        runner,
        context.process_policy(safety, None),
    )
    .with_data_path(data_path))
}

/// Каталог данных автономного сервера `ibcmd` в `workPath` проекта. Блокируется он, а не база,
/// и умолчательный — один на пользователя: без своего каталога запуски в разных проектах
/// отказывают друг другу (`rc=254`). Команды к базе проекта идут под блокировкой `workPath`,
/// поэтому внутри проекта каталог никто не оспаривает.
fn ibcmd_data_path(config: &AppConfig) -> Result<PathBuf, AppError> {
    let data_path = config.work_path.join("ibcmd-data");
    std::fs::create_dir_all(&data_path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to create IBCMD standalone-server data directory '{}': {error}",
            data_path.display()
        ))
    })?;
    Ok(data_path)
}

pub(crate) fn map_ibcmd_error(error: IbcmdError) -> AppError {
    AppError::from(error)
}

pub(super) fn merge_optional_messages(
    left: Option<String>,
    right: Option<String>,
) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(format!("{left}; {right}")),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(super) fn dump_publication_warning(
    command: crate::use_cases::context::CommandName,
    deferred_interruption: Option<crate::use_cases::context::ExecutionInterruption>,
) -> Option<String> {
    deferred_interruption.map(|interruption| {
        interruption::deferred_interruption_warning_for_command(
            "dump publication completed",
            command,
            interruption,
        )
    })
}

pub(super) fn validate_dump_objects(
    mode: &DumpMode,
    objects: &[String],
) -> Result<Option<Vec<PartialDumpSelector>>, AppError> {
    match mode {
        DumpMode::Partial => normalize_partial_objects(objects).map(Some),
        DumpMode::Full | DumpMode::Incremental if !objects.is_empty() => Err(AppError::Validation(
            super::NON_PARTIAL_OBJECTS_ERROR.to_owned(),
        )),
        DumpMode::Full | DumpMode::Incremental => Ok(None),
    }
}

fn normalize_partial_objects(objects: &[String]) -> Result<Vec<PartialDumpSelector>, AppError> {
    if objects.is_empty() {
        return Err(AppError::Validation(
            super::PARTIAL_OBJECTS_REQUIRED_ERROR.to_owned(),
        ));
    }

    objects
        .iter()
        .map(|object| PartialDumpSelector::parse(object))
        .collect()
}

pub(super) fn create_dump_object_list_file(
    work_path: &Path,
    objects: &[PartialDumpSelector],
) -> Result<TempPath, AppError> {
    create_dump_object_list_file_with(work_path, objects, write_partial_object_list)
}

/// Отдаёт путь без дескриптора: на Windows Конфигуратор не читает список, пока файл открыт
/// на запись, и отвечает «Ошибка чтения файла-списка выгружаемых объектов метаданных».
pub(super) fn create_dump_object_list_file_with<F>(
    work_path: &Path,
    objects: &[PartialDumpSelector],
    write_list: F,
) -> Result<TempPath, AppError>
where
    F: FnOnce(&mut NamedTempFile, &[PartialDumpSelector]) -> std::io::Result<()>,
{
    let mut list_file = dump_object_list_file(work_path).map_err(|error| {
        AppError::Runtime(format!("failed to create partial dump list: {error}"))
    })?;
    write_list(&mut list_file, objects).map_err(|error| {
        AppError::Runtime(format!("failed to write partial dump list: {error}"))
    })?;
    Ok(list_file.into_temp_path())
}

fn write_partial_object_list(
    list_file: &mut NamedTempFile,
    objects: &[PartialDumpSelector],
) -> std::io::Result<()> {
    let writer = list_file.as_file_mut();
    writer.set_len(0)?;
    for object in objects {
        writeln!(writer, "{}", object.normalized())?;
    }
    writer.flush()
}

pub(super) fn ibcmd_partial_warning(resolved: &ResolvedDumpTarget) -> String {
    match resolved.extension.as_deref() {
        Some(extension) => format!(
            "IBCMD does not support object-scoped partial dump; ran incremental export for extension '{extension}' instead"
        ),
        None => format!(
            "IBCMD does not support object-scoped partial dump; ran incremental export for source-set '{}' instead",
            resolved.source_set_name
        ),
    }
}

pub(super) fn decorate_ibcmd_partial_error(error: AppError, warning: &str) -> AppError {
    error.with_context(warning)
}

pub(super) fn ensure_platform_success(
    action: &str,
    resolved: &ResolvedDumpTarget,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    ensure_success_of(action, "source-set", &resolved.source_set_name, result)
}

/// Исход вызова платформы по коду выхода; отказ называет действие, предмет и что платформа
/// написала. Текст ответа решения не принимает.
pub(crate) fn ensure_success_of(
    action: &str,
    target_kind: &str,
    target: &str,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    let Err(code) = result.process.outcome() else {
        return Ok(());
    };

    Err(AppError::Platform(format_ibcmd_failure_details(
        action,
        target_kind,
        target,
        code.get(),
        &result.process.stdout,
        &result.process.stderr,
        result.platform_log.as_deref(),
        result.platform_log_path.as_deref(),
    )))
}

pub(super) fn empty_result(
    mode: DumpMode,
    started: Instant,
    source_set: Option<String>,
    extension: Option<String>,
    selectors: Option<Vec<DumpSelectorResult>>,
    target_path: Option<PathBuf>,
    message: Option<String>,
) -> DumpResult {
    DumpResult {
        provider: None,
        // Ответ без исполнения: все одиннадцать мест, которые его строят, лежат выше
        // запуска платформы — десять отказов и само превью. Настоящий запуск строит ответ
        // буквально и ставит признак сам.
        provider_dispatched: false,
        up_to_date: false,
        ok: false,
        source_set,
        extension,
        selectors,
        mode: mode.into(),
        requested_mode: mode,
        mode_reason: None,
        target_path: target_path.unwrap_or_default(),
        platform_log_path: None,
        duration_ms: started.elapsed().as_millis() as u64,
        message,
        losses: None,
    }
}

/// Поколение базы, прочитанное инструментом выгрузки — Конфигуратором или `ibcmd`, — до или
/// после неё. Другой инструмент, отмена и сбой чтения — отсутствие ответа: выгрузку оно не
/// останавливает, а память о поколении тогда не пишется.
pub(super) fn read_dump_generation(
    context: &ExecutionContext,
    config: &AppConfig,
    provider: Provider,
    binary: &Path,
    runner: &dyn ProcessRunner,
    resolved: &ResolvedDumpTarget,
) -> Option<String> {
    use crate::use_cases::generation_reader::{
        designer_log_file, read_generation, GenerationProcess,
    };
    if matches!(
        provider,
        Provider::Agent | Provider::IbcmdRs | Provider::Webinst
    ) {
        return None;
    }
    let process = || match provider {
        Provider::Designer => designer_log_file(
            config,
            &format!("dump-{}-generation", resolved.source_set_name),
        )
        .map(|log_file| GenerationProcess::Designer {
            binary,
            runner,
            log_file,
        }),
        Provider::Ibcmd => Ok(GenerationProcess::Ibcmd { binary, runner }),
        Provider::Agent | Provider::IbcmdRs | Provider::Webinst => Err(AppError::capability(
            format!("{provider} reads no generation by a platform process"),
        )),
    };
    read_generation(context, config, process, resolved.extension.as_deref()).unwrap_or_else(
        |error| {
            tracing::debug!(%error, "the configuration generation is not known");
            None
        },
    )
}

#[cfg(test)]
mod tests {
    use super::decorate_ibcmd_partial_error;
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;

    #[test]
    fn ibcmd_partial_warning_preserves_typed_process_error_source() {
        let warning = "IBCMD does not support object-scoped partial dump";
        let error = decorate_ibcmd_partial_error(
            AppError::PlatformProcess(ProcessError::SpawnFailed {
                cmd: "ibcmd config export".to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing ibcmd"),
            }),
            warning,
        );

        assert!(error.to_string().contains(warning));
        assert!(matches!(
            error,
            AppError::PlatformProcessContext {
                source: ProcessError::SpawnFailed { .. },
                ..
            }
        ));
    }
}
