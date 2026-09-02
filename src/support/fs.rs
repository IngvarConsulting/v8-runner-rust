use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const TOOL_NAME: &str = "v8-runner";

pub fn is_known_tool_name(tool: &str) -> bool {
    tool == TOOL_NAME
}

#[cfg(test)]
thread_local! {
    static TEST_LOCK_WRITE_HOOK: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        std::cell::RefCell::new(None);
}

/// Create a directory and all missing parents.
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Remove all files and directories directly under `dir`.
pub fn clean_dir(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }

    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TempDirKind {
    Stage,
    Backup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempDirMetadata {
    pub tool: String,
    pub kind: TempDirKind,
    pub run_id: String,
    pub target_path: PathBuf,
    pub target_identity: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisoryLockMetadata {
    pub tool: String,
    pub pid: u32,
    pub owner_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct AdvisoryLockGuard {
    #[allow(dead_code)]
    file: Option<File>,
    path: PathBuf,
    metadata: AdvisoryLockMetadata,
}

impl Drop for AdvisoryLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            if lock_file_owned_by(&self.path, &self.metadata.owner_id) {
                let _ = std::fs::remove_file(&self.path);
            }
            let _ = file.unlock();
        }
    }
}

#[derive(Debug)]
pub struct ReplaceDirOutcome {
    pub cleanup_warning: Option<String>,
}

#[derive(Debug)]
pub struct ReplaceFileOutcome {
    pub cleanup_warning: Option<String>,
    pub previous_target_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceFileFailureState {
    Unchanged,
    Restored,
    Uncertain,
}

#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct ReplaceFileError {
    #[source]
    pub source: std::io::Error,
    pub target_state: ReplaceFileFailureState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplaceFileTestPoint {
    PublishStage,
    RestoreBackup,
}

#[cfg(test)]
thread_local! {
    static REPLACE_FILE_TEST_HOOK: std::cell::RefCell<Option<Box<dyn Fn(ReplaceFileTestPoint) -> std::io::Result<()>>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_replace_file_test_hook(point: ReplaceFileTestPoint) -> std::io::Result<()> {
    REPLACE_FILE_TEST_HOOK.with(|hook| match hook.borrow().as_ref() {
        Some(hook) => hook(point),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn run_replace_file_test_hook(_point: ReplaceFileTestPoint) -> std::io::Result<()> {
    Ok(())
}

impl ReplaceFileError {
    fn new(source: std::io::Error, target_state: ReplaceFileFailureState) -> Self {
        Self {
            source,
            target_state,
        }
    }
}

pub fn acquire_advisory_lock(path: &Path) -> std::io::Result<AdvisoryLockGuard> {
    loop {
        match try_acquire_advisory_lock(path) {
            Ok(guard) => return Ok(guard),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn try_acquire_advisory_lock(path: &Path) -> std::io::Result<AdvisoryLockGuard> {
    try_acquire_advisory_lock_impl(path)
}

fn try_acquire_advisory_lock_impl(path: &Path) -> std::io::Result<AdvisoryLockGuard> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_dir(parent)?;

    let metadata = AdvisoryLockMetadata {
        tool: TOOL_NAME.to_owned(),
        pid: std::process::id(),
        owner_id: Uuid::new_v4().to_string(),
        created_at: Utc::now(),
    };
    let encoded = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
    let system_lock_path = advisory_system_lock_path(path)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(system_lock_path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(lock_already_held(path)),
        Err(TryLockError::Error(error)) => return Err(error),
    }

    publish_advisory_lock_metadata(path, parent, &encoded)?;
    Ok(AdvisoryLockGuard {
        file: Some(file),
        path: path.to_path_buf(),
        metadata,
    })
}

fn advisory_system_lock_path(path: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            ErrorKind::InvalidInput,
            format!("lock path has no file name: {}", path.display()),
        )
    })?;
    let mut system_name = name.to_os_string();
    system_name.push(".system");
    Ok(path.with_file_name(system_name))
}

fn publish_advisory_lock_metadata(
    path: &Path,
    parent: &Path,
    encoded: &[u8],
) -> std::io::Result<()> {
    let mut candidate = tempfile::Builder::new()
        .prefix(".v8-runner-lock-candidate-")
        .tempfile_in(parent)?;
    write_advisory_lock_metadata(candidate.as_file_mut(), encoded)?;
    candidate.as_file().sync_all()?;

    match candidate.persist_noclobber(path) {
        Ok(_) => {
            let _ = best_effort_fsync_dir(parent);
            Ok(())
        }
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            Err(legacy_lock_requires_offline_cleanup(path))
        }
        Err(error) => Err(error.error),
    }
}

fn lock_already_held(path: &Path) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::WouldBlock,
        format!("lock is already held: {}", path.display()),
    )
}

fn legacy_lock_requires_offline_cleanup(path: &Path) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::AlreadyExists,
        format!(
            "legacy or crash owner lock remains at '{}'; stop all old and new v8-runner processes, then remove this file manually",
            path.display()
        ),
    )
}

