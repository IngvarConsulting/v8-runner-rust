use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::model::AppConfig;
use crate::support::error::AppError;
use crate::support::fs::publish_file_atomically;
use crate::support::fs::{
    advisory_lock_owner_id, read_advisory_lock_metadata, try_acquire_advisory_lock,
    AdvisoryLockGuard,
};
use crate::support::path::nearest_existing_canonical_path;

const WORKSPACE_LOCK_FILE_NAME: &str = ".v8-runner.workspace.lock";
#[cfg(test)]
const WORKSPACE_LOCK_SIDECAR_FILE_NAME: &str = ".v8-runner.workspace.lock.json";

/// Замок команды: блокировка ОС и рядом с ней запись о команде, которая его держит.
///
/// Владеет замком только блокировка ОС (`support::fs`); запись — диагностика для отказа
/// второй команде. Её удаляет сброс охранника, а запись убитой команды отказ не
/// использует: она названа другим владельцем, чем текущая блокировка.
#[derive(Debug)]
pub(crate) struct CommandLockGuard {
    _lock: AdvisoryLockGuard,
    sidecar_path: PathBuf,
}

impl Drop for CommandLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.sidecar_path);
    }
}

/// Кто держит замок команды: запись рядом с блокировкой.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommandLockHolder {
    pub(crate) pid: u32,
    lock_owner: String,
    pub(crate) command: String,
    pub(crate) started_at: DateTime<Utc>,
    /// Рабочий каталог команды-владельца: он называет и рабочую копию.
    pub(crate) canonical_work_path: PathBuf,
}

pub(crate) fn acquire_workspace_lock(
    config: &AppConfig,
    command_name: &str,
) -> Result<CommandLockGuard, AppError> {
    let canonical_work_path =
        nearest_existing_canonical_path(&config.work_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to canonicalize workPath '{}': {error}",
                config.work_path.display()
            ))
        })?;
    let lock_path = workspace_lock_path(&canonical_work_path);

    take_command_lock(&lock_path, command_name, &canonical_work_path).map_err(|error| {
        match error.kind() {
            ErrorKind::WouldBlock | ErrorKind::AlreadyExists => AppError::WorkspaceBusy(format!(
                "{}; {error}",
                render_busy_message(command_name, &canonical_work_path, &lock_path)
            )),
            _ => AppError::Runtime(format!(
                "failed to acquire {command_name} workspace lock '{}': {error}",
                lock_path.display()
            )),
        }
    })
}

/// Берёт замок команды без ожидания и записывает рядом, кто его держит.
///
/// Ошибка — та же, что у [`try_acquire_advisory_lock`]: `WouldBlock` или `AlreadyExists`
/// значат, что замок держит другой владелец, остальные — что взять его здесь нельзя.
/// Запись о владельце не пишется — замок всё равно взят: она только диагностика.
pub(crate) fn take_command_lock(
    lock_path: &Path,
    command_name: &str,
    canonical_work_path: &Path,
) -> std::io::Result<CommandLockGuard> {
    let sidecar_path = command_lock_sidecar_path(lock_path);
    let lock = try_acquire_advisory_lock(lock_path)?;

    cleanup_sidecar_temp_files(&sidecar_path);

    if let Err(error) = write_lock_metadata(&sidecar_path, command_name, canonical_work_path, &lock)
    {
        let _ = std::fs::remove_file(&sidecar_path);
        warn!(
            command = command_name,
            sidecar_path = %sidecar_path.display(),
            error = %error,
            "failed to write command lock metadata; continuing without sidecar"
        );
    }

    Ok(CommandLockGuard {
        _lock: lock,
        sidecar_path,
    })
}

/// Запись о том, кто держит замок, — если она относится к нынешней блокировке. Запись
/// убитой команды или чужая запись не годится: отказ тогда не называет владельца.
pub(crate) fn command_lock_holder(lock_path: &Path) -> Option<CommandLockHolder> {
    let active_lock = read_advisory_lock_metadata(lock_path).ok()?;
    read_lock_metadata(&command_lock_sidecar_path(lock_path))
        .ok()
        .filter(|holder| holder.lock_owner == active_lock.owner_id)
}

pub(crate) fn workspace_lock_path(work_path: &Path) -> PathBuf {
    work_path.join(WORKSPACE_LOCK_FILE_NAME)
}

/// Файл, который заводит сам замок `workPath`: файл замка и всё, что названо от него, —
/// системный файл блокировки, sidecar с описанием владельца и их временные копии.
pub(crate) fn is_workspace_lock_file(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(WORKSPACE_LOCK_FILE_NAME))
}

/// Запись о владельце лежит рядом с замком под его именем и `.json`.
fn command_lock_sidecar_path(lock_path: &Path) -> PathBuf {
    let mut name = lock_path.file_name().unwrap_or_default().to_os_string();
    name.push(".json");
    lock_path.with_file_name(name)
}

fn write_lock_metadata(
    sidecar_path: &Path,
    command_name: &str,
    canonical_work_path: &Path,
    lock: &AdvisoryLockGuard,
) -> Result<(), AppError> {
    let metadata = CommandLockHolder {
        pid: std::process::id(),
        lock_owner: advisory_lock_owner_id(lock).to_owned(),
        command: command_name.to_owned(),
        started_at: Utc::now(),
        canonical_work_path: canonical_work_path.to_path_buf(),
    };
    let encoded = serde_json::to_vec_pretty(&metadata).map_err(|error| {
        AppError::Runtime(format!("failed to encode command lock metadata: {error}"))
    })?;
    let temp_path = sidecar_path.with_extension(format!(
        "{}.tmp.{}",
        sidecar_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json"),
        std::process::id()
    ));
    std::fs::write(&temp_path, encoded).map_err(|error| {
        AppError::Runtime(format!(
            "failed to write temporary command lock metadata '{}': {error}",
            temp_path.display()
        ))
    })?;
    publish_file_atomically(&temp_path, sidecar_path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        AppError::Runtime(format!(
            "failed to publish command lock metadata '{}': {error}",
            sidecar_path.display()
        ))
    })
}

