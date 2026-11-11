use sha1::{Digest, Sha1};
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
        source: std::io::Error,
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

/// Directory/file names that are always excluded from scanning. `.local` is the default
/// `localPath`: a second guard behind [`runner_dirs_inside`](super::runner_dirs::runner_dirs_inside).
const IGNORED_DIRS: &[&str] = &[
    ".git", ".gradle", ".build", "build", "target", "temp", "tmp", ".yaxunit", ".local",
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
fn is_build_artifact(path: &Path, name: &str) -> bool {
    if path
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
    scan_in_scope(root, &[], interrupted, ScanScope::Push)
}

/// Hash the files selected by the scan scope, excluding the supplied directories.
pub(super) fn scan_in_scope(
    root: &Path,
    excluded: &[PathBuf],
    interrupted: &mut dyn FnMut() -> bool,
    scope: ScanScope,
) -> Result<ScanSnapshot, ScanError> {
    scan_with_workers(root, excluded, interrupted, scope, hash_workers())
}

fn scan_with_workers(
    root: &Path,
    excluded: &[PathBuf],
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
    excluded: &[PathBuf],
    interrupted: &mut dyn FnMut() -> bool,
    scope: ScanScope,
) -> Result<Listing, ScanError> {
    check_interruption(interrupted)?;
    let listing_started = std::time::Instant::now();
    let scan_started_at =
        mtime_nanos(std::time::SystemTime::now(), root).map_err(|source| ScanError::Mtime {
            path: root.to_path_buf(),
            source,
        })?;
    let walk = TreeWalk {
        root,
        excluded,
        scope,
    };
    // The root is read on the calling thread, an entry at a time with the caller's predicate,
    // as a sequential walk reads it; only the directories below it go to the workers.
    check_interruption(interrupted)?;
    let mut top = DirListing::default();
    walk.read_dir(root, &mut top, &mut || interrupted())?;
    let mut candidates = top.files;
    let mut has_untracked_links = top.has_links;
    if !top.dirs.is_empty() {
        let below = walk.read_below(top.dirs, walk_workers(), interrupted)?;
        candidates.extend(below.files);
        has_untracked_links |= below.has_links;
    }
    // The workers finish in any order: the listing does not depend on it.
    candidates.sort_unstable_by(|left, right| left.rel_path.cmp(&right.rel_path));
    tracing::debug!(
        root = %root.display(),
        files = candidates.len(),
        elapsed_ms = listing_started.elapsed().as_millis() as u64,
        "the source tree was listed"
    );

    Ok(Listing {
        scan_started_at,
        files: candidates,
        has_untracked_links,
    })
}

/// Reading a large tree past this many threads gave nothing on the measured disk (#574: 113 000
/// files, 0.6 s on eight threads against 3.3 s on one).
const MAX_WALK_WORKERS: usize = 8;

fn walk_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(MAX_WALK_WORKERS)
}

/// One walk of a source tree: the selection of [`source_entries`] and [`is_source_file`] —
/// generated directories and `excluded` skipped, links never followed — over plain directory
/// reads, so that the directories can be read on several threads (#574).
struct TreeWalk<'w> {
    root: &'w Path,
    excluded: &'w [PathBuf],
    scope: ScanScope,
}

/// What one or more directory reads found.
#[derive(Default)]
struct DirListing {
    files: Vec<Candidate>,
    /// Directories still to read.
    dirs: Vec<PathBuf>,
    has_links: bool,
}

/// The shared queue of the walk workers.
struct WalkQueue {
    dirs: Vec<PathBuf>,
    /// Workers reading a directory now: the walk ends when none is and the queue is empty.
    reading: usize,
}

