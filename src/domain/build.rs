use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildResult {
    pub ok: bool,
    pub steps: Vec<BuildStep>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdfi_recovery: Option<CdfiRecoverySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildStep {
    pub source_set: String,
    pub mode: BuildMode,
    pub ok: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildMode {
    EdtExport,
    Full,
    Partial { file_count: usize },
    Skipped,
}

/// Diagnostics emitted when a failed Designer build needed to protect CDFI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CdfiRecoverySummary {
    pub action: CdfiRecoveryAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// The outcome of attempting CDFI recovery after a failed Designer build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CdfiRecoveryAction {
    RestoredOriginal,
    RemovedCreatedFile,
    RestoreFailed,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{BuildResult, CdfiRecoveryAction, CdfiRecoverySummary};

    #[test]
    fn build_result_serializes_retained_cdfi_recovery_failure() {
        let result = BuildResult {
            ok: false,
            steps: vec![],
            duration_ms: 42,
            cdfi_recovery: Some(CdfiRecoverySummary {
                action: CdfiRecoveryAction::RestoreFailed,
                snapshot_path: Some(PathBuf::from("/work/cdfi-recovery-42/ConfigDumpInfo.xml")),
                failure: Some("permission denied while restoring CDFI".to_owned()),
            }),
        };

        assert_eq!(
            serde_json::to_value(result).expect("serialize build result"),
            json!({
                "ok": false,
                "steps": [],
                "duration_ms": 42,
                "cdfi_recovery": {
                    "action": "restore_failed",
                    "snapshot_path": "/work/cdfi-recovery-42/ConfigDumpInfo.xml",
                    "failure": "permission denied while restoring CDFI",
                },
            })
        );
    }
}
