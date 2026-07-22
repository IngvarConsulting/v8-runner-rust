use redb::{
    CommitError, Database, DatabaseError, ReadableTable, StorageError as RedbStorageError,
    TableDefinition, TableError, TransactionError,
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
}

/// Distinguishes a never-initialized scoped storage from a committed snapshot.
#[derive(Debug, Clone)]
pub enum HashStorageLoad {
    Missing,
    Initialized(StorageSnapshot),
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
}

impl HashStorage {
    /// Create a storage handle for the given `redb` file path.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Return the underlying storage file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load typed initialization state without a separate filesystem existence check.
    pub fn load_state(&self) -> Result<HashStorageLoad, StorageError> {
        match self.path.try_exists() {
            Ok(false) => return Ok(HashStorageLoad::Missing),
            Ok(true) => {}
            Err(error) => return Err(map_filesystem_lookup_error(&self.path, error)),
        }
        let db = match Database::open(&self.path) {
            Ok(database) => database,
            Err(database_error) => match self.path.try_exists() {
                Ok(false) => return Ok(HashStorageLoad::Missing),
                Ok(true) => return Err(map_database_error(&self.path, database_error)),
                Err(error) => return Err(map_filesystem_lookup_error(&self.path, error)),
            },
        };
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

        let mtime_exists = mtime_tbl.is_some();
        let hash_exists = hash_tbl.is_some();
        if !mtime_exists || !hash_exists {
            if !mtime_exists && !hash_exists {
                return if meta_tbl.is_none() {
                    Ok(HashStorageLoad::Missing)
                } else {
                    Err(StorageError::Recoverable {
                        path: self.path.clone(),
                        reason: "metadata exists without snapshot tables".to_owned(),
                    })
                };
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

        Ok(HashStorageLoad::Initialized(StorageSnapshot {
            entries,
            watermark: read_watermark(meta_tbl.as_ref(), &self.path)?,
            generation: read_generation(meta_tbl.as_ref(), &self.path)?,
        }))
    }

    #[cfg(test)]
    pub fn load_snapshot(&self) -> Result<StorageSnapshot, StorageError> {
        match self.load_state()? {
            HashStorageLoad::Missing => Ok(StorageSnapshot::default()),
            HashStorageLoad::Initialized(snapshot) => Ok(snapshot),
        }
    }

    /// Persist a full snapshot if the caller still owns the expected generation.
    pub fn commit_snapshot(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
        expected_generation: u64,
    ) -> Result<(), StorageError> {
        self.ensure_parent_dir()?;
        let db = Database::create(&self.path).map_err(|e| map_database_error(&self.path, e))?;
        let tx = db
            .begin_write()
            .map_err(|e| map_tx_error(&self.path, e, "begin write"))?;

        {
            let mut meta = tx
                .open_table(META)
                .map_err(|e| map_table_error(&self.path, e))?;
            let current_generation = meta
                .get(META_KEY_GENERATION)
                .map_err(|e| map_storage_error(&self.path, "read generation", e))?
                .map(|v| v.value())
                .unwrap_or(0);
            if current_generation != expected_generation {
                return Err(StorageError::ConcurrentStateModified {
                    path: self.path.clone(),
                    expected: expected_generation,
                    actual: current_generation,
                });
            }

            let mut mtime = tx
                .open_table(FILES_MTIME)
                .map_err(|e| map_table_error(&self.path, e))?;
            let mut hash = tx
                .open_table(FILES_HASH)
                .map_err(|e| map_table_error(&self.path, e))?;
            sync_file_tables(&self.path, &mut mtime, &mut hash, snapshot)?;

            meta.insert(META_KEY_WATERMARK, watermark)
                .map_err(|e| map_storage_error(&self.path, "write watermark", e))?;
            meta.insert(META_KEY_GENERATION, expected_generation + 1)
                .map_err(|e| map_storage_error(&self.path, "write generation", e))?;
        }

        tx.commit().map_err(|e| map_commit_error(&self.path, e))?;
        Ok(())
    }

    /// Read the current optimistic-lock generation.
    pub fn current_generation(&self) -> Result<u64, StorageError> {
        Ok(match self.load_state()? {
            HashStorageLoad::Missing => 0,
            HashStorageLoad::Initialized(snapshot) => snapshot.generation,
        })
    }

    /// Read the optimistic-lock generation even when snapshot tables are recoverably incomplete.
    pub fn recoverable_generation(&self) -> Result<u64, StorageError> {
        let database =
            Database::open(&self.path).map_err(|error| map_database_error(&self.path, error))?;
        let transaction = database
            .begin_read()
            .map_err(|error| map_tx_error(&self.path, error, "begin recovery read"))?;
        let meta = match transaction.open_table(META) {
            Ok(table) => Some(table),
            Err(TableError::TableDoesNotExist(_)) => None,
            Err(error) => return Err(map_table_error(&self.path, error)),
        };
        read_generation(meta.as_ref(), &self.path)
    }

    /// Replace a corrupt or missing storage file with a fresh snapshot.
    pub fn recover_and_commit_snapshot(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
    ) -> Result<(), StorageError> {
        if self
            .path
            .try_exists()
            .map_err(|error| map_filesystem_lookup_error(&self.path, error))?
        {
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
        .map_err(|e| map_storage_error(path, "read watermark", e))
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
        .map_err(|e| map_storage_error(path, "read generation", e))
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
        DatabaseError::Storage(RedbStorageError::Io(e)) => map_io_error(path, "I/O error", e),
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
        TableError::Storage(RedbStorageError::Io(e)) => map_io_error(path, "table I/O error", e),
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
        TransactionError::Storage(RedbStorageError::Io(e)) => map_io_error(path, context, e),
        other => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("{context}: {other}"),
        },
    }
}

fn map_storage_error(path: &Path, context: &str, err: RedbStorageError) -> StorageError {
    match err {
        RedbStorageError::Io(error) => map_io_error(path, context, error),
        other => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("{context}: {other}"),
        },
    }
}

fn map_io_error(path: &Path, context: &str, error: std::io::Error) -> StorageError {
    let reason = format!("{context}: {error}");
    if error.kind() == std::io::ErrorKind::InvalidData {
        StorageError::Recoverable {
            path: path.to_path_buf(),
            reason,
        }
    } else {
        StorageError::Hard {
            path: path.to_path_buf(),
            reason,
        }
    }
}

fn map_commit_error(path: &Path, error: CommitError) -> StorageError {
    match error {
        CommitError::Storage(error) => map_storage_error(path, "commit transaction", error),
        other => StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("commit transaction: {other}"),
        },
    }
}

