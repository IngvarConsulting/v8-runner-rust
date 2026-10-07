use std::path::{Path, PathBuf};

use crate::support::path::is_safe_path_segment;

/// Назначение набора исходников — значение ключа `type` в `v8project.yaml`.
#[derive(
    Debug, Clone, Copy, serde::Deserialize, serde::Serialize, PartialEq, Eq, schemars::JsonSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceSetPurpose {
    Configuration,
    Extension,
    ExternalDataProcessors,
    ExternalReports,
}

impl SourceSetPurpose {
    pub const fn is_external(self) -> bool {
        matches!(self, Self::ExternalDataProcessors | Self::ExternalReports)
    }

    /// The YAML `type` spelling. Stable: it is also persisted in hash-memory bindings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "CONFIGURATION",
            Self::Extension => "EXTENSION",
            Self::ExternalDataProcessors => "EXTERNAL_DATA_PROCESSORS",
            Self::ExternalReports => "EXTERNAL_REPORTS",
        }
    }
}

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
    Infobase {
        base: String,
        identity: String,
        subject: MemorySubject,
    },
}

/// Чья память лежит под базой: набора исходников или расширения-инструмента.
#[derive(Debug, Clone)]
enum MemorySubject {
    SourceSet,
    /// `tools.extensions[].name`; его хеши лежат в `hashes/tools/<имя>.redb`.
    ToolExtension(String),
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

    /// Bind hash memory to an infobase: `infobase` names its directory under
    /// `workPath/infobases/` (a declared name or [`connection_memory_key`]); `identity` is
    /// stored and compared with the snapshot.
    pub fn with_infobase_memory(mut self, infobase: &str, identity: String) -> Self {
        assert!(
            is_safe_path_segment(&self.name),
            "source set name must be a safe path segment"
        );
        self.memory = infobase_memory(infobase, identity, MemorySubject::SourceSet);
        self
    }

    /// Bind the hash memory of a tool extension's sources to an infobase, beside the memory
    /// of its source sets but never sharing a file with one of them.
    pub fn with_tool_extension_memory(
        mut self,
        infobase: &str,
        extension: &str,
        identity: String,
    ) -> Self {
        assert!(
            is_safe_path_segment(extension),
            "tool extension name must be a safe path segment"
        );
        self.memory = infobase_memory(
            infobase,
            identity,
            MemorySubject::ToolExtension(extension.to_owned()),
        );
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
            SnapshotMemory::Infobase { base, subject, .. } => {
                let hashes = infobase_memory_dir(work_path, base).join("hashes");
                Some(match subject {
                    MemorySubject::SourceSet => hashes.join(format!("{}.redb", self.name)),
                    MemorySubject::ToolExtension(extension) => {
                        hashes.join("tools").join(format!("{extension}.redb"))
                    }
                })
            }
            SnapshotMemory::Shared => Some(
                work_path
                    .join("hash-storages")
                    .join(format!("{}.redb", self.storage_key)),
            ),
        }
    }

    /// Каталог копии файла версий этого набора: `workPath/infobases/<база>/dump-info/<набор>`.
    /// Копия описывает пару «база ↔ каталог», поэтому есть только у набора с памятью базы.
    pub fn version_file_copy_dir(&self, work_path: &Path) -> Option<PathBuf> {
        self.source_set_base().map(|base| {
            infobase_memory_dir(work_path, base)
                .join("dump-info")
                .join(&self.name)
        })
    }

    /// Журнал поколений базы этого набора: `workPath/infobases/<база>/generation.json`.
    pub fn generation_file(&self, work_path: &Path) -> Option<PathBuf> {
        self.base_memory_dir(work_path)
            .map(|dir| dir.join(GENERATION_FILE_NAME))
    }

    /// Память базы этого набора: `workPath/infobases/<база>`; `None` у набора без памяти базы.
    pub fn base_memory_dir(&self, work_path: &Path) -> Option<PathBuf> {
        self.source_set_base()
            .map(|base| infobase_memory_dir(work_path, base))
    }

    fn source_set_base(&self) -> Option<&str> {
        match &self.memory {
            SnapshotMemory::Infobase {
                base,
                subject: MemorySubject::SourceSet,
                ..
            } => Some(base),
            SnapshotMemory::Infobase { .. } | SnapshotMemory::Shared | SnapshotMemory::Disabled => {
                None
            }
        }
    }
}

/// Имя журнала поколений под каталогом базы.
pub const GENERATION_FILE_NAME: &str = "generation.json";

fn infobase_memory(infobase: &str, identity: String, subject: MemorySubject) -> SnapshotMemory {
    assert!(
        is_safe_path_segment(infobase),
        "infobase memory key must be a safe path segment"
    );
    SnapshotMemory::Infobase {
        base: infobase.to_owned(),
        identity,
        subject,
    }
}

/// Корень памяти о базах: `workPath/infobases`. Всё под ним — состояние раннера.
pub fn infobases_dir(work_path: &Path) -> PathBuf {
    work_path.join("infobases")
}

/// Память об одной базе: `workPath/infobases/<ключ>`, где ключ — имя объявленной базы или
/// [`connection_memory_key`] базы, названной строкой соединения.
pub fn infobase_memory_dir(work_path: &Path, infobase: &str) -> PathBuf {
    infobases_dir(work_path).join(infobase)
}

/// Снимок Конфигуратора набора формата EDT. Он описывает обмен с базой и лежит под её
/// памятью: `workPath/infobases/<ключ>/designer/<набор>`. У набора без памяти базы —
/// внешних обработок и отчётов — `workPath/designer/<набор>`.
pub fn designer_copy_dir(work_path: &Path, infobase: Option<&str>, source_set: &str) -> PathBuf {
    match infobase {
        Some(infobase) => infobase_memory_dir(work_path, infobase),
        None => work_path.to_path_buf(),
    }
    .join("designer")
    .join(source_set)
}

/// Ключ каталога памяти базы, названной строкой соединения: `@` и начало SHA-256
/// нормализованного адреса без учётных данных.
///
/// Имя объявленной базы начинается с буквы или цифры (`INFOBASE_NAME_PATTERN`), поэтому
/// с ним такой ключ не совпадает. Совпадение ключей разных адресов памятью не делится:
/// рядом с памятью лежит полный адрес, и чужая память не используется.
pub fn connection_memory_key(address: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    const KEY_BYTES: usize = 16;
    let digest = Sha256::digest(address.as_bytes());
    let mut key = String::with_capacity(1 + KEY_BYTES * 2);
    key.push('@');
    for byte in &digest[..KEY_BYTES] {
        // Запись в `String` не отказывает.
        let _ = write!(key, "{byte:02x}");
    }
    key
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
