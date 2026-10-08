use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::support::machine::{host_name, is_process_running};
use crate::support::path::{filesystem_object_identity, open_file_identity};

pub const TOOL_NAME: &str = "v8-runner";

pub fn is_known_tool_name(tool: &str) -> bool {
    tool == TOOL_NAME
}

#[cfg(test)]
thread_local! {
    static TEST_LOCK_WRITE_HOOK: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        std::cell::RefCell::new(None);
    /// Runs between opening the system lock file and locking it.
    static TEST_SYSTEM_LOCK_OPENED_HOOK: std::cell::RefCell<Option<Box<dyn Fn()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_system_lock_opened_hook() {
    TEST_SYSTEM_LOCK_OPENED_HOOK.with(|cell| {
        if let Some(hook) = cell.borrow().as_ref() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_system_lock_opened_hook() {}

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
    /// Временная база раннера, в которой `make` собирает пакет из исходников.
    ThrowawayInfobase,
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
    /// The record was published while its owner held the system lock, and the owner
    /// removes it before letting that lock go. Whoever holds the system lock and still
    /// finds such a record knows its owner died. Records without the mark come from
    /// writers that did not hold the system lock and stay fail-closed.
    #[serde(default)]
    pub system_lock: bool,
    /// Machine the owner ran on: `pid` means something only there. A marked record
    /// without it is taken as written on this machine only when this machine has no
    /// name either; otherwise it stays until removed by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// A held advisory lock: the OS lock on the `.system` file and the owner record next
/// to it. Dropping the guard removes both files while the lock is still held and only
/// then releases it, so no lock file outlives its command.
#[derive(Debug)]
pub struct AdvisoryLockGuard {
    file: Option<File>,
    system_path: PathBuf,
    path: PathBuf,
    metadata: AdvisoryLockMetadata,
}

impl Drop for AdvisoryLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            if lock_file_owned_by(&self.path, &self.metadata.owner_id) {
                let _ = std::fs::remove_file(&self.path);
            }
            // Only the holder removes the system file, and only while holding it: a
            // waiter that opened this file before the removal takes a lock on an
            // unnamed file, sees the name no longer refers to it and retries.
            if file_still_named_by(&file, &self.system_path).unwrap_or(false) {
                let _ = std::fs::remove_file(&self.system_path);
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

/// Тестовый шов, вклинивающийся в замену файла в названной точке.
#[cfg(test)]
type ReplaceFileTestHook = Box<dyn Fn(ReplaceFileTestPoint) -> std::io::Result<()>>;

#[cfg(test)]
thread_local! {
    static REPLACE_FILE_TEST_HOOK: std::cell::RefCell<Option<ReplaceFileTestHook>> =
        const { std::cell::RefCell::new(None) };
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

/// How long a blocking acquisition keeps waiting while the system file only stays
/// pending removal. A removal is pending while some process — a former holder, an
/// antivirus or an indexer — still has the removed file open, which passes; an access
/// denial that outlasts this window is taken as a real one and reported.
const PENDING_REMOVAL_WAIT: Duration = Duration::from_secs(30);

/// Takes the lock, waiting while another holder keeps it.
pub fn acquire_advisory_lock(path: &Path) -> std::io::Result<AdvisoryLockGuard> {
    let mut pending_since: Option<Instant> = None;
    loop {
        match try_acquire_advisory_lock(path) {
            Ok(guard) => return Ok(guard),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if is_system_lock_pending_removal(&error) {
                    let since = *pending_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= PENDING_REMOVAL_WAIT {
                        return Err(pending_removal_access_error(error));
                    }
                } else {
                    pending_since = None;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Takes the lock or reports `WouldBlock` when another holder keeps it.
///
/// Does not wait for another holder, but may pause briefly: when a former holder has
/// just removed the system file, the name is opened again, and on Windows a removal
/// still pending is waited out for up to about a second before the lock is reported
/// busy.
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
        system_lock: true,
        host: host_name(),
    };
    let encoded = serde_json::to_vec_pretty(&metadata)
        .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
    let system_path = advisory_system_lock_path(path)?;
    let file = lock_named_system_file(path, &system_path)?;
    // From here on the guard owns the system file: a failed publication drops it, and
    // the drop removes the system file under the lock before releasing it.
    let guard = AdvisoryLockGuard {
        file: Some(file),
        system_path,
        path: path.to_path_buf(),
        metadata,
    };
    publish_advisory_lock_metadata(path, parent, &encoded)?;
    Ok(guard)
}

/// How many times acquisition reopens the system file before it reports the error.
/// A reopen is needed when a previous holder removed the file between this process
/// opening it and locking it, or, on Windows, while that removal is still pending.
/// Together with the pause this bounds one attempt at about a second.
const SYSTEM_LOCK_REOPEN_ATTEMPTS: u32 = 200;
const SYSTEM_LOCK_REOPEN_PAUSE: Duration = Duration::from_millis(5);

/// Locks the file currently named `system_path`. The holder removes that file before
/// releasing it, so a lock taken on a handle opened earlier may belong to a file that
/// no longer has the name; such a lock is dropped and the name is opened again.
fn lock_named_system_file(path: &Path, system_path: &Path) -> std::io::Result<File> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let retry_allowed = attempt < SYSTEM_LOCK_REOPEN_ATTEMPTS;
        let file = match OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(system_path)
        {
            Ok(file) => file,
            Err(error) if is_pending_removal(&error) => {
                if !retry_allowed {
                    return Err(system_lock_pending_removal(path, error));
                }
                thread::sleep(SYSTEM_LOCK_REOPEN_PAUSE);
                continue;
            }
            Err(error) => return Err(error),
        };
        run_system_lock_opened_hook();
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(lock_already_held(path)),
            Err(TryLockError::Error(error)) => return Err(error),
        }
        if file_still_named_by(&file, system_path)? {
            return Ok(file);
        }
        if !retry_allowed {
            return Err(lock_already_held(path));
        }
    }
}

/// Whether `path` still names the file `file` was opened from. A missing name, or on
/// Windows a name whose removal is still pending, does not.
fn file_still_named_by(file: &File, path: &Path) -> std::io::Result<bool> {
    let held = open_file_identity(file)?;
    match filesystem_object_identity(path) {
        Ok(named) => Ok(named == held),
        Err(error) if error.kind() == ErrorKind::NotFound || is_pending_removal(&error) => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// On Windows a removed file keeps its name until the last handle closes, and opening
/// that name fails with access denied or delete pending. Elsewhere a removed name is
/// simply gone.
///
/// An access denial from missing rights carries the same code and cannot be told
/// apart here; the reopen attempts and [`PENDING_REMOVAL_WAIT`] bound how long it is
/// taken for a pending removal.
#[cfg(windows)]
fn is_pending_removal(error: &std::io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_DELETE_PENDING};

    error.raw_os_error().is_some_and(|code| {
        [ERROR_ACCESS_DENIED, ERROR_DELETE_PENDING]
            .into_iter()
            .any(|pending| i32::try_from(pending) == Ok(code))
    })
}

#[cfg(not(windows))]
fn is_pending_removal(_error: &std::io::Error) -> bool {
    false
}

/// The system file stayed pending removal through every reopen. To a caller this is a
/// busy lock; the original denial is kept as the source.
#[derive(Debug, thiserror::Error)]
#[error("lock is already held: {} (its file is still pending removal: {source})", path.display())]
struct SystemLockPendingRemoval {
    path: PathBuf,
    #[source]
    source: std::io::Error,
}

fn system_lock_pending_removal(path: &Path, source: std::io::Error) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::WouldBlock,
        SystemLockPendingRemoval {
            path: path.to_path_buf(),
            source,
        },
    )
}

fn is_system_lock_pending_removal(error: &std::io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<SystemLockPendingRemoval>())
}

/// Gives back the access denial behind a pending removal that never ended.
fn pending_removal_access_error(error: std::io::Error) -> std::io::Error {
    let kind = error.kind();
    match error.into_inner() {
        Some(inner) => match inner.downcast::<SystemLockPendingRemoval>() {
            Ok(pending) => pending.source,
            Err(inner) => std::io::Error::new(kind, inner),
        },
        None => std::io::Error::from(kind),
    }
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
    // The candidate is named after the lock, so one left by a killed process still
    // matches the lock's own name pattern (for a dump lock, `.dump-*.lock*`).
    let mut candidate = candidate_next_to(path, None)?;
    write_advisory_lock_metadata(candidate.as_file_mut(), encoded)?;
    candidate.as_file().sync_all()?;

    match candidate.persist_noclobber(path) {
        Ok(_) => {
            let _ = best_effort_fsync_dir(parent);
            Ok(())
        }
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            refuse_unless_left_by_dead_owner(path)?;
            error.file.persist(path).map_err(|error| error.error)?;
            let _ = best_effort_fsync_dir(parent);
            Ok(())
        }
        Err(error) => Err(error.error),
    }
}

/// The caller holds the system lock and found an owner record in place. Only a record
/// marked as written under the system lock, whose process is known to be gone, may be
/// replaced. A marked record whose process still runs here, although the caller holds
/// the system lock, means either that the pid was reused by an unrelated process or
/// that the file system ignored the system lock (some network file systems do).
/// Neither ends by waiting, so the record stays until removed by hand. A record from
/// another machine cannot be checked from here, and an unmarked one may belong to a
/// writer that never takes the system lock; both stay until removed by hand too.
fn refuse_unless_left_by_dead_owner(path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = read_advisory_lock_metadata(path) else {
        return Err(legacy_lock_requires_offline_cleanup(path));
    };
    if !metadata.system_lock {
        return Err(legacy_lock_requires_offline_cleanup(path));
    }
    // A marked record without a host comes only from a writer that could not learn its
    // own host name; it is taken as written here only by a reader in the same position.
    if metadata.host != host_name() {
        return Err(owner_on_another_host(
            path,
            metadata.pid,
            metadata.host.as_deref(),
        ));
    }
    if is_process_running(metadata.pid) {
        return Err(owner_process_still_running(path, metadata.pid));
    }
    Ok(())
}

fn lock_already_held(path: &Path) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::WouldBlock,
        format!("lock is already held: {}", path.display()),
    )
}

