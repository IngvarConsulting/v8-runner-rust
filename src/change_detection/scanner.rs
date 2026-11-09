use sha2::{Digest, Sha256};
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;
use thiserror::Error;
use walkdir::WalkDir;

use crate::change_detection::file_state::{mtime_nanos, MtimeError};

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("source scan interrupted")]
    Interrupted,

    #[error("failed to walk directory '{path}': {source}")]
    Walk {
        path: PathBuf,
        source: walkdir::Error,
    },

    #[error("failed to read file '{path}': {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read metadata for '{path}': {source}")]
    Meta {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to convert mtime for '{path}': {source}")]
    Mtime { path: PathBuf, source: MtimeError },

    #[error("failed to build path relative to scan root '{root}' for '{path}'")]
    RelativePath { root: PathBuf, path: PathBuf },
}

/// Directory/file names that are always excluded from scanning.
const IGNORED_DIRS: &[&str] = &[
    ".git", ".gradle", ".build", "build", "target", "temp", "tmp", ".yaxunit",
];
const IGNORED_FILES: &[&str] = &["ConfigDumpInfo.xml"];
/// Name prefix of the file an incremental dump writes into the tree to read its clock.
pub(crate) const DUMP_START_PROBE: &str = ".v8-runner-dump-start-";

/// An ignored file, or the temporary file of its atomic replacement left by a killed process.
fn is_ignored_file(name: &str) -> bool {
    // The clock probe of an incremental dump, left behind only by a killed process.
    if name.starts_with(DUMP_START_PROBE) {
        return true;
    }
    IGNORED_FILES.iter().any(|ignored| {
        name.strip_prefix(ignored).is_some_and(|rest| {
            rest.is_empty() || rest.starts_with(crate::support::fs::ATOMIC_WRITE_CANDIDATE_SUFFIX)
        })
    })
}

/// One fully read source file. Metadata never substitutes for its content hash.
#[derive(Debug, Clone)]
pub struct HashedFile {
    pub path: PathBuf,
    pub rel_path: String,
    pub mtime_ns: u64,
    pub hash: String,
}

/// Full scanner output for one source-set root.
#[derive(Debug, Clone)]
pub struct ScanSnapshot {
    pub scan_started_at: u64,
    pub files: Vec<HashedFile>,
    pub has_untracked_links: bool,
}

/// A single-set make may publish inside its source set. Keep its lock, staged
/// packages and publication metadata out of the snapshot of platform inputs.
fn is_build_artifact(entry: &walkdir::DirEntry, name: &str) -> bool {
    if entry
        .path()
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            ["cf", "cfe", "epf", "erf"]
                .iter()
                .any(|package| extension.eq_ignore_ascii_case(package))
        })
    {
        return true;
    }
    if name
        .strip_prefix(".artifacts-")
        .and_then(|value| value.split_once(".lock"))
        .is_some_and(|(hash, _)| {
            hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return true;
    }
    let package_backup = name.starts_with('.')
        && [
            ".cf.backup-",
            ".cfe.backup-",
            ".epf.backup-",
            ".erf.backup-",
        ]
        .iter()
        .any(|suffix| name.contains(suffix));
    package_backup
        || name.ends_with(".meta.json")
            && (name.starts_with(".artifacts-stage-")
                || name.starts_with(".artifacts-backup-")
                || name.starts_with(".convert-stage-"))
}

#[derive(Clone, Copy)]
pub(super) enum ScanScope {
    Push,
    Build,
}

/// Hash every selected regular file, including equal-size edits with exactly the old
/// remembered mtime (#447). The predicate is the caller's existing interruption policy;
/// successful post-publication bookkeeping supplies an always-false predicate.
#[cfg(test)]
pub fn scan(root: &Path, interrupted: &mut dyn FnMut() -> bool) -> Result<ScanSnapshot, ScanError> {
    scan_in_scope(root, None, interrupted, ScanScope::Push)
}

/// `workPath` under a source-set root, named as the walk of that root names it. A set in the
/// workspace root holds it, and it can have any name.
pub fn work_path_inside(root: &Path, work_path: &Path) -> Option<PathBuf> {
    let root_real = root.canonicalize().ok()?;
    let work_real = work_path.canonicalize().ok()?;
    let relative = work_real.strip_prefix(&root_real).ok()?;
    (!relative.as_os_str().is_empty()).then(|| root.join(relative))
}

