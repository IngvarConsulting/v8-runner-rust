use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildResult {
    pub ok: bool,
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    ///
    /// Always present. In a preview each step's `mode` is the mode that would be used and
    /// its message says so; the flag is what separates planned from performed.
    pub provider_dispatched: bool,
    pub steps: Vec<BuildStep>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildStep {
    pub source_set: String,
    pub mode: BuildMode,
    pub ok: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
    /// Recovery of this step's Designer version file, when a snapshot was captured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdfi_recovery: Option<CdfiRecoverySummary>,
}

/// Byte recovery is independent of the best-effort metadata change count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CdfiRecoverySummary {
    pub tracked_path: std::path::PathBuf,
    pub original_existed: bool,
    /// Unknown if either version cannot be interpreted as CDFI metadata.
    pub changed_entry_count: Option<usize>,
    pub action: CdfiRecoveryAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CdfiRecoveryAction {
    NotNeeded,
    Restored,
    RemovedCreatedFile,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildMode {
    EdtExport,
    Full,
    Partial { file_count: usize },
    Skipped,
}