fn render_busy_message(command_name: &str, canonical_work_path: &Path, lock_path: &Path) -> String {
    match command_lock_holder(lock_path) {
        Some(holder) => format!(
            "cannot start {command_name}: workspace '{}' is already locked by '{}' (pid {}, started at {})",
            canonical_work_path.display(),
            holder.command,
            holder.pid,
            holder.started_at.to_rfc3339(),
        ),
        None => format!(
            "cannot start {command_name}: workspace '{}' is already in use by another command",
            canonical_work_path.display()
        ),
    }
}

fn read_lock_metadata(path: &Path) -> std::io::Result<CommandLockHolder> {
    let raw = std::fs::read(path)?;
    serde_json::from_slice(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// Убирает временные копии записи о владельце, оставленные прерванной записью.
fn cleanup_sidecar_temp_files(sidecar_path: &Path) {
    let (Some(dir), Some(name)) = (
        sidecar_path.parent(),
        sidecar_path.file_name().and_then(|name| name.to_str()),
    ) else {
        return;
    };
    let prefix = format!("{name}.tmp.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let matches_prefix = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&prefix));
        if matches_prefix {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_workspace_lock, workspace_lock_path, CommandLockGuard,
        WORKSPACE_LOCK_SIDECAR_FILE_NAME,
    };
    use crate::config::model::{
        AppConfig, BuildConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolsConfig,
    };
    use crate::support::fs::acquire_advisory_lock;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn sample_config(work_path: &Path) -> AppConfig {
        AppConfig {
            base_path: work_path.join("base"),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("main"),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn hold_lock(config: &AppConfig, command_name: &str) -> CommandLockGuard {
        acquire_workspace_lock(config, command_name).expect("workspace lock")
    }

    #[cfg(unix)]
    #[test]
    fn conflicts_use_canonical_workspace_path_and_sidecar_metadata() {
        let dir = tempdir().expect("tempdir");
        let real_work = dir.path().join("real-work");
        fs::create_dir_all(&real_work).expect("work dir");
        let link_work = dir.path().join("work-link");
        std::os::unix::fs::symlink(&real_work, &link_work).expect("symlink");

        let first = sample_config(&real_work);
        let second = sample_config(&link_work);
        let _guard = hold_lock(&first, "build");

        let error = acquire_workspace_lock(&second, "test").expect_err("busy workspace");
        let message = error.to_string();

        assert!(message.contains(
            &std::fs::canonicalize(&real_work)
                .expect("canonical")
                .display()
                .to_string()
        ));
        assert!(message.contains("'build'"));
        assert!(message.contains("pid"));
        assert!(message.contains("started at"));
    }

    #[test]
    fn stale_sidecar_metadata_falls_back_to_generic_busy_message() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = std::fs::canonicalize(&work).expect("canonical");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let sidecar_path = canonical_work.join(WORKSPACE_LOCK_SIDECAR_FILE_NAME);
        fs::write(
            &sidecar_path,
            r#"{"pid":999999,"command":"push","started_at":"2026-01-01T00:00:00Z","canonical_work_path":"/tmp/stale"}"#,
        )
        .expect("sidecar");

        let error = acquire_workspace_lock(&config, "test").expect_err("busy workspace");
        let message = error.to_string();

        assert!(message.contains("already in use by another command"));
        assert!(!message.contains("999999"));
        assert!(!message.contains("'build'"));
    }

    #[test]
    fn next_lock_acquisition_cleans_stale_sidecar_temp_files() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = std::fs::canonicalize(&work).expect("canonical");
        let stale_temp =
            canonical_work.join(format!("{WORKSPACE_LOCK_SIDECAR_FILE_NAME}.tmp.stale"));
        fs::write(&stale_temp, b"stale").expect("stale temp");

        let _guard = hold_lock(&config, "build");

        assert!(!stale_temp.exists());
    }

    /// Замок — файл ОС; sidecar — диагностика. Невозможность записать sidecar не
    /// отменяет владение каталогом: второй запуск всё равно получает «занято».
    #[test]
    fn an_unwritable_sidecar_does_not_release_the_lock() {
        let work = tempdir().expect("work");
        let config = sample_config(work.path());
        let canonical =
            crate::support::path::nearest_existing_canonical_path(work.path()).expect("canonical");
        // Каталог на месте sidecar: переименовать поверх него файл нельзя.
        std::fs::create_dir_all(canonical.join(WORKSPACE_LOCK_SIDECAR_FILE_NAME)).expect("blocker");

        let guard = acquire_workspace_lock(&config, "build")
            .expect("the OS lock is taken even when the sidecar cannot be written");
        let busy = acquire_workspace_lock(&config, "dump").expect_err("second owner is refused");
        assert!(
            matches!(busy, crate::support::error::AppError::WorkspaceBusy(_)),
            "{busy}"
        );
        drop(guard);

        acquire_workspace_lock(&config, "dump").expect("released after the first owner is gone");
    }

    #[test]
    fn drop_removes_sidecar_file() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = std::fs::canonicalize(&work).expect("canonical");
        let sidecar = canonical_work.join(".v8-runner.workspace.lock.json");

        let guard = hold_lock(&config, "build");
        assert!(workspace_lock_path(&canonical_work).exists());
        assert!(sidecar.exists());
        drop(guard);

        assert!(!sidecar.exists());
    }
}
