use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;

use crate::support::error::AppError;
use crate::support::fs::{
    ensure_dir, is_known_tool_name, metadata_sidecar_path, move_file, read_temp_dir_metadata,
    remove_path_if_exists, replace_file_atomically, swap_dir_atomically, write_temp_dir_metadata,
    ReplaceFileFailureState, SupersededBackup, TempDirKind, TempDirMetadata,
};
use crate::use_cases::context::{ExecutionContext, ExecutionInterruption};
use crate::use_cases::destruction_guard::{
    guard_replacement, Destruction, DestructionConsent, Losses,
};
use crate::use_cases::interruption;

const ORPHAN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone)]
pub(super) struct StagedPublication {
    staging_path: PathBuf,
    target_path: PathBuf,
    run_id: String,
    target_identity: String,
    previous_target_present: bool,
    externally_owned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StagedPublicationOutcome {
    pub cleanup_warning: Option<String>,
    /// Что замена уничтожила по согласию; пусто у каталога раннера и без согласия.
    pub discarded: Losses,
    pub deferred_interruption: Option<ExecutionInterruption>,
    pub previous_target_present: bool,
}

/// Как уходит прежнее дерево после успешной подмены каталога.
pub(super) enum BackupRemoval<'a> {
    /// На месте, до ответа команды.
    Inline,
    /// Отделённым процессом раннера, который переживает ответ команды (#616,
    /// `INV.USE-CASES.A-REPLACED-TREE-IS-REMOVED-IN-THE-BACKGROUND`). Чего он не успел, доберёт
    /// уборка остатков следующей команды: прежнее дерево помечено `superseded`. Процесс не
    /// запустился — дерево удаляется на месте.
    Background,
    /// Скрыть резерв сразу, а удаление запустить после обновления памяти.
    AfterMemory(&'a mut Option<SupersededBackup>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicationFailureState {
    Unchanged,
    Restored,
    Uncertain,
}

#[derive(Debug)]
pub(super) struct StagedPublicationError {
    pub error: AppError,
    pub target_state: PublicationFailureState,
}

impl StagedPublication {
    pub fn externally_owned_dir(
        stage: &Path,
        target: &Path,
        target_identity: &str,
    ) -> Result<Self, AppError> {
        let metadata = std::fs::symlink_metadata(stage).map_err(|error| {
            AppError::Validation(format!("cannot inspect external full-pull stage: {error}"))
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(AppError::Validation(
                "external full-pull stage must be an existing directory".into(),
            ));
        }
        let mut publication =
            Self::new(target, target_identity, stage.to_path_buf(), make_run_id());
        publication.externally_owned = true;
        Ok(publication)
    }

    /// Промежуточный каталог рядом с целью, в её родителе.
    pub fn prepare_dir(
        target_path: &Path,
        target_identity: &str,
        stage_prefix: &str,
    ) -> Result<Self, AppError> {
        let target_parent = crate::support::fs::parent_dir(target_path).ok_or_else(|| {
            AppError::Runtime(format!(
                "target path has no parent: {}",
                target_path.display()
            ))
        })?;
        Self::prepare_dir_in(target_parent, target_path, target_identity, stage_prefix)
    }

    /// Промежуточный каталог в `stage_root`, а не рядом с целью: так выгрузка не оставляет
    /// временных файлов в дереве исходников. Публикация остаётся переименованием, пока
    /// `stage_root` на томе цели — это проверяет разрешение цели выгрузки
    /// (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
    pub fn prepare_dir_in(
        stage_root: &Path,
        target_path: &Path,
        target_identity: &str,
        stage_prefix: &str,
    ) -> Result<Self, AppError> {
        let target_parent = crate::support::fs::parent_dir(target_path).ok_or_else(|| {
            AppError::Runtime(format!(
                "target path has no parent: {}",
                target_path.display()
            ))
        })?;
        ensure_dir(target_parent).map_err(|error| {
            AppError::Runtime(format!("failed to create target parent dir: {error}"))
        })?;
        ensure_dir(stage_root).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the stage directory '{}': {error}",
                stage_root.display()
            ))
        })?;
        let run_id = make_run_id();
        let publication = Self::new(
            target_path,
            target_identity,
            stage_root.join(format!("{stage_prefix}-{run_id}")),
            run_id,
        );
        if publication.staging_path.exists() {
            return Err(AppError::Runtime(format!(
                "staging dir already exists unexpectedly: {}",
                publication.staging_path.display()
            )));
        }
        std::fs::create_dir(&publication.staging_path)
            .map_err(|error| AppError::Runtime(format!("failed to create staging dir: {error}")))?;
        publication.write_stage_metadata("failed to write stage metadata")?;
        Ok(publication)
    }

    pub fn prepare_file(
        target_path: &Path,
        target_identity: &str,
        stage_prefix: &str,
        extension: &str,
    ) -> Result<Self, AppError> {
        let target_parent = crate::support::fs::parent_dir(target_path).ok_or_else(|| {
            AppError::Runtime(format!(
                "target path has no parent: {}",
                target_path.display()
            ))
        })?;
        ensure_dir(target_parent).map_err(|error| {
            AppError::Runtime(format!("failed to create target parent dir: {error}"))
        })?;

        let run_id = make_run_id();
        let publication = Self::new(
            target_path,
            target_identity,
            target_parent.join(format!("{stage_prefix}-{run_id}.{extension}")),
            run_id,
        );
        if publication.staging_path.exists() {
            return Err(AppError::Runtime(format!(
                "staging file already exists unexpectedly: {}",
                publication.staging_path.display()
            )));
        }
        publication.write_stage_metadata("failed to write staging metadata")?;
        Ok(publication)
    }

    pub fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn cleanup_failure(&self, error: AppError) -> AppError {
        if self.externally_owned {
            error
        } else {
            cleanup_staging_path(&self.staging_path, error)
        }
    }

    pub fn publish_dir(
        &self,
        context: &ExecutionContext,
        backup_prefix: &str,
        error_prefix: &str,
        consent: &DestructionConsent,
        regenerated: &[&str],
        removal: BackupRemoval<'_>,
    ) -> Result<StagedPublicationOutcome, AppError> {
        // Сторож спрашивает до подмены: после неё прежнего содержимого уже нет.
        let discarded = guard_replacement(
            context,
            &self.target_path,
            consent,
            regenerated,
            Destruction::Replace,
        )?;
        if let Some(error) = interruption_before_publish(context, "staged directory publication") {
            return Err(error);
        }
        tracing::debug!(target = %self.target_path.display(), "replacing the target directory");
        let publish_phase = context.run_no_process_critical_phase(|| {
            swap_dir_atomically(
                &self.staging_path,
                &self.target_path,
                &self.run_id,
                &self.target_identity,
                backup_prefix,
            )
            .map_err(|error| AppError::Runtime(format!("{error_prefix}: {error}")))
        })?;
        let swapped = publish_phase.value;
        let removal_warning = swapped
            .backup
            .and_then(|backup| remove_superseded(backup, removal));
        let warnings: Vec<String> = swapped
            .cleanup_warning
            .into_iter()
            .chain(removal_warning)
            .collect();
        Ok(StagedPublicationOutcome {
            cleanup_warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
            discarded,
            deferred_interruption: publish_phase.deferred_interruption,
            previous_target_present: self.previous_target_present,
        })
    }

    /// Кладёт каталог `produced`, который выгрузка написала внутри промежуточного, поверх
    /// целевого: файлы переносятся на место переименованием, то, чего в выгрузке нет,
    /// остаётся. Файл `kept` в корне `produced` (опись версий) остаётся в стадии: в каталог
    /// набора раннер её не кладёт (#646), а забирает в свою память. Промежуточный каталог
    /// убирает [`Self::discard`].
    pub fn lay_over_dir(
        &self,
        context: &ExecutionContext,
        error_prefix: &str,
        produced: &Path,
        kept: &str,
    ) -> Result<(), AppError> {
        if let Some(error) = interruption_before_publish(context, "staged directory overlay") {
            return Err(self.cleanup_failure(error));
        }
        if !produced.is_dir() {
            return Ok(());
        }
        move_tree_over(produced, &self.target_path, Path::new(kept)).map_err(|error| {
            self.cleanup_failure(AppError::Runtime(format!("{error_prefix}: {error}")))
        })
    }

    /// Убирает промежуточный каталог. Возвращает предупреждение, если убрать его не удалось:
    /// его узнает уборка по описанию.
    pub fn discard(&self) -> Option<String> {
        let sidecar = metadata_sidecar_path(&self.staging_path);
        let cleanup = remove_path_if_exists(&self.staging_path)
            .and_then(|()| remove_path_if_exists(&sidecar));
        cleanup.err().map(|error| {
            format!(
                "failed to remove the dump stage '{}': {error}; the next run removes it once it is stale",
                self.staging_path.display()
            )
        })
    }

    pub fn publish_file(
        &self,
        context: &ExecutionContext,
        error_prefix: &str,
    ) -> Result<StagedPublicationOutcome, AppError> {
        self.publish_file_with_state(context, error_prefix)
            .map_err(|failure| failure.error)
    }

    #[allow(clippy::result_large_err)] // Typed rollback state must stay attached to the publication error.
    pub fn publish_file_with_state(
        &self,
        context: &ExecutionContext,
        error_prefix: &str,
    ) -> Result<StagedPublicationOutcome, StagedPublicationError> {
        if let Some(error) = interruption_before_publish(context, "staged file publication") {
            return Err(StagedPublicationError {
                error,
                target_state: PublicationFailureState::Unchanged,
            });
        }
        let publish_phase = context.run_no_process_critical_phase(|| {
            replace_file_atomically(
                &self.staging_path,
                &self.target_path,
                &self.run_id,
                &self.target_identity,
            )
            .map_err(|error| StagedPublicationError {
                target_state: match error.target_state {
                    ReplaceFileFailureState::Unchanged => PublicationFailureState::Unchanged,
                    ReplaceFileFailureState::Restored => PublicationFailureState::Restored,
                    ReplaceFileFailureState::Uncertain => PublicationFailureState::Uncertain,
                },
                error: AppError::Runtime(format!("{error_prefix}: {error}")),
            })
        })?;
        Ok(StagedPublicationOutcome {
            cleanup_warning: publish_phase.value.cleanup_warning,
            // Замена файла сторожа не спрашивает: уничтоженного по согласию у неё нет.
            discarded: Losses::default(),
            deferred_interruption: publish_phase.deferred_interruption,
            previous_target_present: publish_phase.value.previous_target_present,
        })
    }

    fn new(
        target_path: &Path,
        target_identity: &str,
        staging_path: PathBuf,
        run_id: String,
    ) -> Self {
        Self {
            staging_path,
            target_path: target_path.to_path_buf(),
            run_id,
            target_identity: target_identity.to_owned(),
            previous_target_present: target_path.exists(),
            externally_owned: false,
        }
    }

    fn write_stage_metadata(&self, message: &str) -> Result<(), AppError> {
        write_temp_dir_metadata(
            &self.staging_path,
            TempDirKind::Stage,
            &self.run_id,
            &self.target_path,
            &self.target_identity,
        )
        .map_err(|error| AppError::Runtime(format!("{message}: {error}")))
    }
}