/// Hash the files selected by the scan scope, excluding the supplied directory.
pub(super) fn scan_in_scope(
    root: &Path,
    excluded: Option<&Path>,
    interrupted: &mut dyn FnMut() -> bool,
    scope: ScanScope,
) -> Result<ScanSnapshot, ScanError> {
    scan_with_workers(root, excluded, interrupted, scope, hash_workers())
}

fn scan_with_workers(
    root: &Path,
    excluded: Option<&Path>,
    interrupted: &mut dyn FnMut() -> bool,
    scope: ScanScope,
    workers: usize,
) -> Result<ScanSnapshot, ScanError> {
    let listed = list_in_scope(root, excluded, interrupted, scope)?;
    let files = hash_listed_with(listed.files, workers, interrupted)?;
    Ok(ScanSnapshot {
        scan_started_at: listed.scan_started_at,
        files,
        has_untracked_links: listed.has_untracked_links,
    })
}

/// The selected files of a tree with their metadata, before any content is read.
pub(super) struct Listing {
    pub(super) scan_started_at: u64,
    pub(super) files: Vec<Candidate>,
    pub(super) has_untracked_links: bool,
}

/// Walk the tree and select its source files without reading them.
pub(super) fn list_in_scope(
    root: &Path,
    excluded: Option<&Path>,
    interrupted: &mut dyn FnMut() -> bool,
    scope: ScanScope,
) -> Result<Listing, ScanError> {
    check_interruption(interrupted)?;
    let scan_started_at =
        mtime_nanos(std::time::SystemTime::now(), root).map_err(|source| ScanError::Mtime {
            path: root.to_path_buf(),
            source,
        })?;
    let mut candidates = Vec::new();
    let mut has_untracked_links = false;

    for entry in source_entries(root, excluded, scope) {
        check_interruption(interrupted)?;
        let entry = entry.map_err(|e| ScanError::Walk {
            path: root.to_path_buf(),
            source: e,
        })?;
        if matches!(scope, ScanScope::Build) && entry.file_type().is_symlink() {
            // The platform may consume a link's contents. An untracked link prevents reuse
            // but still permits the same fresh full load that the platform handled before.
            has_untracked_links = true;
        }

        if !is_source_file(&entry, root, scope) {
            continue;
        }
        let path = entry.path();
        // A selected entry is a regular file, never a link, so the walk's own metadata is the
        // file's. On Windows it comes with the directory listing and opens no file; a
        // separate `fs::metadata` opened every file of the tree once more (#571).
        let meta = entry.metadata().map_err(|e| ScanError::Meta {
            path: path.to_path_buf(),
            source: e.into_io_error().unwrap_or_else(|| {
                std::io::Error::other("the walk reported no metadata for a selected file")
            }),
        })?;

        let mtime_ns = mtime_nanos(
            meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            path,
        )
        .map_err(|source| ScanError::Mtime {
            path: path.to_path_buf(),
            source,
        })?;
        candidates.push(Candidate {
            rel_path: rel_path(root, path)?,
            path: path.to_path_buf(),
            mtime_ns,
        });
    }

    Ok(Listing {
        scan_started_at,
        files: candidates,
        has_untracked_links,
    })
}

/// Hash listed files by content, in the order they were listed.
pub(super) fn hash_listed(
    files: Vec<Candidate>,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<HashedFile>, ScanError> {
    hash_listed_with(files, hash_workers(), interrupted)
}

fn hash_listed_with(
    files: Vec<Candidate>,
    workers: usize,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<HashedFile>, ScanError> {
    let hashes = hash_candidates(&files, workers, interrupted, &hash_file)?;
    check_interruption(interrupted)?;
    Ok(files
        .into_iter()
        .zip(hashes)
        .map(|(candidate, hash)| HashedFile {
            path: candidate.path,
            rel_path: candidate.rel_path,
            mtime_ns: candidate.mtime_ns,
            hash,
        })
        .collect())
}

/// A selected file whose content is still to be read.
pub(super) struct Candidate {
    pub(super) path: PathBuf,
    pub(super) rel_path: String,
    pub(super) mtime_ns: u64,
}

/// Below this many files the threads cost more than they save; the set is read in order.
const PARALLEL_HASH_FROM: usize = 64;
/// Reading past this many threads gave nothing on the measured disks (#556).
const MAX_HASH_WORKERS: usize = 8;
/// How often the caller's interruption predicate is asked while the workers read.
const INTERRUPTION_POLL: Duration = Duration::from_millis(5);
/// No candidate has failed yet.
const NO_FAILURE: usize = usize::MAX;

fn hash_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(MAX_HASH_WORKERS)
}

/// Stops every worker when one of them panics, so the others do not read the rest of the set
/// before the panic reaches the caller.
struct StopOnPanic<'a>(&'a AtomicBool);

