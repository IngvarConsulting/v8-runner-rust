use std::path::{Path, PathBuf};

use crate::support::path::is_safe_path_segment;

/// Runtime context for one logical source-set.
#[derive(Debug, Clone)]
pub struct SourceSetContext {
    /// Logical name (matches `SourceSetConfig.name`).
    name: String,
    /// Absolute root directory of the sources.
    path: PathBuf,
    /// Key naming a shared redb hash-storage file (`workPath/hash-storages/<key>.redb`).
    /// Infobase memory lives under `workPath/infobases/<base>/hashes/<name>.redb` instead.
    storage_key: String,
    memory: SnapshotMemory,
}

#[derive(Debug, Clone)]
enum SnapshotMemory {
    Shared,
    Disabled,
    Infobase { name: String, identity: String },
}

impl SourceSetContext {
    pub fn new(name: impl Into<String>, path: PathBuf, storage_key: impl Into<String>) -> Self {
        let name = name.into();
        let storage_key = storage_key.into();
        assert!(
            path.is_absolute(),
            "SourceSetContext.path must be absolute, got: {}",
            path.display()
        );
        assert!(
            is_safe_path_segment(&storage_key),
            "SourceSetContext.storage_key must be a safe single path segment, got: {storage_key}"
        );

        Self {
            name,
            path,
            storage_key,
            memory: SnapshotMemory::Shared,
        }
    }

    /// Bind hash memory to a named infobase; `identity` is stored and compared with the snapshot.
    pub fn with_infobase_memory(mut self, infobase: &str, identity: String) -> Self {
        assert!(
            is_safe_path_segment(&self.name),
            "source set name must be a safe path segment"
        );
        assert!(
            is_safe_path_segment(infobase),
            "infobase name must be a safe path segment"
        );
        self.memory = SnapshotMemory::Infobase {
            name: infobase.to_owned(),
            identity,
        };
        self
    }

    /// A context whose changes are never remembered: it has no storage path at all.
    pub fn without_memory(mut self) -> Self {
        self.memory = SnapshotMemory::Disabled;
        self
    }

    /// Same answer as `storage_path(..).is_some()`, for callers without a `workPath`.
    pub fn persists_snapshot(&self) -> bool {
        !matches!(self.memory, SnapshotMemory::Disabled)
    }

    pub fn storage_identity(&self) -> Option<&str> {
        match &self.memory {
            SnapshotMemory::Infobase { identity, .. } => Some(identity),
            SnapshotMemory::Shared | SnapshotMemory::Disabled => None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Absolute path to the redb hash-storage file for this context, or `None` when the
    /// context keeps no memory. Every storage access goes through this answer.
    pub fn storage_path(&self, work_path: &Path) -> Option<PathBuf> {
        match &self.memory {
            SnapshotMemory::Disabled => None,
            SnapshotMemory::Infobase { name, .. } => Some(
                infobase_memory_dir(work_path, name)
                    .join("hashes")
                    .join(format!("{}.redb", self.name)),
            ),
            SnapshotMemory::Shared => Some(
                work_path
                    .join("hash-storages")
                    .join(format!("{}.redb", self.storage_key)),
            ),
        }
    }

    /// Каталог копии файла версий этого набора: `workPath/infobases/<база>/dump-info/<набор>`.
    /// Копия описывает пару «база ↔ каталог», поэтому есть только у памяти именованной базы.
    pub fn version_file_copy_dir(&self, work_path: &Path) -> Option<PathBuf> {
        match &self.memory {
            SnapshotMemory::Infobase { name, .. } => Some(
                infobase_memory_dir(work_path, name)
                    .join("dump-info")
                    .join(&self.name),
            ),
            SnapshotMemory::Shared | SnapshotMemory::Disabled => None,
        }
    }
}

/// Память об одной именованной базе: `workPath/infobases/<база>`.
pub fn infobase_memory_dir(work_path: &Path, infobase: &str) -> PathBuf {
    work_path.join("infobases").join(infobase)
}

#[cfg(test)]
mod tests {
    use super::SourceSetContext;
    use std::path::PathBuf;

    #[test]
    fn accepts_absolute_path() {
        let context =
            SourceSetContext::new("main", PathBuf::from("/tmp/src-main"), "designer-main");
        assert_eq!(context.name(), "main");
        assert_eq!(context.path(), PathBuf::from("/tmp/src-main").as_path());
        assert_eq!(
            context.storage_path(PathBuf::from("/tmp/work").as_path()),
            Some(PathBuf::from("/tmp/work/hash-storages/designer-main.redb"))
        );
    }

    #[test]
    #[should_panic(expected = "must be absolute")]
    fn rejects_relative_path() {
        let _ = SourceSetContext::new("main", PathBuf::from("relative/path"), "designer-main");
    }

    #[test]
    fn accepts_safe_storage_key() {
        let context =
            SourceSetContext::new("main", PathBuf::from("/tmp/src-main"), "main-config_01");
        assert_eq!(
            context.storage_path(PathBuf::from("/tmp/work").as_path()),
            Some(PathBuf::from("/tmp/work/hash-storages/main-config_01.redb"))
        );
    }

    #[test]
    #[should_panic(expected = "safe single path segment")]
    fn rejects_storage_key_with_parent_traversal() {
        let _ = SourceSetContext::new("main", PathBuf::from("/tmp/src-main"), "../outside");
    }

    #[test]
    #[should_panic(expected = "safe single path segment")]
    fn rejects_storage_key_with_separator() {
        let _ = SourceSetContext::new("main", PathBuf::from("/tmp/src-main"), "bad/name");
    }

    #[test]
    #[should_panic(expected = "safe single path segment")]
    fn rejects_storage_key_with_backslash_separator() {
        let _ = SourceSetContext::new("main", PathBuf::from("/tmp/src-main"), "bad\\name");
    }
}
