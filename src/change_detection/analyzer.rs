use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::change_detection::file_state::mtime_nanos;
use crate::change_detection::git_ignore::{dump_samples, HiddenBy, IgnoreRules};
use crate::change_detection::git_tree::{
    BlobHashError, BlobHasher, GitSource, ObserveError, ObservedFile, Spelling,
};
use crate::change_detection::hash_storage::{
    HashKind, HashStorage, StorageDelta, StorageError, StorageSnapshot, StorageSummary,
    StoredFileState,
};
use crate::change_detection::scanner::{self, ScanError, ScanScope};
use crate::domain::git_worktree::{ChangeDetection, GitWorktree, ScanReason};
use crate::domain::source_set::SourceSetContext;

/// A single detected file change.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// How a file changed relative to the stored state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// Storage update produced by one analysis pass: the rows that differ from the memory of
/// `observed_generation`, so a commit after a change of one file writes one file (#576).
#[derive(Debug, Clone)]
pub struct PreparedStateUpdate {
    pub delta: StorageDelta,
    /// Files the scan found.
    pub scanned_files: usize,
    pub scan_started_at: u64,
    pub observed_generation: u64,
}

/// Where the changes of a set come from (#575).
pub enum ChangeSource<'v, 'h> {
    /// Every file is read; the reason is named when the caller knows why git is not asked.
    Scan(Option<ScanReason>),
    /// Git names the files and their blob ids.
    Git(GitSource<'v, 'h>),
}

/// Result of analyzing one source-set against its persisted snapshot.
#[derive(Debug, Clone)]
pub enum AnalysisOutcome {
    NoChanges,
    Changes {
        changes: Vec<FileChange>,
        prepared: PreparedStateUpdate,
    },
    Fallback,
}

/// Analysis result paired with the source-set context it belongs to.
#[derive(Debug, Clone)]
pub struct ContextAnalysis {
    pub context: SourceSetContext,
    pub outcome: Result<AnalysisOutcome, ChangeDetectionError>,
    /// How the changes were found; `None` when the analysis stopped before it looked.
    pub detection: Option<ChangeDetection>,
}

/// Hard failures that prevent normal change-detection flow.
#[derive(Debug, Clone, Error)]
pub enum ChangeDetectionError {
    /// A pre-load scan stopped cooperatively. This must never become full-load fallback.
    #[error("source analysis interrupted")]
    Interrupted,

    #[error("hard storage error for source-set '{source_set}' at '{storage_path}': {reason}")]
    StorageHard {
        source_set: String,
        storage_path: PathBuf,
        reason: String,
    },

    /// The stored snapshot describes another base or source directory. Its hashes say
    /// nothing about the selected pair, so they are neither used nor silently replaced.
    ///
    /// The text names no way out: a command that replaces the memory must carry the global
    /// keys of the run, and only the use case knows them. The use case appends the ways out
    /// for `source_set` (`build_project::helpers::change_detection_failure`).
    #[error("hash memory for source-set '{source_set}' at '{storage_path}' belongs to {recorded}; the selected target is {selected}")]
    ForeignMemory {
        source_set: String,
        storage_path: PathBuf,
        recorded: String,
        selected: String,
    },

    #[error("concurrent state modification for source-set '{source_set}' at '{storage_path}': expected generation {expected}, found {actual}")]
    ConcurrentStateModified {
        source_set: String,
        storage_path: PathBuf,
        expected: u64,
        actual: u64,
    },

    /// A git ignore rule can hide a file the platform writes into the set: git would not report
    /// it, and its change would never be loaded
    /// (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`).
    #[error("{}", hidden_message(source_set, hidden))]
    HiddenByGit {
        source_set: String,
        hidden: Box<HiddenBy>,
    },
}