/// Убирает прежнее дерево успешной подмены так, как велено; текст — предупреждение ответа.
fn remove_superseded(backup: SupersededBackup, removal: BackupRemoval<'_>) -> Option<String> {
    match removal {
        BackupRemoval::Inline => backup.remove(),
        BackupRemoval::Background | BackupRemoval::AfterMemory(_) => {
            if !std::fs::symlink_metadata(backup.path())
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            {
                return backup.remove();
            }
            if let Err(refusal) = hide_from_git(&backup) {
                tracing::debug!(%refusal, "removing the replaced tree in place");
                return backup.remove();
            }
            if let BackupRemoval::AfterMemory(pending) = removal {
                *pending = Some(backup);
                None
            } else {
                finish_background_removal(Some(backup))
            }
        }
    }
}

pub(super) fn finish_background_removal(backup: Option<SupersededBackup>) -> Option<String> {
    let backup = backup?;
    match crate::platform::backup_removal::spawn(backup.path()) {
        Ok(pid) => {
            tracing::debug!(
                backup = %backup.path().display(),
                pid,
                "removing the replaced tree in the background"
            );
            None
        }
        Err(refusal) => {
            tracing::debug!(%refusal, "removing the replaced tree in place");
            backup.remove()
        }
    }
}

/// Прежнее дерево лежит рядом с набором и после ответа: git его не видит, и `git add -A`
/// сразу после `pull --force` его не возьмёт.
fn hide_from_git(backup: &SupersededBackup) -> std::io::Result<()> {
    use std::io::Write as _;

    // Replace the entry itself: a previous .gitignore may refer to another file.
    let mut ignore = tempfile::NamedTempFile::new_in(backup.path())?;
    ignore.write_all(b"*\n")?;
    ignore
        .persist(backup.path().join(".gitignore"))
        .map_err(|error| error.error)?;
    Ok(())
}

