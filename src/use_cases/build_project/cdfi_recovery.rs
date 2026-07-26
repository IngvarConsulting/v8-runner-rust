use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use tempfile::{Builder, TempDir};

use crate::support::error::AppError;
use crate::support::fs::publish_file_atomically;

const CDFI_FILE_NAME: &str = "ConfigDumpInfo.xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CdfiRecoveryAction {
    RestoredOriginal,
    RemovedCreatedFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CdfiRecoverySummary {
    pub(super) tracked_path: PathBuf,
    pub(super) snapshot_path: PathBuf,
    pub(super) action: CdfiRecoveryAction,
}

#[derive(Debug)]
pub(super) struct CdfiRecoveryGuard {
    tracked_path: PathBuf,
    snapshot_path: PathBuf,
    snapshot_dir: Option<TempDir>,
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
            tracked_path: self.tracked_path.clone(),
            snapshot_path: self.snapshot_path.clone(),
            action,
        })
    }

    pub(super) fn cleanup(&mut self) -> Result<(), AppError> {
        let Some(snapshot_dir) = self.snapshot_dir.as_ref() else {
            return Ok(());
        };
        fs::remove_dir_all(snapshot_dir.path()).map_err(|error| {
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
        publish_file_atomically(staging_file.path(), &self.tracked_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to restore CDFI '{}': {error}",
                self.tracked_path.display()
            ))
        })
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
