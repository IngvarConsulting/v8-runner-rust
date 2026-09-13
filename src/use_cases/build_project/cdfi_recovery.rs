use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, Permissions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;
use tempfile::Builder;
use uuid::Uuid;

use crate::domain::build::{CdfiRecoveryAction, CdfiRecoverySummary};
use crate::support::error::AppError;
use crate::support::fs::{best_effort_fsync_dir, replace_file_atomically};
use crate::support::temp::temp_root;

const CDFI_FILE_NAME: &str = "ConfigDumpInfo.xml";

enum OriginalState {
    Present(Permissions),
    Absent,
}

/// Owns recovery for one Designer load/update pair, not the whole build.
/// Finalization is explicit: a failed restore must retain its durable snapshot.
pub(super) struct CdfiRecovery {
    tracked_path: PathBuf,
    snapshot_dir: PathBuf,
    snapshot_path: PathBuf,
    original: OriginalState,
}

impl CdfiRecovery {
    pub(super) fn capture(source_root: &Path, work_path: &Path) -> Result<Self, AppError> {
        let tracked_path = source_root.join(CDFI_FILE_NAME);
        Self::capture_path(&tracked_path, work_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to capture CDFI '{}' before Designer load: {error}",
                tracked_path.display()
            ))
        })
    }

    fn capture_path(tracked_path: &Path, work_path: &Path) -> std::io::Result<Self> {
        let metadata = regular_file_metadata(tracked_path)?;
        let root = temp_root(work_path)?;
        let mut directory = Builder::new();
        directory.prefix("cdfi-recovery-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            directory.permissions(Permissions::from_mode(0o700));
        }
        let snapshot_dir = directory.tempdir_in(&root)?;
        let (original, snapshot_name, bytes) = match metadata {
            Some(metadata) => (
                OriginalState::Present(metadata.permissions()),
                CDFI_FILE_NAME,
                fs::read(tracked_path)?,
            ),
            None => (
                OriginalState::Absent,
                "original-absent.txt",
                format!("Original CDFI must be absent: {}\n", tracked_path.display()).into_bytes(),
            ),
        };
        let snapshot_path = snapshot_dir.path().join(snapshot_name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Restrict access at creation: chmod after writing would expose the backup.
            options.mode(0o600);
        }
        let mut snapshot = options.open(&snapshot_path)?;
        snapshot.write_all(&bytes)?;
        snapshot.sync_all()?;
        best_effort_fsync_dir(snapshot_dir.path())?;
        best_effort_fsync_dir(&root)?;
        Ok(Self {
            tracked_path: tracked_path.to_path_buf(),
            snapshot_dir: snapshot_dir.keep(),
            snapshot_path,
            original,
        })
    }

    pub(super) fn rollback(self) -> CdfiRecoverySummary {
        let mut summary = self.summary();
        match self.restore() {
            Ok((action, warning)) => {
                summary.action = action;
                summary.cleanup_warning = warning;
                self.cleanup(&mut summary);
            }
            Err(error) => {
                summary.action = CdfiRecoveryAction::Failed;
                let requirement = match self.original {
                    OriginalState::Present(_) => "original bytes and permissions",
                    OriginalState::Absent => "original absence (remove only a regular file)",
                };
                summary.failure = Some(format!(
                    "failed to restore CDFI '{}' to {requirement}: {error}; recovery snapshot retained at '{}'",
                    self.tracked_path.display(),
                    self.snapshot_path.display()
                ));
            }
        }
        summary
    }

    /// Call immediately after successful UpdateDBCfg, before committing hashes.
    pub(super) fn commit(self) -> CdfiRecoverySummary {
        let mut summary = self.summary();
        self.cleanup(&mut summary);
        summary
    }

    fn summary(&self) -> CdfiRecoverySummary {
        let original = match self.original {
            OriginalState::Present(_) => fs::read(&self.snapshot_path).map(Some),
            OriginalState::Absent => Ok(None),
        };
        let current = regular_file_metadata(&self.tracked_path)
            .and_then(|metadata| metadata.map(|_| fs::read(&self.tracked_path)).transpose());
        let changed_entry_count = match (original, current) {
            (Ok(original), Ok(current)) => {
                changed_entry_count(original.as_deref(), current.as_deref())
            }
            (Err(_), _) | (_, Err(_)) => None,
        };
        CdfiRecoverySummary {
            tracked_path: self.tracked_path.clone(),
            original_existed: matches!(self.original, OriginalState::Present(_)),
            changed_entry_count,
            action: CdfiRecoveryAction::NotNeeded,
            snapshot_path: Some(self.snapshot_path.clone()),
            cleanup_warning: None,
            failure: None,
        }
    }

    fn restore(&self) -> std::io::Result<(CdfiRecoveryAction, Option<String>)> {
        let current = regular_file_metadata(&self.tracked_path)?;
        match &self.original {
            OriginalState::Absent => {
                if current.is_none() {
                    return Ok((CdfiRecoveryAction::NotNeeded, None));
                }
                fs::remove_file(&self.tracked_path)?;
                best_effort_fsync_dir(self.source_parent()?)?;
                Ok((CdfiRecoveryAction::RemovedCreatedFile, None))
            }
            OriginalState::Present(permissions) => {
                let bytes = fs::read(&self.snapshot_path)?;
                if let Some(metadata) = current {
                    if metadata.permissions() == *permissions
                        && fs::read(&self.tracked_path).is_ok_and(|current| current == bytes)
                    {
                        return Ok((CdfiRecoveryAction::NotNeeded, None));
                    }
                }
                let parent = self.source_parent()?;
                let mut staging = Builder::new()
                    .prefix(".ConfigDumpInfo.xml.restore-")
                    .tempfile_in(parent)?;
                staging.write_all(&bytes)?;
                staging.as_file().set_permissions(permissions.clone())?;
                staging.as_file().sync_all()?;
                let outcome = replace_file_atomically(
                    staging.path(),
                    &self.tracked_path,
                    &Uuid::new_v4().to_string(),
                    "cdfi-recovery",
                )
                .map_err(std::io::Error::other)?;
                Ok((CdfiRecoveryAction::Restored, outcome.cleanup_warning))
            }
        }
    }

    fn source_parent(&self) -> std::io::Result<&Path> {
        self.tracked_path.parent().ok_or_else(|| {
            std::io::Error::new(ErrorKind::InvalidInput, "CDFI source path has no parent")
        })
    }

    fn cleanup(&self, summary: &mut CdfiRecoverySummary) {
        match fs::remove_dir_all(&self.snapshot_dir) {
            Ok(()) => summary.snapshot_path = None,
            Err(error) => {
                let warning = format!(
                    "failed to remove CDFI recovery snapshot '{}': {error}",
                    self.snapshot_dir.display()
                );
                summary.cleanup_warning = Some(match summary.cleanup_warning.take() {
                    Some(existing) => format!("{existing}; {warning}"),
                    None => warning,
                });
            }
        }
    }
}