fn owner_process_still_running(path: &Path, pid: u32) -> std::io::Error {
    std::io::Error::new(
        ErrorKind::AlreadyExists,
        format!(
            "lock at '{}' is owned by process {pid}, which is still running; if that process is not v8-runner, remove this file manually",
            path.display()
        ),
    )
}

fn owner_on_another_host(path: &Path, pid: u32, owner_host: Option<&str>) -> std::io::Error {
    let host = owner_host.map_or_else(
        || "an unrecorded host".to_owned(),
        |host| format!("host '{host}'"),
    );
    std::io::Error::new(
        ErrorKind::AlreadyExists,
        format!(
            "lock at '{}' is owned by process {pid} on {host}, which this machine cannot check; once that process has stopped, remove this file manually",
            path.display()
        ),
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

/// Читает журнал платформы, сам определяя кодировку.
///
/// 1С пишет `/Out` и `--file` не всегда в UTF-8: на русской Windows это обычно cp1251,
/// а с отметкой порядка байтов — UTF-16. `read_to_string` на таком файле отдаёт
/// `InvalidData`, замечания инструмента пропадают, и проверка выглядит чистой.
///
/// Порядок разбора: отметка порядка байтов важнее содержимого, потому что она
/// однозначна; дальше пробуется UTF-8, потому что он самопроверяемый — случайный
/// cp1251-текст почти никогда не складывается в корректную последовательность; и лишь
/// в остатке текст читается как cp1251, где допустим любой байт и ошибиться уже нельзя.
pub fn read_platform_log(path: &Path) -> std::io::Result<String> {
    decode_platform_log(&std::fs::read(path)?)
}

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

fn decode_platform_log(bytes: &[u8]) -> std::io::Result<String> {
    if let Some(rest) = bytes.strip_prefix(UTF16LE_BOM) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(UTF16BE_BOM) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(_) => Ok(decode_cp1251(bytes)),
    }
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> std::io::Result<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "utf-16 log has an odd number of bytes",
        ));
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| unit(*pair))
        .collect();
    String::from_utf16(&units)
        .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error.to_string()))
}