fn hidden_message(source_set: &str, hidden: &HiddenBy) -> String {
    let paths = if hidden.paths == ["."] {
        "the set root itself".to_owned()
    } else {
        hidden
            .paths
            .iter()
            .map(|path| format!("'{path}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "the git ignore rule '{}' at {}:{} hides files of source-set '{source_set}' ({paths}): git does not report a change of an ignored file, so it would never be loaded. Anchor the rule to the place it is meant for, for example '/build/' instead of 'build/', or remove it",
        hidden.rule,
        hidden.file.display(),
        hidden.line,
    )
}

/// Analyze one source-set context by reading every file and produce either concrete changes
/// or a safe fallback.
pub fn analyze_context(
    context: &SourceSetContext,
    work_path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> ContextAnalysis {
    analyze_context_from(context, work_path, ChangeSource::Scan(None), interrupted)
}

/// What one analysis found, before the final interruption check.
struct Found {
    outcome: Result<AnalysisOutcome, ChangeDetectionError>,
    detection: Option<ChangeDetection>,
}

impl Found {
    fn early(outcome: Result<AnalysisOutcome, ChangeDetectionError>) -> Self {
        Self {
            outcome,
            detection: None,
        }
    }
}

/// Analyze one source-set context with its changes taken from `source` (#575): git names the
/// files and their blob ids, or every file is read and hashed the way git hashes a file without
/// filters. Memory written before #575 compares with nothing and gives a full load.
pub fn analyze_context_from(
    context: &SourceSetContext,
    work_path: &Path,
    source: ChangeSource<'_, '_>,
    interrupted: &mut dyn FnMut() -> bool,
) -> ContextAnalysis {
    // All outcomes, including storage and IO fallback, cross the same final
    // interruption check. No early return can turn cancellation into a full load.
    let found = if interrupted() {
        Found::early(Err(ChangeDetectionError::Interrupted))
    } else {
        let snapshot = match context.storage_path(work_path) {
            None => Ok(Some(StorageSnapshot::default())),
            Some(path) => load_bound_snapshot(context, &HashStorage::new(path)),
        };
        match snapshot {
            Ok(Some(snapshot)) => {
                analyze_snapshot(context, work_path, &snapshot, source, interrupted)
            }
            Ok(None) => Found::early(Ok(AnalysisOutcome::Fallback)),
            Err(error) => Found::early(Err(error)),
        }
    };
    let interrupted_now = interrupted();
    ContextAnalysis {
        context: context.clone(),
        outcome: if interrupted_now {
            Err(ChangeDetectionError::Interrupted)
        } else {
            found.outcome
        },
        detection: found.detection,
    }
}

fn analyze_snapshot(
    context: &SourceSetContext,
    work_path: &Path,
    snapshot: &StorageSnapshot,
    source: ChangeSource<'_, '_>,
    interrupted: &mut dyn FnMut() -> bool,
) -> Found {
    // A memory of another hasher compares with nothing: the set loads whole (#575).
    let other_kind = |kind: HashKind| !snapshot.entries.is_empty() && snapshot.hash_kind != kind;
    let reason = match source {
        ChangeSource::Scan(reason) => reason,
        ChangeSource::Git(mut git) => {
            if let Some(hidden) = hidden_by_git(git.view, &git.root, snapshot) {
                return Found::early(Err(ChangeDetectionError::HiddenByGit {
                    source_set: context.name().to_owned(),
                    hidden: Box::new(hidden),
                }));
            }
            let detection = ChangeDetection::git(git.view.fsmonitor);
            let spelling = Spelling::new(&snapshot.entries, git.view.ignore_case);
            match observe_by_git(context, snapshot, &spelling, &mut git, interrupted) {
                Ok(_) if other_kind(HashKind::GitBlob) => {
                    return Found {
                        outcome: Ok(AnalysisOutcome::Fallback),
                        detection: Some(detection),
                    };
                }
                Ok(observed) => {
                    let changes = git_changes(context, &observed, snapshot, &spelling);
                    return Found {
                        outcome: Ok(if changes.is_empty() {
                            AnalysisOutcome::NoChanges
                        } else {
                            AnalysisOutcome::Changes {
                                changes,
                                prepared: git_prepared_state(&observed, snapshot, Some(&spelling)),
                            }
                        }),
                        detection: Some(detection),
                    };
                }
                Err(ObserveError::Interrupted) => {
                    return Found::early(Err(ChangeDetectionError::Interrupted))
                }
                Err(ObserveError::Unusable { reason, detail }) => {
                    tracing::warn!(
                        source_set = %context.name(),
                        reason = ?reason,
                        detail,
                        "git cannot describe the set; its files are read"
                    );
                    Some(reason)
                }
            }
        }
    };
    let detection = reason.map(ChangeDetection::scan);
    if other_kind(HashKind::RawBlob) {
        return Found {
            outcome: Ok(AnalysisOutcome::Fallback),
            detection,
        };
    }
    let mut found = scan_snapshot(context, work_path, snapshot, interrupted);
    found.detection = detection;
    found
}

/// The rule of an ignore file that can hide a file of the set: one of the remembered files —
/// those a dump wrote — or, with nothing remembered, one of the files a dump writes.
fn hidden_by_git(view: &GitWorktree, root: &str, snapshot: &StorageSnapshot) -> Option<HiddenBy> {
    let rules = IgnoreRules::read(&view.ignore_sources, view.ignore_case);
    if snapshot.entries.is_empty() {
        let samples = dump_samples();
        rules.hiding(root, samples.iter().map(String::as_str))
    } else {
        let mut remembered: Vec<&str> = snapshot.entries.keys().map(String::as_str).collect();
        remembered.sort_unstable();
        rules.hiding(root, remembered)
    }
}

/// Whether a git ignore rule can hide a file of the set at `root` of the working tree `view`:
/// one of the remembered files, or with no memory of the base — memory of another pair, of an
/// old runner, unreadable — one of the files a dump writes
/// (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`). For commands that compare
/// nothing: a full pull, `push --full`, `make`.
pub fn hidden_by_git_rules(
    context: &SourceSetContext,
    work_path: &Path,
    view: &GitWorktree,
    root: &str,
) -> Result<(), ChangeDetectionError> {
    let snapshot = context
        .storage_path(work_path)
        .and_then(|path| {
            load_bound_snapshot(context, &HashStorage::new(path))
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    match hidden_by_git(view, root, &snapshot) {
        Some(hidden) => Err(ChangeDetectionError::HiddenByGit {
            source_set: context.name().to_owned(),
            hidden: Box::new(hidden),
        }),
        None => Ok(()),
    }
}

/// The set's files as git lists them, checked against the memory: a remembered file git does
/// not list but the disk still holds means git's answer misses it, and the set is read.
fn observe_by_git(
    context: &SourceSetContext,
    snapshot: &StorageSnapshot,
    spelling: &Spelling<'_>,
    git: &mut GitSource<'_, '_>,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<Vec<ObservedFile>, ObserveError> {
    let observed = git.observe(context.path(), interrupted)?;
    let seen: HashSet<&str> = observed
        .iter()
        .map(|file| {
            spelling
                .of(&file.rel_path)
                .unwrap_or(file.rel_path.as_str())
        })
        .collect();
    for relative in snapshot.entries.keys() {
        if seen.contains(relative.as_str()) {
            continue;
        }
        let path = context.path().join(relative);
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file()) {
            return Err(ObserveError::Unusable {
                reason: ScanReason::GitFailed,
                detail: format!(
                    "'{}' is on disk, but git lists no such file",
                    path.display()
                ),
            });
        }
    }
    Ok(observed)
}

/// Changes between git's listing and the memory of blob ids.
fn git_changes(
    context: &SourceSetContext,
    observed: &[ObservedFile],
    snapshot: &StorageSnapshot,
    spelling: &Spelling<'_>,
) -> Vec<FileChange> {
    let mut seen = HashSet::new();
    let mut changes: Vec<FileChange> = observed
        .iter()
        .filter_map(|file| {
            let key = spelling.of(&file.rel_path);
            if let Some(key) = key {
                seen.insert(key);
            }
            let kind = match key.and_then(|key| snapshot.entries.get(key)) {
                None => ChangeKind::Added,
                Some(stored) if file.hash.as_deref() != Some(stored.hash.as_str()) => {
                    ChangeKind::Modified
                }
                Some(_) => return None,
            };
            Some(FileChange {
                path: file.path.clone(),
                kind,
            })
        })
        .collect();
    changes.extend(
        snapshot
            .entries
            .keys()
            .filter(|relative| !seen.contains(relative.as_str()))
            .map(|relative| FileChange {
                path: context.path().join(relative),
                kind: ChangeKind::Deleted,
            }),
    );
    changes
}

/// Git's listing as rows of blob ids that differ from `stored`. A file whose id is unknown is
/// neither remembered anew nor forgotten. With `spelling`, a file is remembered under the key
/// the memory already spells it with; without it, the rows are a whole new memory.
fn git_prepared_state(
    observed: &[ObservedFile],
    stored: &StorageSnapshot,
    spelling: Option<&Spelling<'_>>,
) -> PreparedStateUpdate {
    let mut seen = HashSet::new();
    let mut upserts = HashMap::new();
    for file in observed {
        let key = spelling
            .and_then(|spelling| spelling.of(&file.rel_path))
            .unwrap_or(file.rel_path.as_str());
        seen.insert(key.to_owned());
        let Some(hash) = &file.hash else {
            continue;
        };
        let differs = spelling.is_none()
            || stored
                .entries
                .get(key)
                .is_none_or(|entry| entry.mtime_ns != 0 || &entry.hash != hash);
        if differs {
            upserts.insert(
                key.to_owned(),
                StoredFileState {
                    // Git answers for the time; the memory of blob ids keeps none.
                    mtime_ns: 0,
                    hash: hash.clone(),
                },
            );
        }
    }
    let removals = stored
        .entries
        .keys()
        .filter(|key| !seen.contains(key.as_str()))
        .cloned()
        .collect();
    PreparedStateUpdate {
        delta: StorageDelta {
            upserts,
            removals,
            hash_kind: HashKind::GitBlob,
        },
        scanned_files: observed.len(),
        scan_started_at: now_ns(),
        observed_generation: stored.generation,
    }
}

fn now_ns() -> u64 {
    mtime_nanos(std::time::SystemTime::now(), Path::new(".")).unwrap_or(0)
}

/// The analysis by reading every file: SHA-256 against the memory.
fn scan_snapshot(
    context: &SourceSetContext,
    work_path: &Path,
    snapshot: &StorageSnapshot,
    interrupted: &mut dyn FnMut() -> bool,
) -> Found {
    let excluded = scanner::work_path_inside(context.path(), work_path);
    let scan = match scan_sources(context, context.path(), excluded.as_deref(), interrupted) {
        Ok(scan) => scan,
        Err(ScanError::Interrupted) => return Found::early(Err(ChangeDetectionError::Interrupted)),
        Err(error) => {
            tracing::warn!(source_set = %context.name(), error = %error, "scan failed, switching to fallback mode");
            return Found::early(Ok(AnalysisOutcome::Fallback));
        }
    };
    if scan.has_untracked_links {
        return Found::early(Ok(AnalysisOutcome::Fallback));
    }
    let mut changes = detect_changes(&scan.files, &snapshot.entries);
    let seen_rel: HashSet<&str> = scan
        .files
        .iter()
        .map(|file| file.rel_path.as_str())
        .collect();
    changes.extend(
        snapshot
            .entries
            .iter()
            .filter(|(relative, _)| !seen_rel.contains(relative.as_str()))
            .map(|(relative, _)| FileChange {
                path: context.path().join(relative),
                kind: ChangeKind::Deleted,
            }),
    );
    Found::early(Ok(if changes.is_empty() {
        AnalysisOutcome::NoChanges
    } else {
        AnalysisOutcome::Changes {
            changes,
            prepared: build_prepared_state(&scan, snapshot),
        }
    }))
}

/// Load the snapshot and check that it describes this context's base and sources.
/// `Ok(None)` is a recoverable storage problem: the caller falls back to a full load.
fn load_bound_snapshot(
    context: &SourceSetContext,
    storage: &HashStorage,
) -> Result<Option<StorageSnapshot>, ChangeDetectionError> {
    let Some(snapshot) = readable(context, storage, storage.load_snapshot())? else {
        return Ok(None);
    };
    check_bound(
        context,
        storage,
        snapshot.is_blank(),
        snapshot.identity.as_deref(),
    )?;
    // Memory written before #575 compares with nothing (`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`).
    Ok(Some(snapshot).filter(|snapshot| !snapshot.is_legacy()))
}

/// [`load_bound_snapshot`] without the file rows, for a commit that names only the rows it
/// changes (#576).
fn load_bound_summary(
    context: &SourceSetContext,
    storage: &HashStorage,
) -> Result<Option<StorageSummary>, ChangeDetectionError> {
    let Some(summary) = readable(context, storage, storage.load_summary())? else {
        return Ok(None);
    };
    check_bound(
        context,
        storage,
        summary.is_blank(),
        summary.identity.as_deref(),
    )?;
    Ok(Some(summary).filter(|summary| !summary.is_legacy()))
}

/// `Ok(None)` for a recoverable storage problem, a refusal for a hard one.
fn readable<T>(
    context: &SourceSetContext,
    storage: &HashStorage,
    read: Result<T, StorageError>,
) -> Result<Option<T>, ChangeDetectionError> {
    match read {
        Ok(read) => Ok(Some(read)),
        Err(e) if e.is_recoverable() => {
            tracing::warn!(
                source_set = %context.name(),
                error = %e,
                "recoverable storage problem, switching to fallback mode"
            );
            Ok(None)
        }
        Err(e) => Err(map_storage_hard(context, storage.path(), e)),
    }
}

/// Memory of another base or source directory is refused, blank memory is nobody's.
fn check_bound(
    context: &SourceSetContext,
    storage: &HashStorage,
    blank: bool,
    recorded: Option<&str>,
) -> Result<(), ChangeDetectionError> {
    match context.storage_identity() {
        Some(selected) if !blank && recorded != Some(selected) => {
            Err(ChangeDetectionError::ForeignMemory {
                source_set: context.name().to_owned(),
                storage_path: storage.path().to_path_buf(),
                recorded: recorded.unwrap_or("an unidentified infobase").to_owned(),
                selected: selected.to_owned(),
            })
        }
        _ => Ok(()),
    }
}

/// Что хеш-память набора помнит о выбранной базе.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMemory {
    /// Памяти нет: ничего не записано, она пуста или повреждена так, что не читается.
    Nothing,
    /// Непустая память этой пары «база ↔ каталог».
    Own,
    /// Память записана для другой базы или другого каталога.
    Foreign,
    /// Хранилище не открывается — ответ даст анализ изменений своим отказом, — или память
    /// записана раннером до #575, и её хеши ни с чем не сравнить.
    Unreadable,
}

/// Помнит ли хеш-память набора выбранную базу. Ничего не пишет.
pub fn snapshot_memory(context: &SourceSetContext, work_path: &Path) -> SnapshotMemory {
    let Some(path) = context.storage_path(work_path) else {
        return SnapshotMemory::Nothing;
    };
    if context.storage_identity().is_none() {
        return SnapshotMemory::Nothing;
    }
    // The rows are not needed for this answer; the analysis that uses them reads them itself.
    match HashStorage::new(path).load_summary() {
        Ok(summary) if summary.is_blank() => SnapshotMemory::Nothing,
        // Written before #575: its hashes compare with nothing, and it is no memory of the base.
        Ok(summary) if summary.is_legacy() => SnapshotMemory::Unreadable,
        Ok(summary) if summary.identity.as_deref() == context.storage_identity() => {
            SnapshotMemory::Own
        }
        Ok(_) => SnapshotMemory::Foreign,
        Err(error) if error.is_recoverable() => SnapshotMemory::Nothing,
        Err(_) => SnapshotMemory::Unreadable,
    }
}

/// Записывает пустую память набора о базе: база есть, и в ней нет ничего из каталога.
/// Первая отправка после этого грузит весь набор.
pub fn commit_empty_snapshot(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<(), ChangeDetectionError> {
    commit_full_snapshot(
        context,
        work_path,
        &FullSnapshot {
            snapshot: HashMap::new(),
            hash_kind: HashKind::RawBlob,
            scan_started_at: 0,
        },
    )
}

/// Persist a prepared snapshot after the corresponding build/load step succeeded.
pub fn commit_success(
    context: &SourceSetContext,
    work_path: &Path,
    prepared: &PreparedStateUpdate,
) -> Result<(), ChangeDetectionError> {
    let Some(path) = context.storage_path(work_path) else {
        return Ok(());
    };
    let storage = HashStorage::new(path);
    storage
        .commit_delta_with_identity(
            &prepared.delta,
            prepared.scan_started_at,
            prepared.observed_generation,
            context.storage_identity(),
        )
        .map_err(|e| map_commit_error(context, storage.path(), e))
}

/// Re-scan the source-set from scratch and replace the stored snapshot.
pub fn rescan_and_commit_full(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<(), ChangeDetectionError> {
    if !context.persists_snapshot() {
        return Ok(());
    }
    let prepared = prepare_source_snapshot(context, work_path)?;
    commit_full_snapshot(context, work_path, &prepared)
}

/// [`rescan_and_commit_full`] with the files and their ids taken from `source`: under git the
/// memory is written as blob ids, so the next analysis asks git (#575). When git cannot
/// describe the set, every file is read.
pub fn rescan_and_commit_full_from(
    context: &SourceSetContext,
    work_path: &Path,
    source: ChangeSource<'_, '_>,
) -> Result<(), ChangeDetectionError> {
    if !context.persists_snapshot() {
        return Ok(());
    }
    if let ChangeSource::Git(git) = source {
        if commit_full_from_git(context, work_path, git)? {
            return Ok(());
        }
    }
    let prepared = prepare_source_snapshot(context, work_path)?;
    commit_full_snapshot(context, work_path, &prepared)
}

/// Replaces the memory of the set with the blob ids git gives its tree now; a file whose id is
/// unknown — it moved while it was hashed — is not remembered and stays a change. `Ok(false)`:
/// git cannot describe the set, and nothing is written.
pub fn commit_full_from_git(
    context: &SourceSetContext,
    work_path: &Path,
    mut git: GitSource<'_, '_>,
) -> Result<bool, ChangeDetectionError> {
    if !context.persists_snapshot() {
        return Ok(true);
    }
    let observed = match git.observe(context.path(), &mut || false) {
        Ok(observed) => observed,
        Err(ObserveError::Interrupted) => return Err(ChangeDetectionError::Interrupted),
        Err(ObserveError::Unusable { reason, detail }) => {
            tracing::warn!(source_set = %context.name(), reason = ?reason, detail, "git cannot describe the set; its files are read");
            return Ok(false);
        }
    };
    let prepared = FullSnapshot {
        snapshot: observed
            .into_iter()
            .filter_map(|file| {
                Some((
                    file.rel_path,
                    StoredFileState {
                        mtime_ns: 0,
                        hash: file.hash?,
                    },
                ))
            })
            .collect(),
        hash_kind: HashKind::GitBlob,
        scan_started_at: now_ns(),
    };
    commit_full_snapshot(context, work_path, &prepared).map(|()| true)
}

/// The tree a full pull staged and then published into the set, as git would store it (#575).
/// Git names the ids of the published files; a row of `prepared` keeps only when the published
/// file still carries the time it had in staging — publication moves files, it does not
/// rewrite them, — so an edit made after publication stays a change. `None`: git cannot
/// describe the set, and the caller remembers `prepared` as read.
pub fn full_snapshot_by_git(
    context: &SourceSetContext,
    prepared: &FullSnapshot,
    mut git: GitSource<'_, '_>,
) -> Result<Option<FullSnapshot>, ChangeDetectionError> {
    let observed = match git.observe(context.path(), &mut || false) {
        Ok(observed) => observed,
        Err(ObserveError::Interrupted) => return Err(ChangeDetectionError::Interrupted),
        Err(ObserveError::Unusable { reason, detail }) => {
            tracing::warn!(source_set = %context.name(), reason = ?reason, detail, "git cannot describe the published set; it is remembered as read");
            return Ok(None);
        }
    };
    let ids: HashMap<String, String> = observed
        .into_iter()
        .filter_map(|file| Some((file.rel_path, file.hash?)))
        .collect();
    let snapshot = prepared
        .snapshot
        .iter()
        .filter_map(|(rel_path, staged)| {
            let path = context.path().join(rel_path);
            let published = std::fs::metadata(&path)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| mtime_nanos(modified, &path).ok());
            (published == Some(staged.mtime_ns)).then_some(())?;
            Some((
                rel_path.clone(),
                StoredFileState {
                    mtime_ns: staged.mtime_ns,
                    hash: ids.get(rel_path)?.clone(),
                },
            ))
        })
        .collect();
    Ok(Some(FullSnapshot {
        snapshot,
        hash_kind: HashKind::GitBlob,
        scan_started_at: prepared.scan_started_at,
    }))
}

/// The tree of the set as `source` names it: under git the ids `git add` would store, else the
/// ids of the bytes. A file whose id is unknown is not remembered (#575).
pub fn prepare_source_snapshot_from(
    context: &SourceSetContext,
    work_path: &Path,
    source: ChangeSource<'_, '_>,
) -> Result<FullSnapshot, ChangeDetectionError> {
    if let ChangeSource::Git(mut git) = source {
        match git.observe(context.path(), &mut || false) {
            Ok(observed) => {
                return Ok(FullSnapshot {
                    snapshot: observed
                        .into_iter()
                        .filter_map(|file| {
                            Some((
                                file.rel_path,
                                StoredFileState {
                                    mtime_ns: 0,
                                    hash: file.hash?,
                                },
                            ))
                        })
                        .collect(),
                    hash_kind: HashKind::GitBlob,
                    scan_started_at: now_ns(),
                })
            }
            Err(ObserveError::Interrupted) => return Err(ChangeDetectionError::Interrupted),
            Err(ObserveError::Unusable { reason, detail }) => {
                tracing::warn!(source_set = %context.name(), reason = ?reason, detail, "git cannot describe the set; its files are read");
            }
        }
    }
    prepare_source_snapshot(context, work_path)
}

/// Hash the source-set directory itself, which can hold `workPath`.
pub fn prepare_source_snapshot(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<FullSnapshot, ChangeDetectionError> {
    let excluded = scanner::work_path_inside(context.path(), work_path);
    prepare_snapshot(context, context.path(), excluded)
}

/// Hash an exported tree before publishing it; does not open or mutate storage.
pub fn prepare_full_snapshot(
    context: &SourceSetContext,
    source_path: &Path,
) -> Result<FullSnapshot, ChangeDetectionError> {
    prepare_snapshot(context, source_path, None)
}

fn prepare_snapshot(
    context: &SourceSetContext,
    source_path: &Path,
    excluded: Option<std::path::PathBuf>,
) -> Result<FullSnapshot, ChangeDetectionError> {
    // The caller records an already successful load/publication: cancellation must not
    // leave its memory describing the preceding database/tree.
    let scan = scan_sources(context, source_path, excluded.as_deref(), &mut || false)
        .map_err(|error| map_scan_error(context, error))?;
    Ok(FullSnapshot {
        snapshot: scan
            .files
            .into_iter()
            .map(|candidate| {
                (
                    candidate.rel_path,
                    StoredFileState {
                        mtime_ns: candidate.mtime_ns,
                        hash: candidate.hash,
                    },
                )
            })
            .collect(),
        hash_kind: HashKind::RawBlob,
        scan_started_at: scan.scan_started_at,
    })
}

/// Commit after successful publication or full loading, replacing a foreign identity.
pub fn commit_full_snapshot(
    context: &SourceSetContext,
    work_path: &Path,
    prepared: &FullSnapshot,
) -> Result<(), ChangeDetectionError> {
    let Some(path) = context.storage_path(work_path) else {
        return Ok(());
    };
    let storage = HashStorage::new(path);
    let generation = match storage.current_generation() {
        Ok(generation) => generation,
        Err(error) if error.is_recoverable() => {
            return storage
                .recover_and_commit_snapshot_with_identity(
                    &prepared.snapshot,
                    prepared.hash_kind,
                    prepared.scan_started_at,
                    context.storage_identity(),
                )
                .map_err(|error| map_commit_error(context, storage.path(), error))
        }
        Err(error) => return Err(map_storage_hard(context, storage.path(), error)),
    };
    storage
        .commit_snapshot_with_identity(
            &prepared.snapshot,
            prepared.hash_kind,
            prepared.scan_started_at,
            generation,
            context.storage_identity(),
        )
        .map_err(|error| map_commit_error(context, storage.path(), error))
}

fn scan_sources(
    context: &SourceSetContext,
    path: &Path,
    excluded: Option<&Path>,
    interrupted: &mut dyn FnMut() -> bool,
) -> Result<scanner::ScanSnapshot, ScanError> {
    scanner::scan_in_scope(path, excluded, interrupted, scan_scope(context))
}

fn scan_scope(context: &SourceSetContext) -> ScanScope {
    if context.is_build_memory() {
        ScanScope::Build
    } else {
        ScanScope::Push
    }
}

/// When a dump into a directory began, read by the clock of that directory's file system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DumpStart {
    started_ns: u64,
}

/// Marks the start of a dump into `dir`: the one owner of the clock probe (#557, #571).
///
/// The dump's files are stamped by the clock of the directory's file system, which can be
/// coarser than this process's clock or, on a share, apart from it: the start is read from a
/// file written there, and the file is gone before this returns, so the dump never sees it.
pub fn mark_dump_start(dir: &Path) -> std::io::Result<DumpStart> {
    let probe = tempfile::Builder::new()
        .prefix(scanner::DUMP_START_PROBE)
        .tempfile_in(dir)?;
    let modified = probe.as_file().metadata()?.modified()?;
    let started_ns = mtime_nanos(modified, dir).map_err(std::io::Error::other)?;
    probe.close()?;
    Ok(DumpStart { started_ns })
}

/// The source tree as an incremental dump found it: which files it held and when the dump
/// began. Taken before the platform starts writing into the tree.
pub struct DumpWatch {
    started_ns: u64,
    /// Every listed file with its modification time.
    before: HashMap<String, u64>,
}

/// Lists the tree before an incremental dump that began at `start`, without reading files.
/// `None` when the set keeps no memory of this base or the tree cannot be listed: the dump then
/// records nothing and the next push loads what the dump wrote, as before (#557).
pub fn watch_before_dump(
    context: &SourceSetContext,
    work_path: &Path,
    start: DumpStart,
) -> Option<DumpWatch> {
    if !context.persists_snapshot() || snapshot_memory(context, work_path) != SnapshotMemory::Own {
        return None;
    }
    let excluded = scanner::work_path_inside(context.path(), work_path);
    let listed = scanner::list_in_scope(
        context.path(),
        excluded.as_deref(),
        &mut || false,
        scan_scope(context),
    );
    if let Err(error) = &listed {
        tracing::debug!(
            source_set = %context.name(),
            %error,
            "the tree was not listed before the dump; what the dump writes is not remembered"
        );
    }
    Some(DumpWatch {
        started_ns: start.started_ns,
        before: listed
            .ok()?
            .files
            .into_iter()
            .map(|file| (file.rel_path, file.mtime_ns))
            .collect(),
    })
}

/// The tree after a dump, listed in the set's walk scope without reading a file.
///
/// One listing answers both what the dump wrote ([`observe_dump_writes`]) and what it did to
/// the tree ([`Self::fact`]).
pub struct DumpListing {
    files: Vec<scanner::Candidate>,
}

/// What a dump by the version file did to the set's tree
/// (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpFact {
    /// The tree holds files, and none is older than the dump's start: the dump was full.
    RewroteAll,
    /// A file older than the start stayed, or the tree is empty: the dump took the changes.
    Partly,
    /// The start was not marked or the tree was not listed: the mode is not known.
    Unlisted,
}

/// Lists the set's tree after a dump: the set's walk scope, `workPath` and service directories
/// excluded, links never followed. `ConfigDumpInfo.xml` is out of the scope.
pub fn list_after_dump(
    context: &SourceSetContext,
    work_path: &Path,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> Result<DumpListing, ChangeDetectionError> {
    let excluded = scanner::work_path_inside(context.path(), work_path);
    scanner::list_in_scope(
        context.path(),
        excluded.as_deref(),
        &mut || interrupted(),
        scan_scope(context),
    )
    .map(|listed| DumpListing {
        files: listed.files,
    })
    .map_err(|error| map_scan_error(context, error))
}

impl DumpListing {
    /// A dump that began at `start` rewrote everything when the tree is not empty and every
    /// file in it is stamped no earlier than the start.
    pub fn fact(&self, start: DumpStart) -> DumpFact {
        if !self.files.is_empty()
            && self
                .files
                .iter()
                .all(|file| file.mtime_ns >= start.started_ns)
        {
            DumpFact::RewroteAll
        } else {
            DumpFact::Partly
        }
    }
}

/// The files of a Designer dump whose element the dump dropped: the deepest element that owns a
/// file by its path ([`partial_load::dump_element_of`]) was in the description `before` the
/// dump and no name of the description `after` it matches, letter case aside. A file the dump
/// wrote (stamped no earlier than `start`) is never named: on a case-insensitive file system an
/// element renamed only in letter case keeps its files under the old spelling. Files without a
/// recognized owner and files of elements that stayed are not named either. Listed in the set's
/// walk scope, links never followed; this listing precedes the one the dump's mode comes from,
/// because what it names is gone before that one.
pub fn files_of_dropped_elements(
    context: &SourceSetContext,
    work_path: &Path,
    start: DumpStart,
    before: &HashSet<String>,
    after: &HashSet<String>,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<PathBuf>, ChangeDetectionError> {
    let caseless = |names: &HashSet<String>| -> HashSet<String> {
        names.iter().map(|name| name.to_lowercase()).collect()
    };
    let (before, after) = (caseless(before), caseless(after));
    let listing = list_after_dump(context, work_path, interrupted)?;
    Ok(listing
        .files
        .into_iter()
        .filter(|file| file.mtime_ns < start.started_ns)
        .filter(|file| {
            crate::change_detection::partial_load::dump_element_of(Path::new(&file.rel_path))
                .map(|owner| owner.to_lowercase())
                .is_some_and(|owner| before.contains(&owner) && !after.contains(&owner))
        })
        .map(|file| file.path)
        .collect())
}

/// What an incremental dump wrote into the tree, found and hashed but not yet remembered.
///
/// Finding it reads only files and the memory, so it can run while the platform answers
/// whether the base changed during the dump; [`Self::commit`] is the step that waits for
/// that answer (#571). Dropping it forgets what the dump wrote.
#[must_use = "what the dump wrote is remembered only by `DumpWrites::commit`"]
pub struct DumpWrites {
    /// The memory as it was read, without its rows, and the rows the dump changed. `None`:
    /// nothing to remember — no memory for this set, memory unreadable (not repaired here),
    /// or the dump wrote and removed nothing.
    rows: Option<(StorageSummary, StorageDelta)>,
    started_ns: u64,
}

/// After a successful incremental dump, finds what it wrote: a file is written by the dump
/// when its modification time is not before the dump began and it is new or its time moved
/// since the listing — so an edit saved in the very clock tick the dump began stays a change.
/// Written files are hashed by content and replace their entries, files that were there before
/// the dump and are gone are forgotten. Every other entry stays, so a user edit made before
/// the dump remains a change. `ended` marks the end of the dump by the same clock: a file
/// stamped after it was edited after the platform finished and stays a change, and a file
/// whose time moved while it was hashed stays a change too. An edit made while the dump ran
/// cannot be told from the platform's writes and is remembered (#170). `listed` is the tree
/// after the dump ([`list_after_dump`]).
///
/// Under git `blobs` hashes the written files, named from the set root `root` of the working
/// tree, as `git add` stores them; without git they are hashed in-process the same way
/// without filters (#575).
pub fn observe_dump_writes(
    context: &SourceSetContext,
    work_path: &Path,
    watch: &DumpWatch,
    ended: Option<DumpStart>,
    listed: DumpListing,
    blobs: Option<(&str, &mut BlobHasher<'_>)>,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> Result<DumpWrites, ChangeDetectionError> {
    let writes = |rows| DumpWrites {
        rows,
        started_ns: watch.started_ns,
    };
    let Some(path) = context.storage_path(work_path) else {
        return Ok(writes(None));
    };
    let storage = HashStorage::new(path);
    // Only the rows the dump changed are written, so the other rows are not read (#576).
    // The summary sees tables of different length: such memory is not repaired here, and the
    // next push falls back to a full load. Damage that keeps the lengths equal — an orphan row
    // next to a missing row of another key in the other table — passes the summary, and the
    // delta commit keeps it; the next analysis reads every row, reports it as recoverable and
    // falls back to a full load, whose commit repairs both tables (#571).
    let Some(summary) = load_bound_summary(context, &storage)? else {
        return Ok(writes(None));
    };
    let after: HashSet<&str> = listed
        .files
        .iter()
        .map(|file| file.rel_path.as_str())
        .collect();
    let removed: Vec<String> = watch
        .before
        .keys()
        .filter(|rel_path| !after.contains(rel_path.as_str()))
        .cloned()
        .collect();
    let written: Vec<scanner::Candidate> = listed
        .files
        .into_iter()
        .filter(|file| {
            file.mtime_ns >= watch.started_ns
                // Не позже отметки конца: на Windows время файла идёт тиками системных часов
                // (~15 мс), и последняя запись платформы ложится в тот же тик, что и отметка.
                // Правка в этот же тик запоминается как запись — это окно и остаётся за #170.
                && ended.is_none_or(|end| file.mtime_ns <= end.started_ns)
                && watch.before.get(&file.rel_path) != Some(&file.mtime_ns)
        })
        .collect();
    if written.is_empty() && removed.is_empty() {
        return Ok(writes(None));
    }
    // Файл, время которого сдвинулось, пока его читали, в ответ не попадает и остаётся изменением.
    // Записанное хешируется тем же хешером, что записал память: id двух хешеров в одной
    // памяти не встречаются (#575). Память хешами git без git ничего не запоминает.
    let upserts = match (summary.hash_kind, blobs) {
        (HashKind::GitBlob, None) => return Ok(writes(None)),
        (HashKind::GitBlob, Some((root, hash_blobs))) => {
            blob_ids_unmoved(context, root, written, hash_blobs, interrupted)?
        }
        _ => scanner::hash_listed_unmoved(written, &mut || interrupted())
            .map_err(|error| map_scan_error(context, error))?
            .into_iter()
            .map(|file| {
                (
                    file.rel_path,
                    StoredFileState {
                        mtime_ns: file.mtime_ns,
                        hash: file.hash,
                    },
                )
            })
            .collect(),
    };
    let hash_kind = summary.hash_kind;
    Ok(writes(Some((
        summary,
        StorageDelta {
            upserts,
            removals: removed,
            hash_kind,
        },
    ))))
}

/// Blob ids of `written` files whose time stayed put while they were hashed.
fn blob_ids_unmoved(
    context: &SourceSetContext,
    root: &str,
    written: Vec<scanner::Candidate>,
    hash_blobs: &mut BlobHasher<'_>,
    interrupted: &(dyn Fn() -> bool + Sync),
) -> Result<HashMap<String, StoredFileState>, ChangeDetectionError> {
    let names: Vec<String> = written
        .iter()
        .map(|file| {
            if root.is_empty() {
                file.rel_path.clone()
            } else {
                format!("{root}/{}", file.rel_path)
            }
        })
        .collect();
    let ids = hash_blobs(&names, &mut || interrupted()).map_err(|error| match error {
        BlobHashError::Interrupted => ChangeDetectionError::Interrupted,
        BlobHashError::Failed(reason) => ChangeDetectionError::StorageHard {
            source_set: context.name().to_owned(),
            storage_path: context.path().to_path_buf(),
            reason: format!("git hash-object failed: {reason}"),
        },
    })?;
    Ok(written
        .into_iter()
        .zip(ids)
        .filter(|(file, _)| {
            std::fs::metadata(&file.path)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| mtime_nanos(modified, &file.path).ok())
                == Some(file.mtime_ns)
        })
        .map(|(file, hash)| (file.rel_path, StoredFileState { mtime_ns: 0, hash }))
        .collect())
}

impl DumpWrites {
    /// Remembers what the dump wrote. The commit fails if the memory moved since it was read.
    pub fn commit(
        self,
        context: &SourceSetContext,
        work_path: &Path,
    ) -> Result<(), ChangeDetectionError> {
        let (Some((summary, delta)), Some(path)) = (self.rows, context.storage_path(work_path))
        else {
            return Ok(());
        };
        let storage = HashStorage::new(path);
        storage
            .commit_delta_with_identity(
                &delta,
                summary.watermark.unwrap_or(0).max(self.started_ns),
                summary.generation,
                context.storage_identity(),
            )
            .map_err(|error| map_commit_error(context, storage.path(), error))
    }
}

/// [`observe_dump_writes`] and [`DumpWrites::commit`] in one step.
#[cfg(test)]
pub fn record_dump_changes(
    context: &SourceSetContext,
    work_path: &Path,
    watch: &DumpWatch,
) -> Result<(), ChangeDetectionError> {
    let listed = list_after_dump(context, work_path, &|| false)?;
    observe_dump_writes(context, work_path, watch, None, listed, None, &|| false)?
        .commit(context, work_path)
}

fn detect_changes(
    candidates: &[scanner::HashedFile],
    stored: &HashMap<String, StoredFileState>,
) -> Vec<FileChange> {
    candidates
        .iter()
        .filter_map(|candidate| {
            let kind = match stored.get(&candidate.rel_path) {
                None => ChangeKind::Added,
                Some(existing) if existing.hash != candidate.hash => ChangeKind::Modified,
                Some(_) => return None,
            };
            Some(FileChange {
                path: candidate.path.clone(),
                kind,
            })
        })
        .collect()
}

/// The scan as rows that differ from `stored`: a file new or with another time or hash is
/// written, a remembered file the scan did not find is forgotten. This filter only keeps the
/// delta small; the commit's `write_row` compares each named row again inside its write
/// transaction, against what is committed then.
fn build_prepared_state(
    scan: &scanner::ScanSnapshot,
    stored: &StorageSnapshot,
) -> PreparedStateUpdate {
    let upserts = scan
        .files
        .iter()
        .filter(|file| {
            stored
                .entries
                .get(&file.rel_path)
                .is_none_or(|entry| entry.mtime_ns != file.mtime_ns || entry.hash != file.hash)
        })
        .map(|file| {
            (
                file.rel_path.clone(),
                StoredFileState {
                    mtime_ns: file.mtime_ns,
                    hash: file.hash.clone(),
                },
            )
        })
        .collect();
    let scanned: HashSet<&str> = scan
        .files
        .iter()
        .map(|file| file.rel_path.as_str())
        .collect();
    let removals = stored
        .entries
        .keys()
        .filter(|rel_path| !scanned.contains(rel_path.as_str()))
        .cloned()
        .collect();
    PreparedStateUpdate {
        delta: StorageDelta {
            upserts,
            removals,
            hash_kind: HashKind::RawBlob,
        },
        scanned_files: scan.files.len(),
        scan_started_at: scan.scan_started_at,
        observed_generation: stored.generation,
    }
}

pub struct FullSnapshot {
    snapshot: HashMap<String, StoredFileState>,
    hash_kind: HashKind,
    scan_started_at: u64,
}

fn map_storage_hard(
    context: &SourceSetContext,
    storage_path: &Path,
    err: StorageError,
) -> ChangeDetectionError {
    ChangeDetectionError::StorageHard {
        source_set: context.name().to_owned(),
        storage_path: storage_path.to_path_buf(),
        reason: err.to_string(),
    }
}

fn map_commit_error(
    context: &SourceSetContext,
    storage_path: &Path,
    err: StorageError,
) -> ChangeDetectionError {
    match err {
        StorageError::ConcurrentStateModified {
            expected, actual, ..
        } => ChangeDetectionError::ConcurrentStateModified {
            source_set: context.name().to_owned(),
            storage_path: storage_path.to_path_buf(),
            expected,
            actual,
        },
        other => map_storage_hard(context, storage_path, other),
    }
}

fn map_scan_error(context: &SourceSetContext, err: ScanError) -> ChangeDetectionError {
    if matches!(err, ScanError::Interrupted) {
        return ChangeDetectionError::Interrupted;
    }
    ChangeDetectionError::StorageHard {
        source_set: context.name().to_owned(),
        storage_path: context.path().to_path_buf(),
        reason: format!("scan failed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_context, files_of_dropped_elements, mark_dump_start, record_dump_changes,
        rescan_and_commit_full, watch_before_dump, AnalysisOutcome, ChangeDetectionError,
        ChangeKind, FileChange,
    };
    use crate::change_detection::partial_load::decide;
    use crate::domain::source_set::SourceSetContext;
    use std::collections::HashSet;
    use std::fs::File;
    use std::time::SystemTime;
    use tempfile::tempdir;

    /// Память, которую записал раннер до #575 (хеши SHA-256, вида хеша в ней нет), — не память
    /// о базе: `push` откажет `untracked_base`, анализ грузит набор целиком, а запись новой
    /// памяти поверх неё — уже память хешами git.
    #[test]
    fn memory_written_before_git_ids_is_no_memory_of_the_base() {
        use super::{snapshot_memory, SnapshotMemory};
        use crate::change_detection::hash_storage::{HashStorage, META, META_KEY_HASH_KIND};

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(root.join("Module.bsl"), "module").expect("module");
        let context = SourceSetContext::new("main", root, "designer-main")
            .with_infobase_memory("ib", "file:/ib; source=src".to_owned());
        rescan_and_commit_full(&context, &work).expect("memory");
        assert_eq!(snapshot_memory(&context, &work), SnapshotMemory::Own);

        let path = context.storage_path(&work).expect("storage");
        {
            let db = redb::Database::open(&path).expect("open");
            let tx = db.begin_write().expect("write");
            tx.open_table(META)
                .expect("meta")
                .remove(META_KEY_HASH_KIND)
                .expect("forget the kind");
            tx.commit().expect("old memory");
        }
        assert!(HashStorage::new(path.clone())
            .load_summary()
            .expect("summary")
            .is_legacy());
        assert_eq!(snapshot_memory(&context, &work), SnapshotMemory::Unreadable);
        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::Fallback)
        ));

        rescan_and_commit_full(&context, &work).expect("new memory");
        assert_eq!(snapshot_memory(&context, &work), SnapshotMemory::Own);
        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }

    #[test]
    fn partial_load_contract_stays_compatible_with_file_change() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let object_dir = root.join("Catalogs.Items");
        let module = object_dir.join("ObjectModule.bsl");
        std::fs::create_dir_all(&object_dir).expect("object dir");
        std::fs::write(&module, "module").expect("module");

        let changes = vec![FileChange {
            path: module,
            kind: ChangeKind::Modified,
        }];
        let decision = decide(&changes, &root);
        assert!(matches!(
            decision,
            crate::change_detection::partial_load::LoadDecision::Partial(_)
        ));
    }

    /// Файлы `workPath` внутри набора не попадают в память и не становятся правкой.
    #[test]
    fn a_service_file_in_a_work_path_inside_the_set_is_not_a_change() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let work_path = root.join("service-area");
        std::fs::create_dir_all(&work_path).expect("work");
        std::fs::write(root.join("Configuration.xml"), "<xml />").expect("config");
        std::fs::write(work_path.join("state.txt"), "before").expect("state");
        let context = SourceSetContext::new("main", root, "designer-main");
        rescan_and_commit_full(&context, &work_path).expect("snapshot");

        std::fs::write(work_path.join("state.txt"), "after").expect("state");

        assert!(matches!(
            analyze_context(&context, &work_path, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }

    #[test]
    fn hard_storage_errors_stay_hard_during_full_rescan() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        let work_path = dir.path().join("work");
        std::fs::create_dir_all(&source_root).expect("source");
        std::fs::write(source_root.join("Configuration.xml"), "<xml />").expect("config");

        let storage_path = work_path.join("hash-storages").join("designer-main.redb");
        std::fs::create_dir_all(&storage_path).expect("storage dir");

        let context = SourceSetContext::new("main", source_root, "designer-main");
        let error = rescan_and_commit_full(&context, &work_path).expect_err("expected hard error");

        assert!(matches!(error, ChangeDetectionError::StorageHard { .. }));
    }

    /// Кандидата подтверждает хеш: файл, переписанный тем же содержимым, изменением не
    /// считается, а изменённый рядом с ним — считается. Время изменения у обоих одно, так
    /// что в кандидаты они попадают вместе, и найденная правка соседа доказывает, что
    /// переписанный файл тоже хешировали.
    #[test]
    fn a_file_rewritten_with_the_same_content_is_not_a_change() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        let work_path = dir.path().join("work");
        std::fs::create_dir_all(&source_root).expect("source");
        let same = source_root.join("Same.bsl");
        let edited = source_root.join("Edited.bsl");
        std::fs::write(&same, "Процедура А() КонецПроцедуры").expect("same");
        std::fs::write(&edited, "Процедура Б() КонецПроцедуры").expect("edited");
        let context = SourceSetContext::new("main", source_root, "designer-main");
        rescan_and_commit_full(&context, &work_path).expect("prime");

        std::fs::write(&same, "Процедура А() КонецПроцедуры").expect("rewrite");
        std::fs::write(&edited, "Процедура Б() Возврат; КонецПроцедуры").expect("edit");
        let touched = SystemTime::now();
        for path in [&same, &edited] {
            File::options()
                .write(true)
                .open(path)
                .expect("open")
                .set_modified(touched)
                .expect("set mtime");
        }

        let analysis = analyze_context(&context, &work_path, &mut || false);

        let Ok(AnalysisOutcome::Changes {
            changes,
            prepared: _,
        }) = analysis.outcome
        else {
            panic!("the edited file must be a change: {:?}", analysis.outcome);
        };
        let changed: Vec<_> = changes
            .into_iter()
            .map(|change| (change.path, change.kind))
            .collect();
        assert_eq!(changed, [(edited, ChangeKind::Modified)]);
    }
    /// Модуль, восстановленный из сохранённой копии со старым временем изменения (так
    /// делают `cp -p`, `Copy-Item`, распаковка архива), — изменение, хотя его время раньше
    /// отметки последнего анализа (#447, найдено в Unica).
    #[test]
    fn a_module_restored_with_an_old_mtime_is_a_change() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        let work_path = dir.path().join("work");
        std::fs::create_dir_all(&source_root).expect("source");
        let module = source_root.join("Tests.bsl");
        let saved_at = SystemTime::now() - std::time::Duration::from_secs(600);
        std::fs::write(&module, "Процедура ВременныйТест() КонецПроцедуры").expect("temporary");
        let context = SourceSetContext::new("main", source_root, "designer-main");
        rescan_and_commit_full(&context, &work_path).expect("prime");

        std::fs::write(&module, "Процедура Тест() КонецПроцедуры").expect("restore");
        File::options()
            .write(true)
            .open(&module)
            .expect("open")
            .set_modified(saved_at)
            .expect("restore mtime");

        let analysis = analyze_context(&context, &work_path, &mut || false);

        let Ok(AnalysisOutcome::Changes { changes, .. }) = analysis.outcome else {
            panic!(
                "the restored module must be a change: {:?}",
                analysis.outcome
            );
        };
        let changed: Vec<_> = changes
            .into_iter()
            .map(|change| (change.path, change.kind))
            .collect();
        assert_eq!(changed, [(module, ChangeKind::Modified)]);
    }

    #[test]
    fn publishing_a_prepared_snapshot_does_not_absorb_a_later_user_edit() {
        let dir = tempdir().expect("tempdir");
        let staging = dir.path().join("staging");
        let target = dir.path().join("target");
        let work = dir.path().join("work");
        std::fs::create_dir(&staging).expect("staging");
        std::fs::write(staging.join("Module.bsl"), "exported").expect("export");
        let context = SourceSetContext::new("main", target.clone(), "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        let prepared = super::prepare_full_snapshot(&context, &staging).expect("prepare");
        assert!(!work.exists(), "preparation never opens memory");
        std::fs::rename(&staging, &target).expect("publish");
        std::fs::write(target.join("Module.bsl"), "user edit after publication")
            .expect("user edit");
        super::commit_full_snapshot(&context, &work, &prepared).expect("commit prepared");
        let outcome = analyze_context(&context, &work, &mut || false)
            .outcome
            .expect("analysis");
        assert!(
            matches!(outcome, AnalysisOutcome::Changes { ref changes, .. } if changes.len() == 1 && changes[0].kind == ChangeKind::Modified)
        );
    }
    /// A same-size edit with exactly the old stored mtime is loaded once and then skipped.
    #[test]
    fn changed_bytes_with_the_exact_remembered_old_mtime_are_a_change() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("source");
        let module = root.join("Tests.bsl");
        std::fs::write(&module, "Процедура А() КонецПроцедуры").expect("before bytes");
        File::options()
            .write(true)
            .open(&module)
            .expect("open before")
            .set_modified(SystemTime::now() - std::time::Duration::from_secs(600))
            .expect("old mtime");
        let before = std::fs::metadata(&module).expect("before metadata");
        let remembered = before.modified().expect("before mtime");
        let context = SourceSetContext::new("main", root, "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        rescan_and_commit_full(&context, &work).expect("prime A with old mtime");
        let storage = crate::change_detection::hash_storage::HashStorage::new(
            context.storage_path(&work).expect("storage path"),
        );
        let primed = storage.load_snapshot().expect("primed memory");
        let old = primed.entries.get("Tests.bsl").expect("remembered module");
        assert_eq!(
            old.mtime_ns,
            crate::change_detection::file_state::mtime_nanos(remembered, &module)
                .expect("mtime nanos")
        );
        assert!(old.mtime_ns < primed.watermark.expect("watermark") - 2_000_000_000);

        std::fs::write(&module, "Процедура Б() КонецПроцедуры").expect("edited bytes");
        File::options()
            .write(true)
            .open(&module)
            .expect("open edited")
            .set_modified(remembered)
            .expect("restore exact mtime");
        let after = std::fs::metadata(&module).expect("after metadata");
        assert_eq!(before.len(), after.len(), "size is unchanged");
        assert_eq!(remembered, after.modified().expect("after mtime"));

        let outcome = analyze_context(&context, &work, &mut || false)
            .outcome
            .expect("analysis");
        let AnalysisOutcome::Changes { changes, prepared } = outcome else {
            panic!("same-mtime different bytes must be a change: {outcome:?}");
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, module);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        let pending = storage
            .load_snapshot()
            .expect("memory before successful load");
        assert_eq!(
            pending.generation, primed.generation,
            "analysis does not commit"
        );
        assert_eq!(pending.entries["Tests.bsl"].hash, old.hash);
        assert_eq!(pending.entries["Tests.bsl"].mtime_ns, old.mtime_ns);
        let edited_hash = crate::change_detection::scanner::hash_file(&module, &mut || false)
            .expect("edited hash");
        assert_ne!(edited_hash, old.hash);

        super::commit_success(&context, &work, &prepared).expect("successful load commits B");
        let loaded = storage.load_snapshot().expect("memory after load");
        assert_eq!(loaded.generation, primed.generation + 1);
        assert_eq!(loaded.entries["Tests.bsl"].hash, edited_hash);
        assert_eq!(loaded.entries["Tests.bsl"].mtime_ns, old.mtime_ns);
        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }
    /// An edit saved after the dump ended stays a change, not a write of the platform (#170).
    #[test]
    fn an_edit_after_the_dump_ended_stays_a_change() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("root");
        for name in ["Written.bsl", "Late.bsl"] {
            std::fs::write(root.join(name), "before").expect(name);
        }
        let context = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        rescan_and_commit_full(&context, &work).expect("prime");
        let start = mark_dump_start(&root).expect("dump start");
        let watch = watch_before_dump(&context, &work, start).expect("memory of this base");

        // The platform writes one file, the dump ends, then someone edits another file before
        // the memory is written (#170).
        std::fs::write(root.join("Written.bsl"), "from the base").expect("written");
        let ended = mark_dump_start(&root).expect("dump end");
        std::fs::write(root.join("Late.bsl"), "edited after the dump").expect("late edit");
        File::options()
            .write(true)
            .open(root.join("Late.bsl"))
            .expect("open")
            .set_modified(SystemTime::now() + std::time::Duration::from_secs(60))
            .expect("later mtime");

        let listed = super::list_after_dump(&context, &work, &|| false).expect("listing");
        super::observe_dump_writes(&context, &work, &watch, Some(ended), listed, None, &|| {
            false
        })
        .expect("observe")
        .commit(&context, &work)
        .expect("commit");

        let analysis = analyze_context(&context, &work, &mut || false);
        let Ok(AnalysisOutcome::Changes { changes, .. }) = analysis.outcome else {
            panic!("{:?}", analysis.outcome);
        };
        let names: Vec<_> = changes
            .iter()
            .map(|change| change.path.file_name().expect("name").to_owned())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("Late.bsl")]);
    }

    /// What an incremental dump wrote is remembered; a user edit made before it is not (#557).
    #[test]
    fn a_dump_remembers_what_it_wrote_and_keeps_an_earlier_user_edit() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("root");
        for name in [
            "Written.bsl",
            "Removed.bsl",
            "Mine.bsl",
            "Ahead.bsl",
            "Kept.bsl",
        ] {
            std::fs::write(root.join(name), "before").expect(name);
        }
        let context = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        rescan_and_commit_full(&context, &work).expect("prime");

        // The user's edit lands before the dump starts.
        std::fs::write(root.join("Mine.bsl"), "mine").expect("user edit");
        File::options()
            .write(true)
            .open(root.join("Mine.bsl"))
            .expect("open")
            .set_modified(SystemTime::now() - std::time::Duration::from_secs(60))
            .expect("old mtime");
        // An edit stamped no earlier than the dump's start, as in the tick the dump begins.
        std::fs::write(root.join("Ahead.bsl"), "mine too").expect("user edit");
        File::options()
            .write(true)
            .open(root.join("Ahead.bsl"))
            .expect("open")
            .set_modified(SystemTime::now() + std::time::Duration::from_secs(3600))
            .expect("late mtime");
        let start = mark_dump_start(&root).expect("dump start");
        let watch = watch_before_dump(&context, &work, start).expect("memory of this base");

        // The dump rewrites one file, adds one and removes one.
        std::fs::write(root.join("Written.bsl"), "from the base").expect("written");
        std::fs::write(root.join("Added.bsl"), "new in the base").expect("added");
        std::fs::remove_file(root.join("Removed.bsl")).expect("removed");
        record_dump_changes(&context, &work, &watch).expect("record");

        let analysis = analyze_context(&context, &work, &mut || false);
        let Ok(AnalysisOutcome::Changes { changes, .. }) = analysis.outcome else {
            panic!("{:?}", analysis.outcome);
        };
        let mut names: Vec<_> = changes
            .iter()
            .map(|change| {
                (
                    change.path.file_name().expect("name").to_owned(),
                    change.kind,
                )
            })
            .collect();
        // Порядок изменений идёт от обхода каталога и не обещан: сверяется набор.
        names.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            names,
            vec![
                ("Ahead.bsl".into(), ChangeKind::Modified),
                ("Mine.bsl".into(), ChangeKind::Modified)
            ]
        );
    }

    /// Удалению по SFTP достаются только файлы элементов, пропавших из описи: удалённый
    /// объект и удалённая форма уходят, посторонний файл в каталоге оставшегося объекта,
    /// неизвестный вид и файлы вне области обхода остаются
    /// (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
    #[test]
    fn only_the_files_of_dropped_elements_are_named() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = root.join("work");
        for file in [
            "Configuration.xml",
            "Catalogs/Items.xml",
            "Catalogs/Items/Stray.txt",
            "Catalogs/Items/Forms/Kept.xml",
            "Catalogs/Items/Forms/Old.xml",
            "Catalogs/Items/Forms/Old/Ext/Form/Module.bsl",
            "Catalogs/Gone.xml",
            "Catalogs/Gone/Ext/ObjectModule.bsl",
            "Catalogs/Gone/.git/keep",
            "Unknowns/Gone.xml",
            "work/Catalogs/Gone.xml",
            "Documents/Заказ.xml",
            "Documents/Заказ/Ext/ObjectModule.bsl",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(&path, file).expect("file");
            File::options()
                .write(true)
                .open(&path)
                .expect("open")
                .set_modified(SystemTime::now() - std::time::Duration::from_secs(3600))
                .expect("old mtime");
        }
        let start = mark_dump_start(&root).expect("dump start");
        // The dump rewrote a file of an element its description no longer names, as a
        // case-only rename does on a case-insensitive file system: written files stay.
        std::fs::write(root.join("Catalogs/Gone/Ext/ObjectModule.bsl"), "written").expect("w");
        let context = SourceSetContext::new("main", root.clone(), "designer-main");
        let names = |names: &[&str]| -> HashSet<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        let before = names(&[
            "Catalog.Items",
            "Catalog.Items.Form.Kept",
            "Catalog.Items.Form.Old",
            "Catalog.Gone",
            "Unknown.Gone",
            "Document.Заказ",
        ]);
        // A case-only rename: the new description spells the element otherwise.
        let after = names(&["Catalog.Items", "Catalog.Items.Form.Kept", "Document.ЗАКАЗ"]);

        let mut dropped: Vec<String> =
            files_of_dropped_elements(&context, &work, start, &before, &after, &|| false)
                .expect("listing")
                .iter()
                .map(|path| {
                    path.strip_prefix(&root)
                        .expect("inside the set")
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect();
        dropped.sort();

        assert_eq!(
            dropped,
            [
                "Catalogs/Gone.xml",
                "Catalogs/Items/Forms/Old.xml",
                "Catalogs/Items/Forms/Old/Ext/Form/Module.bsl",
            ]
        );
        assert!(
            files_of_dropped_elements(&context, &work, start, &HashSet::new(), &after, &|| false)
                .expect("listing")
                .is_empty(),
            "nothing was in the description before"
        );
    }

    /// After a push and after a pull by changes of one file of a large set, the memory is
    /// written by that file's rows alone: no other row is read or written (#576).
    #[test]
    fn a_commit_after_one_changed_file_touches_only_its_rows() {
        use crate::change_detection::hash_storage::row_audit;

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("root");
        for n in 0..100 {
            std::fs::write(root.join(format!("M{n}.bsl")), "before").expect("module");
        }
        let context = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        rescan_and_commit_full(&context, &work).expect("prime");
        let one_file = row_audit::Rows {
            read: 2,
            written: 2,
        };

        std::fs::write(root.join("M7.bsl"), "pushed").expect("edit");
        let Ok(AnalysisOutcome::Changes { prepared, .. }) =
            analyze_context(&context, &work, &mut || false).outcome
        else {
            panic!("the edit is a change");
        };
        row_audit::take();
        super::commit_success(&context, &work, &prepared).expect("push commit");
        assert_eq!(row_audit::take(), one_file, "after push");

        let start = mark_dump_start(&root).expect("dump start");
        let watch = watch_before_dump(&context, &work, start).expect("memory of this base");
        std::fs::write(root.join("M9.bsl"), "from the base").expect("written");
        let listed = super::list_after_dump(&context, &work, &|| false).expect("listing");
        row_audit::take();
        super::observe_dump_writes(&context, &work, &watch, None, listed, None, &|| false)
            .expect("observe")
            .commit(&context, &work)
            .expect("pull commit");
        assert_eq!(row_audit::take(), one_file, "after pull");

        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }

    /// Damage that keeps both tables of equal length — an orphan row in one table next to a
    /// missing row of another key in it — passes the summary a pull by changes reads, and its
    /// delta commit keeps it. The next analysis reads every row and falls back to a full load,
    /// and the full commit after it leaves both tables with the same keys (#571, #576).
    #[test]
    fn equal_length_damage_after_a_delta_commit_is_repaired_by_a_full_load() {
        use crate::change_detection::hash_storage::{HashStorage, FILES_HASH, FILES_MTIME};
        use redb::{Database, ReadableTable};

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("root");
        for name in ["A.bsl", "B.bsl", "C.bsl"] {
            std::fs::write(root.join(name), "before").expect(name);
        }
        let context = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        rescan_and_commit_full(&context, &work).expect("prime");
        let path = context.storage_path(&work).expect("memory path");
        {
            let db = Database::open(&path).expect("open");
            let tx = db.begin_write().expect("write");
            {
                let mut mtime = tx.open_table(FILES_MTIME).expect("mtime");
                mtime.remove("B.bsl").expect("missing row");
                mtime.insert("Orphan.bsl", 1).expect("orphan row");
            }
            tx.commit().expect("damage");
        }
        let storage = HashStorage::new(path.clone());
        assert!(
            storage.load_summary().is_ok(),
            "equal lengths pass the summary"
        );

        let start = mark_dump_start(&root).expect("dump start");
        let watch = watch_before_dump(&context, &work, start).expect("memory of this base");
        std::fs::write(root.join("C.bsl"), "from the base").expect("written");
        record_dump_changes(&context, &work, &watch).expect("delta commit");
        assert!(
            storage
                .load_snapshot()
                .is_err_and(|error| error.is_recoverable()),
            "the delta commit names only its rows and keeps the damage"
        );

        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::Fallback)
        ));
        rescan_and_commit_full(&context, &work).expect("full load commit");

        let keys = |table: &dyn Fn(&redb::ReadTransaction) -> Vec<String>| {
            let db = Database::open(&path).expect("open");
            let tx = db.begin_read().expect("read");
            let mut keys = table(&tx);
            keys.sort();
            keys
        };
        let mtime_keys = keys(&|tx| {
            tx.open_table(FILES_MTIME)
                .expect("mtime")
                .iter()
                .expect("iter")
                .map(|row| row.expect("row").0.value().to_owned())
                .collect()
        });
        let hash_keys = keys(&|tx| {
            tx.open_table(FILES_HASH)
                .expect("hash")
                .iter()
                .expect("iter")
                .map(|row| row.expect("row").0.value().to_owned())
                .collect()
        });
        assert_eq!(mtime_keys, ["A.bsl", "B.bsl", "C.bsl"]);
        assert_eq!(hash_keys, mtime_keys);
        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }

    #[test]
    fn a_set_without_memory_of_the_base_is_not_watched() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        std::fs::create_dir(&root).expect("root");
        std::fs::write(root.join("A.bsl"), "a").expect("A");
        let start = mark_dump_start(&root).expect("dump start");
        let context = SourceSetContext::new("main", root, "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        assert!(watch_before_dump(&context, &dir.path().join("work"), start).is_none());
    }

    /// Cancellation is distinct from a full-load fallback and leaves committed memory alone.
    #[test]
    fn interrupted_analysis_keeps_memory_and_never_falls_back() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("root");
        std::fs::write(root.join("A.bsl"), "before").expect("A");
        std::fs::write(root.join("B.bsl"), "before").expect("B");
        let context = SourceSetContext::new("main", root.clone(), "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        rescan_and_commit_full(&context, &work).expect("prime");
        let storage = crate::change_detection::hash_storage::HashStorage::new(
            context.storage_path(&work).expect("path"),
        );
        let before = storage.load_snapshot().expect("before");
        std::fs::write(root.join("A.bsl"), "edited").expect("edit");
        for cancel_at in [1, 10] {
            let mut checkpoints = 0;
            let analysis = analyze_context(&context, &work, &mut || {
                checkpoints += 1;
                checkpoints >= cancel_at
            });
            assert!(
                matches!(analysis.outcome, Err(ChangeDetectionError::Interrupted)),
                "{:?}",
                analysis.outcome
            );
            let after = storage.load_snapshot().expect("after");
            assert_eq!(before.generation, after.generation);
            assert_eq!(before.watermark, after.watermark);
            assert_eq!(before.identity, after.identity);
            for (path, original) in &before.entries {
                assert_eq!(after.entries[path].hash, original.hash);
                assert_eq!(after.entries[path].mtime_ns, original.mtime_ns);
            }
        }
        let analysis = analyze_context(&context, &work, &mut || false);
        assert!(
            matches!(analysis.outcome, Ok(AnalysisOutcome::Changes { ref changes, .. }) if changes.len() == 1)
        );
    }
    #[test]
    fn cancellation_during_recoverable_storage_read_is_not_fallback() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("root");
        std::fs::write(root.join("Module.bsl"), "source").expect("source");
        let context = SourceSetContext::new("main", root, "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        let path = context.storage_path(&work).expect("storage");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
        std::fs::write(&path, b"corrupt memory").expect("corrupt fixture");
        assert!(matches!(
            analyze_context(&context, &work, &mut || false).outcome,
            Ok(AnalysisOutcome::Fallback)
        ));
        let before = std::fs::read(&path).expect("before");
        let mut checks = 0;
        let analysis = analyze_context(&context, &work, &mut || {
            checks += 1;
            checks >= 2
        });
        assert!(
            matches!(analysis.outcome, Err(ChangeDetectionError::Interrupted)),
            "{:?}",
            analysis.outcome
        );
        assert_eq!(before, std::fs::read(&path).expect("memory unchanged"));
    }
}
