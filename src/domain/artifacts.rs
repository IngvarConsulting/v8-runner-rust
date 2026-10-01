use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::artifact::ArtifactSet;
use crate::domain::execution::ExecutionOutcome;

pub const CF_RUNNER_ID: &str = "designer-cf";
pub const CFE_RUNNER_ID: &str = "designer-cfe";
pub const EPF_RUNNER_ID: &str = "designer-epf";
pub const ERF_RUNNER_ID: &str = "designer-erf";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactBuildMode {
    Unknown,
    ConfigurationCf,
    ExtensionCfe,
    ExternalDataProcessorEpf,
    ExternalReportErf,
}

impl ArtifactBuildMode {
    pub const fn file_extension(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::ConfigurationCf => "cf",
            Self::ExtensionCfe => "cfe",
            Self::ExternalDataProcessorEpf => "epf",
            Self::ExternalReportErf => "erf",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ArtifactBuildMetadata {
    pub artifact_type: ArtifactBuildMode,
    pub output_path: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_names: Vec<String>,
    pub published: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArtifactsResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    /// Whether an executor got this command's work: a process was started to do it, or the
    /// request's command was handed to a running session. Starting or opening a session and
    /// its own service commands are not work. `false` whenever the executor got none — a
    /// preview, a refusal or interruption before any work, a run with nothing to do, or a
    /// process that could not be started.
    pub provider_dispatched: bool,
    pub mode: ArtifactBuildMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    pub duration_ms: u64,
    pub execution: ExecutionOutcome<ArtifactBuildMetadata>,
}

impl ArtifactSet {
    pub fn is_empty(&self) -> bool {
        self.root_dir.is_none() && self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactBuildMetadata, ArtifactBuildMode, ArtifactsResult};
    use crate::domain::execution::{ExecutionOutcome, ExecutionStatus};
    use std::path::PathBuf;

    #[test]
    fn artifacts_result_serializes_canonical_execution_without_legacy_fields() {
        let result = ArtifactsResult {
            provider: None,
            provider_dispatched: true,
            mode: ArtifactBuildMode::ConfigurationCf,
            source_set: Some("main".to_owned()),
            extension: None,
            duration_ms: 10,
            execution: ExecutionOutcome::new(ExecutionStatus::Succeeded).with_payload(
                ArtifactBuildMetadata {
                    artifact_type: ArtifactBuildMode::ConfigurationCf,
                    output_path: PathBuf::from("/tmp/out.cf"),
                    file_names: vec!["out.cf".to_owned()],
                    published: true,
                },
            ),
        };

        let value = serde_json::to_value(result).expect("json");
        assert!(value.get("ok").is_none());
        assert!(value.get("output_path").is_none());
        assert!(value.get("platform_log_path").is_none());
        assert!(value.get("artifacts").is_none());
        assert!(value.get("message").is_none());
        assert!(value.get("execution").is_some());
    }
}
