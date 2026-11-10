//! Ответ git о рабочей копии под корнями наборов исходников, уже разобранный (#575).
//!
//! Разбирает его `platform::git`, читает поиск изменений: сам он процессов не запускает, а
//! сценарий передаёт ему этот ответ. Пути — от корня рабочей копии, через `/`, как их пишет git.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Запись индекса git под корнями наборов: файл нулевой ступени или ступень конфликта.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitIndexEntry {
    pub path: String,
    /// Вид из индекса: `100644`, `100755`, `120000` — ссылка, `160000` — подмодуль.
    pub mode: u32,
    /// Хеш blob в индексе.
    pub oid: String,
    /// `0` — обычная запись, `1..=3` — ступени незавершённого слияния.
    pub stage: u8,
    /// `assume-unchanged` или `skip-worktree`: правку рабочего дерева git о таком файле не
    /// сообщает, и раннер читает его сам.
    pub flagged: bool,
}

impl GitIndexEntry {
    /// Вид ссылки в индексе.
    pub const SYMLINK_MODE: u32 = 0o120000;
    /// Вид подмодуля в индексе.
    pub const GITLINK_MODE: u32 = 0o160000;
}

/// Файл правил игнорирования git, который действует под корнями наборов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitIgnoreSource {
    /// Где файл лежит на диске.
    pub file: PathBuf,
    /// Каталог, от которого считаются его правила: каталог `.gitignore` от корня рабочей
    /// копии, пустая строка — корень, у `info/exclude` и `core.excludesFile` тоже корень.
    pub base: String,
}

/// Ответ git об одной рабочей копии.
#[derive(Debug, Clone, Default)]
pub struct GitWorktree {
    /// Корень рабочей копии.
    pub toplevel: PathBuf,
    /// В репозитории включён `core.fsmonitor`: git узнаёт об изменениях от демона.
    pub fsmonitor: bool,
    /// `core.ignorecase`: имена путей сравниваются без учёта регистра.
    pub ignore_case: bool,
    /// Репозиторий хранит объекты хешами SHA-256 (`extensions.objectFormat`).
    pub sha256: bool,
    /// Записи индекса под корнями наборов.
    pub index: Vec<GitIndexEntry>,
    /// Пути, у которых рабочее дерево разошлось с индексом: правка, удаление, смена вида,
    /// незавершённое слияние, правка внутри подмодуля.
    pub worktree_changed: HashSet<String>,
    /// Файлы вне индекса. Путь с `/` на конце — вложенный репозиторий: внутрь git не смотрит.
    pub untracked: Vec<String>,
    /// Файлы правил игнорирования в порядке возрастания силы: `core.excludesFile`,
    /// `.git/info/exclude`, затем отслеживаемые `.gitignore` от мелких каталогов к глубоким.
    pub ignore_sources: Vec<GitIgnoreSource>,
}

/// Каким путём найдены изменения набора (`INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub enum DetectionPath {
    /// Изменения назвал git.
    #[serde(rename = "git")]
    Git,
    /// Изменения назвал git, которому о файлах сообщает демон `core.fsmonitor`.
    #[serde(rename = "git+fsmonitor")]
    GitFsmonitor,
    /// Раннер прочёл каждый файл набора сам.
    #[serde(rename = "scan")]
    Scan,
}

/// Почему изменения набора найдены чтением файлов, а не у git.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanReason {
    /// Набор не лежит в рабочей копии git, или git не найден.
    NoRepository,
    /// Набор лежит в подмодуле.
    Submodule,
    /// Внутри набора вложенный репозиторий или подмодуль: его файлы git не перечисляет.
    NestedRepository,
    /// Git отказал или его ответ неполон.
    GitFailed,
    /// Репозиторий хранит объекты хешами SHA-256, а раннер считает хеши git только SHA-1.
    Sha256Repository,
    /// Память записана хешами байтов (например, до того, как каталог стал рабочей копией
    /// git): её сравнивает тот же хешер, чтением файлов, пока полная выгрузка или отправка не
    /// запишет хеши git.
    BytesMemory,
    /// Исходники формата EDT: набор, который грузится в базу, — копия под `workPath`.
    EdtFormat,
}

/// Путь поиска изменений набора и, у чтения файлов, его причина.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ChangeDetection {
    pub path: DetectionPath,
    /// Причина пути `scan`; у git её нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ScanReason>,
}

impl ChangeDetection {
    pub const fn scan(reason: ScanReason) -> Self {
        Self {
            path: DetectionPath::Scan,
            reason: Some(reason),
        }
    }

    pub const fn git(fsmonitor: bool) -> Self {
        Self {
            path: if fsmonitor {
                DetectionPath::GitFsmonitor
            } else {
                DetectionPath::Git
            },
            reason: None,
        }
    }
}
