use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InitResult {
    pub ok: bool,
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    ///
    /// Always present, so an absent field never has to be read as "nothing ran".
    pub provider_dispatched: bool,
    pub steps: Vec<InitStep>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InitStep {
    pub target: String,
    pub action: String,
    pub status: InitStepStatus,
    pub message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InitStepStatus {
    Ok,
    Skipped,
    Failed,
    /// The step was decided but not performed, because the run is a preview.
    Planned,
}