/// Переносит дерево `source` поверх `target`, кроме файла `skip` в корне: каталоги
/// создаются, файлы заменяются переименованием, лишнее в `target` остаётся.
fn move_tree_over(source: &Path, target: &Path, skip: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if Path::new(&name) == skip {
            continue;
        }
        let destination = target.join(&name);
        if entry.file_type()?.is_dir() {
            move_tree_over(&entry.path(), &destination, Path::new(""))?;
        } else {
            move_file(&entry.path(), &destination)?;
        }
    }
    Ok(())
}

pub(super) fn cleanup_staging_path(staging_path: &Path, error: AppError) -> AppError {
    let sidecar = metadata_sidecar_path(staging_path);
    let _ = remove_path_if_exists(staging_path);
    let _ = remove_path_if_exists(&sidecar);
    error
}

/// Reintroduction guard: this is the single scanner for stale staged/backup
/// files published through [`StagedPublication`]. Callers supply only naming
/// policy; metadata ownership and target identity remain centralized here.
pub(super) fn cleanup_owned_orphan_files(
    scan_roots: &[PathBuf],
    target_path: &Path,
    target_identity: &str,
    stage_prefixes: &[&str],
    backup_prefixes: &[&str],
    allow_descendant_targets: bool,
) -> Result<(), AppError> {
    let mut roots = scan_roots.to_vec();
    roots.sort();
    roots.dedup();

    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&root)
            .map_err(|error| AppError::Runtime(format!("failed to read output dir: {error}")))?
        {
            let entry = entry
                .map_err(|error| AppError::Runtime(format!("failed to read dir entry: {error}")))?;
            let path = entry.path();
            let (temp_path, metadata_path) = orphan_cleanup_paths(&path);
            let Ok(metadata) = read_orphan_metadata(&temp_path, &metadata_path) else {
                continue;
            };
            let Some(temp_name) = temp_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let target_matches = metadata.target_path == target_path
                || (allow_descendant_targets && metadata.target_path.starts_with(target_path));
            if !is_known_tool_name(&metadata.tool)
                || !target_matches
                || metadata.target_identity != target_identity
                || !orphan_name_matches_contract(
                    temp_name,
                    target_path,
                    &metadata,
                    stage_prefixes,
                    backup_prefixes,
                )
            {
                continue;
            }
            if !is_a_leftover(&metadata, &temp_path) {
                continue;
            }

            remove_path_if_exists(&temp_path).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to remove stale publication temp '{}': {error}",
                    temp_path.display()
                ))
            })?;
            remove_path_if_exists(&metadata_path).map_err(|error| {
                AppError::Runtime(format!(
                    "failed to remove stale publication metadata '{}': {error}",
                    metadata_path.display()
                ))
            })?;
        }
    }
    Ok(())
}