impl TreeWalk<'_> {
    /// Reads one directory: its selected files with their metadata, its directories to read
    /// next. The predicate is asked before every entry the walk keeps.
    fn read_dir(
        &self,
        dir: &Path,
        into: &mut DirListing,
        interrupted: &mut dyn FnMut() -> bool,
    ) -> Result<(), ScanError> {
        let walk_error = |source| ScanError::Walk {
            path: dir.to_path_buf(),
            source,
        };
        for entry in std::fs::read_dir(dir).map_err(walk_error)? {
            let entry = entry.map_err(walk_error)?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(walk_error)?;
            let name = entry.file_name();
            let name = name.to_str();
            if file_type.is_dir()
                && (name.is_some_and(|name| is_ignored_dir_name(name, self.scope))
                    || self.excluded.iter().any(|excluded| *excluded == path))
            {
                continue;
            }
            check_interruption(interrupted)?;
            if file_type.is_dir() {
                into.dirs.push(path);
                continue;
            }
            if matches!(self.scope, ScanScope::Build) && file_type.is_symlink() {
                // The platform may consume a link's contents. An untracked link prevents reuse
                // but still permits the same fresh full load that the platform handled before.
                into.has_links = true;
            }
            if !file_type.is_file()
                || name.is_some_and(|name| !selects_file_name(&path, name, self.root, self.scope))
            {
                continue;
            }
            // A selected entry is a regular file, never a link, so the listing's own metadata
            // is the file's. On Windows it comes with the directory read and opens no file; a
            // separate `fs::metadata` opened every file of the tree once more (#571).
            let meta = entry.metadata().map_err(|source| ScanError::Meta {
                path: path.clone(),
                source,
            })?;
            let mtime_ns = mtime_nanos(
                meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                &path,
            )
            .map_err(|source| ScanError::Mtime {
                path: path.clone(),
                source,
            })?;
            into.files.push(Candidate {
                rel_path: rel_path(self.root, &path)?,
                path,
                mtime_ns,
            });
        }
        Ok(())
    }

    /// Reads every directory below `dirs` on `workers` threads. The caller's predicate stays on
    /// the calling thread and is polled while the workers read, as [`hash_candidates`] polls
    /// it; a cancellation or the first failure stops the workers, and no partial listing is
    /// returned. Of the failures seen before the workers stopped, the one of the first path in
    /// name order is reported.
    fn read_below(
        &self,
        dirs: Vec<PathBuf>,
        workers: usize,
        interrupted: &mut dyn FnMut() -> bool,
    ) -> Result<DirListing, ScanError> {
        if workers <= 1 {
            let mut listing = DirListing {
                dirs,
                ..DirListing::default()
            };
            while let Some(dir) = listing.dirs.pop() {
                self.read_dir(&dir, &mut listing, interrupted)?;
            }
            return Ok(listing);
        }
        let queue = Mutex::new(WalkQueue { dirs, reading: 0 });
        let ready = std::sync::Condvar::new();
        let stop = AtomicBool::new(false);
        let found: Mutex<DirListing> = Mutex::new(DirListing::default());
        let failure: Mutex<Option<ScanError>> = Mutex::new(None);
        let mut cancelled = false;

        std::thread::scope(|scope| {
            let (done, finished) = mpsc::channel::<()>();
            for _ in 0..workers {
                let done = done.clone();
                let (queue, ready, stop, found, failure) =
                    (&queue, &ready, &stop, &found, &failure);
                scope.spawn(move || {
                    let _guard = StopOnPanic(stop);
                    let mut own = DirListing::default();
                    loop {
                        let next = {
                            let mut state = queue.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                if stop.load(Ordering::Relaxed) {
                                    break None;
                                }
                                if let Some(dir) = state.dirs.pop() {
                                    state.reading += 1;
                                    break Some(dir);
                                }
                                if state.reading == 0 {
                                    break None;
                                }
                                state = ready
                                    .wait_timeout(state, INTERRUPTION_POLL)
                                    .unwrap_or_else(|e| e.into_inner())
                                    .0;
                            }
                        };
                        let Some(dir) = next else {
                            break;
                        };
                        let read =
                            self.read_dir(&dir, &mut own, &mut || stop.load(Ordering::Relaxed));
                        let below = std::mem::take(&mut own.dirs);
                        {
                            let mut state = queue.lock().unwrap_or_else(|e| e.into_inner());
                            state.dirs.extend(below);
                            state.reading -= 1;
                        }
                        ready.notify_all();
                        match read {
                            Ok(()) => {}
                            // Raised by the flag: a cancellation or another worker's failure.
                            Err(ScanError::Interrupted) => break,
                            Err(error) => {
                                stop.store(true, Ordering::Relaxed);
                                let mut slot = failure.lock().unwrap_or_else(|e| e.into_inner());
                                if slot.as_ref().is_none_or(|earlier| {
                                    failure_path(&error) < failure_path(earlier)
                                }) {
                                    *slot = Some(error);
                                }
                                ready.notify_all();
                                break;
                            }
                        }
                    }
                    let mut all = found.lock().unwrap_or_else(|e| e.into_inner());
                    all.files.append(&mut own.files);
                    all.has_links |= own.has_links;
                    drop(done);
                });
            }
            drop(done);
            // Every sender is dropped when its worker ends; the loop ends with the last of them.
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                finished.recv_timeout(INTERRUPTION_POLL)
            {
                if !cancelled && interrupted() {
                    cancelled = true;
                    stop.store(true, Ordering::Relaxed);
                    ready.notify_all();
                }
            }
        });

        if cancelled {
            return Err(ScanError::Interrupted);
        }
        if let Some(error) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
            return Err(error);
        }
        // Workers faster than one poll still answer a cancellation raised meanwhile.
        check_interruption(interrupted)?;
        Ok(found.into_inner().unwrap_or_else(|e| e.into_inner()))
    }
}

