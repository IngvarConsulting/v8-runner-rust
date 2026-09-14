use redb::{
    Database, DatabaseError, ReadableTable, StorageError as RedbStorageError, TableDefinition,
    TableError, TransactionError,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// `redb` table with per-file mtimes keyed by relative path.
pub const FILES_MTIME: TableDefinition<&str, u64> = TableDefinition::new("files_mtime");
/// `redb` table with per-file hashes keyed by relative path.
pub const FILES_HASH: TableDefinition<&str, &str> = TableDefinition::new("files_hash");
/// `redb` table with storage metadata.
pub const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
/// Metadata key storing the latest scan watermark.
pub const META_KEY_WATERMARK: &str = "watermark";
/// Metadata key storing optimistic-lock generation.
pub const META_KEY_GENERATION: &str = "generation";
const RUNTIME: TableDefinition<&str, &str> = TableDefinition::new("runtime");
const RUNTIME_BINDING: &str = "binding";
const PENDING_PUBLICATION: &str = "pending_publication";

/// Keep filesystem publication errors distinct from storage failures.
#[derive(Debug)]
pub enum SnapshotPublicationError<E> {
    Storage(StorageError),
    Publication(E),
}

/// Persisted state for one file entry inside the storage snapshot.
#[derive(Debug, Clone)]
pub struct StoredFileState {
    pub mtime_ns: u64,
    pub hash: String,
}

/// Full snapshot loaded from storage, including metadata used by the scanner.
#[derive(Debug, Clone, Default)]
pub struct StorageSnapshot {
    pub entries: HashMap<String, StoredFileState>,
    pub watermark: Option<u64>,
    pub generation: u64,
    pub runtime_binding: Option<String>,
    pub pending_publication: Option<String>,
}

/// Storage-layer failures split into recoverable and hard categories.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("recoverable storage problem for '{path}': {reason}")]
    Recoverable { path: PathBuf, reason: String },

    #[error("hard storage problem for '{path}': {reason}")]
    Hard { path: PathBuf, reason: String },

    #[error("storage state changed concurrently for '{path}': expected generation {expected}, found {actual}")]
    ConcurrentStateModified {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
}

impl StorageError {
    /// Whether the caller may ignore the storage state and rebuild it from disk.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Recoverable { .. })
    }
}

/// `redb`-backed hash storage for one logical source-set.
#[derive(Debug, Clone)]
pub struct HashStorage {
    path: PathBuf,
    runtime_binding: Option<String>,
    expected_publication: Option<String>,
}

