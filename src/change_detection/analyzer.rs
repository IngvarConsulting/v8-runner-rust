use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::change_detection::file_state::mtime_nanos;
use crate::change_detection::hash_storage::{
    HashStorage, StorageError, StorageSnapshot, StoredFileState,
};
use crate::change_detection::scanner::{self, ScanError, ScanScope};
use crate::domain::source_set::SourceSetContext;

/// A single detected file change.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// How a file changed relative to the stored state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// File state prepared for the next successful storage commit.
#[derive(Debug, Clone)]
pub struct PreparedFileState {
    pub rel_path: String,
    pub mtime_ns: u64,
    pub hash: String,
}

/// Complete storage update payload produced by one analysis pass.
#[derive(Debug, Clone)]
pub struct PreparedStateUpdate {
    pub snapshot: Vec<PreparedFileState>,
    pub scan_started_at: u64,
    pub observed_generation: u64,
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
}

/// Analyze one source-set context and produce either concrete changes or a safe fallback.
pub fn analyze_context(
    context: &SourceSetContext,
    work_path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> ContextAnalysis {
    // All outcomes, including storage and IO fallback, cross the same final
    // interruption check. No early return can turn cancellation into a full load.
    let outcome = if interrupted() {
        Err(ChangeDetectionError::Interrupted)
    } else {
        (|| {
            let snapshot = match context.storage_path(work_path) {
                None => Default::default(),
                Some(path) => match load_bound_snapshot(context, &HashStorage::new(path)) {
                    Ok(Some(snapshot)) => snapshot,
                    Ok(None) => return Ok(AnalysisOutcome::Fallback),
                    Err(error) => return Err(error),
                },
            };
            let excluded = scanner::work_path_inside(context.path(), work_path);
            let scan = match scan_sources(context, context.path(), excluded.as_deref(), interrupted)
            {
                Ok(scan) => scan,
                Err(ScanError::Interrupted) => return Err(ChangeDetectionError::Interrupted),
                Err(error) => {
                    tracing::warn!(source_set = %context.name(), error = %error, "scan failed, switching to fallback mode");
                    return Ok(AnalysisOutcome::Fallback);
                }
            };
            if scan.has_untracked_links {
                return Ok(AnalysisOutcome::Fallback);
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
            Ok(if changes.is_empty() {
                AnalysisOutcome::NoChanges
            } else {
                AnalysisOutcome::Changes {
                    changes,
                    prepared: build_prepared_state(&scan, snapshot.generation),
                }
            })
        })()
    };
    ContextAnalysis {
        context: context.clone(),
        outcome: if interrupted() {
            Err(ChangeDetectionError::Interrupted)
        } else {
            outcome
        },
    }
}

/// Load the snapshot and check that it describes this context's base and sources.
/// `Ok(None)` is a recoverable storage problem: the caller falls back to a full load.
fn load_bound_snapshot(
    context: &SourceSetContext,
    storage: &HashStorage,
) -> Result<Option<StorageSnapshot>, ChangeDetectionError> {
    let snapshot = match storage.load_snapshot() {
        Ok(snapshot) => snapshot,
        Err(e) if e.is_recoverable() => {
            tracing::warn!(
                source_set = %context.name(),
                error = %e,
                "recoverable storage problem, switching to fallback mode"
            );
            return Ok(None);
        }
        Err(e) => return Err(map_storage_hard(context, storage.path(), e)),
    };
    if let Some(selected) = context.storage_identity() {
        if !snapshot.is_blank() && snapshot.identity.as_deref() != Some(selected) {
            return Err(ChangeDetectionError::ForeignMemory {
                source_set: context.name().to_owned(),
                storage_path: storage.path().to_path_buf(),
                recorded: snapshot
                    .identity
                    .unwrap_or_else(|| "an unidentified infobase".to_owned()),
                selected: selected.to_owned(),
            });
        }
    }
    Ok(Some(snapshot))
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
    /// Хранилище не открывается: ответ даст анализ изменений своим отказом.
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
            scan_started_at: 0,
        },
    )
}

