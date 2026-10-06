//! Замок команды: блокировка ОС и рядом с ней запись о команде, которая его держит.
//!
//! Одна реализация на замок `workPath` и замок файловой базы; блокировку держит общий
//! механизм `support::fs`, а здесь — только запись о владельце для отказа второй команде.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::support::error::AppError;
use crate::support::fs::{
    advisory_lock_owner_id, publish_file_atomically, read_advisory_lock_metadata,
    try_acquire_advisory_lock, AdvisoryLockGuard,
};

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
