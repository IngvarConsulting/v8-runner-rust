use crate::domain::capability::ProviderReceipt;
use crate::domain::execution::{ExecutionInterruptionDetails, ExecutionStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Receipt for explicitly applying or discarding an unapplied configuration.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfigurationTransitionResult {
    pub duration_ms: u64,
    pub dry_run: bool,
    pub extension: Option<String>,
    /// None means process execution was attempted but its dispatch could not be established.
    pub provider_dispatched: Option<bool>,
    pub completed: bool,
    pub status: ExecutionStatus,
    pub provider: Option<ProviderReceipt>,
    pub platform_log_path: Option<PathBuf>,
    pub interruption: Option<ExecutionInterruptionDetails>,
    pub warnings: Vec<String>,
}
