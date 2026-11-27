#![cfg(windows)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command};

fn command(dir: &Path) -> Command {
    let mut command = v8_runner_command();
    command
        .current_dir(dir)
        .env_remove("V8TR_CONFIG")
        .env("LOCALAPPDATA", dir.join("profile"))
        .arg("--json-message");
    command
}

fn install(root: &Path, version: &str, name: &str) -> PathBuf {
    let bin = root.join(version).join("bin");
    fs::create_dir_all(&bin).unwrap();
    let executable = bin.join(name);
    fs::write(&executable, []).unwrap();
    executable
}

fn canonical_display(path: &Path) -> String {
    let path = fs::canonicalize(path).expect("canonical test path");
    let text = path.to_str().expect("Unicode test path");
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(text).to_owned()
}

fn output_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}; exit: {}; stdout: {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn inventory(command: &mut Command) -> Value {
    let output = command.output().unwrap();
    let envelope = output_json(&output);
    assert!(output.status.success(), "{envelope}");
    assert_data_matches_its_command_form(&envelope, "platforms");
    envelope
}

#[test]
fn platforms_and_clone_discover_a_windows_user_installation_before_yaml_exists() {
    let dir = temp_workspace();
    let root = dir.path().join("profile").join("Programs").join("1cv8_x86");
    let version = "8.3.99.2222";
    let designer = install(&root, version, "1cv8.exe");
    let envelope = inventory(command(dir.path()).arg("platforms"));
    let data = &envelope["data"];
    assert!(data["source_format"].is_null());
    assert!(data["installations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| { entry["version"] == version && entry["components"]["designer"] == true }));

    let output = command(dir.path())
        .args([
            "--dry-run",
            "clone",
            "--from",
            "File=source",
            "--platform-version",
            version,
        ])
        .arg("--project-dir")
        .arg(dir.path().join("project"))
        .output()
        .unwrap();
    let preview = output_json(&output);
    assert!(output.status.success(), "{preview}");
    assert!(preview["data"]["message"]
        .as_str()
        .unwrap()
        .contains(&canonical_display(&designer)));
    assert!(!dir.path().join("project").exists());
    assert!(!dir.path().join("v8project.yaml").exists());
    assert!(!dir.path().join(".build").exists());
}

#[test]
fn a_windows_launcher_directory_leads_to_the_matching_versioned_utility() {
    let dir = temp_workspace();
    let root = dir.path().join("custom install");
    let common = root.join("common");
    fs::create_dir_all(&common).unwrap();
    fs::write(common.join("1cestart.exe"), []).unwrap();
    fs::create_dir(common.join("8.3.99.9876")).unwrap();
    let designer = install(&root, "8.3.99.1111", "1cv8.exe");
    let thin = install(&root, "8.3.99.2222", "1cv8c.exe");
    let outside = install(&dir.path().join("outside"), "8.3.99.3333", "ibcmd.exe");
    let envelope = inventory(
        command(dir.path())
            .env("PATH", outside.parent().unwrap())
            .args(["platforms", "--platform-path", "custom install/COMMON/"]),
    );
    let data = &envelope["data"];
    let selected = &data["selected"];
    assert_eq!(data["installations"].as_array().unwrap().len(), 2);
    assert_eq!(selected["designer"]["version"], "8.3.99.1111");
    assert_eq!(selected["designer"]["path"], canonical_display(&designer));
    assert_eq!(selected["thin_client"]["version"], "8.3.99.2222");
    assert_eq!(selected["thin_client"]["path"], canonical_display(&thin));
    assert!(selected["ibcmd"].is_null());
    assert!(envelope["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| { warning.as_str().unwrap().contains("ibcmd.exe") }));
    assert!(!dir.path().join("v8project.yaml").exists());
    assert!(!dir.path().join(".build").exists());

    let config = dir.path().join("v8project.yaml");
    fs::write(
        &config,
        "format: DESIGNER\ninfobase:\n  connection: File=source\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: custom install/common\n",
    )
    .unwrap();
    let configured = inventory(
        command(dir.path())
            .env("PATH", outside.parent().unwrap())
            .arg("platforms"),
    );
    let configured_installations = configured["data"]["installations"].as_array().unwrap();
    assert_eq!(configured_installations.len(), 1);
    assert_eq!(configured_installations[0]["version"], "8.3.99.9876");
    assert!(configured["data"]["selected"]["designer"].is_null());
    assert!(configured["data"]["selected"]["ibcmd"].is_null());

    let inside = install(&common, "8.3.99.4444", "1cv8.exe");
    let own_layout = inventory(
        command(dir.path())
            .args(["platforms", "--platform-path"])
            .arg(&common),
    );
    assert_eq!(
        own_layout["data"]["selected"]["designer"]["path"],
        canonical_display(&inside)
    );
    assert!(own_layout["data"]["selected"]["thin_client"].is_null());
}

#[test]
fn platforms_keeps_config_errors_and_names_an_empty_explicit_root() {
    let dir = temp_workspace();
    let root = dir.path().join("empty install");
    fs::create_dir(&root).unwrap();
    let root = canonical_display(&root);
    let envelope = inventory(
        command(dir.path())
            .args(["platforms", "--platform-path"])
            .arg(&root),
    );
    assert_eq!(envelope["data"]["installations"], serde_json::json!([]));
    assert!(envelope["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| {
            let warning = warning.as_str().unwrap();
            warning.contains("1cv8.exe") && warning.contains(&root)
        }));

    let project = dir.path().join("project");
    let refused = command(dir.path())
        .args([
            "--dry-run",
            "clone",
            "--from",
            "File=source",
            "--platform-version",
            "8.3.99.1111",
        ])
        .arg("--project-dir")
        .arg(&project)
        .arg("--platform-path")
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    let refusal = output_json(&refused);
    assert_eq!(refusal["error"]["code"], "environment_unavailable");
    let message = refusal["error"]["message"].as_str().unwrap();
    assert!(message.contains("1cv8.exe") && message.contains(&root));
    assert!(!project.exists());

    let missing = command(dir.path())
        .args(["--config", "missing.yaml", "platforms"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(output_json(&missing)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("missing.yaml"));

    let conflicting = command(dir.path())
        .args(["--config", "missing.yaml", "platforms", "--platform-path"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!conflicting.status.success());
    assert!(output_json(&conflicting)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--platform-path cannot be combined with --config"));

    fs::write(dir.path().join("v8project.yaml"), ": [").unwrap();
    let broken = command(dir.path()).arg("platforms").output().unwrap();
    assert!(!broken.status.success());
    assert_eq!(output_json(&broken)["ok"], false);
}