impl Drop for StopOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}

/// Hash every candidate, the digests in candidate order whatever the number of workers.
///
/// The interruption predicate stays on the calling thread: it is polled while the workers
/// read and raises a shared cancel flag that every worker checks between files and around
/// each block. A read failure stops the workers on later candidates only, so the earliest
/// failing candidate is reported, as a sequential read reports it. No digest is returned
/// after a cancellation or a failure.
fn hash_candidates<H>(
    candidates: &[Candidate],
    workers: usize,
    interrupted: &mut dyn FnMut() -> bool,
    hash: &H,
) -> Result<Vec<String>, ScanError>
where
    H: Fn(&Path, &mut dyn FnMut() -> bool) -> Result<String, ScanError> + Sync,
{
    let workers = workers.min(candidates.len());
    if workers <= 1 || candidates.len() < PARALLEL_HASH_FROM {
        return candidates
            .iter()
            .map(|candidate| hash(&candidate.path, interrupted))
            .collect();
    }

    let next = AtomicUsize::new(0);
    let cancel = AtomicBool::new(false);
    let first_failure = AtomicUsize::new(NO_FAILURE);
    let digests: Vec<OnceLock<String>> = candidates.iter().map(|_| OnceLock::new()).collect();
    let failure: Mutex<Option<(usize, ScanError)>> = Mutex::new(None);
    let mut cancelled = false;

    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel::<()>();
        for _ in 0..workers {
            let done = done.clone();
            let (next, cancel, first_failure, digests, failure) =
                (&next, &cancel, &first_failure, &digests, &failure);
            scope.spawn(move || {
                let _guard = StopOnPanic(cancel);
                let stopped = |index: usize| {
                    cancel.load(Ordering::Relaxed) || first_failure.load(Ordering::Relaxed) < index
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(candidate) = candidates.get(index) else {
                        break;
                    };
                    if stopped(index) {
                        break;
                    }
                    match hash(&candidate.path, &mut || stopped(index)) {
                        Ok(digest) => {
                            let stored = digests[index].set(digest);
                            debug_assert!(stored.is_ok(), "candidate {index} hashed twice");
                        }
                        // Raised by a flag: the reason is a cancellation or an earlier failure.
                        Err(ScanError::Interrupted) => break,
                        Err(error) => {
                            first_failure.fetch_min(index, Ordering::Relaxed);
                            let mut slot = failure.lock().unwrap_or_else(|e| e.into_inner());
                            if slot.as_ref().is_none_or(|(earliest, _)| index < *earliest) {
                                *slot = Some((index, error));
                            }
                            break;
                        }
                    }
                }
                drop(done);
            });
        }
        drop(done);
        // Every sender is dropped when its worker ends; the loop ends with the last of them.
        while let Err(mpsc::RecvTimeoutError::Timeout) = finished.recv_timeout(INTERRUPTION_POLL) {
            if !cancelled && interrupted() {
                cancelled = true;
                cancel.store(true, Ordering::Relaxed);
            }
        }
    });

    if cancelled {
        return Err(ScanError::Interrupted);
    }
    if let Some((_, error)) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(error);
    }
    Ok(digests
        .into_iter()
        .map(|digest| {
            digest
                .into_inner()
                .expect("without a cancellation or a failure every candidate is hashed")
        })
        .collect())
}