/// A backup of a finished replacement is a leftover at any age: its tree is older than the
/// target, and the background removal (#616) may have left part of it. Anything else is a
/// leftover once stale. Neither goes while it may be the only copy of its target.
fn is_a_leftover(metadata: &TempDirMetadata, temp_path: &Path) -> bool {
    (metadata.is_superseded_backup()
        || (Utc::now() - metadata.created_at)
            .to_std()
            .unwrap_or_default()
            >= ORPHAN_TTL)
        && !backup_is_the_only_copy(&metadata.kind, temp_path, &metadata.target_path)
}

/// A process that died between the two renames of a replacement leaves the backup and no
/// target: then the backup is not a leftover but the only copy, whatever its age. A target
/// that cannot be inspected counts as missing: the backup goes only beside a target seen.
fn backup_is_the_only_copy(kind: &TempDirKind, backup: &Path, target: &Path) -> bool {
    matches!(kind, TempDirKind::Backup)
        && std::fs::symlink_metadata(backup).is_ok()
        && std::fs::symlink_metadata(target).is_err()
}

fn orphan_name_matches_contract(
    file_name: &str,
    target_path: &Path,
    metadata: &TempDirMetadata,
    stage_prefixes: &[&str],
    backup_prefixes: &[&str],
) -> bool {
    match metadata.kind {
        // Временная база раннера — каталог `<префикс>-<запуск>` под корнем временных баз:
        // её префикс `throwaway_infobase` передаёт как префикс промежуточных копий.
        TempDirKind::Stage | TempDirKind::ThrowawayInfobase => {
            stage_prefixes
                .iter()
                .any(|prefix| file_name.starts_with(prefix))
                && file_name.contains(&metadata.run_id)
        }
        TempDirKind::Backup => {
            let named_backup = backup_prefixes
                .iter()
                .any(|prefix| file_name.starts_with(prefix))
                && file_name.contains(&metadata.run_id);
            let file_backup = target_path
                .file_name()
                .map(|target_name| {
                    file_name
                        == format!(
                            ".{}.backup-{}",
                            target_name.to_string_lossy(),
                            metadata.run_id
                        )
                })
                .unwrap_or(false);
            named_backup || file_backup
        }
    }
}

