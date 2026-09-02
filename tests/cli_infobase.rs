#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn write_designer(path: &Path, calls: &Path) {
    write_shell_script(
        path,
        &format!(
            r#"printf '%s\n' "$*" >> "{}"
previous=''
out=''
for argument in "$@"; do
  case "$previous" in
    /DumpCfg|/DumpDBCfg|/DumpIB) printf 'payload' > "$argument" ;;
    /Out) out="$argument" ;;
  esac
  previous="$argument"
done
if [ -n "$out" ]; then printf 'platform log' > "$out"; fi
exit 0"#,
            calls.display()
        ),
    );
}

fn write_ibcmd(path: &Path, calls: &Path) {
    write_shell_script(
        path,
        &format!(
            r#"printf '%s\n' "$*" >> "{}"
last=''
for argument in "$@"; do last="$argument"; done
printf 'payload' > "$last"
exit 0"#,
            calls.display()
        ),
    );
}

fn write_config(path: &Path, base: &Path, work: &Path, builder: &str, platform: &Path) {
    fs::create_dir_all(base.join("main")).expect("base");
    fs::create_dir_all(work).expect("work");
    fs::write(
        path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: {}\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            builder,
            platform.display(),
        ),
    )
    .expect("config");
}

fn setup(builder: &str) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let binary = dir
        .path()
        .join(if builder == "IBCMD" { "ibcmd" } else { "1cv8" });
    let calls = dir.path().join("calls.log");
    if builder == "IBCMD" {
        write_ibcmd(&binary, &calls);
    } else {
        write_designer(&binary, &calls);
    }
    write_config(&config, &base, &work, builder, &binary);
    (dir, config, base, calls)
}

#[test]
fn designer_exports_database_extension_to_cfe_with_typed_json() {
    let (_dir, config, base, calls) = setup("DESIGNER");
    let output = base.join("dist/sales.cfe");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "database",
            "--extension",
            "SalesAddon",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run export");

    assert!(
        command.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&command.stdout),
        String::from_utf8_lossy(&command.stderr)
    );
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.configuration.export");
    assert_eq!(envelope["data"]["state"], "database");
    assert_eq!(envelope["data"]["subject"]["kind"], "extension");
    assert_eq!(envelope["data"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["artifact_kind"], "cfe");
    assert_eq!(envelope["data"]["published"], true);
    assert_eq!(envelope["data"]["execution"]["status"], "succeeded");
    assert_eq!(envelope["steps"].as_array().map(Vec::len), Some(2));
    assert_eq!(fs::read(&output).expect("published cfe"), b"payload");
    let argv = fs::read_to_string(calls).expect("calls");
    assert!(argv.contains("/DumpDBCfg"));
    assert!(argv.contains("-Extension SalesAddon"));
}

#[test]
fn designer_dumps_dt_and_labels_it_as_transfer_snapshot() {
    let (_dir, config, base, calls) = setup("DESIGNER");
    let output = base.join("dist/base.dt");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "dump",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run dump");

    assert!(
        command.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&command.stdout),
        String::from_utf8_lossy(&command.stderr)
    );
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.dump");
    assert_eq!(envelope["data"]["subject"]["kind"], "infobase");
    assert_eq!(envelope["data"]["artifact_kind"], "dt");
    assert_eq!(envelope["data"]["published"], true);
    assert_eq!(fs::read(&output).expect("published dt"), b"payload");
    assert!(fs::read_to_string(calls)
        .expect("calls")
        .contains("/DumpIB"));
}

#[test]
fn ibcmd_dt_refuses_before_dispatch_with_unverified_evidence() {
    let (_dir, config, base, calls) = setup("IBCMD");
    let output = base.join("dist/base.dt");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "dump",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run dump");

    assert_eq!(command.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.dump");
    assert_eq!(envelope["data"]["provider"], Value::Null);
    assert_eq!(envelope["data"]["evidence"], "unverified");
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["execution"]["status"], "failed");
    assert_eq!(envelope["error"]["code"], "capability_unavailable");
    assert_eq!(envelope["error"]["kind"], "capability");
    assert!(!output.exists());
    assert!(
        !calls.exists(),
        "unverified IBCMD DT must not dispatch a process"
    );
}

#[test]
fn ibcmd_exports_working_configuration_without_designer_fallback() {
    let (_dir, config, base, calls) = setup("IBCMD");
    let output = base.join("dist/main.cf");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run export");

    assert!(
        command.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&command.stdout),
        String::from_utf8_lossy(&command.stderr)
    );
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["provider"], "ibcmd-process");
    assert_eq!(envelope["data"]["state"], "working");
    assert_eq!(envelope["data"]["published"], true);
    assert_eq!(fs::read(&output).expect("published cf"), b"payload");
    let argv = fs::read_to_string(calls).expect("calls");
    assert!(argv.contains("config save"));
    assert!(!argv.split_whitespace().any(|argument| argument == "--db"));
    assert!(!argv.contains("DESIGNER"));
}