impl HashStorage {
    /// Create a storage handle for the given `redb` file path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            runtime_binding: None,
            expected_publication: None,
        }
    }

    pub fn with_runtime_binding(mut self, binding: Option<&str>) -> Self {
        self.runtime_binding = binding.map(str::to_owned);
        self
    }

    pub fn with_publication(mut self, token: Option<&str>) -> Self {
        self.expected_publication = token.map(str::to_owned);
        self
    }

    /// Return the underlying storage file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the stored state should be discarded and rebuilt from disk.
    ///
    /// One owner for a question two use cases used to answer with two copies of the same
    /// substring search over an error message. Recoverability is a property of the error, and
    /// the error knows it.
    pub fn needs_recovery(&self) -> bool {
        match self.current_generation() {
            Err(error) => error.is_recoverable(),
            Ok(_) => false,
        }
    }

    /// Load the current snapshot from storage.
    pub fn load_snapshot(&self) -> Result<StorageSnapshot, StorageError> {
        if !self.path.exists() {
            return Ok(StorageSnapshot::default());
        }
        let db = Database::open(&self.path).map_err(|e| map_database_error(&self.path, e))?;
        let tx = db
            .begin_read()
            .map_err(|e| map_tx_error(&self.path, e, "begin read"))?;

        let mtime_tbl = match tx.open_table(FILES_MTIME) {
            Ok(t) => Some(t),
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(e) => return Err(map_table_error(&self.path, e)),
        };
        let hash_tbl = match tx.open_table(FILES_HASH) {
            Ok(t) => Some(t),
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(e) => return Err(map_table_error(&self.path, e)),
        };
        let meta_tbl = match tx.open_table(META) {
            Ok(t) => Some(t),
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(e) => return Err(map_table_error(&self.path, e)),
        };

        let runtime = match tx.open_table(RUNTIME) {
            Ok(table) => Some(table),
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(map_table_error(&self.path, error)),
        };
        let runtime_value = |key| -> Result<Option<String>, StorageError> {
            match &runtime {
                Some(table) => Ok(table
                    .get(key)
                    .map_err(|error| map_storage_error(&self.path, "read runtime binding", error))?
                    .map(|value| value.value().to_owned())),
                None => Ok(None),
            }
        };
        let runtime_binding = runtime_value(RUNTIME_BINDING)?;
        let pending_publication = runtime_value(PENDING_PUBLICATION)?;
        let mtime_exists = mtime_tbl.is_some();
        let hash_exists = hash_tbl.is_some();
        if !mtime_exists || !hash_exists {
            if !mtime_exists && !hash_exists {
                return Ok(StorageSnapshot {
                    entries: HashMap::new(),
                    watermark: read_watermark(meta_tbl.as_ref(), &self.path)?,
                    generation: read_generation(meta_tbl.as_ref(), &self.path)?,
                    runtime_binding,
                    pending_publication,
                });
            }
            return Err(StorageError::Recoverable {
                path: self.path.clone(),
                reason: "mtime/hash tables are inconsistent".to_owned(),
            });
        }
        let (Some(mtime_tbl), Some(hash_tbl)) = (mtime_tbl, hash_tbl) else {
            unreachable!("table presence checked above");
        };

        let mut entries = HashMap::new();
        for item in mtime_tbl
            .iter()
            .map_err(|e| map_storage_error(&self.path, "iterate mtime table", e))?
        {
            let (k, mtime) =
                item.map_err(|e| map_storage_error(&self.path, "read mtime row", e))?;
            let rel = k.value().to_owned();
            let hash = hash_tbl
                .get(rel.as_str())
                .map_err(|e| map_storage_error(&self.path, "read hash row", e))?
                .map(|v| v.value().to_owned())
                .ok_or_else(|| StorageError::Recoverable {
                    path: self.path.clone(),
                    reason: format!("missing hash for key '{rel}'"),
                })?;
            entries.insert(
                rel,
                StoredFileState {
                    mtime_ns: mtime.value(),
                    hash,
                },
            );
        }

        // Detect hash-only orphan rows.
        for item in hash_tbl
            .iter()
            .map_err(|e| map_storage_error(&self.path, "iterate hash table", e))?
        {
            let (k, _) = item.map_err(|e| map_storage_error(&self.path, "read hash row", e))?;
            let rel = k.value().to_owned();
            if !entries.contains_key(&rel) {
                return Err(StorageError::Recoverable {
                    path: self.path.clone(),
                    reason: format!("missing mtime for key '{rel}'"),
                });
            }
        }

        Ok(StorageSnapshot {
            entries,
            watermark: read_watermark(meta_tbl.as_ref(), &self.path)?,
            generation: read_generation(meta_tbl.as_ref(), &self.path)?,
            runtime_binding,
            pending_publication,
        })
    }

    /// Persist a full snapshot if the caller still owns the expected generation.
    pub fn commit_snapshot(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
        expected_generation: u64,
    ) -> Result<(), StorageError> {
        match self.commit_snapshot_with(snapshot, watermark, expected_generation, || {
            Ok::<_, std::convert::Infallible>(())
        }) {
            Ok(()) => Ok(()),
            Err(SnapshotPublicationError::Storage(error)) => Err(error),
            Err(SnapshotPublicationError::Publication(never)) => match never {},
        }
    }

    /// Persist an intent before filesystem publication. Entries and generation stay unchanged.
    /// A surviving intent makes the analyzer refuse NoChanges, including after a process crash.
    pub fn begin_publication(
        &self,
        expected_generation: u64,
        token: &str,
    ) -> Result<(), StorageError> {
        self.ensure_parent_dir()?;
        let db = Database::create(&self.path).map_err(|e| map_database_error(&self.path, e))?;
        let mut tx = db
            .begin_write()
            .map_err(|e| map_tx_error(&self.path, e, "begin publication"))?;
        tx.set_durability(redb::Durability::Immediate);
        {
            let meta = tx
                .open_table(META)
                .map_err(|e| map_table_error(&self.path, e))?;
            self.check_generation(&meta, expected_generation)?;
            let mut runtime = tx
                .open_table(RUNTIME)
                .map_err(|e| map_table_error(&self.path, e))?;
            // A previous interrupted publication is deliberately not silently replaced.
            self.check_publication(&runtime)?;
            runtime
                .insert(PENDING_PUBLICATION, token)
                .map_err(|e| map_storage_error(&self.path, "record pending publication", e))?;
        }
        tx.commit()
            .map_err(|e| map_storage_error(&self.path, "commit publication intent", e))
    }

    /// Reintroduction guard: one transaction owns binding, hashes, generation and pending intent.
    /// Validate the generation and intent before invoking the publisher; commit only afterwards.
    /// If publication or commit fails, the independently committed intent survives.
    pub fn commit_snapshot_with<T, E>(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
        expected_generation: u64,
        publish: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, SnapshotPublicationError<E>> {
        use SnapshotPublicationError::{Publication, Storage};
        self.ensure_parent_dir().map_err(Storage)?;
        let db =
            Database::create(&self.path).map_err(|e| Storage(map_database_error(&self.path, e)))?;
        let tx = db
            .begin_write()
            .map_err(|e| Storage(map_tx_error(&self.path, e, "begin write")))?;
        {
            let mut meta = tx
                .open_table(META)
                .map_err(|e| Storage(map_table_error(&self.path, e)))?;
            self.check_generation(&meta, expected_generation)
                .map_err(Storage)?;
            let mut runtime = tx
                .open_table(RUNTIME)
                .map_err(|e| Storage(map_table_error(&self.path, e)))?;
            self.check_publication(&runtime).map_err(Storage)?;
            match &self.runtime_binding {
                Some(binding) => {
                    runtime
                        .insert(RUNTIME_BINDING, binding.as_str())
                        .map_err(|e| {
                            Storage(map_storage_error(&self.path, "write runtime binding", e))
                        })?;
                }
                None => {
                    runtime.remove(RUNTIME_BINDING).map_err(|e| {
                        Storage(map_storage_error(&self.path, "clear runtime binding", e))
                    })?;
                }
            }
            runtime.remove(PENDING_PUBLICATION).map_err(|e| {
                Storage(map_storage_error(&self.path, "clear publication intent", e))
            })?;
            let mut mtime = tx
                .open_table(FILES_MTIME)
                .map_err(|e| Storage(map_table_error(&self.path, e)))?;
            let mut hash = tx
                .open_table(FILES_HASH)
                .map_err(|e| Storage(map_table_error(&self.path, e)))?;
            sync_file_tables(&self.path, &mut mtime, &mut hash, snapshot).map_err(Storage)?;
            meta.insert(META_KEY_WATERMARK, watermark)
                .map_err(|e| Storage(map_storage_error(&self.path, "write watermark", e)))?;
            meta.insert(META_KEY_GENERATION, expected_generation + 1)
                .map_err(|e| Storage(map_storage_error(&self.path, "write generation", e)))?;
        }
        let published = publish().map_err(Publication)?;
        tx.commit()
            .map_err(|e| Storage(map_storage_error(&self.path, "commit transaction", e)))?;
        Ok(published)
    }

    fn check_generation(
        &self,
        meta: &redb::Table<&str, u64>,
        expected: u64,
    ) -> Result<(), StorageError> {
        let actual = meta
            .get(META_KEY_GENERATION)
            .map_err(|e| map_storage_error(&self.path, "read generation", e))?
            .map(|v| v.value())
            .unwrap_or(0);
        if actual != expected {
            return Err(StorageError::ConcurrentStateModified {
                path: self.path.clone(),
                expected,
                actual,
            });
        }
        Ok(())
    }

    fn check_publication(&self, runtime: &redb::Table<&str, &str>) -> Result<(), StorageError> {
        let pending = runtime
            .get(PENDING_PUBLICATION)
            .map_err(|e| map_storage_error(&self.path, "read pending publication", e))?;
        if pending.as_ref().map(|value| value.value()) != self.expected_publication.as_deref() {
            return Err(StorageError::Hard {
                path: self.path.clone(),
                reason:
                    "pending publication ownership changed; a successful full rebuild is required"
                        .to_owned(),
            });
        }
        Ok(())
    }

    /// Read the current optimistic-lock generation.
    pub fn current_generation(&self) -> Result<u64, StorageError> {
        Ok(self.load_snapshot()?.generation)
    }

    /// Replace a corrupt or missing storage file with a fresh snapshot.
    pub fn recover_and_commit_snapshot(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
    ) -> Result<(), StorageError> {
        if self.path.exists() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backup = self.path.with_extension(format!("corrupt.{ts}.redb"));
            std::fs::rename(&self.path, &backup).map_err(|e| StorageError::Hard {
                path: self.path.clone(),
                reason: format!("failed to rename corrupt db to '{}': {e}", backup.display()),
            })?;
        }
        self.commit_snapshot(snapshot, watermark, 0)
    }

    fn ensure_parent_dir(&self) -> Result<(), StorageError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::Hard {
                path: parent.to_path_buf(),
                reason: format!("failed to create parent dir: {e}"),
            })?;
        }
        Ok(())
    }
}

