use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExtensionsResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    pub provider_dispatched: bool,
    pub steps: Vec<ExtensionsStep>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExtensionsStep {
    pub target: String,
    pub action: String,
    pub ok: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
}

/// One extension installed in the target infobase, as reported by the platform.
///
/// `ibcmd` reads the applied name prefix from a saved DB configuration.
/// The agent provider cannot attest that property and reports `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InstalledExtension {
    pub name: String,
    /// `None` when the platform reported the field empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `None` means the selected provider cannot attest the applied prefix;
    /// `Some("")` is a known empty prefix.
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub name_prefix: Option<String>,
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

/// What the read was asked for, named as data.
///
/// A change preview names its `target` and `action` as fields; the read names its
/// subject the same way, so a caller pairs the answer with its request without
/// parsing the wording of `plan`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RequestedInventory {
    /// Every extension installed in the infobase.
    All,
    /// One extension by its platform name.
    Named { name: String },
}

/// Result of reading the extension composition of an infobase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExtensionInventoryResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// `false` when the run stopped at a preview instead of dispatching the platform.
    ///
    /// Reading the composition is an action, not a look: the platform starts, a session
    /// opens, the account authenticates and a journal trace is left. So the read has a
    /// preview too, and in it `extensions` is empty because nothing was asked.
    pub provider_dispatched: bool,
    /// The subject of the read, present in the preview and in the answer alike.
    pub requested: RequestedInventory,
    /// What the apply would do, named without any secret from the connection string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Extensions in the order the platform reported them.
    ///
    /// That order is not the creation order and the platform does not document one,
    /// so callers must not rely on it.
    pub extensions: Vec<InstalledExtension>,
    pub duration_ms: u64,
}
