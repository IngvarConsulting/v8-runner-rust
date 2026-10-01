use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct BuildResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// Whether an executor got this command's work: a process was started to do it, or the
    /// request's command was handed to a running session. Starting or opening a session and
    /// its own service commands are not work. `false` whenever the executor got none — a
    /// preview, a refusal or interruption before any work, a run with nothing to do, or a
    /// process that could not be started.
    ///
    /// Always present. In a preview each step's `mode` is the mode that would be used and
    /// its message says so.
    pub provider_dispatched: bool,
    pub steps: Vec<BuildStep>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct BuildStep {
    pub source_set: String,
    pub mode: BuildMode,
    pub ok: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BuildMode {
    EdtExport,
    Full,
    Partial { file_count: usize },
    Skipped,
}