fn map_filesystem_lookup_error(path: &Path, error: std::io::Error) -> StorageError {
    StorageError::Hard {
        path: path.to_path_buf(),
        reason: format!("failed to inspect storage path: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{HashStorage, HashStorageLoad, StorageError, META, META_KEY_GENERATION};
    use redb::Database;
    use tempfile::tempdir;

    #[test]
    fn missing_file_has_typed_missing_state() {
        let dir = tempdir().expect("tempdir");
        let storage = HashStorage::new(dir.path().join("missing.redb"));

        assert!(matches!(storage.load_state(), Ok(HashStorageLoad::Missing)));
    }

    #[test]
    fn existing_empty_database_has_typed_missing_state() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("empty.redb");
        Database::create(&path).expect("empty database");

        assert!(matches!(
            HashStorage::new(path).load_state(),
            Ok(HashStorageLoad::Missing)
        ));
    }

    #[test]
    fn metadata_without_snapshot_tables_is_recoverable() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("metadata-only.redb");
        let database = Database::create(&path).expect("database");
        let write = database.begin_write().expect("write");
        {
            let mut meta = write.open_table(META).expect("meta");
            meta.insert(META_KEY_GENERATION, 1).expect("generation");
        }
        write.commit().expect("commit");
        drop(database);

        assert!(matches!(
            HashStorage::new(path).load_state(),
            Err(StorageError::Recoverable { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_lookup_errors_are_hard_instead_of_missing() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("loop.redb");
        std::os::unix::fs::symlink(&path, &path).expect("self symlink");

        assert!(matches!(
            HashStorage::new(path).load_state(),
            Err(StorageError::Hard { .. })
        ));
    }
}
