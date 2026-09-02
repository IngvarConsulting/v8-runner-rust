use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;

use crate::support::error::AppError;
use crate::support::fs::{
    ensure_dir, is_known_tool_name, metadata_sidecar_path, read_temp_dir_metadata,
    remove_path_if_exists, replace_dir_atomically, replace_file_atomically,
    write_temp_dir_metadata, ReplaceFileFailureState, TempDirKind, TempDirMetadata,
};
use crate::use_cases::context::{ExecutionContext, ExecutionInterruption};
use crate::use_cases::interruption;

const ORPHAN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone)]
pub(super) struct StagedPublication {
    staging_path: PathBuf,
    target_path: PathBuf,
    run_id: String,
    target_identity: String,
    previous_target_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StagedPublicationOutcome {
    pub cleanup_warning: Option<String>,
    pub deferred_interruption: Option<ExecutionInterruption>,
    pub previous_target_present: bool,
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
    pub fn prepare_dir(
        target_path: &Path,
        target_identity: &str,
        stage_prefix: &str,
    ) -> Result<Self, AppError> {
        let target_parent = target_path.parent().ok_or_else(|| {
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
            target_parent.join(format!("{stage_prefix}-{run_id}")),
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
        let target_parent = target_path.parent().ok_or_else(|| {
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
        cleanup_staging_path(&self.staging_path, error)
    }

    pub fn publish_dir(
        &self,
        context: &ExecutionContext,
        backup_prefix: &str,
        error_prefix: &str,
    ) -> Result<StagedPublicationOutcome, AppError> {
        if let Some(error) = interruption_before_publish(context, "staged directory publication") {
            return Err(error);
        }
        let publish_phase = context.run_no_process_critical_phase(|| {
            replace_dir_atomically(
                &self.staging_path,
                &self.target_path,
                &self.run_id,
                &self.target_identity,
                backup_prefix,
            )
            .map_err(|error| AppError::Runtime(format!("{error_prefix}: {error}")))
        })?;
        Ok(StagedPublicationOutcome {
            cleanup_warning: publish_phase.value.cleanup_warning,
            deferred_interruption: publish_phase.deferred_interruption,
            previous_target_present: self.previous_target_present,
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
            if (Utc::now() - metadata.created_at)
                .to_std()
                .unwrap_or_default()
                < ORPHAN_TTL
            {
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

fn orphan_name_matches_contract(
    file_name: &str,
    target_path: &Path,
    metadata: &TempDirMetadata,
    stage_prefixes: &[&str],
    backup_prefixes: &[&str],
) -> bool {
    match metadata.kind {
        TempDirKind::Stage => {
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
        path.parent()
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

pub(super) fn interruption_before_publish(
    context: &ExecutionContext,
    safe_point: impl Into<String>,
) -> Option<AppError> {
    interruption::interruption_before_safe_point(context, safe_point.into())
}

fn make_run_id() -> String {
    let timestamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{}-{timestamp:x}", std::process::id())
}

#[cfg(test)]
mod tests {
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
            )
            .expect("publish");

        assert_eq!(outcome.deferred_interruption, None);
        assert_eq!(
            fs::read_to_string(target.join("payload.txt")).expect("target"),
            "payload"
        );
        assert!(!stage_metadata.exists());
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
            .publish_dir(&context, ".backup", "failed to publish staged test dir")
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
