use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Structured result of a `launch` command.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LaunchResult {
    /// `true` when the process was spawned successfully.
    pub ok: bool,
    /// Requested launch mode.
    pub mode: LaunchMode,
    /// OS process identifier if the launcher exposed one.
    pub pid: Option<u32>,
    /// Selected binary path used to spawn the process; for `web` — the system URL opener.
    pub binary: PathBuf,
    /// Canonical platform installation metadata for the selected binary. Absent for
    /// `web`: a browser is not a platform utility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_resolution: Option<PlatformResolution>,
    /// Which address the base was opened by. Present for every mode, not only for the
    /// ones that have a choice: no caller has to read an absent field as "by connection".
    pub via: LaunchVia,
    /// Client address opened by `launch web` or by a thin client going through the web.
    /// Any userinfo password in it is masked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether the program in `binary` was started: the client, or the system URL opener
    /// for `launch web`. `false` in a preview; a refusal or a start that failed answers the
    /// shared refusal form, without this field.
    pub provider_dispatched: bool,
    /// Compact machine-facing plan produced by a non-executing preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<LaunchPlan>,
    /// Human-readable launch summary.
    pub message: Option<String>,
    /// Client-side MCP endpoint readiness details when readiness was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_readiness: Option<McpReadinessResult>,
    /// Direct external EPF wait outcome when explicitly requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_epf_wait: Option<ExternalEpfWaitResult>,
}

/// Which of the target's two addresses opens the base.
///
/// A target has an administrative address and a client one, and a client can be opened by
/// either. The target kind sets the default; `--via` overrides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LaunchVia {
    /// The administrative address — `infobase.connection`.
    Connection,
    /// The client address — `infobase.web.url`, as a ws connection.
    Web,
}

impl LaunchVia {
    /// Парсит объявленный адрес. Словарь один на обе поверхности: разойдись CLI и MCP
    /// в значениях, они разошлись бы и в поведении.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "web" => Some(Self::Web),
            "connection" => Some(Self::Connection),
            _ => None,
        }
    }
}

/// Compact machine-facing plan produced by a non-executing launch preview.
///
/// The plan names what the runner selected, because only the runner discovers a
/// platform installation: a caller cannot compose these arguments itself.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LaunchPlan {
    /// Program the runner would have spawned.
    pub program: PathBuf,
    /// Composed arguments with every credential value masked.
    pub args: Vec<String>,
}

/// Observed outcome of an opt-in bounded external EPF client launch.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExternalEpfWaitResult {
    pub pid: u32,
    pub execute_path: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_path: String,
    pub stderr_path: String,
}

/// Canonical platform installation metadata exposed by `launch` JSON results.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    Designer,
    Thin,
    Thick,
    Ordinary,
    Mcp,
    Web,
}
