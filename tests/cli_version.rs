mod support;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command};

#[test]
fn version_command_prints_application_version_without_config() {
    let workspace = temp_workspace();

    let output = v8_runner_command()
        .current_dir(workspace.path())
        .args(["version"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("v8-runner {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn version_command_supports_json_output_without_config() {
    let workspace = temp_workspace();

    let output = v8_runner_command()
        .current_dir(workspace.path())
        .args(["--json-message", "version"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "version");
    assert_eq!(payload["data"]["name"], "v8-runner");
    assert_eq!(payload["data"]["version"], env!("CARGO_PKG_VERSION"));
}