/// Compute SHA-256 hex digest of a file's contents.
pub fn hash_file(path: &Path, interrupted: &mut dyn FnMut() -> bool) -> Result<String, ScanError> {
    check_interruption(interrupted)?;
    let file = std::fs::File::open(path).map_err(|e| ScanError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    hash_reader(file, path, interrupted)
}

fn hash_reader(
    mut reader: impl Read,
    path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<String, ScanError> {
    // Working storage, not a maximum file size. Files of any size are streamed.
    let mut buffer = [0u8; 64 * 1024];
    let mut digest = Sha256::new();
    loop {
        check_interruption(interrupted)?;
        let length = match reader.read(&mut buffer) {
            Ok(length) => length,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(ScanError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        check_interruption(interrupted)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn check_interruption(interrupted: &mut dyn FnMut() -> bool) -> Result<(), ScanError> {
    if interrupted() {
        Err(ScanError::Interrupted)
    } else {
        Ok(())
    }
}

fn rel_path(root: &Path, path: &Path) -> Result<String, ScanError> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| ScanError::RelativePath {
            root: root.to_path_buf(),
            path: path.to_path_buf(),
        })?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// A set copied for the agent keeps a form named `build`: below the top level such a name is
/// metadata. Generated directories live at the top, the others are named by their paths.
pub(crate) fn top_level_source_entries(
    root: &Path,
    excluded: Vec<PathBuf>,
) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !((entry.depth() == 1 && is_ignored_dir(entry, ScanScope::Push))
                    || excluded.iter().any(|path| path == entry.path()))
        })
}

/// The entries of a source tree, without the generated directories and `excluded`.
pub(super) fn source_entries<'a>(
    root: &Path,
    excluded: Option<&'a Path>,
    scope: ScanScope,
) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> + 'a {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            entry.depth() == 0 || !(is_ignored_dir(entry, scope) || excluded == Some(entry.path()))
        })
}

pub(super) fn is_source_file(entry: &walkdir::DirEntry, root: &Path, scope: ScanScope) -> bool {
    entry.file_type().is_file()
        && !entry.file_name().to_str().is_some_and(|name| match scope {
            ScanScope::Push => is_ignored_file(name),
            ScanScope::Build => {
                entry.path().parent() == Some(root) && is_ignored_file(name)
                    || is_build_artifact(entry, name)
            }
        })
}

fn is_ignored_dir(entry: &walkdir::DirEntry, scope: ScanScope) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let Some(name) = entry.file_name().to_str() else {
        return false;
    };
    match scope {
        ScanScope::Push => IGNORED_DIRS.contains(&name),
        // A metadata object can itself be named build, target, temp, or tmp.
        ScanScope::Build => matches!(name, ".git" | ".gradle" | ".build" | ".yaxunit"),
    }
}

#[cfg(test)]
mod tests {
    use super::{scan, scan_in_scope, work_path_inside, ScanScope, ScanSnapshot};
    use crate::change_detection::file_state::mtime_nanos;
    use std::fs::{self, File};
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;

    fn scanned_names(snapshot: &ScanSnapshot) -> Vec<&str> {
        snapshot
            .files
            .iter()
            .map(|file| file.rel_path.as_str())
            .collect()
    }

    /// `.build` — служебный каталог на любой глубине; выбранный корень с таким именем обходится.
    #[test]
    fn the_build_directory_is_not_a_source_below_the_root() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join(".build");
        fs::create_dir_all(root.join(".build/unica")).expect("service dir");
        fs::write(root.join(".build/unica/lock"), "lock").expect("service file");
        fs::write(root.join("Configuration.xml"), "<x/>").expect("source");

        let snapshot = scan(&root, &mut || false).expect("scan");

