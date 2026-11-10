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
/// `redb` table naming the base and source directory the snapshot describes.
const IDENTITY: TableDefinition<&str, &str> = TableDefinition::new("identity");
/// Key of the single row in [`IDENTITY`].
const IDENTITY_KEY: &str = "target";
/// Metadata key storing the latest scan watermark.
pub const META_KEY_WATERMARK: &str = "watermark";
/// Metadata key storing optimistic-lock generation.
pub const META_KEY_GENERATION: &str = "generation";
/// Metadata key naming how the file hashes were taken ([`HashKind`]); absent in memory written
/// before #575.
pub const META_KEY_HASH_KIND: &str = "hash_kind";
/// [`META_KEY_HASH_KIND`] of a memory of git blob ids: the only kind the runner writes.
const GIT_BLOB_KIND: u64 = 1;

/// How the hashes of one memory were taken. One memory holds one kind (#575).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashKind {
    /// Written before #575: SHA-256 of the bytes, or a kind this runner does not know. The
    /// runner no longer compares with such memory: it is no memory of the base
    /// (`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`).
    #[default]
    Legacy,
    /// Git blob ids: as `git add` stores the file under git, of its bytes without git.
    GitBlob,
}

impl HashKind {
    fn from_stored(value: Option<u64>) -> Self {
        match value {
            Some(GIT_BLOB_KIND) => Self::GitBlob,
            _ => Self::Legacy,
        }
    }
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
    pub identity: Option<String>,
    pub hash_kind: HashKind,
}

impl StorageSnapshot {
    /// Nothing has ever been committed: there is no memory to be foreign.
    pub fn is_blank(&self) -> bool {
        self.generation == 0 && self.watermark.is_none() && self.entries.is_empty()
    }

    /// Committed by a runner before #575: its hashes compare with nothing.
    pub fn is_legacy(&self) -> bool {
        !self.is_blank() && self.hash_kind == HashKind::Legacy
    }
}

/// The rows one commit changes, taken against the memory of the generation it expects (#576).
#[derive(Debug, Clone, Default)]
pub struct StorageDelta {
    /// Files remembered anew or again; a row is written only where the stored one differs.
    pub upserts: HashMap<String, StoredFileState>,
    /// Files forgotten.
    pub removals: Vec<String>,
}

/// Rows of the file tables read and written by the current thread, so a test can prove which
/// rows a commit touched (#576). Counted where a row is read or written, never by timing.
#[cfg(test)]
pub(crate) mod row_audit {
    use std::cell::Cell;

    thread_local! {
        static READ: Cell<u64> = const { Cell::new(0) };
        static WRITTEN: Cell<u64> = const { Cell::new(0) };
    }

    /// Rows read and written since the previous call.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct Rows {
        pub(crate) read: u64,
        pub(crate) written: u64,
    }

    pub(super) fn read() {
        READ.with(|rows| rows.set(rows.get() + 1));
    }

    pub(super) fn written() {
        WRITTEN.with(|rows| rows.set(rows.get() + 1));
    }

    /// The rows counted since the previous call; the count starts again.
    pub(crate) fn take() -> Rows {
        Rows {
            read: READ.with(|rows| rows.replace(0)),
            written: WRITTEN.with(|rows| rows.replace(0)),
        }
    }
}

#[cfg(not(test))]
mod row_audit {
    #[inline(always)]
    pub(super) fn read() {}

    #[inline(always)]
    pub(super) fn written() {}
}

/// What the storage remembers, read without its file rows.
#[derive(Debug, Clone, Default)]
pub struct StorageSummary {
    pub identity: Option<String>,
    pub generation: u64,
    pub watermark: Option<u64>,
    /// Files remembered, as the table header counts them.
    pub files: u64,
    pub hash_kind: HashKind,
}

impl StorageSummary {
    /// The same answer as [`StorageSnapshot::is_blank`] for the same storage.
    pub fn is_blank(&self) -> bool {
        self.generation == 0 && self.watermark.is_none() && self.files == 0
    }

