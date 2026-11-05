use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::capability::ProviderReceipt;

/// Configuration compared with the working configuration by Designer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffAgainst {
    Database,
    Vendor(String),
    File(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DiffResult {
    pub provider: ProviderReceipt,
    pub provider_dispatched: bool,
    pub changes: Vec<MetadataChange>,
    /// The platform needs a full dump and cannot enumerate individual changes.
    pub full_dump: bool,
    /// Absolute path to the original platform report when `--against` succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<PathBuf>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MetadataChange {
    pub object: String,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
}