pub fn advisory_lock_owner_id(guard: &AdvisoryLockGuard) -> &str {
    &guard.metadata.owner_id
}

pub fn read_advisory_lock_metadata(path: &Path) -> std::io::Result<AdvisoryLockMetadata> {
    let raw = std::fs::read(path)?;
    serde_json::from_slice(&raw).map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))
}

fn lock_file_owned_by(path: &Path, owner_id: &str) -> bool {
    read_advisory_lock_metadata(path)
        .map(|metadata| metadata.owner_id == owner_id)
        .unwrap_or(false)
}

#[cfg(not(test))]
fn write_advisory_lock_metadata(file: &mut File, encoded: &[u8]) -> std::io::Result<()> {
    file.write_all(encoded)
}

#[cfg(test)]
fn write_advisory_lock_metadata(file: &mut File, encoded: &[u8]) -> std::io::Result<()> {
    let has_hook = TEST_LOCK_WRITE_HOOK.with(|cell| cell.borrow().is_some());
    if has_hook && !encoded.is_empty() {
        file.write_all(&encoded[..1])?;
        TEST_LOCK_WRITE_HOOK.with(|cell| {
            if let Some(hook) = cell.borrow().as_ref() {
                hook();
            }
        });
        file.write_all(&encoded[1..])
    } else {
        file.write_all(encoded)
    }
}

#[cfg(test)]
fn try_acquire_advisory_lock_with_hook<F>(
    path: &Path,
    publish_hook: F,
) -> std::io::Result<AdvisoryLockGuard>
where
    F: Fn() + 'static,
{
    TEST_LOCK_WRITE_HOOK.with(|cell| {
        *cell.borrow_mut() = Some(Box::new(publish_hook));
    });
    let result = try_acquire_advisory_lock(path);
    TEST_LOCK_WRITE_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

pub fn best_effort_fsync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let dir = File::open(path)?;

        // SAFETY: `dir` owns a valid file descriptor for the duration of the call,
        // and `fsync` does not retain it. The return code is checked below.
        unsafe {
            let rc = libc::fsync(std::os::fd::AsRawFd::as_raw_fd(&dir));
            if rc == 0 {
                return Ok(());
            }

            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINVAL) {
                return Ok(());
            }
            Err(error)
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

pub fn publish_file_atomically(temp_path: &Path, destination_path: &Path) -> std::io::Result<()> {
    publish_file_atomically_impl(
        temp_path,
        destination_path,
        &|from, to| std::fs::rename(from, to),
        &|path| remove_path_if_exists(path),
    )
}

fn publish_file_atomically_impl(
    temp_path: &Path,
    destination_path: &Path,
    rename: &dyn for<'a, 'b> Fn(&'a Path, &'b Path) -> std::io::Result<()>,
    cleanup: &dyn Fn(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if !destination_path.exists() {
        return rename(temp_path, destination_path);
    }

    let parent = destination_path.parent().ok_or_else(|| {
        std::io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "destination path has no parent: {}",
                destination_path.display()
            ),
        )
    })?;
    let backup_path = parent.join(format!(
        ".{}.backup-{}",
        destination_path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "artifact".to_owned()),
        Uuid::new_v4()
    ));

    rename(destination_path, &backup_path)?;
    let publish_result = rename(temp_path, destination_path);
    match publish_result {
        Ok(()) => {
            let _ = cleanup(&backup_path);
            Ok(())
        }
        Err(error) => {
            let rollback_result = rename(&backup_path, destination_path);
            match rollback_result {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(std::io::Error::new(
                    error.kind(),
                    format!(
                        "failed to publish '{}' atomically: {error}; rollback failed: {rollback_error}",
                        destination_path.display()
                    ),
                )),
            }
        }
    }
}

