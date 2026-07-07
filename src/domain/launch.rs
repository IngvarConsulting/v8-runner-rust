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
    /// Human-readable launch summary.
    pub message: Option<String>,
    /// Client-side MCP endpoint readiness details when readiness was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_readiness: Option<McpReadinessResult>,
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