    /// The same answer as [`StorageSnapshot::is_legacy`] for the same storage.
    pub fn is_legacy(&self) -> bool {
        !self.is_blank() && self.hash_kind == HashKind::Legacy
    }
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

    /// Whether the stored state should be discarded and rebuilt from disk.
    ///
    /// One owner for a question two use cases used to answer with two copies of the same
    /// substring search over an error message. Recoverability is a property of the error, and
    /// the error knows it.
    pub fn needs_recovery(&self) -> bool {
        // Every row is read: a table damaged inside must be found here, not only a bad header.
        match self.load_snapshot() {
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
        let identity = read_identity(&tx, &self.path)?;
        let mtime_tbl = optional_table(&tx, FILES_MTIME, &self.path)?;
        let hash_tbl = optional_table(&tx, FILES_HASH, &self.path)?;
        let meta_tbl = optional_table(&tx, META, &self.path)?;

        let mtime_exists = mtime_tbl.is_some();
        let hash_exists = hash_tbl.is_some();
        if !mtime_exists || !hash_exists {
            if !mtime_exists && !hash_exists {
                return Ok(StorageSnapshot {
                    identity,
                    entries: HashMap::new(),
                    watermark: read_watermark(meta_tbl.as_ref(), &self.path)?,
                    generation: read_generation(meta_tbl.as_ref(), &self.path)?,
                    hash_kind: read_hash_kind(meta_tbl.as_ref(), &self.path)?,
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
            row_audit::read();
            let rel = k.value().to_owned();
            row_audit::read();
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
            row_audit::read();
            let rel = k.value().to_owned();
            if !entries.contains_key(&rel) {
                return Err(StorageError::Recoverable {
                    path: self.path.clone(),
                    reason: format!("missing mtime for key '{rel}'"),
                });
            }
        }

        Ok(StorageSnapshot {
            identity,
            entries,
            watermark: read_watermark(meta_tbl.as_ref(), &self.path)?,
            generation: read_generation(meta_tbl.as_ref(), &self.path)?,
            hash_kind: read_hash_kind(meta_tbl.as_ref(), &self.path)?,
        })
    }

    /// Persist a full snapshot if the caller still owns the expected generation.
    ///
    /// Every key of both tables is read: an orphan row in either one is removed even when the
    /// two tables have the same length (#571). Full pulls and full rescans commit this way.
    pub fn commit_snapshot_with_identity(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
        expected_generation: u64,
        identity: Option<&str>,
    ) -> Result<(), StorageError> {
        self.commit_rows(
            watermark,
            expected_generation,
            identity,
            |mtime, hash, _| sync_file_tables(&self.path, mtime, hash, snapshot),
        )
    }

    /// Persist only the rows a delta names if the caller still owns the expected generation:
    /// after a change of one file the other rows are neither read nor written (#576).
    ///
    /// The delta is taken against the memory of `expected_generation`, so with the generation
    /// unchanged it leaves the same rows a full commit of the whole new snapshot leaves. Tables
    /// of different length — damage from outside — are reconciled whole first, so an orphan
    /// row does not outlive the commit (#571). An orphan next to a missing row of the other
    /// table keeps the lengths equal and is not seen here: the next analysis reads every row,
    /// reports it as recoverable and falls back to a full load, whose commit repairs it.
    pub fn commit_delta_with_identity(
        &self,
        delta: &StorageDelta,
        watermark: u64,
        expected_generation: u64,
        identity: Option<&str>,
    ) -> Result<(), StorageError> {
        use redb::ReadableTableMetadata;

        self.commit_rows(
            watermark,
            expected_generation,
            identity,
            |mtime, hash, stored| {
                if stored == HashKind::Legacy {
                    // Rows of another kind compare with nothing: the delta is the whole memory.
                    return sync_file_tables(&self.path, mtime, hash, &delta.upserts);
                }
                let mtime_rows = mtime
                    .len()
                    .map_err(|e| map_storage_error(&self.path, "count mtime rows", e))?;
                let hash_rows = hash
                    .len()
                    .map_err(|e| map_storage_error(&self.path, "count hash rows", e))?;
                if mtime_rows != hash_rows {
                    remove_orphan_rows(&self.path, mtime, hash)?;
                }
                for key in &delta.removals {
                    remove_row(&self.path, mtime, hash, key)?;
                }
                for (key, state) in &delta.upserts {
                    write_row(&self.path, mtime, hash, key, state)?;
                }
                Ok(())
            },
        )
    }

    /// One write transaction: the generation check, the file rows `rows` writes, the identity,
    /// the watermark and the next generation — [`first_generation`] for a memory without one.
    fn commit_rows(
        &self,
        watermark: u64,
        expected_generation: u64,
        identity: Option<&str>,
        rows: impl FnOnce(
            &mut redb::Table<&str, u64>,
            &mut redb::Table<&str, &str>,
            HashKind,
        ) -> Result<(), StorageError>,
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

            let stored_kind = HashKind::from_stored(
                meta.get(META_KEY_HASH_KIND)
                    .map_err(|e| map_storage_error(&self.path, "read hash kind", e))?
                    .map(|v| v.value()),
            );
            let mut mtime = tx
                .open_table(FILES_MTIME)
                .map_err(|e| map_table_error(&self.path, e))?;
            let mut hash = tx
                .open_table(FILES_HASH)
                .map_err(|e| map_table_error(&self.path, e))?;
            rows(&mut mtime, &mut hash, stored_kind)?;
            meta.insert(META_KEY_HASH_KIND, GIT_BLOB_KIND)
                .map_err(|e| map_storage_error(&self.path, "write hash kind", e))?;
            if let Some(identity) = identity {
                tx.open_table(IDENTITY)
                    .map_err(|e| map_table_error(&self.path, e))?
                    .insert(IDENTITY_KEY, identity)
                    .map_err(|e| map_storage_error(&self.path, "write identity", e))?;
            }

            meta.insert(META_KEY_WATERMARK, watermark)
                .map_err(|e| map_storage_error(&self.path, "write watermark", e))?;
            let next_generation = if current_generation == 0 {
                first_generation(&self.path)?
            } else {
                current_generation + 1
            };
            meta.insert(META_KEY_GENERATION, next_generation)
                .map_err(|e| map_storage_error(&self.path, "write generation", e))?;
        }

        tx.commit()
            .map_err(|e| map_storage_error(&self.path, "commit transaction", e))?;
        Ok(())
    }

    /// Read the current optimistic-lock generation.
    pub fn current_generation(&self) -> Result<u64, StorageError> {
        Ok(self.load_summary()?.generation)
    }

    /// Read the identity, generation, watermark and file count without the file rows: a
    /// large source set remembers tens of thousands of files, and a question about the
    /// memory itself read every one of them (#571). Tables of different length are the
    /// same recoverable problem [`Self::load_snapshot`] reports for them.
    pub fn load_summary(&self) -> Result<StorageSummary, StorageError> {
        use redb::ReadableTableMetadata;

        if !self.path.exists() {
            return Ok(StorageSummary::default());
        }
        let db = Database::open(&self.path).map_err(|e| map_database_error(&self.path, e))?;
        let tx = db
            .begin_read()
            .map_err(|e| map_tx_error(&self.path, e, "begin read"))?;
        let identity = read_identity(&tx, &self.path)?;
        let mtime_rows = optional_table(&tx, FILES_MTIME, &self.path)?
            .map(|table| table.len())
            .transpose()
            .map_err(|e| map_storage_error(&self.path, "count mtime rows", e))?;
        let hash_rows = optional_table(&tx, FILES_HASH, &self.path)?
            .map(|table| table.len())
            .transpose()
            .map_err(|e| map_storage_error(&self.path, "count hash rows", e))?;
        let files = match (mtime_rows, hash_rows) {
            (None, None) => 0,
            (Some(mtime), Some(hash)) if mtime == hash => mtime,
            _ => {
                return Err(StorageError::Recoverable {
                    path: self.path.clone(),
                    reason: "mtime/hash tables are inconsistent".to_owned(),
                })
            }
        };
        let meta_tbl = optional_table(&tx, META, &self.path)?;
        Ok(StorageSummary {
            identity,
            generation: read_generation(meta_tbl.as_ref(), &self.path)?,
            watermark: read_watermark(meta_tbl.as_ref(), &self.path)?,
            files,
            hash_kind: read_hash_kind(meta_tbl.as_ref(), &self.path)?,
        })
    }

    /// Replace a corrupt or missing storage file with a fresh snapshot. The fresh memory starts
    /// from a random generation, so a commit prepared against the replaced one is refused.
    pub fn recover_and_commit_snapshot_with_identity(
        &self,
        snapshot: &HashMap<String, StoredFileState>,
        watermark: u64,
        identity: Option<&str>,
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
        self.commit_snapshot_with_identity(snapshot, watermark, 0, identity)
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

/// The generation of the first commit of a memory created anew: the first time, after its file
/// was deleted, or after [`HashStorage::recover_and_commit_snapshot_with_identity`]. Counting
/// from 1 again would let a delta prepared against an older memory of the same number land on a
/// different base (#576); a random epoch in the high 31 bits makes that a 1-in-2^31 chance per
/// recreation. The low 32 bits count the commits from 1, and the value never reaches `u64::MAX`.
fn first_generation(path: &Path) -> Result<u64, StorageError> {
    let mut seed = [0u8; 4];
    getrandom::fill(&mut seed).map_err(|e| StorageError::Hard {
        path: path.to_path_buf(),
        reason: format!("random start of the memory generation: {e}"),
    })?;
    let epoch = u64::from(u32::from_le_bytes(seed) >> 1);
    Ok((epoch << 32) | 1)
}

/// The commits since the memory was created, read from its generation: what a test compares
/// when it counts commits.
#[cfg(test)]
pub(crate) fn commits_of(generation: u64) -> u64 {
    generation & u64::from(u32::MAX)
}

/// A table that may not exist yet: the one reader both the snapshot and the summary use.
fn optional_table<K: redb::Key + 'static, V: redb::Value + 'static>(
    tx: &redb::ReadTransaction,
    table: redb::TableDefinition<K, V>,
    path: &Path,
) -> Result<Option<redb::ReadOnlyTable<K, V>>, StorageError> {
    match tx.open_table(table) {
        Ok(table) => Ok(Some(table)),
        Err(TableError::TableDoesNotExist(_)) => Ok(None),
        Err(e) => Err(map_table_error(path, e)),
    }
}

fn read_identity(tx: &redb::ReadTransaction, path: &Path) -> Result<Option<String>, StorageError> {
    let Some(table) = optional_table(tx, IDENTITY, path)? else {
        return Ok(None);
    };
    table
        .get(IDENTITY_KEY)
        .map_err(|e| map_storage_error(path, "read identity", e))
        .map(|value| value.map(|value| value.value().to_owned()))
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

fn read_hash_kind(
    meta: Option<&redb::ReadOnlyTable<&str, u64>>,
    path: &Path,
) -> Result<HashKind, StorageError> {
    let Some(meta) = meta else {
        return Ok(HashKind::Legacy);
    };
    meta.get(META_KEY_HASH_KIND)
        .map_err(|e| StorageError::Recoverable {
            path: path.to_path_buf(),
            reason: format!("read hash kind: {e}"),
        })
        .map(|value| HashKind::from_stored(value.map(|v| v.value())))
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
    // Keys of both tables: an orphan row in either one is removed, so every commit leaves the
    // two tables with the same keys even after outside damage the summary cannot see (#571).
    let (mtime_keys, hash_keys) = table_keys(path, mtime, hash)?;
    for key in mtime_keys.union(&hash_keys) {
        if !snapshot.contains_key(key) {
            remove_row(path, mtime, hash, key)?;
        }
    }

    // Only rows that differ are written: after a change of one file the commit touched every
    // row of a large set, and a write costs far more than a lookup (#571).
    for (key, state) in snapshot {
        write_row(path, mtime, hash, key, state)?;
    }
    Ok(())
}

/// The keys of each file table, every row read.
fn table_keys(
    path: &Path,
    mtime: &redb::Table<&str, u64>,
    hash: &redb::Table<&str, &str>,
) -> Result<(HashSet<String>, HashSet<String>), StorageError> {
    let mut mtime_keys = HashSet::new();
    for row in mtime
        .iter()
        .map_err(|e| map_storage_error(path, "iterate mtime table", e))?
    {
        let (k, _) = row.map_err(|e| map_storage_error(path, "read mtime row", e))?;
        row_audit::read();
        mtime_keys.insert(k.value().to_owned());
    }
    let mut hash_keys = HashSet::new();
    for row in hash
        .iter()
        .map_err(|e| map_storage_error(path, "iterate hash table", e))?
    {
        let (k, _) = row.map_err(|e| map_storage_error(path, "read hash row", e))?;
        row_audit::read();
        hash_keys.insert(k.value().to_owned());
    }
    Ok((mtime_keys, hash_keys))
}

/// Removes the rows whose key only one table has, so both tables keep the same keys (#571).
/// The file such a row describes is no longer remembered and is a change on the next analysis.
fn remove_orphan_rows(
    path: &Path,
    mtime: &mut redb::Table<&str, u64>,
    hash: &mut redb::Table<&str, &str>,
) -> Result<(), StorageError> {
    let (mtime_keys, hash_keys) = table_keys(path, mtime, hash)?;
    for key in mtime_keys.symmetric_difference(&hash_keys) {
        remove_row(path, mtime, hash, key)?;
    }
    Ok(())
}

/// Forgets one file in both tables.
fn remove_row(
    path: &Path,
    mtime: &mut redb::Table<&str, u64>,
    hash: &mut redb::Table<&str, &str>,
    key: &str,
) -> Result<(), StorageError> {
    if mtime
        .remove(key)
        .map_err(|e| map_storage_error(path, "remove stale mtime key", e))?
        .is_some()
    {
        row_audit::written();
    }
    if hash
        .remove(key)
        .map_err(|e| map_storage_error(path, "remove stale hash key", e))?
        .is_some()
    {
        row_audit::written();
    }
    Ok(())
}

/// Remembers one file, writing a table's row only where it differs. The comparison runs inside
/// the commit's write transaction, after the generation check, so it is against what is
/// committed now and not against what was read earlier.
fn write_row(
    path: &Path,
    mtime: &mut redb::Table<&str, u64>,
    hash: &mut redb::Table<&str, &str>,
    key: &str,
    state: &StoredFileState,
) -> Result<(), StorageError> {
    row_audit::read();
    let stored_mtime = mtime
        .get(key)
        .map_err(|e| map_storage_error(path, "read mtime", e))?
        .map(|value| value.value());
    if stored_mtime != Some(state.mtime_ns) {
        row_audit::written();
        mtime
            .insert(key, state.mtime_ns)
            .map_err(|e| map_storage_error(path, "insert mtime", e))?;
    }
    row_audit::read();
    let hash_matches = hash
        .get(key)
        .map_err(|e| map_storage_error(path, "read hash", e))?
        .is_some_and(|value| value.value() == state.hash);
    if !hash_matches {
        row_audit::written();
        hash.insert(key, state.hash.as_str())
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
        // it; the message text must not be asked (DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES).
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

#[cfg(test)]
mod tests {
    use super::{
        commits_of, row_audit, HashStorage, StorageDelta, StorageError, StoredFileState,
        FILES_MTIME,
    };
    use redb::Database;
    use std::collections::HashMap;

    fn state(mtime_ns: u64, hash: &str) -> StoredFileState {
        StoredFileState {
            mtime_ns,
            hash: hash.to_owned(),
        }
    }

    fn rows(snapshot: &HashMap<String, StoredFileState>) -> Vec<(String, u64, String)> {
        let mut rows: Vec<_> = snapshot
            .iter()
            .map(|(key, value)| (key.clone(), value.mtime_ns, value.hash.clone()))
            .collect();
        rows.sort();
        rows
    }

    #[test]
    fn summary_answers_as_the_snapshot_without_its_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = HashStorage::new(dir.path().join("memory.redb"));
        let missing = storage.load_summary().expect("no file");
        assert!(missing.is_blank());
        assert_eq!(missing.generation, 0);

        let snapshot = HashMap::from([
            ("a.xml".to_owned(), state(1, "h1")),
            ("b.bsl".to_owned(), state(2, "h2")),
        ]);
        storage
            .commit_snapshot_with_identity(&snapshot, 7, 0, Some("base"))
            .expect("commit");
        let summary = storage.load_summary().expect("summary");
        let full = storage.load_snapshot().expect("snapshot");
        assert_eq!(summary.identity, full.identity);
        assert_eq!(summary.generation, full.generation);
        assert_eq!(summary.watermark, full.watermark);
        assert_eq!(summary.files, full.entries.len() as u64);
        assert_eq!(summary.is_blank(), full.is_blank());
        let generation = storage.current_generation().expect("generation");
        assert_eq!(commits_of(generation), 1);

        storage
            .commit_snapshot_with_identity(&HashMap::new(), 0, generation, Some("base"))
            .expect("empty commit");
        let emptied = storage.load_summary().expect("emptied");
        assert_eq!(emptied.files, 0);
        assert!(!emptied.is_blank(), "a committed generation is memory");
    }

    #[test]
    fn summary_reports_tables_of_different_length_as_recoverable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("memory.redb");
        let storage = HashStorage::new(path.clone());
        storage
            .commit_snapshot_with_identity(
                &HashMap::from([("a.xml".to_owned(), state(1, "h1"))]),
                1,
                0,
                Some("base"),
            )
            .expect("commit");
        {
            let db = Database::open(&path).expect("open");
            let tx = db.begin_write().expect("write");
            tx.open_table(FILES_MTIME)
                .expect("mtime")
                .insert("orphan.xml", 5)
                .expect("insert");
            tx.commit().expect("commit orphan");
        }
        let error = storage.load_summary().expect_err("inconsistent");
        assert!(error.is_recoverable(), "{error}");
        assert!(storage.needs_recovery());
    }

    /// Tables of equal length with different keys pass the summary; the next commit must still
    /// leave them with the same keys, so the damage does not outlive one write.
    #[test]
    fn a_commit_removes_orphan_rows_of_either_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("memory.redb");
        let storage = HashStorage::new(path.clone());
        storage
            .commit_snapshot_with_identity(
                &HashMap::from([
                    ("a.xml".to_owned(), state(1, "h1")),
                    ("x.xml".to_owned(), state(2, "h2")),
                ]),
                1,
                0,
                Some("base"),
            )
            .expect("commit");
        {
            let db = Database::open(&path).expect("open");
            let tx = db.begin_write().expect("write");
            {
                let mut hash = tx.open_table(super::FILES_HASH).expect("hash");
                hash.remove("x.xml").expect("remove");
                hash.insert("y.xml", "hy").expect("insert");
            }
            tx.commit().expect("commit damage");
        }
        let generation = storage.current_generation().expect("summary passes");
        storage
            .commit_snapshot_with_identity(
                &HashMap::from([
                    ("a.xml".to_owned(), state(1, "h1")),
                    ("b.xml".to_owned(), state(3, "h3")),
                ]),
                2,
                generation,
                Some("base"),
            )
            .expect("commit over damage");
        let snapshot = storage.load_snapshot().expect("consistent again");
        let mut keys: Vec<_> = snapshot.entries.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["a.xml", "b.xml"]);
    }

    fn many(files: u64) -> HashMap<String, StoredFileState> {
        (0..files)
            .map(|n| (format!("f{n}.xml"), state(n, "same")))
            .collect()
    }

    /// After a change of one file and the removal of another the delta commit reads and writes
    /// only their rows, and leaves what the full commit of the whole new snapshot leaves (#576).
    #[test]
    fn a_delta_commit_touches_only_the_rows_of_the_changed_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = HashStorage::new(dir.path().join("memory.redb"));
        let full = HashStorage::new(dir.path().join("full.redb"));
        let first = many(200);
        for memory in [&storage, &full] {
            memory
                .commit_snapshot_with_identity(&first, 1, 0, Some("base"))
                .expect("first");
        }
        let generation = storage.current_generation().expect("generation");
        let full_generation = full.current_generation().expect("full generation");
        let mut second = first.clone();
        second.insert("f7.xml".to_owned(), state(1_000, "edited"));
        second.remove("f9.xml");
        let delta = StorageDelta {
            upserts: HashMap::from([("f7.xml".to_owned(), state(1_000, "edited"))]),
            removals: vec!["f9.xml".to_owned()],
        };

        row_audit::take();
        storage
            .commit_delta_with_identity(&delta, 2, generation, Some("base"))
            .expect("delta");
        let touched = row_audit::take();
        full.commit_snapshot_with_identity(&second, 2, full_generation, Some("base"))
            .expect("full");
        let whole = row_audit::take();

        // One changed file: its two rows looked up and rewritten; one removed file: two rows.
        assert_eq!(
            touched,
            row_audit::Rows {
                read: 2,
                written: 4
            }
        );
        assert!(
            whole.read >= 2 * 200,
            "the full commit reads every row: {whole:?}"
        );
        let loaded = storage.load_snapshot().expect("loaded");
        assert_eq!(rows(&loaded.entries), rows(&second));
        assert_eq!(
            rows(&loaded.entries),
            rows(&full.load_snapshot().expect("full").entries)
        );
        assert_eq!(loaded.generation, generation + 1);
        assert_eq!(loaded.watermark, Some(2));
    }

    /// Tables of different length are reconciled whole by a delta commit: the orphan row goes
    /// and the rows the delta does not name stay (#571, #576).
    #[test]
    fn a_delta_commit_over_tables_of_different_length_reconciles_them_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("memory.redb");
        let storage = HashStorage::new(path.clone());
        storage
            .commit_snapshot_with_identity(
                &HashMap::from([("a.xml".to_owned(), state(1, "h1"))]),
                1,
                0,
                Some("base"),
            )
            .expect("commit");
        let generation = storage.current_generation().expect("generation");
        {
            let db = Database::open(&path).expect("open");
            let tx = db.begin_write().expect("write");
            tx.open_table(FILES_MTIME)
                .expect("mtime")
                .insert("orphan.xml", 5)
                .expect("insert");
            tx.commit().expect("commit orphan");
        }
        assert!(storage.load_snapshot().is_err(), "the damage is seen");

        storage
            .commit_delta_with_identity(
                &StorageDelta {
                    upserts: HashMap::from([("b.xml".to_owned(), state(2, "h2"))]),
                    removals: Vec::new(),
                },
                2,
                generation,
                Some("base"),
            )
            .expect("delta over damage");

        let snapshot = storage.load_snapshot().expect("consistent again");
        assert_eq!(
            rows(&snapshot.entries),
            [
                ("a.xml".to_owned(), 1, "h1".to_owned()),
                ("b.xml".to_owned(), 2, "h2".to_owned())
            ]
        );
    }

    /// A delta is taken against one generation: it is refused once the memory moved on.
    #[test]
    fn a_delta_commit_fails_if_the_memory_moved_since_it_was_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = HashStorage::new(dir.path().join("memory.redb"));
        storage
            .commit_snapshot_with_identity(&many(3), 1, 0, Some("base"))
            .expect("first");
        let read = storage.current_generation().expect("generation");
        storage
            .commit_snapshot_with_identity(&many(3), 2, read, Some("base"))
            .expect("second");
        let error = storage
            .commit_delta_with_identity(
                &StorageDelta {
                    upserts: HashMap::from([("f0.xml".to_owned(), state(9, "late"))]),
                    removals: Vec::new(),
                },
                3,
                read,
                Some("base"),
            )
            .expect_err("moved");
        assert!(
            matches!(
                error,
                StorageError::ConcurrentStateModified { expected, actual, .. }
                    if expected == read && actual == read + 1
            ),
            "{error}"
        );
        assert_eq!(
            storage.load_snapshot().expect("kept").entries["f0.xml"].hash,
            "same"
        );
    }

    /// A memory created anew — recovered, or its file deleted — does not count its generations
    /// from 1 again: a delta prepared against the memory it replaced is refused, not written
    /// onto a different base (#576).
    #[test]
    fn a_delta_prepared_before_the_memory_was_recreated_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("memory.redb");
        let storage = HashStorage::new(path.clone());
        let late = StorageDelta {
            upserts: HashMap::from([("f0.xml".to_owned(), state(9, "late"))]),
            removals: Vec::new(),
        };

        for recreate in ["recover", "delete"] {
            storage
                .commit_snapshot_with_identity(&many(3), 1, 0, Some("base"))
                .expect("memory");
            let read = storage.current_generation().expect("generation");
            assert_eq!(commits_of(read), 1, "{recreate}");

            match recreate {
                "recover" => storage
                    .recover_and_commit_snapshot_with_identity(&many(3), 2, Some("base"))
                    .expect("recovered"),
                _ => {
                    std::fs::remove_file(&path).expect("delete");
                    storage
                        .commit_snapshot_with_identity(&many(3), 2, 0, Some("base"))
                        .expect("created anew");
                }
            }
            let fresh = storage.current_generation().expect("fresh generation");
            assert_eq!(commits_of(fresh), 1, "{recreate}");
            assert_ne!(fresh, read, "{recreate}: a new memory, a new generation");

            let error = storage
                .commit_delta_with_identity(&late, 3, read, Some("base"))
                .expect_err("prepared against the replaced memory");
            assert!(
                matches!(error, StorageError::ConcurrentStateModified { .. }),
                "{recreate}: {error}"
            );
            assert_eq!(
                storage.load_snapshot().expect("kept").entries["f0.xml"].hash,
                "same",
                "{recreate}"
            );
            std::fs::remove_file(&path).expect("next round starts without memory");
        }
    }

    #[test]
    fn a_commit_that_writes_only_differences_leaves_exactly_the_new_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = HashStorage::new(dir.path().join("memory.redb"));
        let first = HashMap::from([
            ("kept.xml".to_owned(), state(1, "same")),
            ("touched.xml".to_owned(), state(2, "same")),
            ("edited.bsl".to_owned(), state(3, "old")),
            ("removed.xml".to_owned(), state(4, "gone")),
        ]);
        storage
            .commit_snapshot_with_identity(&first, 1, 0, Some("base"))
            .expect("first");
        let generation = storage.current_generation().expect("generation");
        let second = HashMap::from([
            ("kept.xml".to_owned(), state(1, "same")),
            ("touched.xml".to_owned(), state(9, "same")),
            ("edited.bsl".to_owned(), state(3, "new")),
            ("added.xml".to_owned(), state(5, "fresh")),
        ]);
        storage
            .commit_snapshot_with_identity(&second, 2, generation, Some("base"))
            .expect("second");
        let loaded = storage.load_snapshot().expect("loaded");
        assert_eq!(rows(&loaded.entries), rows(&second));
        assert_eq!(commits_of(loaded.generation), 2);
        assert_eq!(loaded.watermark, Some(2));
    }
}
