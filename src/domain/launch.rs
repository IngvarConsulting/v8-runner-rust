use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Structured result of a `launch` command.
///
/// One wait answers in one field: `external_epf_wait` for a wait on an
/// external EPF, `client_wait` for a wait without one; never both.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(extend("not" = {"required": ["external_epf_wait", "client_wait"]}))]
pub struct LaunchResult {
    /// `true` when the process was spawned and, when the command waits on it, the wait
    /// succeeded: under `--wait-for-exit` the client exited on its own, under `--wait-ready`
    /// its MCP endpoint became ready. A timeout, an interrupted wait or a failed readiness
    /// check answers `false` although the client was started. A preview answers `true`
    /// without starting anything: `provider_dispatched` tells the two apart.
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
    /// for `launch web`. `false` in a preview. A refusal before the start, or a start that
    /// failed, answers the shared refusal form, without this field; a failure after the
    /// start answers this form with `true`.
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
    /// Bounded thin-client wait without an external EPF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_wait: Option<Box<ClientWaitResult>>,
}

/// Which of the target's two addresses opens the base.
///
/// A target has an administrative address and a client one, and a client can be opened by
/// either. The connection string is the default, the client address when no connection
/// string is declared; `--via` overrides it.
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

/// Legacy EPF wire projection; execute_path remains required and non-null.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExternalEpfWaitResult {
    pub execute_path: String,
    #[serde(flatten)]
    pub wait: ClientWaitResult,
}

/// Observed outcome shared by both bounded thin-client launch forms.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientWaitResult {
    /// Process identifier of the waited client.
    pub pid: u32,
    /// The client's exit code; `null` when the wait ended without one: the client was
    /// terminated at the timeout (`timed_out: true`), or the wait was interrupted before the
    /// client exited (`timed_out: false`).
    pub exit_code: Option<i32>,
    /// `true` when the client was terminated at the declared timeout.
    pub timed_out: bool,
    /// Where the client was asked to write its output.
    pub output_path: String,
    /// Where the client's stderr was captured.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_epf_wait_requires_its_string_identity_and_keeps_flat_shape() {
        let old = json!({"execute_path":"/tmp/check.epf","pid":42,"exit_code":7,
            "timed_out":false,"output_path":"out","stderr_path":"err"});
        let parsed: ExternalEpfWaitResult = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), old);
        for path in [None, Some(serde_json::Value::Null)] {
            let mut malformed = old.clone();
            malformed.as_object_mut().unwrap().remove("execute_path");
            if let Some(path) = path {
                malformed["execute_path"] = path;
            }
            assert!(serde_json::from_value::<ExternalEpfWaitResult>(malformed).is_err());
        }
        let schema = serde_json::to_value(schemars::schema_for!(ExternalEpfWaitResult)).unwrap();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("execute_path")));
        assert_eq!(schema["properties"]["execute_path"]["type"], "string");
        for field in ["pid", "timed_out", "output_path", "stderr_path"] {
            assert!(
                schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(field)),
                "{schema}"
            );
        }
    }
}