pub fn metadata_sidecar_path(dir: &Path) -> PathBuf {
    let file_name = dir
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "temp-dir".to_owned());
    dir.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{file_name}.meta.json"))
}

pub fn write_temp_dir_metadata(
    dir: &Path,
    kind: TempDirKind,
    run_id: &str,
    target_path: &Path,
    target_identity: &str,
) -> std::io::Result<()> {
    let metadata = TempDirMetadata {
        tool: TOOL_NAME.to_owned(),
        kind,
        run_id: run_id.to_owned(),
        target_path: target_path.to_path_buf(),
        target_identity: target_identity.to_owned(),
        created_at: Utc::now(),
    };

    std::fs::write(
        metadata_sidecar_path(dir),
        serde_json::to_vec_pretty(&metadata)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
    )
}

pub fn read_temp_dir_metadata(dir: &Path) -> std::io::Result<TempDirMetadata> {
    let raw = std::fs::read(metadata_sidecar_path(dir))?;
    serde_json::from_slice(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

pub fn remove_path_if_exists(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

pub fn replace_dir_atomically(
    staging_dir: &Path,
    target_dir: &Path,
    run_id: &str,
    target_identity: &str,
    backup_prefix: &str,
) -> std::io::Result<ReplaceDirOutcome> {
    let parent = target_dir.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("target path has no parent: {}", target_dir.display()),
        )
    })?;
    let backup_dir = parent.join(format!("{backup_prefix}-{run_id}"));
    let stage_metadata_path = metadata_sidecar_path(staging_dir);
    let backup_metadata_path = metadata_sidecar_path(&backup_dir);

    if !target_dir.exists() {
        std::fs::rename(staging_dir, target_dir)?;
        let fsync_result = best_effort_fsync_dir(parent);
        let _ = remove_path_if_exists(&stage_metadata_path);
        fsync_result?;
        return Ok(ReplaceDirOutcome {
            cleanup_warning: None,
        });
    }

    std::fs::rename(target_dir, &backup_dir)?;
    if let Err(error) = best_effort_fsync_dir(parent) {
        let rollback_result =
            std::fs::rename(&backup_dir, target_dir).and_then(|()| best_effort_fsync_dir(parent));
        return Err(with_rollback_context(
            error,
            rollback_result.err(),
            "failed to fsync parent after moving target to backup",
        ));
    }

    if let Err(error) = write_temp_dir_metadata(
        &backup_dir,
        TempDirKind::Backup,
        run_id,
        target_dir,
        target_identity,
    ) {
        let rollback_result =
            std::fs::rename(&backup_dir, target_dir).and_then(|()| best_effort_fsync_dir(parent));
        return Err(with_rollback_context(
            error,
            rollback_result.err(),
            "failed to write backup metadata",
        ));
    }

    if let Err(error) = std::fs::rename(staging_dir, target_dir) {
        let rollback_result =
            std::fs::rename(&backup_dir, target_dir).and_then(|()| best_effort_fsync_dir(parent));
        return Err(with_rollback_context(
            error,
            rollback_result.err(),
            "failed to publish staged dump",
        ));
    }

    if let Err(error) = best_effort_fsync_dir(parent) {
        let rollback_result = std::fs::rename(target_dir, staging_dir)
            .and_then(|()| std::fs::rename(&backup_dir, target_dir))
            .and_then(|()| best_effort_fsync_dir(parent));
        return Err(with_rollback_context(
            error,
            rollback_result.err(),
            "failed to fsync parent after publishing staged dump",
        ));
    }

    let _ = remove_path_if_exists(&stage_metadata_path);

    let mut warnings = Vec::new();
    if let Err(error) = remove_path_if_exists(&backup_dir) {
        warnings.push(format!(
            "failed to remove backup dir '{}': {error}",
            backup_dir.display()
        ));
    } else if let Err(error) = remove_path_if_exists(&backup_metadata_path) {
        warnings.push(format!(
            "failed to remove backup metadata '{}': {error}",
            backup_metadata_path.display()
        ));
    }

    Ok(ReplaceDirOutcome {
        cleanup_warning: if warnings.is_empty() {
            None
        } else {
            Some(warnings.join("; "))
        },
    })
}