/// Верхняя половина cp1251: младшие 128 позиций совпадают с ASCII.
const CP1251_HIGH: [char; 128] = [
    'Ђ', 'Ѓ', '‚', 'ѓ', '„', '…', '†', '‡', '€', '‰', 'Љ', '‹', 'Њ', 'Ќ', 'Ћ', 'Џ', 'ђ', '‘', '’',
    '“', '”', '•', '–', '—', '\u{98}', '™', 'љ', '›', 'њ', 'ќ', 'ћ', 'џ', '\u{a0}', 'Ў', 'ў', 'Ј',
    '¤', 'Ґ', '¦', '§', 'Ё', '©', 'Є', '«', '¬', '\u{ad}', '®', 'Ї', '°', '±', 'І', 'і', 'ґ', 'µ',
    '¶', '·', 'ё', '№', 'є', '»', 'ј', 'Ѕ', 'ѕ', 'ї', 'А', 'Б', 'В', 'Г', 'Д', 'Е', 'Ж', 'З', 'И',
    'Й', 'К', 'Л', 'М', 'Н', 'О', 'П', 'Р', 'С', 'Т', 'У', 'Ф', 'Х', 'Ц', 'Ч', 'Ш', 'Щ', 'Ъ', 'Ы',
    'Ь', 'Э', 'Ю', 'Я', 'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'й', 'к', 'л', 'м', 'н', 'о',
    'п', 'р', 'с', 'т', 'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ъ', 'ы', 'ь', 'э', 'ю', 'я',
];

