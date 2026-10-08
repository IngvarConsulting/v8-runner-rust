use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::change_detection::hash_storage::{
    HashStorage, StorageError, StorageSnapshot, StoredFileState,
};
use crate::change_detection::scanner::{self, ScanError};
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
pub fn analyze_context(context: &SourceSetContext, work_path: &Path) -> ContextAnalysis {
    let snapshot = match context.storage_path(work_path) {
        None => Default::default(),
        Some(path) => match load_bound_snapshot(context, &HashStorage::new(path)) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                return ContextAnalysis {
                    context: context.clone(),
                    outcome: Ok(AnalysisOutcome::Fallback),
                }
            }
            Err(error) => {
                return ContextAnalysis {
                    context: context.clone(),
                    outcome: Err(error),
                }
            }
        },
    };

    let stored_mtimes: HashMap<String, u64> = snapshot
        .entries
        .iter()
        .map(|(rel, state)| (rel.clone(), state.mtime_ns))
        .collect();
    let scan = match scanner::scan(context.path(), snapshot.watermark, &stored_mtimes) {
        Ok(scan) => scan,
        Err(e) => {
            tracing::warn!(
                source_set = %context.name(),
                error = %e,
                "scan failed, switching to fallback mode"
            );
            return ContextAnalysis {
                context: context.clone(),
                outcome: Ok(AnalysisOutcome::Fallback),
            };
        }
    };

    let mut changes = detect_changes(&scan.candidates, &snapshot.entries);
    let seen_rel: HashSet<&str> = scan
        .seen_files
        .iter()
        .map(|f| f.rel_path.as_str())
        .collect();
    changes.extend(
        snapshot
            .entries
            .iter()
            .filter(|(rel, _)| !seen_rel.contains(rel.as_str()))
            .map(|(rel, _)| FileChange {
                path: context.path().join(rel),
                kind: ChangeKind::Deleted,
            }),
    );

    let prepared = build_prepared_state(&scan, &snapshot.entries, snapshot.generation);
    let outcome = if changes.is_empty() {
        AnalysisOutcome::NoChanges
    } else {
        AnalysisOutcome::Changes { changes, prepared }
    };

    ContextAnalysis {
        context: context.clone(),
        outcome: Ok(outcome),
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
    match load_bound_snapshot(context, &HashStorage::new(path)) {
        Ok(Some(snapshot)) if !snapshot.is_blank() => SnapshotMemory::Own,
        Ok(_) => SnapshotMemory::Nothing,
        Err(ChangeDetectionError::ForeignMemory { .. }) => SnapshotMemory::Foreign,
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
pub fn analyze_contexts(contexts: &[SourceSetContext], work_path: &Path) -> Vec<ContextAnalysis> {
    contexts
        .iter()
        .map(|ctx| analyze_context(ctx, work_path))
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
    let prepared = prepare_full_snapshot(context, context.path())?;
    commit_full_snapshot(context, work_path, &prepared)
}

/// Hash an exported tree before publishing it; does not open or mutate storage.
pub fn prepare_full_snapshot(
    context: &SourceSetContext,
    source_path: &Path,
) -> Result<FullSnapshot, ChangeDetectionError> {
    let scan = scanner::scan(source_path, None, &HashMap::new())
        .map_err(|error| map_scan_error(context, error))?;
    Ok(FullSnapshot {
        snapshot: scan
            .candidates
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

fn detect_changes(
    candidates: &[scanner::CandidateFile],
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
    stored: &HashMap<String, StoredFileState>,
    observed_generation: u64,
) -> PreparedStateUpdate {
    let seen_rel: HashSet<&str> = scan
        .seen_files
        .iter()
        .map(|f| f.rel_path.as_str())
        .collect();
    let candidate_map: HashMap<&str, &scanner::CandidateFile> = scan
        .candidates
        .iter()
        .map(|candidate| (candidate.rel_path.as_str(), candidate))
        .collect();

    let mut merged = HashMap::<String, StoredFileState>::new();
    for file in &scan.seen_files {
        let state = if let Some(candidate) = candidate_map.get(file.rel_path.as_str()) {
            StoredFileState {
                mtime_ns: candidate.mtime_ns,
                hash: candidate.hash.clone(),
            }
        } else {
            stored
                .get(&file.rel_path)
                .cloned()
                .unwrap_or_else(|| StoredFileState {
                    mtime_ns: file.mtime_ns,
                    hash: String::new(),
                })
        };
        merged.insert(file.rel_path.clone(), state);
    }

    // Drop deleted entries.
    for rel in stored.keys() {
        if !seen_rel.contains(rel.as_str()) {
            merged.remove(rel);
        }
    }
    // Remove invalid placeholders introduced by missing stored state.
    merged.retain(|_, state| !state.hash.is_empty());

    PreparedStateUpdate {
        snapshot: merged
            .into_iter()
            .map(|(rel_path, state)| PreparedFileState {
                rel_path,
                mtime_ns: state.mtime_ns,
                hash: state.hash,
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
    ChangeDetectionError::StorageHard {
        source_set: context.name().to_owned(),
        storage_path: context.path().to_path_buf(),
        reason: format!("scan failed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_context, rescan_and_commit_full, AnalysisOutcome, ChangeDetectionError, ChangeKind,
        FileChange,
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

        let analysis = analyze_context(&context, &work_path);

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

        let analysis = analyze_context(&context, &work_path);

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
        let outcome = analyze_context(&context, &work).outcome.expect("analysis");
        assert!(
            matches!(outcome, AnalysisOutcome::Changes { ref changes, .. } if changes.len() == 1 && changes[0].kind == ChangeKind::Modified)
        );
    }
}
