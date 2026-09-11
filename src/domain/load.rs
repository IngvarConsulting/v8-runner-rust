use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::ArtifactBuildMode;
use crate::domain::execution::ExecutionOutcome;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadMode {
    Load,
    Merge,
    Update,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadTargetKind {
    Unknown,
    Configuration,
    Extension,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityState {
    Absent,
    Supported,
    NotSupported,
    Unknown,
    /// The compatibility probe was deliberately not run, because the run is a preview.
    ///
    /// Distinct from `Unknown`, which means the probe ran or was attempted and its answer
    /// could not be established: "not asked" and "asked without an answer" are different
    /// facts for a caller deciding whether to apply.
    NotProbed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadExecutionMetadata {
    pub applied: bool,
    pub target_kind: LoadTargetKind,
    pub compatibility_state: CompatibilityState,
    pub update_db_cfg_ran: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadResult {
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    pub provider_dispatched: bool,
    pub mode: LoadMode,
    pub artifact_path: PathBuf,
    pub artifact_type: ArtifactBuildMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    pub duration_ms: u64,
    pub execution: ExecutionOutcome<LoadExecutionMetadata>,
}

#[cfg(test)]
mod tests {
    use super::{CompatibilityState, LoadExecutionMetadata, LoadMode, LoadResult, LoadTargetKind};
    use crate::domain::artifacts::ArtifactBuildMode;
    use crate::domain::execution::{ExecutionOutcome, ExecutionStatus};
    use std::path::PathBuf;

    #[test]
    fn load_result_serializes_canonical_execution_without_legacy_fields() {
        let result = LoadResult {
            provider_dispatched: true,
            mode: LoadMode::Load,
            artifact_path: PathBuf::from("/tmp/main.cf"),
            artifact_type: ArtifactBuildMode::ConfigurationCf,
            extension: None,
            duration_ms: 10,
            execution: ExecutionOutcome::new(ExecutionStatus::Succeeded).with_payload(
                LoadExecutionMetadata {
                    applied: true,
                    target_kind: LoadTargetKind::Configuration,
                    compatibility_state: CompatibilityState::NotSupported,
                    update_db_cfg_ran: true,
                },
            ),
        };

        let value = serde_json::to_value(result).expect("json");
        assert!(value.get("ok").is_none());
        assert!(value.get("target_kind").is_none());
        assert!(value.get("compatibility_state").is_none());
        assert!(value.get("platform_log_path").is_none());
        assert!(value.get("message").is_none());
        assert!(value.get("execution").is_some());
    }
}