fn orphan_cleanup_paths(path: &Path) -> (PathBuf, PathBuf) {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return (path.to_path_buf(), metadata_sidecar_path(path));
    };
    let Some(temp_name) = file_name.strip_suffix(".meta.json") else {
        return (path.to_path_buf(), metadata_sidecar_path(path));
    };
    (
        crate::support::fs::parent_dir(path)
            .unwrap_or_else(|| Path::new("."))
            .join(temp_name),
        path.to_path_buf(),
    )
}

fn read_orphan_metadata(
    temp_path: &Path,
    metadata_path: &Path,
) -> std::io::Result<TempDirMetadata> {
    if metadata_path == metadata_sidecar_path(temp_path) {
        return read_temp_dir_metadata(temp_path);
    }
    let raw = std::fs::read(metadata_path)?;
    serde_json::from_slice(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[must_use = "an interruption must stop the publication"]
pub(super) fn interruption_before_publish(
    context: &ExecutionContext,
    safe_point: impl Into<String>,
) -> Option<AppError> {
    interruption::interruption_before_safe_point(context, safe_point.into())
}

pub(super) fn make_run_id() -> String {
    let timestamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{}-{timestamp:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use crate::use_cases::destruction_guard::DestructionConsent;
    use std::fs;

    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use crate::support::error::AppError;
    use crate::support::fs::{
        metadata_sidecar_path, read_temp_dir_metadata, write_temp_dir_metadata, TempDirKind,
    };
    use crate::use_cases::context::{CommandName, ExecutionContext};

    use super::{
        cleanup_owned_orphan_files, cleanup_staging_path, interruption_before_publish,
        StagedPublication,
    };

    fn make_stale(path: &std::path::Path) {
        let metadata_path = metadata_sidecar_path(path);
        let mut metadata = read_temp_dir_metadata(path).expect("metadata");
        metadata.created_at -= chrono::Duration::days(2);
        fs::write(
            metadata_path,
            serde_json::to_vec_pretty(&metadata).expect("json"),
        )
        .expect("stale metadata");
    }

    #[test]
    fn prepare_dir_creates_stage_dir_and_metadata_then_publishes() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let publication =
            StagedPublication::prepare_dir(&target, "identity", ".stage").expect("prepare");
        fs::write(publication.staging_path().join("payload.txt"), "payload").expect("payload");
        let stage_metadata = metadata_sidecar_path(publication.staging_path());

        let outcome = publication
            .publish_dir(
                &ExecutionContext::cli(CommandName::Dump),
                ".backup",
                "failed to publish staged test dir",
                &DestructionConsent::RunnerOwned,
                &[],
                super::BackupRemoval::Inline,
            )
            .expect("publish");

        assert_eq!(outcome.deferred_interruption, None);
        assert_eq!(
            fs::read_to_string(target.join("payload.txt")).expect("target"),
            "payload"
        );
        assert!(!stage_metadata.exists());
    }

    /// Промежуточный результат лежит в одном каталоге с целью: публикация — это
    /// переименование, а не копирование через границу файловой системы.
    #[test]
    fn a_staging_path_shares_the_parent_directory_of_its_target() {
        let dir = tempdir().expect("tempdir");
        let target_dir = dir.path().join("nested").join("target");
        let publication =
            StagedPublication::prepare_dir(&target_dir, "identity", ".stage").expect("prepare");
        assert_eq!(publication.staging_path().parent(), target_dir.parent());

        let target_file = dir.path().join("nested").join("main.cf");
        let publication = StagedPublication::prepare_file(&target_file, "identity", ".stage", "cf")
            .expect("prepare");
        assert_eq!(publication.staging_path().parent(), target_file.parent());
    }

    #[test]
    fn prepare_file_writes_metadata_without_materializing_stage_file() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");

        let publication =
            StagedPublication::prepare_file(&target, "identity", ".stage", "cf").expect("prepare");

        assert!(!publication.staging_path().exists());
        let metadata = read_temp_dir_metadata(publication.staging_path()).expect("metadata");
        assert_eq!(metadata.target_identity, "identity");
        assert_eq!(metadata.target_path, target);
    }

    #[test]
    fn publish_file_uses_caller_created_stage_file() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        let publication =
            StagedPublication::prepare_file(&target, "identity", ".stage", "cf").expect("prepare");
        fs::write(publication.staging_path(), "package").expect("stage");

        let outcome = publication
            .publish_file(
                &ExecutionContext::cli(CommandName::Artifacts),
                "failed to publish staged test file",
            )
            .expect("publish");

        assert_eq!(fs::read_to_string(target).expect("target"), "package");
        assert!(!outcome.previous_target_present);
    }

    #[test]
    fn file_publication_observes_target_presence_at_commit_time() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        let publication =
            StagedPublication::prepare_file(&target, "identity", ".stage", "cf").expect("prepare");
        fs::write(publication.staging_path(), "package").expect("stage");
        fs::write(&target, "existing").expect("target created after prepare");

        let outcome = publication
            .publish_file(
                &ExecutionContext::cli(CommandName::Artifacts),
                "failed to publish staged test file",
            )
            .expect("publish");

        assert!(outcome.previous_target_present);
        assert_eq!(fs::read_to_string(target).expect("target"), "package");
    }

    #[test]
    fn explicit_cleanup_policy_removes_stage_path_and_sidecar() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let publication =
            StagedPublication::prepare_dir(&target, "identity", ".stage").expect("prepare");
        let metadata = metadata_sidecar_path(publication.staging_path());

        let error = cleanup_staging_path(
            publication.staging_path(),
            AppError::Runtime("failed before publish".to_owned()),
        );

        assert_eq!(error.to_string(), "runtime error: failed before publish");
        assert!(!publication.staging_path().exists());
        assert!(!metadata.exists());
    }

    #[test]
    fn orphan_cleanup_requires_exact_target_kind_and_run_name_contract() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        let wrong_target = dir.path().join("other.cf");
        let wrong_target_stage = dir.path().join(".stage-wrong-target.cf");
        let wrong_kind_stage = dir.path().join(".stage-wrong-kind.cf");
        let unrelated_backup = dir.path().join("unrelated.backup-backup-run");
        fs::write(&wrong_target_stage, "stage").expect("stage");
        fs::write(&wrong_kind_stage, "stage").expect("stage");
        fs::write(&unrelated_backup, "backup").expect("backup");
        write_temp_dir_metadata(
            &wrong_target_stage,
            TempDirKind::Stage,
            "wrong-target",
            &wrong_target,
            "identity",
        )
        .expect("wrong target metadata");
        write_temp_dir_metadata(
            &wrong_kind_stage,
            TempDirKind::Backup,
            "wrong-kind",
            &target,
            "identity",
        )
        .expect("wrong kind metadata");
        write_temp_dir_metadata(
            &unrelated_backup,
            TempDirKind::Backup,
            "backup-run",
            &target,
            "identity",
        )
        .expect("unrelated backup metadata");
        make_stale(&wrong_target_stage);
        make_stale(&wrong_kind_stage);
        make_stale(&unrelated_backup);

        // Рядом с целью резерв — обычный след: выжить он может только по договору об имени.
        fs::write(&target, "target").expect("target");
        cleanup_owned_orphan_files(
            &[dir.path().to_path_buf()],
            &target,
            "identity",
            &[".stage-"],
            &[],
            false,
        )
        .expect("cleanup");

        assert!(wrong_target_stage.exists());
        assert!(wrong_kind_stage.exists());
        assert!(unrelated_backup.exists());
    }

    #[test]
    fn cleanup_keeps_a_stale_backup_when_its_target_is_gone() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        let backup = dir.path().join(".target.cf.backup-run");
        fs::write(&backup, "kept").expect("backup");
        write_temp_dir_metadata(&backup, TempDirKind::Backup, "run", &target, "identity")
            .expect("metadata");
        let sidecar = metadata_sidecar_path(&backup);
        let mut metadata: crate::support::fs::TempDirMetadata =
            serde_json::from_slice(&fs::read(&sidecar).expect("read")).expect("json");
        metadata.created_at = chrono::Utc::now() - chrono::Duration::days(2);
        fs::write(&sidecar, serde_json::to_vec(&metadata).expect("json")).expect("rewrite");

        cleanup_owned_orphan_files(
            &[dir.path().to_path_buf()],
            &target,
            "identity",
            &[],
            &[],
            false,
        )
        .expect("cleanup");
        assert!(backup.exists(), "the target is gone: this is the only copy");

        fs::write(&target, "target").expect("target");
        cleanup_owned_orphan_files(
            &[dir.path().to_path_buf()],
            &target,
            "identity",
            &[],
            &[],
            false,
        )
        .expect("cleanup");
        assert!(
            !backup.exists(),
            "beside its target the backup is a leftover"
        );
    }

    /// Прежнее дерево завершённой замены — остаток в любом возрасте (#616): фоновое удаление
    /// могло не успеть, и следующая команда добирает его той же уборкой.
    #[test]
    fn cleanup_removes_a_fresh_superseded_backup() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        fs::create_dir_all(&target).expect("target");
        let backup = dir.path().join(".backup-run");
        fs::create_dir_all(backup.join("nested")).expect("backup");
        fs::write(backup.join("nested").join("old.txt"), "old").expect("old");
        write_temp_dir_metadata(&backup, TempDirKind::Backup, "run", &target, "identity")
            .expect("metadata");
        let sidecar = metadata_sidecar_path(&backup);
        let mut metadata = read_temp_dir_metadata(&backup).expect("metadata");
        metadata.superseded = true;
        fs::write(&sidecar, serde_json::to_vec(&metadata).expect("json")).expect("mark");

        cleanup_owned_orphan_files(
            &[dir.path().to_path_buf()],
            &target,
            "identity",
            &[],
            &[".backup-"],
            false,
        )
        .expect("cleanup");

        assert!(!backup.exists());
        assert!(!sidecar.exists());
    }

    /// Успешная подмена с фоновым удалением в процессе, который раннером не является, удаляет
    /// прежнее дерево на месте, как раньше.
    #[test]
    fn background_removal_outside_the_runner_removes_the_previous_tree_in_place() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        fs::create_dir_all(&target).expect("target");
        fs::write(target.join("old.txt"), "old").expect("old");
        let publication =
            StagedPublication::prepare_dir(&target, "identity", ".stage").expect("prepare");
        fs::write(publication.staging_path().join("payload.txt"), "payload").expect("payload");

        let outcome = publication
            .publish_dir(
                &ExecutionContext::cli(CommandName::Dump),
                ".backup",
                "failed to publish staged test dir",
                &DestructionConsent::RunnerOwned,
                &[],
                super::BackupRemoval::Background,
            )
            .expect("publish");

        assert_eq!(outcome.cleanup_warning, None);
        assert!(target.join("payload.txt").exists());
        let left: Vec<_> = fs::read_dir(dir.path())
            .expect("dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("target")]);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_replaced_directory_link_does_not_change_its_referent() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let referent = dir.path().join("referent");
        fs::create_dir(&referent).expect("referent");
        fs::write(referent.join(".gitignore"), "keep me").expect("original ignore");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&referent, &target).expect("link");
        #[cfg(windows)]
        junction::create(&referent, &target).expect("junction");
        let publication =
            StagedPublication::prepare_dir(&target, "identity", ".stage").expect("prepare");
        fs::write(publication.staging_path().join("payload.txt"), "payload").expect("payload");
        let mut pending = None;
        let outcome = publication
            .publish_dir(
                &ExecutionContext::cli(CommandName::Dump),
                ".backup",
                "failed to publish staged test dir",
                &DestructionConsent::RunnerOwned,
                &[],
                super::BackupRemoval::AfterMemory(&mut pending),
            )
            .expect("publish");
        assert_eq!(outcome.cleanup_warning, None);
        assert_eq!(
            fs::read_to_string(referent.join(".gitignore")).expect("original ignore"),
            "keep me"
        );
        assert!(pending.is_none(), "a directory link is removed inline");
        assert!(target.join("payload.txt").is_file());
    }

    #[test]
    fn an_ignore_link_in_a_replaced_directory_does_not_change_its_referent() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let referent = dir.path().join("original-ignore");
        fs::create_dir(&target).expect("target");
        fs::write(&referent, "keep me").expect("original ignore");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&referent, target.join(".gitignore")).expect("link");
        #[cfg(not(unix))]
        fs::hard_link(&referent, target.join(".gitignore")).expect("hard link");
        let publication =
            StagedPublication::prepare_dir(&target, "identity", ".stage").expect("prepare");
        let mut pending = None;
        publication
            .publish_dir(
                &ExecutionContext::cli(CommandName::Dump),
                ".backup",
                "failed to publish staged test dir",
                &DestructionConsent::RunnerOwned,
                &[],
                super::BackupRemoval::AfterMemory(&mut pending),
            )
            .expect("publish");

        let backup = pending.expect("ordinary directory stays until memory is written");
        assert_eq!(
            fs::read_to_string(&referent).expect("original ignore"),
            "keep me"
        );
        assert_eq!(
            fs::read_to_string(backup.path().join(".gitignore")).expect("backup ignore"),
            "*\n"
        );
        assert_eq!(backup.remove(), None);
    }

    /// Проверяется цель из записи резерва, а не корень уборки: у потомка он есть всегда.
    #[test]
    fn cleanup_looks_at_the_target_named_by_the_backup() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).expect("root");
        let target = root.join("child.cf");
        let backup = root.join(".child.cf.backup-run");
        fs::write(&backup, "kept").expect("backup");
        write_temp_dir_metadata(&backup, TempDirKind::Backup, "run", &target, "identity")
            .expect("metadata");
        let sidecar = metadata_sidecar_path(&backup);
        let mut metadata: crate::support::fs::TempDirMetadata =
            serde_json::from_slice(&fs::read(&sidecar).expect("read")).expect("json");
        metadata.created_at = chrono::Utc::now() - chrono::Duration::days(2);
        fs::write(&sidecar, serde_json::to_vec(&metadata).expect("json")).expect("rewrite");

        cleanup_owned_orphan_files(
            std::slice::from_ref(&root),
            &root,
            "identity",
            &[],
            &[],
            true,
        )
        .expect("cleanup");

        assert!(backup.exists());
    }

    #[test]
    fn interruption_check_reports_command_safe_point() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Dump).with_cancellation(cancellation);

        let error = interruption_before_publish(&context, "dump publication").expect("error");

        assert!(error
            .to_string()
            .contains("before entering dump publication safe point"));
    }

    #[test]
    fn cancelled_directory_publication_does_not_enter_critical_phase() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target");
        let publication =
            StagedPublication::prepare_dir(&target, "identity", ".stage").expect("prepare");
        fs::write(publication.staging_path().join("payload.txt"), "payload").expect("payload");
        let staging_path = publication.staging_path().to_path_buf();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Dump).with_cancellation(cancellation);

        let error = publication
            .publish_dir(
                &context,
                ".backup",
                "failed to publish staged test dir",
                &DestructionConsent::RunnerOwned,
                &[],
                super::BackupRemoval::Inline,
            )
            .expect_err("cancelled publication");

        assert!(error
            .to_string()
            .contains("before entering staged directory publication safe point"));
        assert!(!target.exists());
        assert!(staging_path.exists());
    }

    #[test]
    fn cancelled_file_publication_reports_unchanged_target() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        fs::write(&target, "original").expect("target");
        let publication =
            StagedPublication::prepare_file(&target, "identity", ".stage", "cf").expect("prepare");
        fs::write(publication.staging_path(), "replacement").expect("stage");
        let staging_path = publication.staging_path().to_path_buf();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Artifacts).with_cancellation(cancellation);

        let failure = publication
            .publish_file_with_state(&context, "failed to publish staged test file")
            .expect_err("cancelled publication");

        assert_eq!(
            failure.target_state,
            super::PublicationFailureState::Unchanged
        );
        assert_eq!(fs::read_to_string(target).expect("target"), "original");
        assert!(staging_path.exists());
    }
}