pub fn replace_file_atomically(
    staging_file: &Path,
    target_file: &Path,
    run_id: &str,
    target_identity: &str,
) -> Result<ReplaceFileOutcome, ReplaceFileError> {
    let parent = target_file.parent().ok_or_else(|| {
        ReplaceFileError::new(
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("target path has no parent: {}", target_file.display()),
            ),
            ReplaceFileFailureState::Unchanged,
        )
    })?;
    let backup_name = target_file
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_owned());
    let backup_file = parent.join(format!(".{backup_name}.backup-{run_id}"));
    let stage_metadata_path = metadata_sidecar_path(staging_file);
    let backup_metadata_path = metadata_sidecar_path(&backup_file);

    if !target_file.exists() {
        publish_file_atomically(staging_file, target_file)
            .map_err(|error| ReplaceFileError::new(error, ReplaceFileFailureState::Unchanged))?;
        if let Err(error) = best_effort_fsync_dir(parent) {
            let rollback_result = std::fs::rename(target_file, staging_file)
                .and_then(|()| best_effort_fsync_dir(parent));
            return Err(replace_file_rollback_error(
                error,
                rollback_result,
                "failed to fsync parent after creating target file",
                ReplaceFileFailureState::Unchanged,
            ));
        }
        let _ = remove_path_if_exists(&stage_metadata_path);
        return Ok(ReplaceFileOutcome {
            cleanup_warning: None,
            previous_target_present: false,
        });
    }

    std::fs::rename(target_file, &backup_file)
        .map_err(|error| ReplaceFileError::new(error, ReplaceFileFailureState::Unchanged))?;
    if let Err(error) = best_effort_fsync_dir(parent) {
        let rollback_result =
            std::fs::rename(&backup_file, target_file).and_then(|()| best_effort_fsync_dir(parent));
        return Err(replace_file_rollback_error(
            error,
            rollback_result,
            "failed to fsync parent after moving target file to backup",
            ReplaceFileFailureState::Restored,
        ));
    }

    if let Err(error) = write_temp_dir_metadata(
        &backup_file,
        TempDirKind::Backup,
        run_id,
        target_file,
        target_identity,
    ) {
        let rollback_result =
            std::fs::rename(&backup_file, target_file).and_then(|()| best_effort_fsync_dir(parent));
        return Err(replace_file_rollback_error(
            error,
            rollback_result,
            "failed to write backup file metadata",
            ReplaceFileFailureState::Restored,
        ));
    }

    if let Err(error) = run_replace_file_test_hook(ReplaceFileTestPoint::PublishStage)
        .and_then(|()| publish_file_atomically(staging_file, target_file))
    {
        let rollback_result = run_replace_file_test_hook(ReplaceFileTestPoint::RestoreBackup)
            .and_then(|()| publish_file_atomically(&backup_file, target_file))
            .and_then(|()| best_effort_fsync_dir(parent));
        return Err(replace_file_rollback_error(
            error,
            rollback_result,
            "failed to publish staged artifact file",
            ReplaceFileFailureState::Restored,
        ));
    }

    if let Err(error) = best_effort_fsync_dir(parent) {
        let rollback_result = std::fs::rename(target_file, staging_file)
            .and_then(|()| publish_file_atomically(&backup_file, target_file))
            .and_then(|()| best_effort_fsync_dir(parent));
        return Err(replace_file_rollback_error(
            error,
            rollback_result,
            "failed to fsync parent after publishing staged artifact file",
            ReplaceFileFailureState::Restored,
        ));
    }

    let _ = remove_path_if_exists(&stage_metadata_path);

    let mut warnings = Vec::new();
    if let Err(error) = remove_path_if_exists(&backup_file) {
        warnings.push(format!(
            "failed to remove backup file '{}': {error}",
            backup_file.display()
        ));
    } else if let Err(error) = remove_path_if_exists(&backup_metadata_path) {
        warnings.push(format!(
            "failed to remove backup metadata '{}': {error}",
            backup_metadata_path.display()
        ));
    }

    Ok(ReplaceFileOutcome {
        cleanup_warning: if warnings.is_empty() {
            None
        } else {
            Some(warnings.join("; "))
        },
        previous_target_present: true,
    })
}

