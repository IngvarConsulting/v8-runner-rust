use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::change_detection::hash_storage::{
    HashStorage, HashStorageLoad, StorageError, StoredFileState,
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
    /// No scoped state exists yet; callers must perform a full bootstrap.
    Bootstrap,
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
pub fn analyze_context(context: &SourceSetContext) -> ContextAnalysis {
    let storage = HashStorage::new(context.storage_path());
    let snapshot = match storage.load_state() {
        Ok(HashStorageLoad::Missing) => {
            return ContextAnalysis {
                context: context.clone(),
                outcome: Ok(AnalysisOutcome::Bootstrap),
            }
        }
        Ok(HashStorageLoad::Initialized(snapshot)) => snapshot,
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
pub fn analyze_contexts(contexts: &[SourceSetContext]) -> Vec<ContextAnalysis> {
    contexts.iter().map(analyze_context).collect()
}

/// Persist a prepared snapshot after the corresponding build/load step succeeded.
pub fn commit_success(
    context: &SourceSetContext,
    prepared: &PreparedStateUpdate,
) -> Result<(), ChangeDetectionError> {
    let storage = HashStorage::new(context.storage_path());
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
pub fn rescan_and_commit_full(context: &SourceSetContext) -> Result<(), ChangeDetectionError> {
    let storage = HashStorage::new(context.storage_path());
    let current_generation = match storage.current_generation() {
        Ok(generation) => generation,
        Err(e) if e.is_recoverable() => {
            let full = full_snapshot(context, &StorageSnapshotInputs::empty())?;
            return storage
                .recover_and_commit_snapshot(&full.snapshot, full.scan_started_at)
                .map_err(|err| map_commit_error(context, storage.path(), err));
        }
        Err(e) => return Err(map_storage_hard(context, storage.path(), e)),
    };

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
    use super::{
        analyze_context, rescan_and_commit_full, AnalysisOutcome, ChangeDetectionError, ChangeKind,
        FileChange,
    };
    use crate::change_detection::hash_storage::{META, META_KEY_GENERATION};
    use crate::change_detection::partial_load::decide;
    use crate::config::model::{BuilderBackend, InfobaseConfig, SourceFormat, SourceSetPurpose};
    use crate::domain::runtime_state::{
        InfobaseIdentity, LogicalSourceRole, RuntimeSourceDescriptor, RuntimeSourceIdentityInputs,
        RuntimeStateLayout,
    };
    use crate::domain::source_set::SourceSetContext;
    use std::path::Path;
    use tempfile::tempdir;

    fn test_context(state_root: &Path, source_root: &Path) -> SourceSetContext {
        let identity = InfobaseIdentity::normalize(&InfobaseConfig::file(format!(
            "File={}",
            state_root.join("ib").display()
        )))
        .expect("identity");
        let layout = RuntimeStateLayout::new(state_root.join("work"), identity).expect("layout");
        let descriptor = RuntimeSourceDescriptor::new(RuntimeSourceIdentityInputs {
            configured_source_identity: Path::new("src"),
            source_root,
            purpose: SourceSetPurpose::Configuration,
            format: SourceFormat::Designer,
            backend: BuilderBackend::Designer,
            logical_role: LogicalSourceRole::DesignerSource,
        })
        .expect("descriptor");
        SourceSetContext::new(
            "main",
            source_root.to_path_buf(),
            layout.source_state("main", &descriptor),
        )
    }

    #[test]
    fn missing_scoped_storage_requires_bootstrap_even_for_empty_source() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        std::fs::create_dir(&source_root).expect("source");
        let context = test_context(dir.path(), &source_root);

        let analysis = analyze_context(&context);

        assert!(matches!(analysis.outcome, Ok(AnalysisOutcome::Bootstrap)));
        assert!(!context.storage_path().exists());
    }

    #[test]
    fn existing_empty_scoped_database_requires_bootstrap() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        std::fs::create_dir(&source_root).expect("source");
        let context = test_context(dir.path(), &source_root);
        std::fs::create_dir_all(context.storage_path().parent().expect("storage parent"))
            .expect("storage parent");
        redb::Database::create(context.storage_path()).expect("empty database");

        assert!(matches!(
            analyze_context(&context).outcome,
            Ok(AnalysisOutcome::Bootstrap)
        ));
    }

    #[test]
    fn metadata_only_storage_falls_back_and_can_be_recovered() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        std::fs::create_dir(&source_root).expect("source");
        std::fs::write(source_root.join("Configuration.xml"), "<xml />").expect("config");
        let context = test_context(dir.path(), &source_root);
        std::fs::create_dir_all(context.storage_path().parent().expect("storage parent"))
            .expect("storage parent");
        let database = redb::Database::create(context.storage_path()).expect("database");
        let write = database.begin_write().expect("write");
        {
            let mut meta = write.open_table(META).expect("meta");
            meta.insert(META_KEY_GENERATION, 1).expect("generation");
        }
        write.commit().expect("commit");
        drop(database);

        assert!(matches!(
            analyze_context(&context).outcome,
            Ok(AnalysisOutcome::Fallback)
        ));
        rescan_and_commit_full(&context).expect("recover storage");
        assert!(matches!(
            analyze_context(&context).outcome,
            Ok(AnalysisOutcome::NoChanges)
        ));
    }

    #[test]
    fn directory_at_scoped_storage_path_is_a_hard_error_not_bootstrap() {
        let dir = tempdir().expect("tempdir");
        let source_root = dir.path().join("src");
        std::fs::create_dir(&source_root).expect("source");
        let context = test_context(dir.path(), &source_root);
        std::fs::create_dir_all(context.storage_path()).expect("invalid storage directory");

        let analysis = analyze_context(&context);

        assert!(matches!(
            analysis.outcome,
            Err(ChangeDetectionError::StorageHard { .. })
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
        std::fs::create_dir_all(&source_root).expect("source");
        std::fs::write(source_root.join("Configuration.xml"), "<xml />").expect("config");

        let context = test_context(dir.path(), &source_root);
        let storage_path = context.storage_path();
        std::fs::create_dir_all(&storage_path).expect("storage dir");
        let error = rescan_and_commit_full(&context).expect_err("expected hard error");

        assert!(matches!(error, ChangeDetectionError::StorageHard { .. }));
    }
}
