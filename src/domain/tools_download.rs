use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExtensionInstallMode {
    Sources,
    Artifacts,
}

/// Какой выпуск инструмента брать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolReleaseChannel {
    /// Выпуск, который GitHub отдаёт как `releases/latest`; pre-release туда не попадает.
    Latest,
    /// Наибольшая версия среди всех опубликованных выпусков, pre-release тоже.
    NewestIncludingPrerelease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDownloadTarget {
    Yaxunit,
    VanessaAutomationSingle,
    ClientMcp,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ToolsDownloadResult {
    pub ok: bool,
    pub tool: String,
    pub mode: String,
    pub destinations: Vec<ToolDownloadDestination>,
    pub config_path: PathBuf,
    pub local_config_path: PathBuf,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ToolDownloadDestination {
    pub tool: String,
    pub tag: String,
    pub source: String,
    pub path: PathBuf,
    pub config: String,
}