#[test]
fn invalid_suffix_is_rejected_before_workspace_lock_and_provider_dispatch() {
    let (_dir, config, base, calls) = setup("DESIGNER");
    let output = base.join("dist/main.cfe");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run invalid export");

    assert_eq!(command.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.configuration.export");
    assert_eq!(envelope["error"]["kind"], "validation");
    assert!(
        !calls.exists(),
        "invalid request must not dispatch a provider"
    );
    assert!(
        !base.join("../work/.v8-runner.workspace.lock").exists(),
        "invalid request must not acquire the workspace lock"
    );
}

#[test]
fn text_output_uses_the_same_canonical_provider_name_as_json() {
    let (_dir, config, base, _calls) = setup("DESIGNER");
    let output = base.join("dist/base.dt");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "infobase",
            "dump",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run text dump");

    assert!(command.status.success());
    let stdout = String::from_utf8_lossy(&command.stdout);
    assert!(stdout.contains("command: infobase.dump"));
    assert!(stdout.contains("Infobase DT export"));
    assert!(stdout.contains("provider: designer-batch"));
    assert!(!stdout.contains("DesignerBatch"));
    assert!(stdout.contains("published: true"));
    assert!(stdout.contains("subject: infobase"));
    assert!(stdout.contains("evidence: available"));
    assert!(stdout.contains("artifact kind: dt"));
    assert!(stdout.contains("execution status: succeeded"));
}

#[test]
fn provider_failure_preserves_an_existing_target() {
    let (dir, config, base, calls) = setup("DESIGNER");
    let output = base.join("dist/main.cf");
    fs::create_dir_all(output.parent().expect("parent")).expect("output parent");
    fs::write(&output, "old package").expect("old target");
    write_shell_script(
        &dir.path().join("1cv8"),
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'; printf 'provider stderr' >&2; exit 19",
            calls.display()
        ),
    );

    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run failed export");

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["execution"]["status"], "failed");
    assert_eq!(
        envelope["data"]["execution"]["errors"][0]["code"],
        "platform_failure"
    );
    assert!(envelope["data"]["execution"]["errors"][0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("provider stderr")));
    let provider_steps = envelope["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter(|step| step["name"] == "provider command")
        .collect::<Vec<_>>();
    assert_eq!(provider_steps.len(), 1);
    assert_eq!(provider_steps[0]["status"], "failed");
    assert_eq!(
        fs::read_to_string(&output).expect("preserved target"),
        "old package"
    );
    assert_eq!(
        fs::read_to_string(calls)
            .expect("single provider call")
            .lines()
            .count(),
        1
    );
}

#[test]
fn infobase_export_does_not_require_project_source_sets() {
    let (dir, config, base, calls) = setup("DESIGNER");
    let yaml = fs::read_to_string(&config).expect("config");
    let source_set =
        "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\n";
    fs::write(&config, yaml.replace(source_set, "source-set: []\n")).expect("source-free config");
    let output = base.join("dist/main.cf");

    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run source-free export");

    assert!(
        command.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&command.stdout),
        String::from_utf8_lossy(&command.stderr)
    );
    assert_eq!(fs::read(&output).expect("published cf"), b"payload");
    assert!(fs::read_to_string(calls)
        .expect("calls")
        .contains("/DumpCfg"));
    drop(dir);
}

#[test]
fn missing_provider_artifact_has_invalid_output_terminal_status() {
    let (dir, config, base, _calls) = setup("DESIGNER");
    write_shell_script(&dir.path().join("1cv8"), "exit 0");
    let output = base.join("dist/main.cf");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run invalid provider output");

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["execution"]["status"], "invalid_output");
    assert_eq!(
        envelope["data"]["execution"]["errors"][0]["code"],
        "invalid_output"
    );
    assert_eq!(envelope["data"]["published"], false);
}

#[test]
fn provider_timeout_has_timed_out_terminal_status_and_is_not_retryable() {
    let (dir, config, base, _calls) = setup("DESIGNER");
    let yaml = fs::read_to_string(&config).expect("config");
    fs::write(
        &config,
        yaml.replacen("workPath:", "execution_timeout: 50\nworkPath:", 1),
    )
    .expect("short timeout config");
    write_shell_script(&dir.path().join("1cv8"), "sleep 1");
    let output = base.join("dist/main.cf");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run timed out provider");

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["execution"]["status"], "timed_out");
    assert_eq!(
        envelope["data"]["execution"]["errors"][0]["code"],
        "timed_out"
    );
    assert!(envelope["data"]["execution"]["errors"][0]["retryable"].is_null());
    assert_eq!(envelope["steps"][0]["status"], "failed");
    assert_eq!(envelope["data"]["published"], false);
}
