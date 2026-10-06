use serde::{Deserialize, Serialize};

/// Ответ `init`. Различитель `kind` говорит, что записано: проектный файл вместе с
/// местным слоем или, в уже объявленном проекте, только местный слой. Поля проектного
/// файла есть только у первого варианта: у второго им нечего описывать.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigInitResult {
    /// Записан проектный файл: каталог был без него или его переписал `--force`.
    Project(ProjectInitResult),
    /// Проектный файл уже был и остался нетронутым: записан только местный слой.
    Local(LocalLayerInitResult),
}

impl ConfigInitResult {
    pub fn duration_ms(&self) -> u64 {
        match self {
            Self::Project(result) => result.duration_ms,
            Self::Local(result) => result.duration_ms,
        }
    }

    pub fn warnings(&self) -> &[String] {
        match self {
            Self::Project(result) => &result.warnings,
            Self::Local(_) => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ProjectInitResult {
    pub ok: bool,
    pub path: String,
    pub local_path: String,
    pub gitignore_path: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_version: Option<String>,
    pub source_sets: Vec<ConfigInitSourceSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub overwritten: bool,
    pub origin: OriginDeclaration,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct LocalLayerInitResult {
    pub ok: bool,
    pub local_path: String,
    pub gitignore_path: String,
    pub origin: OriginDeclaration,
    pub duration_ms: u64,
}

/// Что `init` сделал с `infobases.origin` местного слоя. Пароль в адресах замаскирован;
/// учётные данные секций в ответ не попадают.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct OriginDeclaration {
    pub change: OriginChange,
    /// Адрес, который стоит в `origin` после команды. Нет у автономного сервера.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Заменённый адрес: прежняя секция `origin` теперь лежит под именем `upstream`.
    /// Есть только при `change: redirected` и только у прежней секции с адресом.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OriginChange {
    /// `origin` не было или у него не было адреса: адрес записан.
    Declared,
    /// `origin` уже объявлен тем же адресом или адрес не назван: секция не менялась.
    Unchanged,
    /// `origin` получил новый адрес, прежняя секция сохранена под именем `upstream`.
    Redirected,
}

impl OriginChange {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Unchanged => "unchanged",
            Self::Redirected => "redirected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ConfigInitSourceSet {
    pub name: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub path: String,
}
