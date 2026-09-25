use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::ArtifactBuildMode;
use crate::domain::execution::ExecutionOutcome;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoadMode {
    Load,
    /// Объединение по файлу настроек. Прежнее имя режима — `merge`, оно принимается
    /// ещё один цикл выпуска, а ответ называет режим новым именем.
    #[serde(rename = "combine", alias = "merge")]
    Merge,
    Update,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoadTargetKind {
    Unknown,
    Configuration,
    Extension,
}

/// What the compatibility probe established, and nothing more.
///
/// The probe asks the platform to compare the target with its counterpart, and the only part
/// of the answer the platform guarantees is whether the comparison ran: exit code zero, and
/// exactly then a comparison report appears. Why it did not run is said in prose, and prose is
/// not a contract — see INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS — so this enum has no variant for a reason.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityState {
    /// The comparison ran, so both sides exist: a configuration is on support, an extension
    /// is installed and comparable.
    Supported,
    /// Proven absent from the infobase, by the infobase's own extension list — keyed output,
    /// not a sentence. A first installation is exactly this state; a merge has nothing to
    /// merge into.
    Absent,
    /// The comparison did not run, and the runner does not guess why. Never permits a merge,
    /// never blocks a first installation.
    NotEstablished,
    /// Nobody asked. The run is a preview, or the target cannot be named — a configuration
    /// needs the vendor configuration's name before the platform will compare it.
    NotProbed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct LoadExecutionMetadata {
    pub applied: bool,
    pub target_kind: LoadTargetKind,
    pub compatibility_state: CompatibilityState,
    pub update_db_cfg_ran: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LoadResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    /// Whether an executor got this command's work: a process was started to do it, or the
    /// request's command was handed to a running session. Starting or opening a session and
    /// its own service commands are not work. `false` whenever the executor got none — a
    /// preview, a refusal or interruption before any work, a run with nothing to do, or a
    /// process that could not be started.
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
            provider: None,
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
                    compatibility_state: CompatibilityState::NotEstablished,
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