fn regular_file_metadata(path: &Path) -> std::io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata)),
        Ok(_) => Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "CDFI must be a regular file, not a directory, symlink or special file: '{}'",
                path.display()
            ),
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

type EntrySignature = Vec<(String, String)>;

/// Diagnostic only. Recovery always uses the original bytes, including invalid XML.
fn changed_entry_count(original: Option<&[u8]>, current: Option<&[u8]>) -> Option<usize> {
    if original == current {
        return Some(0);
    }
    let original = match original {
        Some(bytes) => parse_entries(bytes)?,
        None => BTreeMap::new(),
    };
    let current = match current {
        Some(bytes) => parse_entries(bytes)?,
        None => BTreeMap::new(),
    };
    if original.is_empty() && current.is_empty() {
        return None;
    }
    let names = original
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>();
    Some(
        names
            .into_iter()
            .filter(|name| original.get(*name) != current.get(*name))
            .count(),
    )
}

fn parse_entries(bytes: &[u8]) -> Option<BTreeMap<String, EntrySignature>> {
    let mut reader = Reader::from_reader(bytes);
    let mut entries = BTreeMap::new();
    let mut depth = 0usize;
    let mut root_seen = false;
    loop {
        let event = reader.read_event().ok()?;
        let is_start = matches!(&event, Event::Start(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                if depth == 0 {
                    if root_seen || element.local_name().as_ref() != b"ConfigDumpInfo" {
                        return None;
                    }
                    root_seen = true;
                }
                let mut signature = element
                    .attributes()
                    .map(|attribute| {
                        let attribute = attribute.ok()?;
                        Some((
                            std::str::from_utf8(attribute.key.as_ref()).ok()?.to_owned(),
                            attribute
                                .decode_and_unescape_value(reader.decoder())
                                .ok()?
                                .into_owned(),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                if element.local_name().as_ref() == b"Metadata" {
                    signature.sort();
                    let name = signature.iter().find_map(|(key, value)| {
                        (key == "name" && !value.is_empty()).then(|| value.clone())
                    })?;
                    if entries.insert(name, signature).is_some() {
                        return None;
                    }
                }
                if is_start {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.checked_sub(1)?,
            Event::Text(text) => {
                let text = text.unescape().ok()?;
                if depth == 0 && !text.trim().is_empty() {
                    return None;
                }
            }
            Event::CData(_) if depth == 0 => return None,
            Event::Eof => return (root_seen && depth == 0).then_some(entries),
            Event::DocType(_) => return None,
            Event::CData(_) | Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{changed_entry_count, CdfiRecovery, CDFI_FILE_NAME};
    use crate::domain::build::CdfiRecoveryAction;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn rollback_restores_exact_bytes_and_cleans_snapshot() {
        let root = tempdir().expect("root");
        let path = root.path().join(CDFI_FILE_NAME);
        let bytes =
            b"\xef\xbb\xbf<?xml version=\"1.0\"?>\r\n<ConfigDumpInfo>\r\n</ConfigDumpInfo>\r\n";
        fs::write(&path, bytes).expect("baseline");
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        let snapshot = recovery.snapshot_path.clone();
        fs::write(&path, b"platform replacement").expect("mutation");

        let summary = recovery.rollback();

        assert_eq!(summary.action, CdfiRecoveryAction::Restored);
        assert_eq!(fs::read(&path).expect("restored"), bytes);
        assert!(!snapshot.exists());
        assert!(summary.failure.is_none());
        assert!(summary.snapshot_path.is_none());
    }

    #[test]
    fn absent_baseline_removes_created_regular_file() {
        let root = tempdir().expect("root");
        let path = root.path().join(CDFI_FILE_NAME);
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        assert!(recovery.snapshot_path.is_file());
        fs::write(&path, b"created").expect("mutation");

        let summary = recovery.rollback();

        assert_eq!(summary.action, CdfiRecoveryAction::RemovedCreatedFile);
        assert!(!summary.original_existed);
        assert!(!path.exists());
    }

    #[test]
    fn unchanged_and_absent_baselines_need_no_restore() {
        for original in [None, Some(b"malformed XML".as_slice())] {
            let root = tempdir().expect("root");
            if let Some(original) = original {
                fs::write(root.path().join(CDFI_FILE_NAME), original).expect("baseline");
            }
            let recovery =
                CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
            let summary = recovery.rollback();
            assert_eq!(summary.action, CdfiRecoveryAction::NotNeeded);
            assert_eq!(summary.changed_entry_count, Some(0));
            assert!(summary.snapshot_path.is_none());
        }
    }

    #[test]
    fn malformed_xml_does_not_block_restore() {
        let root = tempdir().expect("root");
        let path = root.path().join(CDFI_FILE_NAME);
        let baseline = b"<ConfigDumpInfo><Metadata name='broken'";
        fs::write(&path, baseline).expect("baseline");
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        fs::write(&path, b"<ConfigDumpInfo/>").expect("mutation");

        let summary = recovery.rollback();

        assert_eq!(summary.action, CdfiRecoveryAction::Restored);
        assert_eq!(summary.changed_entry_count, None);
        assert_eq!(fs::read(path).expect("restored"), baseline);
    }

    #[test]
    fn rollback_refuses_directory_and_retains_snapshot() {
        for original in [None, Some(b"baseline".as_slice())] {
            let root = tempdir().expect("root");
            let path = root.path().join(CDFI_FILE_NAME);
            if let Some(bytes) = original {
                fs::write(&path, bytes).expect("baseline");
            }
            let recovery =
                CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
            if original.is_some() {
                fs::remove_file(&path).expect("remove");
            }
            fs::create_dir(&path).expect("replacement directory");
            fs::write(path.join("unrelated"), "keep me").expect("unrelated");

            let summary = recovery.rollback();

            assert_eq!(summary.action, CdfiRecoveryAction::Failed);
            assert_eq!(summary.tracked_path, path);
            let snapshot = summary.snapshot_path.expect("retained snapshot");
            assert!(snapshot.is_file());
            assert!(summary
                .failure
                .expect("diagnostic")
                .contains(if original.is_some() {
                    "original bytes and permissions"
                } else {
                    "original absence"
                }));
            assert_eq!(
                fs::read(path.join("unrelated")).expect("unrelated remains"),
                b"keep me"
            );
            if let Some(bytes) = original {
                assert_eq!(fs::read(snapshot).expect("backup"), bytes);
            }
        }
    }

    #[test]
    fn commit_preserves_platform_cdfi_and_removes_snapshot() {
        let root = tempdir().expect("root");
        let path = root.path().join(CDFI_FILE_NAME);
        fs::write(&path, b"original").expect("baseline");
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        let snapshot = recovery.snapshot_path.clone();
        fs::write(&path, b"applied").expect("mutation");

        let summary = recovery.commit();

        assert_eq!(summary.action, CdfiRecoveryAction::NotNeeded);
        assert_eq!(fs::read(path).expect("applied"), b"applied");
        assert!(!snapshot.exists());
        assert!(summary.failure.is_none());
    }

    #[test]
    fn cleanup_failure_is_warning_and_keeps_snapshot_location() {
        let root = tempdir().expect("root");
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        let snapshot_dir = recovery.snapshot_dir.clone();
        fs::remove_dir_all(&snapshot_dir).expect("remove snapshot directory");
        fs::write(&snapshot_dir, b"replacement obstructs cleanup").expect("obstruction");

        let summary = recovery.commit();

        assert_eq!(summary.action, CdfiRecoveryAction::NotNeeded);
        assert!(summary.failure.is_none());
        assert!(summary.cleanup_warning.is_some());
        assert!(summary.snapshot_path.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn capture_and_rollback_reject_symlinks_without_touching_target() {
        use std::os::unix::fs::symlink;
        let root = tempdir().expect("root");
        let path = root.path().join(CDFI_FILE_NAME);
        let target = root.path().join("other.xml");
        fs::write(&target, b"unrelated").expect("target");
        symlink(&target, &path).expect("symlink");
        assert!(CdfiRecovery::capture(root.path(), &root.path().join("work")).is_err());
        fs::remove_file(&path).expect("remove symlink");
        fs::write(&path, b"baseline").expect("baseline");
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        fs::remove_file(&path).expect("remove baseline");
        symlink(&target, &path).expect("replace with symlink");

        let summary = recovery.rollback();

        assert_eq!(summary.action, CdfiRecoveryAction::Failed);
        assert!(summary.snapshot_path.expect("backup").is_file());
        assert!(fs::symlink_metadata(path).expect("link").is_symlink());
        assert_eq!(fs::read(target).expect("unrelated"), b"unrelated");
    }

    #[cfg(unix)]
    #[test]
    fn rollback_restores_permissions_even_when_bytes_are_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempdir().expect("root");
        let path = root.path().join(CDFI_FILE_NAME);
        fs::write(&path, b"baseline").expect("baseline");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("baseline mode");
        let recovery =
            CdfiRecovery::capture(root.path(), &root.path().join("work")).expect("capture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("changed mode");

        let summary = recovery.rollback();

        assert_eq!(summary.action, CdfiRecoveryAction::Restored);
        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn counts_added_removed_and_changed_metadata_entries() {
        let original = br#"<ConfigDumpInfo><ConfigVersions><Metadata name="Changed" configVersion="1"/><Metadata name="Removed"/></ConfigVersions></ConfigDumpInfo>"#;
        let current = br#"<ConfigDumpInfo><ConfigVersions><Metadata configVersion="2" name="Changed"/><Metadata name="Added"/></ConfigVersions></ConfigDumpInfo>"#;
        assert_eq!(changed_entry_count(Some(original), Some(current)), Some(3));
        assert_eq!(changed_entry_count(None, Some(current)), Some(2));
        assert_eq!(changed_entry_count(Some(original), None), Some(2));
    }

    #[test]
    fn count_is_unknown_for_invalid_or_ambiguous_xml_or_no_metadata() {
        let empty = b"<ConfigDumpInfo/>";
        for invalid in [
            b"<ConfigDumpInfo><Metadata name='X'/>".as_slice(),
            b"<ConfigDumpInfo><Metadata name='X'></Wrong></ConfigDumpInfo>",
            b"<ConfigDumpInfo/><ConfigDumpInfo/>",
            b"<Other><Metadata name='X'/></Other>",
            b"<ConfigDumpInfo><Metadata name='X'/><Metadata name='X'/></ConfigDumpInfo>",
            b"<ConfigDumpInfo><Metadata name='&unknown;'/></ConfigDumpInfo>",
            b"<ConfigDumpInfo>invalid &unknown;</ConfigDumpInfo>",
            b"<ConfigDumpInfo></ConfigDumpInfo>",
        ] {
            assert_eq!(changed_entry_count(Some(empty), Some(invalid)), None);
        }
    }
}