fn decode_cp1251(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| match *byte {
            ascii @ 0x00..=0x7F => ascii as char,
            high => CP1251_HIGH[usize::from(high - 0x80)],
        })
        .collect()
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
        // Родитель голого относительного пути (`Deploy`) — пустой путь, то есть текущий
        // каталог (#443).
        let path = if path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            path
        };
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

/// Suffix of the temporary file [`write_file_atomically`] creates next to its target:
/// `<имя цели>.candidate-<случайное>`. A file left by a killed process keeps this name.
pub const ATOMIC_WRITE_CANDIDATE_SUFFIX: &str = ".candidate-";

/// Contents of a file, or `None` when there is no such file.
pub fn read_optional(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Temporary file next to `path`, named `<имя цели>.candidate-<случайное>`: one left by a
/// killed process keeps the target's name, and whoever owns the target can find and remove
/// it. The only place that creates such files.
fn candidate_next_to(
    path: &Path,
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<tempfile::NamedTempFile> {
    let invalid = |what: &str| {
        std::io::Error::new(
            ErrorKind::InvalidInput,
            format!("path has no {what}: {}", path.display()),
        )
    };
    let parent = path.parent().ok_or_else(|| invalid("parent"))?;
    let mut prefix = path
        .file_name()
        .ok_or_else(|| invalid("file name"))?
        .to_os_string();
    prefix.push(ATOMIC_WRITE_CANDIDATE_SUFFIX);
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix);
    if let Some(permissions) = permissions {
        builder.permissions(permissions);
    }
    builder.tempfile_in(parent)
}

/// Replace `path` with what `fill` writes, so a reader sees the previous file or the new
/// one whole. The replacement keeps the permissions of the file it replaces; a new file
/// gets the ordinary mode of a created file (`0o666` less the umask).
pub fn write_file_atomically(
    path: &Path,
    fill: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let previous = match std::fs::metadata(path) {
        Ok(previous) => Some(previous.permissions()),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let mut candidate = candidate_next_to(path, new_file_permissions())?;
    if let Some(previous) = previous {
        candidate.as_file().set_permissions(previous)?;
    }
    fill(candidate.as_file_mut())?;
    candidate.as_file().sync_all()?;
    candidate.persist(path).map_err(|error| error.error)?;
    if let Some(parent) = path.parent() {
        let _ = best_effort_fsync_dir(parent);
    }
    Ok(())
}

/// The mode `File::create` would give, so the umask applies; other systems keep defaults.
fn new_file_permissions() -> Option<std::fs::Permissions> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::Permissions::from_mode(0o666))
    }
    #[cfg(not(unix))]
    {
        None
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

/// Copy a directory tree into `destination`, creating it; existing files are overwritten.
pub fn copy_dir_recursively(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursively(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Move a file: a rename when the filesystem allows it, a copy and removal otherwise.
pub fn move_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    if std::fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    std::fs::copy(source, destination)?;
    std::fs::remove_file(source)
}

/// Move a directory: a rename when the filesystem allows it, a copy and removal otherwise.
pub fn move_dir(source: &Path, destination: &Path) -> std::io::Result<()> {
    if std::fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    copy_dir_recursively(source, destination)?;
    std::fs::remove_dir_all(source)
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
        decode_platform_log, is_system_lock_pending_removal, pending_removal_access_error,
        publish_file_atomically, publish_file_atomically_impl, read_advisory_lock_metadata,
        remove_path_if_exists, replace_file_atomically, replace_file_rollback_error,
        system_lock_pending_removal, try_acquire_advisory_lock,
        try_acquire_advisory_lock_with_hook, AdvisoryLockMetadata, ReplaceFileFailureState,
        ReplaceFileTestPoint, CP1251_HIGH, REPLACE_FILE_TEST_HOOK, TEST_SYSTEM_LOCK_OPENED_HOOK,
        TOOL_NAME,
    };
    use crate::support::machine::host_name;
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

    fn directory_entries(dir: &Path) -> Vec<String> {
        let mut entries: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn released_advisory_lock_leaves_no_files_and_can_be_reacquired() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("released.lock");
        let first = acquire_advisory_lock(&lock_path).expect("first lock");
        assert!(advisory_system_lock_path(&lock_path)
            .expect("system lock path")
            .is_file());
        drop(first);

        assert!(directory_entries(dir.path()).is_empty());
        let second = try_acquire_advisory_lock(&lock_path).expect("second lock");
        assert!(!advisory_lock_owner_id(&second).is_empty());
        drop(second);
        assert!(directory_entries(dir.path()).is_empty());
    }

    #[test]
    fn files_left_by_a_killed_owner_do_not_block_and_are_removed() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("killed.lock");
        let system_path = advisory_system_lock_path(&lock_path).expect("system lock path");
        let dead = AdvisoryLockMetadata {
            tool: TOOL_NAME.to_owned(),
            pid: i32::MAX as u32,
            owner_id: "killed-owner".to_owned(),
            created_at: chrono::Utc::now(),
            system_lock: true,
            host: host_name(),
        };
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&dead).expect("record"),
        )
        .expect("record left by the killed owner");
        fs::write(&system_path, b"").expect("system file left by the killed owner");

        let guard = try_acquire_advisory_lock(&lock_path).expect("lock after a killed owner");
        assert_eq!(
            read_advisory_lock_metadata(&lock_path)
                .expect("own record")
                .owner_id,
            advisory_lock_owner_id(&guard)
        );
        drop(guard);

        assert!(directory_entries(dir.path()).is_empty());
    }

    #[test]
    fn a_failed_acquisition_leaves_no_system_file() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("refused.lock");
        fs::write(&lock_path, b"legacy owner").expect("legacy lock");

        try_acquire_advisory_lock(&lock_path).expect_err("legacy owner keeps the lock");

        assert_eq!(directory_entries(dir.path()), ["refused.lock"]);
    }

    /// Holders remove the system file while still holding it, so a contender may open the
    /// file, lose the name to a newer one and only then lock it. That lock guards
    /// nothing: the contender must see the name moved on and find the newer file held.
    #[test]
    fn a_lock_taken_on_a_removed_system_file_does_not_admit_a_second_holder() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("contended.lock");
        let first = acquire_advisory_lock(&lock_path).expect("first holder");
        let (opened_tx, opened_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel::<()>();
        let contender_path = lock_path.clone();

        let contender = thread::spawn(move || {
            let paused = std::cell::Cell::new(false);
            TEST_SYSTEM_LOCK_OPENED_HOOK.with(|cell| {
                *cell.borrow_mut() = Some(Box::new(move || {
                    if !paused.replace(true) {
                        opened_tx.send(()).expect("signal opened");
                        resume_rx.recv().expect("resume");
                    }
                }));
            });
            let result = try_acquire_advisory_lock(&contender_path).map(drop);
            TEST_SYSTEM_LOCK_OPENED_HOOK.with(|cell| *cell.borrow_mut() = None);
            result
        });

        opened_rx.recv().expect("contender opened the first file");
        drop(first);
        let second = try_acquire_advisory_lock(&lock_path).expect("second holder");
        resume_tx.send(()).expect("resume contender");

        let error = contender
            .join()
            .expect("join contender")
            .expect_err("the second holder still holds the lock");
        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        drop(second);
        assert!(directory_entries(dir.path()).is_empty());
    }

    fn write_owner_record(lock_path: &Path, pid: u32, host: Option<String>) {
        let record = AdvisoryLockMetadata {
            tool: TOOL_NAME.to_owned(),
            pid,
            owner_id: "recorded-owner".to_owned(),
            created_at: chrono::Utc::now(),
            system_lock: true,
            host,
        };
        fs::write(
            lock_path,
            serde_json::to_vec_pretty(&record).expect("record"),
        )
        .expect("owner record");
    }

    fn recorded_owner_id(lock_path: &Path) -> String {
        read_advisory_lock_metadata(lock_path)
            .expect("owner record")
            .owner_id
    }

    /// Where the OS lock is ignored (some network file systems), the system file no
    /// longer keeps a second process out, and a marked record of a running owner is the
    /// only sign that the lock is held. The same record with a reused pid would never
    /// clear by itself, so acquisition refuses at once instead of waiting.
    #[test]
    fn a_marked_record_of_a_running_owner_is_not_replaced() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("running.lock");
        write_owner_record(&lock_path, std::process::id(), host_name());

        let error = try_acquire_advisory_lock(&lock_path).expect_err("the owner still runs");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(error
            .to_string()
            .contains(&format!("process {}", std::process::id())));
        assert!(error.to_string().contains("remove this file manually"));
        assert_eq!(recorded_owner_id(&lock_path), "recorded-owner");
        assert_eq!(directory_entries(dir.path()), ["running.lock"]);

        let started = std::time::Instant::now();
        let error = acquire_advisory_lock(&lock_path).expect_err("blocking acquisition refuses");
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(recorded_owner_id(&lock_path), "recorded-owner");
    }

    #[test]
    fn a_marked_record_of_a_stopped_owner_on_this_host_is_replaced() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("stopped.lock");
        write_owner_record(&lock_path, i32::MAX as u32, host_name());

        let guard = try_acquire_advisory_lock(&lock_path).expect("the owner has stopped");

        assert_eq!(
            recorded_owner_id(&lock_path),
            advisory_lock_owner_id(&guard)
        );
        drop(guard);
        assert!(directory_entries(dir.path()).is_empty());
    }

    #[test]
    fn a_marked_record_from_another_host_is_not_replaced() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("remote.lock");
        let other_host = format!("{}-elsewhere", host_name().unwrap_or_default());
        write_owner_record(&lock_path, i32::MAX as u32, Some(other_host));

        let error = try_acquire_advisory_lock(&lock_path)
            .expect_err("a process on another host cannot be checked");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("remove this file manually"));
        assert_eq!(recorded_owner_id(&lock_path), "recorded-owner");
        assert_eq!(directory_entries(dir.path()), ["remote.lock"]);
    }

    /// Only a writer that could not learn its host name leaves a marked record without
    /// one, so a reader that knows its own name cannot take the record as written here.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_marked_record_without_a_host_is_not_replaced() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("hostless.lock");
        write_owner_record(&lock_path, i32::MAX as u32, None);

        let error =
            try_acquire_advisory_lock(&lock_path).expect_err("the host of the owner is unknown");

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("remove this file manually"));
        assert_eq!(recorded_owner_id(&lock_path), "recorded-owner");
        assert_eq!(directory_entries(dir.path()), ["hostless.lock"]);
    }

    /// A runner killed with `kill -9` stays a zombie until its parent waits for it, and
    /// its record already counts as left by a stopped owner.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_marked_record_of_a_killed_unreaped_owner_is_replaced() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("zombie.lock");
        let mut owner = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn owner");
        owner.kill().expect("kill owner");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while crate::support::machine::is_process_running(owner.id())
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            crate::support::machine::is_process_alive(owner.id()),
            "the killed owner is not reaped yet"
        );
        write_owner_record(&lock_path, owner.id(), host_name());

        let acquired = try_acquire_advisory_lock(&lock_path);
        owner.wait().expect("reap owner");

        let guard = acquired.expect("a killed owner waiting to be reaped has stopped");
        assert_eq!(
            recorded_owner_id(&lock_path),
            advisory_lock_owner_id(&guard)
        );
        drop(guard);
        assert!(directory_entries(dir.path()).is_empty());
    }

    #[test]
    fn a_published_record_carries_this_host() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("host.lock");
        let _guard = try_acquire_advisory_lock(&lock_path).expect("lock");

        let record = read_advisory_lock_metadata(&lock_path).expect("owner record");

        assert_eq!(record.host, host_name());
    }

    /// A process killed while writing its record leaves the candidate behind; it must
    /// still carry the lock's name so the lock's own name pattern finds it.
    #[test]
    fn a_record_candidate_is_named_after_its_lock() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join(".dump-0123.lock");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let hook_dir = dir.path().to_path_buf();
        let hook_seen = Arc::clone(&seen);

        let guard = try_acquire_advisory_lock_with_hook(&lock_path, move || {
            *hook_seen.lock().expect("seen") = directory_entries(&hook_dir);
        })
        .expect("lock");
        drop(guard);

        let seen = seen.lock().expect("seen").clone();
        let candidates: Vec<&String> = seen
            .iter()
            .filter(|name| name.as_str() != ".dump-0123.lock.system")
            .collect();
        assert_eq!(candidates.len(), 1, "{seen:?}");
        assert!(
            candidates[0].starts_with(".dump-0123.lock.candidate-"),
            "{seen:?}"
        );
    }

    #[test]
    fn a_pending_removal_reads_as_busy_and_gives_back_its_denial() {
        let denial = std::io::Error::new(ErrorKind::PermissionDenied, "access denied");
        let error = system_lock_pending_removal(Path::new("pending.lock"), denial);

        assert_eq!(error.kind(), ErrorKind::WouldBlock);
        assert!(is_system_lock_pending_removal(&error));
        assert!(!is_system_lock_pending_removal(&std::io::Error::from(
            ErrorKind::WouldBlock
        )));
        let denial = pending_removal_access_error(error);
        assert_eq!(denial.kind(), ErrorKind::PermissionDenied);
        assert_eq!(denial.to_string(), "access denied");
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
            system_lock: false,
            host: None,
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
            system_lock: false,
            host: None,
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
            system_lock: false,
            host: None,
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

    /// 1С пишет журнал не только в UTF-8: на русской Windows это обычно cp1251, а с
    /// отметкой порядка байтов — UTF-16. Раньше такой журнал не читался вовсе, и
    /// замечания инструмента пропадали.
    #[test]
    fn a_platform_log_is_decoded_by_its_own_encoding() {
        let text = "{CommonModules.Тест(12,3)}: Ошибка компиляции";

        let utf8 = decode_platform_log(text.as_bytes()).expect("utf-8");
        assert_eq!(utf8, text);

        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice(text.as_bytes());
        assert_eq!(decode_platform_log(&with_bom).expect("utf-8 bom"), text);

        let mut utf16le = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            utf16le.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_platform_log(&utf16le).expect("utf-16le"), text);

        let mut utf16be = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            utf16be.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(decode_platform_log(&utf16be).expect("utf-16be"), text);

        // cp1251: кириллица — по одному байту, начиная с 0xC0 для «А».
        let cp1251: Vec<u8> = text
            .chars()
            .map(|ch| match ch {
                ascii if ascii.is_ascii() => ascii as u8,
                cyrillic => {
                    let index = CP1251_HIGH
                        .iter()
                        .position(|candidate| *candidate == cyrillic)
                        .expect("character is representable in cp1251");
                    0x80 + index as u8
                }
            })
            .collect();
        assert!(
            std::str::from_utf8(&cp1251).is_err(),
            "the cp1251 sample must not be valid utf-8, or the test proves nothing"
        );
        assert_eq!(decode_platform_log(&cp1251).expect("cp1251"), text);
    }

    /// The temporary file keeps the target's name and the replaced file keeps its mode.
    #[cfg(unix)]
    #[test]
    fn an_atomic_write_keeps_the_mode_and_leaves_no_candidate() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("dir");
        let target = dir.path().join("ConfigDumpInfo.xml");
        fs::write(&target, "old").expect("old");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o664)).expect("mode");
        let mut seen = None;
        super::write_file_atomically(&target, |file| {
            seen = fs::read_dir(dir.path())
                .expect("list")
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .find(|name| name != "ConfigDumpInfo.xml");
            file.write_all(b"new")
        })
        .expect("write");
        let candidate = seen.expect("a candidate exists while writing");
        assert!(
            candidate.starts_with("ConfigDumpInfo.xml.candidate-"),
            "{candidate}"
        );
        assert_eq!(fs::read(&target).expect("target"), b"new");
        let mode = fs::metadata(&target).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o664);
        assert_eq!(fs::read_dir(dir.path()).expect("list").count(), 1);
    }

    /// A new file gets the mode `File::create` gives under the same umask, not `0o600`.
    #[cfg(unix)]
    #[test]
    fn an_atomic_write_of_a_new_file_follows_the_umask() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("dir");
        let reference = dir.path().join("reference");
        fs::File::create(&reference).expect("reference");
        let target = dir.path().join("identity");
        super::write_file_atomically(&target, |_| Ok(())).expect("write");
        let mode = |path: &Path| fs::metadata(path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&target), mode(&reference));
    }

    #[test]
    fn an_atomic_write_refuses_a_path_without_a_file_name() {
        let error =
            super::write_file_atomically(Path::new("/"), |_| Ok(())).expect_err("no file name");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    /// Пустой путь — родитель голого относительного имени — означает текущий каталог (#443).
    #[test]
    fn an_empty_parent_is_the_current_directory_for_fsync() {
        super::best_effort_fsync_dir(std::path::Path::new("")).expect("fsync of the current dir");
    }
}