/// The path a walk failure names: of several, the first in name order is reported.
fn failure_path(error: &ScanError) -> Option<&Path> {
    match error {
        ScanError::Walk { path, .. }
        | ScanError::Read { path, .. }
        | ScanError::Meta { path, .. }
        | ScanError::Mtime { path, .. }
        | ScanError::RelativePath { path, .. } => Some(path),
        ScanError::Interrupted => None,
    }
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

/// Hash listed files by content and keep only those whose modification time stayed put while
/// they were read: an edit saved during the read may already be in the digest, and such a file
/// is left out, so it stays a change (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`). The
/// second look at the time happens in the reading worker, right after the read.
pub(super) fn hash_listed_unmoved(
    files: Vec<Candidate>,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<HashedFile>, ScanError> {
    let read = |path: &Path, stop: &mut dyn FnMut() -> bool| {
        let hash = hash_file(path, stop)?;
        let after = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| mtime_nanos(modified, path).ok());
        Ok((hash, after))
    };
    let hashes = hash_candidates(&files, hash_workers(), interrupted, &read)?;
    check_interruption(interrupted)?;
    Ok(files
        .into_iter()
        .zip(hashes)
        .filter(|(candidate, (_, after))| *after == Some(candidate.mtime_ns))
        .map(|(candidate, (hash, _))| HashedFile {
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
fn hash_candidates<H, T>(
    candidates: &[Candidate],
    workers: usize,
    interrupted: &mut dyn FnMut() -> bool,
    hash: &H,
) -> Result<Vec<T>, ScanError>
where
    H: Fn(&Path, &mut dyn FnMut() -> bool) -> Result<T, ScanError> + Sync,
    T: Send + Sync,
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
    let digests: Vec<OnceLock<T>> = candidates.iter().map(|_| OnceLock::new()).collect();
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

/// How many times a file whose length moved while it was read is read again.
const LENGTH_RETRIES: usize = 2;

/// The git blob id of a file's contents, as `git hash-object --no-filters` computes it: the
/// SHA-1 of `blob <length>\0` and the bytes (#575). Without git the memory holds the ids git
/// would give the file without filters, so one memory serves a set with and without git.
pub fn hash_file(path: &Path, interrupted: &mut dyn FnMut() -> bool) -> Result<String, ScanError> {
    let read_error = |source| ScanError::Read {
        path: path.to_path_buf(),
        source,
    };
    for _ in 0..=LENGTH_RETRIES {
        check_interruption(interrupted)?;
        let file = std::fs::File::open(path).map_err(read_error)?;
        let length = file.metadata().map_err(read_error)?.len();
        if let Some(id) = hash_reader(file, length, path, interrupted)? {
            return Ok(id);
        }
    }
    Err(read_error(std::io::Error::other(
        "the file length kept changing while it was read",
    )))
}

/// The blob id of `length` bytes read from `reader`; `None` when the reader gave another number
/// of bytes — the file changed while it was read.
fn hash_reader(
    mut reader: impl Read,
    length: u64,
    path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Option<String>, ScanError> {
    // Working storage, not a maximum file size. Files of any size are streamed.
    let mut buffer = [0u8; 64 * 1024];
    let mut digest = Sha1::new();
    digest.update(format!("blob {length}\0").as_bytes());
    let mut read: u64 = 0;
    loop {
        check_interruption(interrupted)?;
        let got = match reader.read(&mut buffer) {
            Ok(got) => got,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(ScanError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        check_interruption(interrupted)?;
        if got == 0 {
            break;
        }
        read += got as u64;
        digest.update(&buffer[..got]);
    }
    Ok((read == length).then(|| format!("{:x}", digest.finalize())))
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
    excluded: &'a [PathBuf],
    scope: ScanScope,
) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> + 'a {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !(is_ignored_dir(entry, scope)
                    || excluded.iter().any(|path| path == entry.path()))
        })
}

pub(super) fn is_source_file(entry: &walkdir::DirEntry, root: &Path, scope: ScanScope) -> bool {
    entry.file_type().is_file()
        && entry
            .file_name()
            .to_str()
            .is_none_or(|name| selects_file_name(entry.path(), name, root, scope))
}

/// The file-name half of [`is_source_file`]: whether a regular file `name` at `path` is a source.
fn selects_file_name(path: &Path, name: &str, root: &Path, scope: ScanScope) -> bool {
    match scope {
        ScanScope::Push => !is_ignored_file(name),
        ScanScope::Build => {
            !(path.parent() == Some(root) && is_ignored_file(name) || is_build_artifact(path, name))
        }
    }
}

fn is_ignored_dir(entry: &walkdir::DirEntry, scope: ScanScope) -> bool {
    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| is_ignored_dir_name(name, scope))
}

/// Whether a file below a set root, named by its path from the root with `/`, is one the push
/// walk selects: the same name predicates as [`source_entries`] and [`is_source_file`], so a
/// listing that comes from git selects exactly what the walk selects (#575). Every component
/// but the last names a directory below the root. Links and `workPath` are the caller's.
pub(super) fn is_push_source_rel_path(rel: &str) -> bool {
    let mut parts = rel.split('/');
    let Some(name) = parts.next_back() else {
        return false;
    };
    !name.is_empty()
        && !is_ignored_file(name)
        && !parts.any(|dir| is_ignored_dir_name(dir, ScanScope::Push))
}

fn is_ignored_dir_name(name: &str, scope: ScanScope) -> bool {
    match scope {
        ScanScope::Push => IGNORED_DIRS.contains(&name),
        // A metadata object can itself be named build, target, temp, or tmp.
        ScanScope::Build => matches!(name, ".git" | ".gradle" | ".build" | ".yaxunit" | ".local"),
    }
}

#[cfg(test)]
mod tests {
    use super::{scan, scan_in_scope, ScanScope, ScanSnapshot};
    use crate::change_detection::file_state::mtime_nanos;
    use crate::change_detection::runner_dirs::{runner_dirs_inside, RunnerDirs};
    use std::fs::{self, File};
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;

    /// Файл, время которого не то, что было в перечне, — его правили, пока его читали, — из
    /// ответа уходит; нетронутый остаётся с хешем (`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`).
    #[test]
    fn a_file_whose_time_moved_while_it_was_read_is_left_out() {
        let dir = tempdir().expect("dir");
        let candidate = |name: &str, shift: u64| {
            let path = dir.path().join(name);
            fs::write(&path, name).expect("file");
            let mtime_ns = mtime_nanos(
                fs::metadata(&path)
                    .expect("meta")
                    .modified()
                    .expect("mtime"),
                &path,
            )
            .expect("ns");
            super::Candidate {
                path,
                rel_path: name.to_owned(),
                mtime_ns: mtime_ns - shift,
            }
        };
        let files = vec![candidate("Kept.bsl", 0), candidate("Moved.bsl", 1)];
        let hashed = super::hash_listed_unmoved(files, &mut || false).expect("hashed");
        let names: Vec<_> = hashed.iter().map(|file| file.rel_path.as_str()).collect();
        assert_eq!(names, ["Kept.bsl"]);
    }

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

    /// `workPath` и `localPath` внутри набора не исходники, как бы они ни назывались.
    #[test]
    fn a_work_path_inside_the_root_is_not_a_source_whatever_its_name() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let work = root.join("service-area");
        let local = root.join("personal");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(local.join("db")).expect("local");
        fs::write(work.join("log.txt"), "log").expect("service file");
        fs::write(local.join("db").join("1Cv8.1CD"), "base").expect("personal file");
        fs::write(root.join("Configuration.xml"), "<x/>").expect("source");

        let excluded = runner_dirs_inside(&root, RunnerDirs::new(&work, Some(&local)));
        assert_eq!(excluded, [work.clone(), local.clone()]);
        for scope in [ScanScope::Push, ScanScope::Build] {
            let snapshot = scan_in_scope(&root, &excluded, &mut || false, scope).expect("scan");
            assert_eq!(scanned_names(&snapshot), ["Configuration.xml"]);
        }
        assert!(runner_dirs_inside(&root, RunnerDirs::new(dir.path(), None)).is_empty());
        assert!(runner_dirs_inside(&root, RunnerDirs::new(&root, None)).is_empty());
    }

    /// `.local` под корнем набора пропускается по имени и без настроек: вторая защита
    /// `localPath` по умолчанию.
    #[test]
    fn the_default_local_directory_is_skipped_by_its_name() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join(".local").join("db")).expect("local");
        fs::write(root.join(".local").join("db").join("1Cv8.1CD"), "base").expect("base");
        fs::write(root.join("Configuration.xml"), "<x/>").expect("source");
        for scope in [ScanScope::Push, ScanScope::Build] {
            let snapshot = scan_in_scope(root, &[], &mut || false, scope).expect("scan");
            assert_eq!(scanned_names(&snapshot), ["Configuration.xml"]);
        }
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
        let hash = super::hash_reader(&mut reader, length as u64, Path::new("large"), &mut || {
            false
        })
        .expect("hash")
        .expect("all bytes");
        use sha1::{Digest, Sha1};
        let mut expected = format!("blob {length}\0").into_bytes();
        expected.extend(vec![17; length]);
        assert_eq!(hash, format!("{:x}", Sha1::digest(expected)));
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
            let error =
                super::hash_reader(reader, 3, Path::new("partial"), &mut || cancelled.get())
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
        let digest =
            super::hash_reader(OnceInterrupted(true), 0, Path::new("empty"), &mut || false)
                .expect("retry then EOF");
        // The id git gives an empty file.
        assert_eq!(
            digest.as_deref(),
            Some("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")
        );
    }

    /// Without git a file is hashed as `git hash-object --no-filters` hashes it, so one memory
    /// serves a set with and without git (#575); a file whose length moved while it was read
    /// gives no id.
    #[test]
    fn a_file_is_hashed_as_git_hashes_it_without_filters() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("Module.bsl");
        fs::write(&file, "line\r\n").expect("file");
        let id = super::hash_file(&file, &mut || false).expect("id");
        let git = std::process::Command::new("git")
            .args(["hash-object", "--no-filters"])
            .arg(&file)
            .output()
            .expect("git");
        assert_eq!(id, String::from_utf8_lossy(&git.stdout).trim());
        assert_eq!(
            super::hash_reader(&b"abc"[..], 4, Path::new("short"), &mut || false).expect("read"),
            None
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

    /// A tree with nested folders, generated folders at the top and deeper, an excluded folder,
    /// ignored files and build artifacts: what each walk must leave out.
    fn walked_tree(root: &Path) -> std::path::PathBuf {
        many_files(root, 120);
        for (folder, file) in [
            ("build", "Hidden.bsl"),
            ("Catalogs/Goods/build", "Form.xml"),
            ("Catalogs/Goods/.git", "HEAD"),
            ("Catalogs/Goods/Forms/target", "Form.xml"),
            ("work", "Inside.bsl"),
            ("Catalogs/Goods", "ConfigDumpInfo.xml"),
            ("Catalogs/Goods", "main.cf"),
            ("", "ConfigDumpInfo.xml"),
            ("", "Configuration.xml"),
        ] {
            let dir = root.join(folder);
            fs::create_dir_all(&dir).expect("folder");
            fs::write(dir.join(file), folder).expect("file");
        }
        root.join("work")
    }

    /// What the sequential walk of [`super::source_entries`] selects, as the listing names it.
    fn sequential_selection(
        root: &Path,
        excluded: &[std::path::PathBuf],
        scope: ScanScope,
    ) -> Vec<(String, u64)> {
        let mut selected: Vec<(String, u64)> = super::source_entries(root, excluded, scope)
            .map(|entry| entry.expect("entry"))
            .filter(|entry| super::is_source_file(entry, root, scope))
            .map(|entry| {
                let modified = entry.metadata().expect("meta").modified().expect("mtime");
                (
                    super::rel_path(root, entry.path()).expect("relative"),
                    mtime_nanos(modified, entry.path()).expect("nanos"),
                )
            })
            .collect();
        selected.sort();
        selected
    }

    /// The listing reads folders on several threads and selects exactly what the sequential
    /// walk selects, in name order whatever the number of threads (#574).
    #[test]
    fn a_parallel_listing_selects_what_the_sequential_walk_selects() {
        let dir = tempdir().expect("tempdir");
        let excluded = vec![walked_tree(dir.path())];
        for scope in [ScanScope::Push, ScanScope::Build] {
            let expected = sequential_selection(dir.path(), &excluded, scope);
            assert!(expected.len() > 120, "{}", expected.len());
            for workers in [1, 2, 8] {
                let walk = super::TreeWalk {
                    root: dir.path(),
                    excluded: &excluded,
                    scope,
                };
                let mut top = super::DirListing::default();
                walk.read_dir(dir.path(), &mut top, &mut || false)
                    .expect("root");
                let below = walk
                    .read_below(top.dirs, workers, &mut || false)
                    .expect("below");
                let mut listed: Vec<(String, u64)> = top
                    .files
                    .into_iter()
                    .chain(below.files)
                    .map(|file| (file.rel_path, file.mtime_ns))
                    .collect();
                listed.sort();
                assert_eq!(listed, expected, "{workers} workers");
            }
            let listing =
                super::list_in_scope(dir.path(), &excluded, &mut || false, scope).expect("listing");
            let names: Vec<&str> = listing
                .files
                .iter()
                .map(|file| file.rel_path.as_str())
                .collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted, "the listing comes in name order");
            assert_eq!(names.len(), expected.len());
        }
    }

    /// Замер перечня на большом дереве: `V8R_LIST_ROOT=<каталог> cargo test --release -- --ignored
    /// the_listing_of_a_large_tree_is_timed --nocapture`.
    #[test]
    #[ignore = "a measurement on a tree named by V8R_LIST_ROOT"]
    fn the_listing_of_a_large_tree_is_timed() {
        let root = std::path::PathBuf::from(std::env::var("V8R_LIST_ROOT").expect("V8R_LIST_ROOT"));
        for workers in [1, 4, 8, 8] {
            let clock = std::time::Instant::now();
            let walk = super::TreeWalk {
                root: &root,
                excluded: &[],
                scope: ScanScope::Push,
            };
            let mut top = super::DirListing::default();
            walk.read_dir(&root, &mut top, &mut || false).expect("root");
            let below = walk
                .read_below(top.dirs, workers, &mut || false)
                .expect("below");
            println!(
                "{workers} workers: {} files in {:?}",
                top.files.len() + below.files.len(),
                clock.elapsed()
            );
        }
    }

    /// A cancellation while the folders below the root are read gives no listing at all.
    #[test]
    fn a_cancelled_parallel_listing_returns_nothing() {
        let dir = tempdir().expect("tempdir");
        many_files(dir.path(), 300);
        let mut polls = 0;
        let error = super::list_in_scope(
            dir.path(),
            &[],
            &mut || {
                polls += 1;
                // The root and its seven folders pass; the stop comes while the workers read.
                polls > 9
            },
            ScanScope::Push,
        )
        .err()
        .expect("a cancelled listing");
        assert!(matches!(error, super::ScanError::Interrupted), "{error:?}");
    }

    /// A folder that cannot be read fails the listing and is named, on any number of threads.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_fails_the_parallel_listing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().expect("tempdir");
        many_files(dir.path(), 70);
        let locked = dir.path().join("Dir3").join("Ext");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock");
        if fs::read_dir(&locked).is_ok() {
            // Root ignores permissions: nothing to observe.
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
            return;
        }
        let error = super::list_in_scope(dir.path(), &[], &mut || false, ScanScope::Push)
            .err()
            .expect("an unreadable folder");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
        assert!(
            matches!(&error, super::ScanError::Walk { path, .. } if *path == locked),
            "{error:?}"
        );
    }

    #[test]
    fn a_parallel_read_gives_the_snapshot_of_a_sequential_one() {
        let dir = tempdir().expect("tempdir");
        many_files(dir.path(), 300);
        let read = |workers| {
            super::scan_with_workers(dir.path(), &[], &mut || false, ScanScope::Push, workers)
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
