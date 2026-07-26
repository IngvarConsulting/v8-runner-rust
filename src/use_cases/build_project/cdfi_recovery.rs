use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use tempfile::Builder;
use uuid::Uuid;

use crate::domain::build::{CdfiRecoveryAction, CdfiRecoverySummary};
use crate::support::error::AppError;
use crate::support::fs::replace_file_atomically;

const CDFI_FILE_NAME: &str = "ConfigDumpInfo.xml";

#[derive(Debug)]
pub(super) struct CdfiRecoveryGuard {
    tracked_path: PathBuf,
    snapshot_path: PathBuf,
    snapshot_dir: Option<PathBuf>,
    original_exists: bool,
    original_permissions: Option<fs::Permissions>,
}

impl CdfiRecoveryGuard {
    pub(super) fn capture(source_root: &Path, work_path: &Path) -> Result<Self, AppError> {
        let tracked_path = source_root.join(CDFI_FILE_NAME);
        fs::create_dir_all(work_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create CDFI recovery work directory '{}': {error}",
                work_path.display()
            ))
        })?;
        let snapshot_dir = Builder::new()
            .prefix("cdfi-recovery-")
            .tempdir_in(work_path)
            .map_err(|error| {
                AppError::Runtime(format!(
                    "failed to create CDFI recovery snapshot under '{}': {error}",
                    work_path.display()
                ))
            })?;
        let snapshot_path = snapshot_dir.path().join(CDFI_FILE_NAME);

        let original_permissions = match fs::metadata(&tracked_path) {
            Ok(metadata) => {
                let bytes = fs::read(&tracked_path).map_err(|error| {
                    AppError::Runtime(format!(
                        "failed to capture CDFI '{}': {error}",
                        tracked_path.display()
                    ))
                })?;
                fs::write(&snapshot_path, bytes).map_err(|error| {
                    AppError::Runtime(format!(
                        "failed to write CDFI recovery snapshot '{}': {error}",
                        snapshot_path.display()
                    ))
                })?;
                Some(metadata.permissions())
            }
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::Runtime(format!(
                    "failed to capture CDFI '{}': {error}",
                    tracked_path.display()
                )));
            }
        };
        let original_exists = original_permissions.is_some();
        let snapshot_dir = snapshot_dir.keep();

        Ok(Self {
            tracked_path,
            snapshot_path,
            snapshot_dir: Some(snapshot_dir),
            original_exists,
            original_permissions,
        })
    }

    pub(super) fn restore(&mut self) -> Result<CdfiRecoverySummary, AppError> {
        let action = if self.original_exists {
            self.restore_snapshot()?;
            CdfiRecoveryAction::RestoredOriginal
        } else {
            self.remove_created_file()?;
            CdfiRecoveryAction::RemovedCreatedFile
        };

        Ok(CdfiRecoverySummary {
            action,
            snapshot_path: None,
            failure: None,
        })
    }

    pub(super) fn failed_summary(&self, error: &AppError) -> CdfiRecoverySummary {
        CdfiRecoverySummary {
            action: CdfiRecoveryAction::RestoreFailed,
            snapshot_path: Some(self.snapshot_path.clone()),
            failure: Some(error.to_string()),
        }
    }

    pub(super) fn finalize_successful_restore(
        &mut self,
        mut summary: CdfiRecoverySummary,
    ) -> CdfiRecoverySummary {
        if let Err(error) = self.cleanup() {
            summary.snapshot_path = Some(self.snapshot_path.clone());
            summary.failure = Some(format!(
                "failed to remove CDFI recovery snapshot after restoration: {error}"
            ));
        }
        summary
    }

    pub(super) fn cleanup(&mut self) -> Result<(), AppError> {
        let Some(snapshot_dir) = self.snapshot_dir.as_ref() else {
            return Ok(());
        };
        fs::remove_dir_all(snapshot_dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to remove CDFI recovery snapshot '{}': {error}",
                self.snapshot_path.display()
            ))
        })?;
        self.snapshot_dir = None;
        Ok(())
    }

    fn restore_snapshot(&self) -> Result<(), AppError> {
        let bytes = fs::read(&self.snapshot_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to read CDFI recovery snapshot '{}': {error}",
                self.snapshot_path.display()
            ))
        })?;
        let parent = self.tracked_path.parent().ok_or_else(|| {
            AppError::Runtime(format!(
                "CDFI path has no parent: '{}'",
                self.tracked_path.display()
            ))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create CDFI directory '{}': {error}",
                parent.display()
            ))
        })?;
        let mut staging_file = Builder::new()
            .prefix(".ConfigDumpInfo.xml.restore-")
            .tempfile_in(parent)
            .map_err(|error| {
                AppError::Runtime(format!(
                    "failed to create CDFI restore staging file in '{}': {error}",
                    parent.display()
                ))
            })?;
        staging_file.write_all(&bytes).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write CDFI restore staging file for '{}': {error}",
                self.tracked_path.display()
            ))
        })?;
        if let Some(permissions) = self.original_permissions.as_ref() {
            staging_file
                .as_file()
                .set_permissions(permissions.clone())
                .map_err(|error| {
                    AppError::Runtime(format!(
                        "failed to preserve CDFI permissions for '{}': {error}",
                        self.tracked_path.display()
                    ))
                })?;
        }
        staging_file.as_file().sync_all().map_err(|error| {
            AppError::Runtime(format!(
                "failed to write CDFI restore staging file for '{}': {error}",
                self.tracked_path.display()
            ))
        })?;
        replace_file_atomically(
            staging_file.path(),
            &self.tracked_path,
            &Uuid::new_v4().to_string(),
            "cdfi-recovery",
        )
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to restore CDFI '{}': {error}",
                self.tracked_path.display()
            ))
        })
        .map(|_| ())
    }

    fn remove_created_file(&self) -> Result<(), AppError> {
        match fs::remove_file(&self.tracked_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::Runtime(format!(
                "failed to remove CDFI created during build '{}': {error}",
                self.tracked_path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::CdfiRecoveryGuard;

    const CDFI_FILE_NAME: &str = "ConfigDumpInfo.xml";

    #[test]
    fn restore_recreates_original_cdfi_bytes_without_rewriting_xml() {
        let temp = tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        let work_path = temp.path().join("work");
        let tracked_path = source_root.join(CDFI_FILE_NAME);
        let original = b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\r\n<ConfigDumpInfo>\r\n  <Version>1</Version>\r\n</ConfigDumpInfo>\r\n";

        fs::create_dir_all(&source_root).expect("source root");
        fs::write(&tracked_path, original).expect("original CDFI");
        let mut guard = CdfiRecoveryGuard::capture(&source_root, &work_path).expect("capture");
        fs::write(
            &tracked_path,
            b"<ConfigDumpInfo><Version>changed</Version></ConfigDumpInfo>",
        )
        .expect("mutate CDFI");

        guard.restore().expect("restore");

        assert_eq!(fs::read(&tracked_path).expect("restored CDFI"), original);
    }

    #[test]
    fn restore_removes_cdfi_created_after_absent_capture() {
        let temp = tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        let work_path = temp.path().join("work");
        let tracked_path = source_root.join(CDFI_FILE_NAME);

        fs::create_dir_all(&source_root).expect("source root");
        let mut guard = CdfiRecoveryGuard::capture(&source_root, &work_path).expect("capture");
        fs::write(&tracked_path, b"<ConfigDumpInfo/>").expect("create CDFI");

        guard.restore().expect("restore");

        assert!(!tracked_path.exists());
    }

    #[test]
    fn cleanup_removes_private_snapshot() {
        let temp = tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        let work_path = temp.path().join("work");

        fs::create_dir_all(&source_root).expect("source root");
        let mut guard = CdfiRecoveryGuard::capture(&source_root, &work_path).expect("capture");
        let snapshot_path = guard.snapshot_path.clone();

        guard.cleanup().expect("cleanup");

        assert!(!snapshot_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn failed_restore_keeps_pristine_snapshot_after_guard_is_dropped() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        let work_path = temp.path().join("work");
        let tracked_path = source_root.join(CDFI_FILE_NAME);
        let original = b"\xEF\xBB\xBF<ConfigDumpInfo/>\r\n";

        fs::create_dir_all(&source_root).expect("source root");
        fs::write(&tracked_path, original).expect("original CDFI");
        let mut guard = CdfiRecoveryGuard::capture(&source_root, &work_path).expect("capture");
        let snapshot_path = guard.snapshot_path.clone();
        fs::set_permissions(&source_root, fs::Permissions::from_mode(0o500))
            .expect("block restore staging");

        guard.restore().expect_err("restore must fail");
        drop(guard);
        fs::set_permissions(&source_root, fs::Permissions::from_mode(0o700))
            .expect("restore source permissions");

        assert_eq!(
            fs::read(snapshot_path).expect("retained snapshot"),
            original
        );
    }

    #[cfg(unix)]
    #[test]
    fn restore_preserves_original_cdfi_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        let work_path = temp.path().join("work");
        let tracked_path = source_root.join(CDFI_FILE_NAME);

        fs::create_dir_all(&source_root).expect("source root");
        fs::write(&tracked_path, b"<ConfigDumpInfo/>").expect("original CDFI");
        fs::set_permissions(&tracked_path, fs::Permissions::from_mode(0o640))
            .expect("set permissions");
        let mut guard = CdfiRecoveryGuard::capture(&source_root, &work_path).expect("capture");
        fs::write(
            &tracked_path,
            b"<ConfigDumpInfo><Changed/></ConfigDumpInfo>",
        )
        .expect("mutate CDFI");

        guard.restore().expect("restore");

        assert_eq!(
            fs::metadata(&tracked_path)
                .expect("restored metadata")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
}