/// Analyze multiple source-set contexts using the same work directory.
pub fn analyze_contexts(
    contexts: &[SourceSetContext],
    work_path: &Path,
    interrupted: &mut dyn FnMut() -> bool,
) -> Vec<ContextAnalysis> {
    contexts
        .iter()
        .map(|ctx| analyze_context(ctx, work_path, interrupted))
        .collect()
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
    let snapshot = to_storage_snapshot(&prepared.snapshot);
    storage
        .commit_snapshot_with_identity(
            &snapshot,
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

/// The source tree as an incremental dump found it: which files it held and when the dump
/// began. Taken before the platform starts writing into the tree.
pub struct DumpWatch {
    started_ns: u64,
    /// Every listed file with its modification time.
    before: HashMap<String, u64>,
}

/// Lists the tree before an incremental dump, without reading files. `None` when the set keeps
/// no memory of this base or the tree cannot be listed: the dump then records nothing and the
/// next push loads what the dump wrote, as before (#557).
pub fn watch_before_dump(context: &SourceSetContext, work_path: &Path) -> Option<DumpWatch> {
    if !context.persists_snapshot() || snapshot_memory(context, work_path) != SnapshotMemory::Own {
        return None;
    }
    let watch = list_before_dump(context, work_path);
    if let Err(error) = &watch {
        tracing::debug!(
            source_set = %context.name(),
            %error,
            "the tree was not listed before the dump; what the dump writes is not remembered"
        );
    }
    watch.ok()
}

fn list_before_dump(context: &SourceSetContext, work_path: &Path) -> Result<DumpWatch, String> {
    // The dump's files are stamped by the clock of the tree's file system, which can be coarser
    // than this process's clock or, on a share, apart from it: the start is read from a file
    // written there, and the file is gone before the tree is listed.
    let started_ns = {
        let probe = tempfile::Builder::new()
            .prefix(scanner::DUMP_START_PROBE)
            .tempfile_in(context.path())
            .map_err(|error| format!("clock probe: {error}"))?;
        let modified = probe
            .as_file()
            .metadata()
            .and_then(|meta| meta.modified())
            .map_err(|error| format!("clock probe: {error}"))?;
        mtime_nanos(modified, context.path()).map_err(|error| error.to_string())?
    };
    let excluded = scanner::work_path_inside(context.path(), work_path);
    let listed = scanner::list_in_scope(
        context.path(),
        excluded.as_deref(),
        &mut || false,
        scan_scope(context),
    )
    .map_err(|error| error.to_string())?;
    Ok(DumpWatch {
        started_ns,
        before: listed
            .files
            .into_iter()
            .map(|file| (file.rel_path, file.mtime_ns))
            .collect(),
    })
}

/// What an incremental dump wrote into the tree, found and hashed but not yet remembered.
///
/// Finding it reads only files and the memory, so it can run while the platform answers
/// whether the base changed during the dump; [`Self::commit`] is the step that waits for
/// that answer (#571).
pub struct DumpWrites {
    /// `None`: the memory is unreadable and is not repaired here.
    snapshot: Option<StorageSnapshot>,
    started_ns: u64,
}

/// After a successful incremental dump, finds what it wrote: a file is written by the dump
/// when its modification time is not before the dump began and it is new or its time moved
/// since the listing — so an edit saved in the very clock tick the dump began stays a change.
/// Written files are hashed by content and replace their entries, files that were there before
/// the dump and are gone are forgotten. Every other entry stays, so a user edit made before
/// the dump remains a change. An edit made while the dump ran cannot be told from the
/// platform's writes and is remembered too (#170).
pub fn observe_dump_writes(
    context: &SourceSetContext,
    work_path: &Path,
    watch: &DumpWatch,
) -> Result<DumpWrites, ChangeDetectionError> {
    let unchanged = |snapshot| DumpWrites {
        snapshot,
        started_ns: watch.started_ns,
    };
    let Some(path) = context.storage_path(work_path) else {
        return Ok(unchanged(None));
    };
    let storage = HashStorage::new(path);
    // Unreadable memory is not repaired here: the next push falls back to a full load.
    let Some(mut snapshot) = load_bound_snapshot(context, &storage)? else {
        return Ok(unchanged(None));
    };
    let excluded = scanner::work_path_inside(context.path(), work_path);
    let listed = scanner::list_in_scope(
        context.path(),
        excluded.as_deref(),
        &mut || false,
        scan_scope(context),
    )
    .map_err(|error| map_scan_error(context, error))?;
    let after: HashSet<&str> = listed
        .files
        .iter()
        .map(|file| file.rel_path.as_str())
        .collect();
    let removed: Vec<&str> = watch
        .before
        .keys()
        .map(String::as_str)
        .filter(|rel_path| !after.contains(rel_path))
        .collect();
    let written: Vec<scanner::Candidate> = listed
        .files
        .into_iter()
        .filter(|file| {
            file.mtime_ns >= watch.started_ns
                && watch.before.get(&file.rel_path) != Some(&file.mtime_ns)
        })
        .collect();
    if written.is_empty() && removed.is_empty() {
        return Ok(unchanged(None));
    }
    for rel_path in removed {
        snapshot.entries.remove(rel_path);
    }
    for file in scanner::hash_listed(written, &mut || false)
        .map_err(|error| map_scan_error(context, error))?
    {
        snapshot.entries.insert(
            file.rel_path,
            StoredFileState {
                mtime_ns: file.mtime_ns,
                hash: file.hash,
            },
        );
    }
    Ok(unchanged(Some(snapshot)))
}

impl DumpWrites {
    /// Remembers what the dump wrote. The commit fails if the memory moved since it was read.
    pub fn commit(
        self,
        context: &SourceSetContext,
        work_path: &Path,
    ) -> Result<(), ChangeDetectionError> {
        let (Some(snapshot), Some(path)) = (self.snapshot, context.storage_path(work_path)) else {
            return Ok(());
        };
        let storage = HashStorage::new(path);
        storage
            .commit_snapshot_with_identity(
                &snapshot.entries,
                snapshot.watermark.unwrap_or(0).max(self.started_ns),
                snapshot.generation,
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
    observe_dump_writes(context, work_path, watch)?.commit(context, work_path)
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

fn build_prepared_state(
    scan: &scanner::ScanSnapshot,
    observed_generation: u64,
) -> PreparedStateUpdate {
    PreparedStateUpdate {
        snapshot: scan
            .files
            .iter()
            .map(|file| PreparedFileState {
                rel_path: file.rel_path.clone(),
                mtime_ns: file.mtime_ns,
                hash: file.hash.clone(),
            })
            .collect(),
        scan_started_at: scan.scan_started_at,
        observed_generation,
    }
}

pub struct FullSnapshot {
    snapshot: HashMap<String, StoredFileState>,
    scan_started_at: u64,
}

fn to_storage_snapshot(snapshot: &[PreparedFileState]) -> HashMap<String, StoredFileState> {
    snapshot
        .iter()
        .map(|entry| {
            (
                entry.rel_path.clone(),
                StoredFileState {
                    mtime_ns: entry.mtime_ns,
                    hash: entry.hash.clone(),
                },
            )
        })
        .collect()
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
        analyze_context, record_dump_changes, rescan_and_commit_full, watch_before_dump,
        AnalysisOutcome, ChangeDetectionError, ChangeKind, FileChange,
    };
    use crate::change_detection::partial_load::decide;
    use crate::domain::source_set::SourceSetContext;
    use std::fs::File;
    use std::time::SystemTime;
    use tempfile::tempdir;

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
        let watch = watch_before_dump(&context, &work).expect("memory of this base");

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
                    change.kind.clone(),
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

    #[test]
    fn a_set_without_memory_of_the_base_is_not_watched() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("src");
        std::fs::create_dir(&root).expect("root");
        std::fs::write(root.join("A.bsl"), "a").expect("A");
        let context = SourceSetContext::new("main", root, "designer-main")
            .with_infobase_memory("origin", "safe-base-identity".to_owned());
        assert!(watch_before_dump(&context, &dir.path().join("work")).is_none());
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
