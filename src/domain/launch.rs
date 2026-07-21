use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Structured result of a `launch` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchResult {
    /// `true` when the process was spawned successfully.
    pub ok: bool,
    /// Requested launch mode.
    pub mode: LaunchMode,
    /// OS process identifier if the launcher exposed one.
    pub pid: Option<u32>,
    /// Selected binary path used to spawn the process.
    pub binary: PathBuf,
    /// Canonical platform installation metadata for the selected binary.
    pub platform_resolution: PlatformResolution,
    /// Human-readable launch summary.
    pub message: Option<String>,
    /// Client-side MCP endpoint readiness details when readiness was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_readiness: Option<McpReadinessResult>,
}

/// Canonical platform installation metadata exposed by `launch` JSON results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformResolution {
    /// Absolute canonical path to the selected executable.
    pub path: PathBuf,
    /// Platform version inferred from the canonical installation path, when known.
    pub version: Option<String>,
    /// Discovery source used for the selected executable.
    pub source: PlatformResolutionSource,
    /// Absolute canonical root shared by platform utilities from this installation.
    pub installation_root: PathBuf,
}

/// Typed discovery sources exposed by `launch` resolution metadata.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformResolutionSource {
    /// The configured utility or installation hint.
    Explicit,
    /// An operating-system-specific default installation root.
    DefaultRoot,
    /// A directory captured from `PATH` when the locator was created.
    Path,
}

/// Result of probing a client-side MCP endpoint after launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpReadinessResult {
    /// `true` when initialize and tools/list succeeded and required tools were present.
    pub ok: bool,
    /// Probed HTTP endpoint URL.
    pub url: String,
    /// Tool names returned by `tools/list`.
    pub tools: Vec<String>,
    /// Required tool names that were not returned by `tools/list`.
    pub missing_tools: Vec<String>,
    /// Human-readable readiness summary.
    pub message: Option<String>,
}

/// Supported application launch modes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    Designer,
    Thin,
    Thick,
    Ordinary,
    Mcp,
}
