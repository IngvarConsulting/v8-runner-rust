use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::change_detection::hash_storage::{
    HashStorage, SnapshotPublicationError, StorageError, StoredFileState,
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
    let storage = HashStorage::new(context.storage_path(work_path))
        .with_runtime_binding(context.runtime_binding());
    let snapshot = match storage.load_snapshot() {
        Ok(snapshot) => snapshot,
        Err(e) => {
            if e.is_recoverable() {
                tracing::warn!(
                    source_set = %context.name(),
                    error = %e,
                    "recoverable storage problem, switching to fallback mode"
                );
                return ContextAnalysis {
                    context: context.clone(),
                    outcome: Ok(AnalysisOutcome::Fallback),
                };
            }
            return ContextAnalysis {
                context: context.clone(),
                outcome: Err(map_storage_hard(context, storage.path(), e)),
            };
        }
    };

    if snapshot.pending_publication.is_some()
        || snapshot.runtime_binding.as_deref() != context.runtime_binding()
    {
        return ContextAnalysis {
            context: context.clone(),
            outcome: Ok(AnalysisOutcome::Fallback),
        };
    }

    let stored_keys: HashSet<String> = snapshot.entries.keys().cloned().collect();
    let scan = match scanner::scan(context.path(), snapshot.watermark, &stored_keys) {
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
    let storage = HashStorage::new(context.storage_path(work_path))
        .with_runtime_binding(context.runtime_binding());
    let snapshot = to_storage_snapshot(&prepared.snapshot);
    storage
        .commit_snapshot(
            &snapshot,
            prepared.scan_started_at,
            prepared.observed_generation,
        )
        .map_err(|e| map_commit_error(context, storage.path(), e))
}

/// Re-scan the source-set from scratch and replace the stored snapshot.
pub fn rescan_and_commit_full(
    context: &SourceSetContext,
    work_path: &Path,
) -> Result<(), ChangeDetectionError> {
    let storage = HashStorage::new(context.storage_path(work_path))
        .with_runtime_binding(context.runtime_binding());
    let current = match storage.load_snapshot() {
        Ok(snapshot) => snapshot,
        Err(e) if e.is_recoverable() => {
            let full = full_snapshot(context, &StorageSnapshotInputs::empty())?;
            return storage
                .recover_and_commit_snapshot(&full.snapshot, full.scan_started_at)
                .map_err(|err| map_commit_error(context, storage.path(), err));
        }
        Err(e) => return Err(map_storage_hard(context, storage.path(), e)),
    };
    // Only a successful full operation may reconcile a surviving publication intent.
    let storage = storage.with_publication(current.pending_publication.as_deref());
    let current_generation = current.generation;

    let full = full_snapshot(
        context,
        &StorageSnapshotInputs {
            watermark: None,
            stored_keys: HashSet::new(),
            observed_generation: current_generation,
        },
    )?;
    storage
        .commit_snapshot(
            &full.snapshot,
            full.scan_started_at,
            full.observed_generation,
        )
        .map_err(|e| map_commit_error(context, storage.path(), e))
}

/// Prepare exactly the bytes exported to a private stage, never user edits after publication.
pub fn prepare_publication(
    context: &SourceSetContext,
    staging_path: &Path,
    observed_generation: u64,
) -> Result<PreparedStateUpdate, ChangeDetectionError> {
    let scan = scanner::scan(staging_path, None, &HashSet::new())
        .map_err(|error| map_scan_error(context, error))?;
    Ok(build_prepared_state(
        &scan,
        &HashMap::new(),
        observed_generation,
    ))
}

/// Couple a prepared snapshot to publication, preserving the publisher's typed failure.
/// The caller keeps this entire operation in its cancellation-deferred publication phase.
pub fn publish_prepared<T, E>(
    context: &SourceSetContext,
    work_path: &Path,
    prepared: &PreparedStateUpdate,
    token: &str,
    publish: impl FnOnce() -> Result<T, E>,
) -> Result<T, SnapshotPublicationError<E>> {
    let storage = HashStorage::new(context.storage_path(work_path))
        .with_runtime_binding(context.runtime_binding());
    storage
        .begin_publication(prepared.observed_generation, token)
        .map_err(SnapshotPublicationError::Storage)?;
    storage.with_publication(Some(token)).commit_snapshot_with(
        &to_storage_snapshot(&prepared.snapshot),
        prepared.scan_started_at,
        prepared.observed_generation,
        publish,
    )
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

struct StorageSnapshotInputs {
    watermark: Option<u64>,
    stored_keys: HashSet<String>,
    observed_generation: u64,
}

impl StorageSnapshotInputs {
    fn empty() -> Self {
        Self {
            watermark: None,
            stored_keys: HashSet::new(),
            observed_generation: 0,
        }
    }
}

struct FullSnapshot {
    snapshot: HashMap<String, StoredFileState>,
    scan_started_at: u64,
    observed_generation: u64,
}

fn full_snapshot(
    context: &SourceSetContext,
    input: &StorageSnapshotInputs,
) -> Result<FullSnapshot, ChangeDetectionError> {
    let scan = scanner::scan(context.path(), input.watermark, &input.stored_keys)
        .map_err(|e| map_scan_error(context, e))?;
    let mut snapshot = HashMap::new();
    for candidate in scan.candidates {
        snapshot.insert(
            candidate.rel_path,
            StoredFileState {
                mtime_ns: candidate.mtime_ns,
                hash: candidate.hash,
            },
        );
    }
    Ok(FullSnapshot {
        snapshot,
        scan_started_at: scan.scan_started_at,
        observed_generation: input.observed_generation,
    })
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
    use super::{rescan_and_commit_full, ChangeDetectionError, ChangeKind, FileChange};
    use crate::change_detection::partial_load::decide;
    use crate::domain::source_set::SourceSetContext;
    use tempfile::tempdir;

    fn publication_fixture() -> (tempfile::TempDir, SourceSetContext, std::path::PathBuf) {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("source");
        let work = dir.path().join("work");
        std::fs::create_dir(&root).expect("source");
        std::fs::write(root.join("ObjectModule.bsl"), "same bytes").expect("module");
        let source = SourceSetContext::new("main", root, "designer-main")
            .with_runtime_binding("database-A".to_owned());
        rescan_and_commit_full(&source, &work).expect("seed");
        (dir, source, work)
    }

    #[test]
    fn legacy_or_different_binding_requires_full_execution_even_for_identical_sources() {
        let (_dir, source, work) = publication_fixture();
        let other = source.clone().with_runtime_binding("database-B".to_owned());
        assert!(matches!(
            super::analyze_context(&source, &work).outcome,
            Ok(super::AnalysisOutcome::NoChanges)
        ));
        assert!(matches!(
            super::analyze_context(&other, &work).outcome,
            Ok(super::AnalysisOutcome::Fallback)
        ));
        rescan_and_commit_full(&other, &work).expect("successful B load");
        assert!(
            matches!(
                super::analyze_context(&source, &work).outcome,
                Ok(super::AnalysisOutcome::Fallback)
            ),
            "single slot must not revive an old A cache"
        );
        let legacy = SourceSetContext::new("main", source.path().to_path_buf(), "designer-main");
        rescan_and_commit_full(&legacy, &work).expect("legacy snapshot");
        assert!(matches!(
            super::analyze_context(&source, &work).outcome,
            Ok(super::AnalysisOutcome::Fallback)
        ));
    }

    #[test]
    fn published_bytes_without_snapshot_commit_cannot_skip_even_if_old_hashes_match() {
        let (dir, source, work) = publication_fixture();
        let stage = dir.path().join("stage");
        std::fs::create_dir(&stage).expect("stage");
        std::fs::write(stage.join("ObjectModule.bsl"), "same bytes").expect("dump");
        let storage = super::HashStorage::new(source.storage_path(&work));
        let before = storage.load_snapshot().expect("before");
        let prepared =
            super::prepare_publication(&source, &stage, before.generation).expect("prepare");
        let result =
            super::publish_prepared(&source, &work, &prepared, "failed-publication", || {
                std::fs::copy(
                    stage.join("ObjectModule.bsl"),
                    source.path().join("ObjectModule.bsl"),
                )
                .expect("publish");
                Err::<(), _>("failure after filesystem publication")
            });
        assert!(matches!(
            result,
            Err(super::SnapshotPublicationError::Publication(_))
        ));
        let after = storage.load_snapshot().expect("after");
        assert_eq!(after.generation, before.generation);
        assert_eq!(
            after.entries["ObjectModule.bsl"].hash,
            before.entries["ObjectModule.bsl"].hash
        );
        assert_eq!(
            after.pending_publication.as_deref(),
            Some("failed-publication")
        );
        assert!(matches!(
            super::analyze_context(&source, &work).outcome,
            Ok(super::AnalysisOutcome::Fallback)
        ));
        // Only after a successful full load may its rescan recover this state.
        rescan_and_commit_full(&source, &work).expect("recover after full load");
        assert!(storage
            .load_snapshot()
            .expect("recovered")
            .pending_publication
            .is_none());
        assert!(matches!(
            super::analyze_context(&source, &work).outcome,
            Ok(super::AnalysisOutcome::NoChanges)
        ));
    }

    #[test]
    fn generation_race_and_foreign_pending_prevent_publication() {
        let (_dir, source, work) = publication_fixture();
        let storage = super::HashStorage::new(source.storage_path(&work));
        let generation = storage.current_generation().expect("generation");
        let prepared =
            super::prepare_publication(&source, source.path(), generation).expect("prepare");
        rescan_and_commit_full(&source, &work).expect("concurrent commit");
        let result = super::publish_prepared(&source, &work, &prepared, "stale", || {
            panic!("stale generation must be rejected before publication");
            #[allow(unreachable_code)]
            Ok::<(), ()>(())
        });
        assert!(matches!(
            result,
            Err(super::SnapshotPublicationError::Storage(
                super::StorageError::ConcurrentStateModified { .. }
            ))
        ));
        let generation = storage.current_generation().expect("generation");
        storage
            .begin_publication(generation, "owner")
            .expect("pending");
        let prepared =
            super::prepare_publication(&source, source.path(), generation).expect("prepare");
        assert!(
            super::commit_success(&source, &work, &prepared).is_err(),
            "ordinary prepared commits cannot clear someone else's pending intent"
        );
        assert!(storage
            .clone()
            .with_publication(Some("intruder"))
            .commit_snapshot(&std::collections::HashMap::new(), 0, generation)
            .is_err());
        assert_eq!(
            storage
                .load_snapshot()
                .expect("snapshot")
                .pending_publication
                .as_deref(),
            Some("owner")
        );
    }

    #[test]
    fn publication_commits_stage_hashes_and_defers_cancellation_through_commit() {
        let (dir, source, work) = publication_fixture();
        let stage = dir.path().join("stage");
        std::fs::create_dir(&stage).expect("stage");
        std::fs::write(stage.join("ObjectModule.bsl"), "dumped bytes").expect("dump");
        let storage = super::HashStorage::new(source.storage_path(&work));
        let generation = storage.current_generation().expect("generation");
        let prepared = super::prepare_publication(&source, &stage, generation).expect("prepare");
        let cancellation = tokio_util::sync::CancellationToken::new();
        let context = crate::use_cases::context::ExecutionContext::cli(
            crate::use_cases::context::CommandName::Dump,
        )
        .with_cancellation(cancellation.clone());
        let result = context
            .run_no_process_critical_phase(|| {
                super::publish_prepared(&source, &work, &prepared, "owner", || {
                    // A user edit after publication must not be adopted by a target rescan.
                    std::fs::write(source.path().join("ObjectModule.bsl"), "user edit")
                        .expect("edit");
                    cancellation.cancel();
                    Ok::<_, ()>(())
                })
            })
            .expect("commit");
        assert!(result.deferred_interruption.is_some());
        let snapshot = storage.load_snapshot().expect("snapshot");
        assert_eq!(snapshot.generation, generation + 1);
        assert!(snapshot.pending_publication.is_none());
        assert_eq!(snapshot.runtime_binding.as_deref(), Some("database-A"));
        assert!(
            matches!(
                super::analyze_context(&source, &work).outcome,
                Ok(super::AnalysisOutcome::Changes { .. })
            ),
            "post-publication edit must stay dirty"
        );
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
        let decision = decide(
            &changes,
            &root,
            crate::change_detection::partial_load::DEFAULT_PARTIAL_LOAD_THRESHOLD,
        );
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
}
