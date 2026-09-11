use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionsResult {
    pub ok: bool,
    pub steps: Vec<ExtensionsStep>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionsStep {
    pub target: String,
    pub action: String,
    pub ok: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
}

/// One extension installed in the target infobase, as reported by the platform.
///
/// Field set mirrors `ibcmd config extension list` on 8.3.27 exactly. The name
/// prefix is deliberately absent: the platform does not report it on read, it lives
/// only in the extension's own `Configuration.xml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledExtension {
    pub name: String,
    /// `None` when the platform reported the field empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub active: bool,
    pub purpose: String,
    pub safe_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_profile_name: Option<String>,
    pub unsafe_action_protection: bool,
    pub used_in_distributed_infobase: bool,
    pub scope: String,
    pub hash_sum: String,
}

/// Result of reading the extension composition of an infobase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionInventoryResult {
    pub ok: bool,
    /// Extensions in the order the platform reported them.
    ///
    /// That order is not the creation order and the platform does not document one,
    /// so callers must not rely on it.
    pub extensions: Vec<InstalledExtension>,
    pub duration_ms: u64,
}