fn replace_file_rollback_error(
    error: std::io::Error,
    rollback_result: std::io::Result<()>,
    context: &str,
    restored_state: ReplaceFileFailureState,
) -> ReplaceFileError {
    match rollback_result {
        Ok(()) => ReplaceFileError::new(
            std::io::Error::new(error.kind(), format!("{context}: {error}")),
            restored_state,
        ),
        Err(rollback_error) => ReplaceFileError::new(
            std::io::Error::new(
                error.kind(),
                format!("{context}: {error}; rollback failed: {rollback_error}"),
            ),
            ReplaceFileFailureState::Uncertain,
        ),
    }
}

fn with_rollback_context(
    error: std::io::Error,
    rollback_error: Option<std::io::Error>,
    context: &str,
) -> std::io::Error {
    match rollback_error {
        Some(rollback_error) => std::io::Error::new(
            error.kind(),
            format!("{context}: {error}; rollback failed: {rollback_error}"),
        ),
        None => std::io::Error::new(error.kind(), format!("{context}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::replace_dir_atomically;
    use super::{
        acquire_advisory_lock, advisory_lock_owner_id, advisory_system_lock_path,
        publish_file_atomically, publish_file_atomically_impl, read_advisory_lock_metadata,
        remove_path_if_exists, replace_file_atomically, replace_file_rollback_error,
        try_acquire_advisory_lock, try_acquire_advisory_lock_with_hook, AdvisoryLockMetadata,
        ReplaceFileFailureState, ReplaceFileTestPoint, REPLACE_FILE_TEST_HOOK, TOOL_NAME,
    };
    use std::fs;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::sync::mpsc;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn try_acquire_advisory_lock_reports_busy() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("busy.lock");
        let _guard = acquire_advisory_lock(&lock_path).expect("lock");

        let error = try_acquire_advisory_lock(&lock_path).expect_err("busy");

        assert_eq!(error.kind(), ErrorKind::WouldBlock);
    }

    #[test]
    fn advisory_lock_writes_owner_metadata() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("owner.lock");
        let guard = acquire_advisory_lock(&lock_path).expect("lock");

        let metadata = read_advisory_lock_metadata(&lock_path).expect("metadata");

        assert_eq!(metadata.pid, std::process::id());
        assert_eq!(metadata.owner_id, advisory_lock_owner_id(&guard));
    }

    #[test]
    fn released_advisory_lock_keeps_system_file_and_can_be_reacquired() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("persistent.lock");
        let first = acquire_advisory_lock(&lock_path).expect("first lock");
        drop(first);

        assert!(!lock_path.exists());
        assert!(advisory_system_lock_path(&lock_path)
            .expect("system lock path")
            .is_file());
        let second = try_acquire_advisory_lock(&lock_path).expect("second lock");
        assert!(!advisory_lock_owner_id(&second).is_empty());
    }

    #[test]
    fn dead_legacy_lock_metadata_is_fail_closed() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("stale.lock");
        let stale = AdvisoryLockMetadata {
            tool: TOOL_NAME.to_owned(),
            pid: i32::MAX as u32,
            owner_id: "stale-owner".to_owned(),
            created_at: chrono::Utc::now(),
        };
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&stale).expect("metadata"),
        )
        .expect("stale lock");

        let error = try_acquire_advisory_lock(&lock_path).expect_err("legacy lock remains busy");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("remove this file manually"));
        assert_eq!(
            read_advisory_lock_metadata(&lock_path)
                .expect("stale metadata")
                .owner_id,
            "stale-owner"
        );
    }

    #[test]
    fn blocking_acquisition_fails_fast_for_legacy_owner_lock() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("legacy.lock");
        fs::write(&lock_path, b"legacy owner").expect("legacy lock");
        let started = std::time::Instant::now();

        let error = acquire_advisory_lock(&lock_path).expect_err("legacy lock must fail fast");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("remove this file manually"));
    }

    #[test]
    fn advisory_lock_serializes_blocking_waiters() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("serialized.lock");
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

    #[test]
    fn publish_file_atomically_replaces_existing_destination() {
        let dir = tempdir().expect("tempdir");
        let temp = dir.path().join("temp.json");
        let destination = dir.path().join("dest.json");
        fs::write(&temp, "new").expect("temp");
        fs::write(&destination, "old").expect("dest");

        publish_file_atomically(&temp, &destination).expect("publish");

        assert_eq!(fs::read_to_string(&destination).expect("dest"), "new");
        assert!(!temp.exists());
    }

    #[test]
    fn publish_file_atomically_restores_backup_when_publish_fails() {
        let dir = tempdir().expect("tempdir");
        let temp = dir.path().join("temp.json");
        let destination = dir.path().join("dest.json");
        fs::write(&temp, "new").expect("temp");
        fs::write(&destination, "old").expect("dest");

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let temp_path = temp.clone();
        let destination_path = destination.clone();
        let rename = move |from: &Path, to: &Path| {
            let count = calls_clone.fetch_add(1, Ordering::SeqCst);
            if count == 1 && from == temp_path.as_path() && to == destination_path.as_path() {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "simulated failure",
                ));
            }
            fs::rename(from, to)
        };

        let error = publish_file_atomically_impl(&temp, &destination, &rename, &|path| {
            remove_path_if_exists(path)
        })
        .expect_err("publish");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&destination).expect("dest"), "old");
        assert!(temp.exists());
    }

    #[test]
    fn publish_file_atomically_reports_when_publish_and_rollback_both_fail() {
        let dir = tempdir().expect("tempdir");
        let temp = dir.path().join("temp.json");
        let destination = dir.path().join("dest.json");
        fs::write(&temp, "new").expect("temp");
        fs::write(&destination, "old").expect("dest");

        let calls = AtomicUsize::new(0);
        let rename = |from: &Path, to: &Path| {
            let count = calls.fetch_add(1, Ordering::SeqCst);
            if count > 0 {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    if count == 1 {
                        "simulated publish failure"
                    } else {
                        "simulated rollback failure"
                    },
                ));
            }
            fs::rename(from, to)
        };

        let error = publish_file_atomically_impl(&temp, &destination, &rename, &|path| {
            remove_path_if_exists(path)
        })
        .expect_err("publish and rollback must fail");

        let message = error.to_string();
        assert!(message.contains("simulated publish failure"));
        assert!(message.contains("rollback failed"));
        assert!(message.contains("simulated rollback failure"));
        assert!(!destination.exists(), "target needs manual inspection");
    }

    #[test]
    fn replace_file_rollback_failure_is_typed_as_uncertain() {
        let error = replace_file_rollback_error(
            std::io::Error::new(ErrorKind::PermissionDenied, "publish failed"),
            Err(std::io::Error::new(
                ErrorKind::PermissionDenied,
                "rollback failed",
            )),
            "failed to publish staged artifact file",
            ReplaceFileFailureState::Restored,
        );

        assert_eq!(error.target_state, ReplaceFileFailureState::Uncertain);
        assert!(error.to_string().contains("rollback failed"));
    }

    #[test]
    fn successful_replace_file_rollback_is_typed_as_restored() {
        let error = replace_file_rollback_error(
            std::io::Error::new(ErrorKind::PermissionDenied, "publish failed"),
            Ok(()),
            "failed to publish staged artifact file",
            ReplaceFileFailureState::Restored,
        );

        assert_eq!(error.target_state, ReplaceFileFailureState::Restored);
        assert!(!error.to_string().contains("rollback failed"));
    }

    #[test]
    fn replace_file_restores_original_bytes_when_stage_disappeared() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        let missing_stage = dir.path().join("missing-stage.cf");
        fs::write(&target, "original").expect("target");

        let error = replace_file_atomically(&missing_stage, &target, "run-1", "identity")
            .expect_err("publish must fail");

        assert_eq!(error.target_state, ReplaceFileFailureState::Restored);
        assert_eq!(
            fs::read_to_string(&target).expect("restored target"),
            "original"
        );
    }

    #[test]
    fn replace_file_reports_uncertain_and_retains_backup_when_rollback_fails() {
        struct HookReset;
        impl Drop for HookReset {
            fn drop(&mut self) {
                REPLACE_FILE_TEST_HOOK.with(|hook| *hook.borrow_mut() = None);
            }
        }

        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("target.cf");
        let stage = dir.path().join("stage.cf");
        let backup = dir.path().join(".target.cf.backup-run-1");
        fs::write(&target, "original").expect("target");
        fs::write(&stage, "replacement").expect("stage");
        REPLACE_FILE_TEST_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|point| {
                Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    match point {
                        ReplaceFileTestPoint::PublishStage => "injected publish failure",
                        ReplaceFileTestPoint::RestoreBackup => "injected rollback failure",
                    },
                ))
            }));
        });
        let _reset = HookReset;

        let error = replace_file_atomically(&stage, &target, "run-1", "identity")
            .expect_err("publish and rollback must fail");

        assert_eq!(error.target_state, ReplaceFileFailureState::Uncertain);
        assert!(!target.exists());
        assert_eq!(
            fs::read_to_string(&backup).expect("retained backup"),
            "original"
        );
        assert_eq!(
            fs::read_to_string(&stage).expect("retained stage"),
            "replacement"
        );
        assert!(error.to_string().contains("rollback failed"));
    }

    #[test]
    fn fresh_corrupt_lock_file_cannot_be_stolen() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("corrupt.lock");
        let original = b"{not valid json".to_vec();
        fs::write(&lock_path, &original).expect("lock");

        let error =
            try_acquire_advisory_lock(&lock_path).expect_err("fresh malformed lock is busy");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&lock_path).expect("lock"), original);
    }

    #[test]
    fn live_advisory_lock_metadata_remains_busy() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("live.lock");
        let metadata = AdvisoryLockMetadata {
            tool: TOOL_NAME.to_owned(),
            pid: std::process::id(),
            owner_id: "live-owner".to_owned(),
            created_at: chrono::Utc::now(),
        };
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&metadata).expect("metadata"),
        )
        .expect("live lock");

        let error = try_acquire_advisory_lock(&lock_path).expect_err("busy lock");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            read_advisory_lock_metadata(&lock_path)
                .expect("metadata")
                .owner_id,
            "live-owner"
        );
    }

    #[test]
    fn concurrent_acquisition_cannot_enter_during_metadata_write() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("publish.lock");
        let (hook_ready_tx, hook_ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let lock_path_clone = lock_path.clone();

        let handle = thread::spawn(move || {
            let hook = move || {
                hook_ready_tx.send(()).expect("signal hook");
                release_rx.recv().expect("release hook");
            };
            try_acquire_advisory_lock_with_hook(&lock_path_clone, hook)
        });

        hook_ready_rx.recv().expect("hook reached");
        let contender = try_acquire_advisory_lock(&lock_path).expect_err("lock remains held");
        assert_eq!(contender.kind(), ErrorKind::WouldBlock);
        release_tx.send(()).expect("release hook");

        let first_guard = handle.join().expect("join first").expect("first lock");
        let published = read_advisory_lock_metadata(&lock_path).expect("complete metadata");
        assert_eq!(published.owner_id, advisory_lock_owner_id(&first_guard));
    }

    #[test]
    fn legacy_writer_can_win_without_being_overwritten_by_new_protocol() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("legacy-race.lock");
        let (hook_ready_tx, hook_ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let lock_path_clone = lock_path.clone();

        let handle = thread::spawn(move || {
            let hook = move || {
                hook_ready_tx.send(()).expect("signal hook");
                release_rx.recv().expect("release hook");
            };
            try_acquire_advisory_lock_with_hook(&lock_path_clone, hook)
        });

        hook_ready_rx.recv().expect("hook reached");
        let legacy = AdvisoryLockMetadata {
            tool: TOOL_NAME.to_owned(),
            pid: std::process::id(),
            owner_id: "legacy-owner".to_owned(),
            created_at: chrono::Utc::now(),
        };
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&legacy).expect("legacy metadata"),
        )
        .expect("legacy lock");
        release_tx.send(()).expect("release hook");

        let result = handle.join().expect("join new protocol");
        assert!(matches!(result, Err(error) if error.kind() == ErrorKind::AlreadyExists));
        assert_eq!(
            read_advisory_lock_metadata(&lock_path)
                .expect("legacy metadata")
                .owner_id,
            "legacy-owner"
        );
    }

    #[test]
    fn publish_file_atomically_ignores_backup_cleanup_failure() {
        let dir = tempdir().expect("tempdir");
        let temp = dir.path().join("temp.json");
        let destination = dir.path().join("dest.json");
        let backup_path = dir.path().join(format!(
            ".{}.backup-test",
            destination
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "artifact".to_owned())
        ));
        fs::write(&temp, "new").expect("temp");
        fs::write(&destination, "old").expect("dest");
        fs::write(&backup_path, "stale backup").expect("backup");

        let cleanup = |_path: &Path| {
            Err(std::io::Error::new(
                ErrorKind::PermissionDenied,
                "cleanup failed",
            ))
        };
        let result = publish_file_atomically_impl(
            &temp,
            &destination,
            &|from, to| fs::rename(from, to),
            &cleanup,
        );

        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&destination).expect("dest"), "new");
    }

    #[cfg(windows)]
    #[test]
    fn windows_publishes_staged_directory_to_new_target() {
        let dir = tempdir().expect("tempdir");
        let staging_dir = dir.path().join(".stage");
        let target_dir = dir.path().join("target");
        fs::create_dir(&staging_dir).expect("staging dir");
        fs::write(staging_dir.join("payload.txt"), "payload").expect("payload");
        assert!(!target_dir.exists());

        let outcome = replace_dir_atomically(
            &staging_dir,
            &target_dir,
            "test-run",
            "test-target",
            ".backup",
        )
        .expect("publish staged directory");

        assert_eq!(outcome.cleanup_warning, None);
        assert!(!staging_dir.exists());
        assert_eq!(
            fs::read_to_string(target_dir.join("payload.txt")).expect("target payload"),
            "payload"
        );
    }
}
