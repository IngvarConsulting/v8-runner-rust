//! Установленные платформы и выбор раннера: ответ `platforms`.
//!
//! Команда платформу не запускает и ничего не пишет: она называет установки, которые видит
//! поиск, формат исходников проекта и утилиты, которые раннер выбрал бы для этого проекта.
//! Форма закреплена `CTR.WIRE.PLATFORMS-DATA`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::launch::PlatformResolution;

/// Ответ `platforms`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlatformsResult {
    /// Установки, которые видит поиск, новейшие первыми. Перечень полный.
    pub installations: Vec<PlatformInstallation>,
    /// Что о платформе сказано в настройках проекта (`tools.platform`).
    pub requirement: PlatformRequirement,
    /// Формат исходников основного набора; `null`, если формат проекта не Конфигуратора или
    /// `Configuration.xml` основного набора нет.
    pub source_format: Option<SourceFormatInfo>,
    /// Утилиты, которые раннер выбрал бы для этого проекта.
    pub selected: SelectedPlatforms,
    pub duration_ms: u64,
}

/// Установка платформы: версия, каталог и состав.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlatformInstallation {
    pub version: String,
    /// Каталог версии.
    pub root: PathBuf,
    pub components: InstalledComponents,
}

/// Какие компоненты есть в установке.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InstalledComponents {
    /// Конфигуратор и толстый клиент (`1cv8`).
    pub designer: bool,
    /// Тонкий клиент (`1cv8c`).
    pub thin_client: bool,
    /// Автономный сервер (`ibcmd`).
    pub ibcmd: bool,
    /// Расширение веб-сервера (`webinst`).
    pub web_extension: bool,
}

/// `tools.platform` проекта вместе с местным слоем.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlatformRequirement {
    /// Маска версии; `null` — не задана.
    pub version: Option<String>,
    /// Путь к установке или утилите; `null` — не задан.
    pub path: Option<PathBuf>,
    pub strict: bool,
}

/// Формат исходников проекта.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SourceFormatInfo {
    /// Версия формата; `null`, если файл есть, а версии в его корне нет.
    pub version: Option<String>,
    /// Прочитанный `Configuration.xml`.
    pub file: PathBuf,
    /// Выпуск платформы, который пишет этот формат; `null`, если раннер такого не знает.
    pub release: Option<String>,
}

/// Выбор по утилитам: раннер ищет их по отдельности. `null` — утилиты нет ни в одной
/// подходящей установке.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SelectedPlatforms {
    pub designer: Option<SelectedPlatform>,
    pub thin_client: Option<SelectedPlatform>,
    pub ibcmd: Option<SelectedPlatform>,
}

/// Выбранная утилита и почему выбрана её версия.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SelectedPlatform {
    #[serde(flatten)]
    pub resolution: PlatformResolution,
    pub chosen_by: PlatformChoice,
}

/// Почему выбрана эта версия платформы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformChoice {
    /// Маска `tools.platform.version`.
    Version,
    /// Выпуск, который пишет формат исходников проекта.
    SourceFormat,
    /// Новейшая из подходящих: версию ничто не задало.
    Newest,
}