fn read_watermark(
    meta: Option<&redb::ReadOnlyTable<&str, u64>>,
    path: &Path,
) -> Result<Option<u64>, StorageError> {
    let Some(meta) = meta else {
        return Ok(None);
    };
    meta.get(META_KEY_WATERMARK)
        .map_err(|e| StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("read watermark: {e}"),
        })
        .map(|opt| opt.map(|v| v.value()))
}

fn read_generation(
    meta: Option<&redb::ReadOnlyTable<&str, u64>>,
    path: &Path,
) -> Result<u64, StorageError> {
    let Some(meta) = meta else {
        return Ok(0);
    };
    meta.get(META_KEY_GENERATION)
        .map_err(|e| StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("read generation: {e}"),
        })
        .map(|opt| opt.map(|v| v.value()).unwrap_or(0))
}

fn sync_file_tables(
    path: &Path,
    mtime: &mut redb::Table<&str, u64>,
    hash: &mut redb::Table<&str, &str>,
    snapshot: &HashMap<String, StoredFileState>,
) -> Result<(), StorageError> {
    let keys: Vec<String> = mtime
        .iter()
        .map_err(|e| map_storage_error(path, "iterate mtime table", e))?
        .map(|row| {
            row.map(|(k, _)| k.value().to_owned())
                .map_err(|e| map_storage_error(path, "read mtime row", e))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let target_keys: HashSet<&str> = snapshot.keys().map(String::as_str).collect();
    for key in keys {
        if !target_keys.contains(key.as_str()) {
            mtime
                .remove(key.as_str())
                .map_err(|e| map_storage_error(path, "remove stale mtime key", e))?;
            hash.remove(key.as_str())
                .map_err(|e| map_storage_error(path, "remove stale hash key", e))?;
        }
    }

    for (key, state) in snapshot {
        mtime
            .insert(key.as_str(), state.mtime_ns)
            .map_err(|e| map_storage_error(path, "insert mtime", e))?;
        hash.insert(key.as_str(), state.hash.as_str())
            .map_err(|e| map_storage_error(path, "insert hash", e))?;
    }
    Ok(())
}

fn map_database_error(path: &Path, err: DatabaseError) -> StorageError {
    match err {
        DatabaseError::Storage(RedbStorageError::Corrupted(msg)) => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: msg,
        },
        DatabaseError::Storage(RedbStorageError::PreviousIo) => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: "previous I/O error in database".to_owned(),
        },
        // `InvalidData` is redb saying the file's contents are unusable, which is the same
        // fact as `Corrupted` above and is recoverable by rebuilding from disk. The kind says
        // it; the message text must not be asked (ADR-0029).
        DatabaseError::Storage(RedbStorageError::Io(e))
            if e.kind() == std::io::ErrorKind::InvalidData =>
        {
            StorageError::Recoverable {
                path: path.to_path_buf(),
                reason: format!("I/O error: {e}"),
            }
        }
        DatabaseError::Storage(RedbStorageError::Io(e)) => StorageError::Hard {
            path: path.to_path_buf(),
            reason: format!("I/O error: {e}"),
        },
        DatabaseError::DatabaseAlreadyOpen => StorageError::Hard {
            path: path.to_path_buf(),
            reason: "database is already open".to_owned(),
        },
        other => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: other.to_string(),
        },
    }
}