        assert_eq!(scanned_names(&snapshot), ["Configuration.xml"]);
    }

    /// `workPath` внутри набора не исходник, как бы он ни назывался.
    #[test]
    fn a_work_path_inside_the_root_is_not_a_source_whatever_its_name() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let work = root.join("service-area");
        fs::create_dir_all(&work).expect("work");
        fs::write(work.join("log.txt"), "log").expect("service file");
        fs::write(root.join("Configuration.xml"), "<x/>").expect("source");

        let excluded = work_path_inside(&root, &work);
        assert_eq!(excluded.as_deref(), Some(work.as_path()));
        let snapshot = scan_in_scope(&root, excluded.as_deref(), &mut || false, ScanScope::Push)
            .expect("scan");

        assert_eq!(scanned_names(&snapshot), ["Configuration.xml"]);
        assert_eq!(work_path_inside(&root, dir.path()), None);
        assert_eq!(work_path_inside(&root, &root), None);
    }

    fn write_touched(path: &Path, contents: &str, modified: SystemTime) {
        fs::write(path, contents).expect("write");
        File::options()
            .write(true)
            .open(path)
            .expect("open")
            .set_modified(modified)
            .expect("set mtime");
    }

    fn candidates(snapshot: &ScanSnapshot) -> Vec<&str> {
        let mut names: Vec<&str> = snapshot
            .files
            .iter()
            .map(|candidate| candidate.rel_path.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// Every selected file is read, including old files and an unchanged mtime.
    #[test]
    fn every_selected_file_is_hashed_regardless_of_mtime() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let old = SystemTime::now() - Duration::from_secs(600);
        write_touched(&root.join("Old.bsl"), "old", old);
        write_touched(&root.join("Near.bsl"), "near", SystemTime::now());
        write_touched(&root.join("New.bsl"), "new", old);
        let snapshot = scan(root, &mut || false).expect("scan");
        assert_eq!(candidates(&snapshot), ["Near.bsl", "New.bsl", "Old.bsl"]);
        for file in &snapshot.files {
            assert_eq!(
                file.hash,
                super::hash_file(&file.path, &mut || false).expect("hash")
            );
        }
    }

    /// A restored copy whose mtime moved backwards remains in the full content scan.
    #[test]
    fn a_restored_copy_with_an_old_mtime_is_hashed() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        write_touched(
            &root.join("Module.bsl"),
            "restored bytes",
            SystemTime::now() - Duration::from_secs(600),
        );
        let snapshot = scan(root, &mut || false).expect("scan");
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
    }

    /// Служебные и порождённые каталоги и файл состояния выгрузки в обход не входят.
    #[test]
    fn service_and_generated_paths_are_never_scanned() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("Module.bsl"), "module").expect("module");
        fs::write(root.join("ConfigDumpInfo.xml"), "<info/>").expect("dump info");
        fs::write(root.join("ConfigDumpInfo.xml.candidate-x1"), "<info/>").expect("left write");
        for ignored in [
            ".git", ".gradle", "build", "target", "temp", "tmp", ".yaxunit",
        ] {
            let nested = root.join(ignored).join("nested");
            fs::create_dir_all(&nested).expect("ignored dir");
            fs::write(nested.join("File.bsl"), "generated").expect("ignored file");
        }

        let snapshot = scan(root, &mut || false).expect("scan");

        let seen: Vec<&str> = snapshot
            .files
            .iter()
            .map(|file| file.rel_path.as_str())
            .collect();
        assert_eq!(seen, ["Module.bsl"]);
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
    }

    /// Корень набора сканируется всегда, даже названный как служебный каталог; служебные
    /// каталоги внутри него пропускаются.
    #[test]
    fn selected_roots_are_scanned_while_ignored_descendants_stay_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        for root_name in super::IGNORED_DIRS {
            let root = dir.path().join(root_name);
            std::fs::create_dir(&root).expect("root");
            std::fs::write(root.join("Module.bsl"), "source").expect("source");
            for child_name in super::IGNORED_DIRS {
                let child = root.join(child_name);
                std::fs::create_dir(&child).expect("ignored child");
                std::fs::write(child.join("Module.bsl"), "generated").expect("child source");
            }
            let scanned = scan(&root, &mut || false).expect("scan");
            assert_eq!(scanned.files.len(), 1, "root {root_name}");
            assert_eq!(scanned.files[0].rel_path, "Module.bsl");
        }
    }
    /// Equal file size and the exact remembered mtime do not hide different bytes.
    #[test]
    fn changed_bytes_with_the_exact_remembered_old_mtime_are_hashed() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let module = root.join("Module.bsl");
        let watermark = SystemTime::now();
        write_touched(
            &module,
            "before bytes",
            watermark - Duration::from_secs(600),
        );
        let before = fs::metadata(&module).expect("before metadata");
        let remembered = before.modified().expect("before mtime");
        let remembered_ns = mtime_nanos(remembered, &module).expect("mtime nanos");
        let old_hash = super::hash_file(&module, &mut || false).expect("old hash");

        write_touched(&module, "edited bytes", remembered);
        let after = fs::metadata(&module).expect("after metadata");
        assert_eq!(before.len(), after.len(), "size is unchanged");
        assert_eq!(remembered, after.modified().expect("after mtime"));
        assert!(remembered_ns < mtime_nanos(watermark, root).expect("watermark") - 2_000_000_000);

        let snapshot = scan(root, &mut || false).expect("scan");
        assert_eq!(candidates(&snapshot), ["Module.bsl"]);
        assert_eq!(snapshot.files[0].mtime_ns, remembered_ns);
        assert_ne!(snapshot.files[0].hash, old_hash, "changed bytes are read");
        assert_eq!(
            snapshot.files[0].hash,
            super::hash_file(&module, &mut || false).expect("edited hash")
        );
    }
    /// Buffer capacity bounds memory, never accepted file size.
    #[test]
    fn hashing_streams_files_larger_than_its_working_buffer() {
        use std::io::{Read, Result};
        struct LargeReader {
            left: usize,
            largest: usize,
        }
        impl Read for LargeReader {
            fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
                self.largest = self.largest.max(buffer.len());
                let count = self.left.min(buffer.len());
                buffer[..count].fill(17);
                self.left -= count;
                Ok(count)
            }
        }
        let length = 3 * 64 * 1024 + 7;
        let mut reader = LargeReader {
            left: length,
            largest: 0,
        };
        let hash =
            super::hash_reader(&mut reader, Path::new("large"), &mut || false).expect("hash");
        use sha2::{Digest, Sha256};
        assert_eq!(hash, format!("{:x}", Sha256::digest(vec![17; length])));
        assert_eq!(reader.left, 0);
        assert!(reader.largest <= 64 * 1024);
    }

    /// A partially read file never yields a digest on IO failure or cancellation.
    #[test]
    fn partial_reads_fail_without_a_hash_and_cancel_before_the_next_read() {
        use std::{
            cell::Cell,
            io::{self, Read},
            rc::Rc,
        };
        struct PartialReader {
            reads: Rc<Cell<usize>>,
            cancelled: Rc<Cell<bool>>,
            cancel: bool,
        }
        impl Read for PartialReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.reads.get();
                self.reads.set(count + 1);
                if count > 0 {
                    return Err(io::Error::other("read failed after a prefix"));
                }
                buffer[..3].copy_from_slice(b"abc");
                self.cancelled.set(self.cancel);
                Ok(3)
            }
        }
        for cancel in [false, true] {
            let reads = Rc::new(Cell::new(0));
            let cancelled = Rc::new(Cell::new(false));
            let reader = PartialReader {
                reads: reads.clone(),
                cancelled: cancelled.clone(),
                cancel,
            };
            let error = super::hash_reader(reader, Path::new("partial"), &mut || cancelled.get())
                .expect_err("no partial hash");
            if cancel {
                assert!(matches!(error, super::ScanError::Interrupted));
                assert_eq!(reads.get(), 1, "no read after cancellation");
            } else {
                assert!(
                    matches!(error, super::ScanError::Read { path, .. } if path == Path::new("partial"))
                );
                assert_eq!(reads.get(), 2);
            }
        }
    }

    #[test]
    fn interrupted_io_is_retried_and_empty_files_have_the_complete_digest() {
        use std::io::{self, Read};
        struct OnceInterrupted(bool);
        impl Read for OnceInterrupted {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    self.0 = false;
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Ok(0)
                }
            }
        }
        let digest = super::hash_reader(OnceInterrupted(true), Path::new("empty"), &mut || false)
            .expect("retry then EOF");
        assert_eq!(
            digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn an_interrupted_scan_returns_no_partial_snapshot() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("A.bsl"), "first").expect("first");
        fs::write(dir.path().join("B.bsl"), "second").expect("second");
        // The walk ends, the first file is read, and the ninth check comes at its end.
        let mut checkpoints = 0;
        let error = scan(dir.path(), &mut || {
            checkpoints += 1;
            checkpoints >= 9
        })
        .expect_err("cancelled");
        assert!(matches!(error, super::ScanError::Interrupted));
        assert_eq!(checkpoints, 9);
    }

    fn many_files(root: &Path, count: usize) {
        for index in 0..count {
            let dir = root.join(format!("Dir{}", index % 7)).join("Ext");
            fs::create_dir_all(&dir).expect("dir");
            // Some files span several working buffers.
            let size = if index % 50 == 0 {
                200 * 1024
            } else {
                index * 13
            };
            let body: Vec<u8> = (0..size).map(|byte| (byte * 31 + index) as u8).collect();
            fs::write(dir.join(format!("Module{index}.bsl")), body).expect("file");
        }
    }

    #[test]
    fn a_parallel_read_gives_the_snapshot_of_a_sequential_one() {
        let dir = tempdir().expect("tempdir");
        many_files(dir.path(), 300);
        let read = |workers| {
            super::scan_with_workers(dir.path(), None, &mut || false, ScanScope::Push, workers)
                .expect("scan")
        };
        let sequential = read(1);
        assert_eq!(sequential.files.len(), 300);
        for workers in [2, 8] {
            let parallel = read(workers);
            let entries = |snapshot: &ScanSnapshot| {
                snapshot
                    .files
                    .iter()
                    .map(|file| {
                        (
                            file.path.clone(),
                            file.rel_path.clone(),
                            file.mtime_ns,
                            file.hash.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                entries(&parallel),
                entries(&sequential),
                "{workers} workers"
            );
            assert_eq!(parallel.has_untracked_links, sequential.has_untracked_links);
        }
    }

    fn fake_candidates(count: usize) -> Vec<super::Candidate> {
        (0..count)
            .map(|index| super::Candidate {
                path: format!("file-{index}").into(),
                rel_path: format!("file-{index}"),
                mtime_ns: 0,
            })
            .collect()
    }

    /// Reads until the shared stop flag is raised, or gives up so a broken flag fails the test.
    fn read_until_stopped(
        interrupted: &mut dyn FnMut() -> bool,
    ) -> Result<String, super::ScanError> {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            if interrupted() {
                return Err(super::ScanError::Interrupted);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok("never stopped".to_owned())
    }

    #[test]
    fn a_failed_read_stops_the_other_workers_without_a_snapshot() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let candidates = fake_candidates(200);
        let started = AtomicUsize::new(0);
        let hash = |path: &Path, interrupted: &mut dyn FnMut() -> bool| {
            started.fetch_add(1, Ordering::Relaxed);
            match path.to_str().expect("name") {
                // Files before the failing one are read to the end, as a sequential read would.
                "file-0" | "file-1" | "file-2" | "file-3" | "file-4" => Ok("read".to_owned()),
                "file-5" => Err(super::ScanError::Read {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("unreadable"),
                }),
                _ => read_until_stopped(interrupted),
            }
        };
        let clock = std::time::Instant::now();
        let error = super::hash_candidates(&candidates, 8, &mut || false, &hash)
            .expect_err("a failed read gives no snapshot");
        assert!(
            matches!(&error, super::ScanError::Read { path, .. } if path == Path::new("file-5")),
            "{error:?}"
        );
        assert!(
            clock.elapsed() < Duration::from_secs(10),
            "workers were not stopped"
        );
        assert!(
            started.load(Ordering::Relaxed) < 20,
            "files kept being read after the failure"
        );
    }

    #[test]
    fn the_earliest_failing_file_is_reported_whichever_fails_first() {
        let candidates = fake_candidates(200);
        let unreadable = |path: &Path| super::ScanError::Read {
            path: path.to_path_buf(),
            source: std::io::Error::other("unreadable"),
        };
        let hash = |path: &Path, interrupted: &mut dyn FnMut() -> bool| {
            let index: usize = path.to_str().expect("name")[5..].parse().expect("index");
            match index {
                0..10 => Ok("read".to_owned()),
                // The earlier file fails late, after the later one has already failed.
                10 => {
                    std::thread::sleep(Duration::from_millis(300));
                    if interrupted() {
                        return Err(super::ScanError::Interrupted);
                    }
                    Err(unreadable(path))
                }
                11 => Err(unreadable(path)),
                _ => read_until_stopped(interrupted),
            }
        };
        let error = super::hash_candidates(&candidates, 8, &mut || false, &hash)
            .expect_err("a failed read gives no snapshot");
        assert!(
            matches!(&error, super::ScanError::Read { path, .. } if path == Path::new("file-10")),
            "{error:?}"
        );
    }

    #[test]
    fn a_cancellation_stops_the_workers_without_a_snapshot() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let candidates = fake_candidates(200);
        let started = AtomicUsize::new(0);
        let hash = |_: &Path, interrupted: &mut dyn FnMut() -> bool| {
            started.fetch_add(1, Ordering::Relaxed);
            read_until_stopped(interrupted)
        };
        let mut polls = 0;
        let clock = std::time::Instant::now();
        let error = super::hash_candidates(
            &candidates,
            8,
            &mut || {
                polls += 1;
                polls >= 3
            },
            &hash,
        )
        .expect_err("a cancelled read gives no snapshot");
        assert!(matches!(error, super::ScanError::Interrupted), "{error:?}");
        assert_eq!(
            polls, 3,
            "the predicate is asked on the calling thread only until it says stop"
        );
        assert!(
            clock.elapsed() < Duration::from_secs(10),
            "workers were not stopped"
        );
        assert!(
            started.load(Ordering::Relaxed) <= 8,
            "no file starts after the cancellation"
        );
    }
}
