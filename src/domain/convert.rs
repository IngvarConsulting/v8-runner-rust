use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConvertDirection {
    EdtToDesigner,
    DesignerToEdt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConvertScope {
    All,
    Single,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConvertOutput {
    pub source_set: String,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConvertResult {
    pub ok: bool,
    /// Whether the EDT CLI got this command's work: a process was started to do it, or the
    /// request's command was handed to a running EDT session. Starting the session and its
    /// own service commands are not work. `false` whenever it got none — a preview, a refusal
    /// or interruption before any work, or a process that could not be started.
    ///
    /// Always present, so an absent field never has to be read as "no work was given". In a
    /// preview `outputs` names what would be written.
    pub provider_dispatched: bool,
    pub direction: ConvertDirection,
    pub scope: ConvertScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_set: Option<String>,
    pub workspace_path: PathBuf,
    pub outputs: Vec<ConvertOutput>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