fn map_table_error(path: &Path, err: TableError) -> StorageError {
    match err {
        TableError::Storage(RedbStorageError::Io(e))
            if e.kind() == std::io::ErrorKind::InvalidData =>
        {
            StorageError::Recoverable {
                path: path.to_path_buf(),
                reason: format!("table I/O error: {e}"),
            }
        }
        TableError::Storage(RedbStorageError::Io(e)) => StorageError::Hard {
            path: path.to_path_buf(),
            reason: format!("table I/O error: {e}"),
        },
        TableError::Storage(RedbStorageError::Corrupted(msg)) => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: msg,
        },
        other => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: other.to_string(),
        },
    }
}

fn map_tx_error(path: &Path, err: TransactionError, context: &str) -> StorageError {
    match err {
        TransactionError::Storage(RedbStorageError::Io(e))
            if e.kind() == std::io::ErrorKind::InvalidData =>
        {
            StorageError::Recoverable {
                path: path.to_path_buf(),
                reason: format!("{context}: {e}"),
            }
        }
        TransactionError::Storage(RedbStorageError::Io(e)) => StorageError::Hard {
            path: path.to_path_buf(),
            reason: format!("{context}: {e}"),
        },
        other => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("{context}: {other}"),
        },
    }
}

fn map_storage_error(path: &Path, context: &str, err: impl std::fmt::Display) -> StorageError {
    StorageError::Recoverable {
        path: path.to_path_buf(),
        reason: format!("{context}: {err}"),
    }
}
