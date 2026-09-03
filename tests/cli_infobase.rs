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
    let infobase = base.join("ib");
    fs::create_dir_all(&infobase).expect("infobase");
    fs::write(infobase.join("1Cv8.1CD"), "database").expect("infobase file");
    fs::write(
        path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: {}\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            builder,
            infobase.display(),
            platform.display(),
        ),
    )
    .expect("config");
}

#[test]
fn configuration_cf_dry_run_selects_provider_without_process_or_filesystem_mutation() {
    let (dir, config, base, calls) = setup("DESIGNER");
    let work = dir.path().join("work");
    fs::remove_dir_all(&work).expect("remove setup work path");
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
            "--dry-run",
        ])
        .output()
        .expect("preview configuration export");

    assert!(
        command.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&command.stdout),
        String::from_utf8_lossy(&command.stderr)
    );
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.configuration.export");
    assert_eq!(envelope["data"]["mode"], "preview");
    assert_eq!(envelope["data"]["provider_dispatched"], false);
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["artifact_kind"], "cf");
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["target_state"], "unchanged");
    assert_eq!(envelope["data"]["execution"]["status"], "succeeded");
    let expected_output = fs::canonicalize(&base)
        .expect("canonical base")
        .join("dist/main.cf");
    assert_eq!(
        envelope["data"]["plan"]["output"],
        expected_output.display().to_string()
    );
    assert_eq!(envelope["data"]["plan"]["provider"], "designer-batch");
    assert!(!calls.exists(), "preview must not start the platform");
    assert!(!work.exists(), "preview must not recreate workPath");
    assert!(!output.exists(), "preview must not create the output");
    assert!(!output.parent().expect("output parent").exists());
}

#[test]
fn configuration_cfe_dry_run_preserves_extension_intent_without_dispatch() {
    let (dir, config, base, calls) = setup("IBCMD");
    let work = dir.path().join("work");
    fs::remove_dir_all(&work).expect("remove setup work path");
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
            "--dry-run",
        ])
        .output()
        .expect("preview extension export");

    assert!(command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["mode"], "preview");
    assert_eq!(envelope["data"]["subject"]["kind"], "extension");
    assert_eq!(envelope["data"]["subject"]["name"], "SalesAddon");
    assert_eq!(envelope["data"]["selection"]["provider"], "ibcmd-process");
    assert_eq!(envelope["data"]["artifact_kind"], "cfe");
    assert_eq!(envelope["data"]["provider_dispatched"], false);
    assert!(!calls.exists());
    assert!(!work.exists());
    assert!(!output.parent().expect("output parent").exists());
}

#[test]
fn dt_dry_run_reports_designer_fallback_without_dispatch() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_ibcmd(&platform.join("ibcmd"), &calls);
    write_designer(&platform.join("1cv8"), &calls);
    write_config(&config, &base, &work, "IBCMD", &platform);
    fs::remove_dir_all(&work).expect("remove setup work path");
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
            "--dry-run",
        ])
        .output()
        .expect("preview DT export");

    assert!(command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.dump");
    assert_eq!(envelope["data"]["mode"], "preview");
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["artifact_kind"], "dt");
    assert_eq!(envelope["data"]["provider_dispatched"], false);
    assert!(!calls.exists());
    assert!(!work.exists());
    assert!(!output.parent().expect("output parent").exists());
}

#[test]
fn unavailable_dry_run_is_typed_as_preview_failure_without_side_effects() {
    let (dir, config, base, calls) = setup("DESIGNER");
    fs::remove_file(base.join("ib/1Cv8.1CD")).expect("remove infobase file");
    let work = dir.path().join("work");
    fs::remove_dir_all(&work).expect("remove setup work path");
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
            "--dry-run",
        ])
        .output()
        .expect("preview unavailable export");

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "environment_unavailable");
    assert_eq!(envelope["data"]["mode"], "preview");
    assert_eq!(envelope["data"]["provider_dispatched"], false);
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["target_state"], "unchanged");
    assert!(!calls.exists());
    assert!(!work.exists());
    assert!(!output.parent().expect("output parent").exists());
}

#[test]
fn dry_run_rejects_an_existing_output_directory_before_runtime_mutation() {
    let (dir, config, base, calls) = setup("DESIGNER");
    let work = dir.path().join("work");
    fs::remove_dir_all(&work).expect("remove setup work path");
    let output = base.join("dist/main.cf");
    fs::create_dir_all(&output).expect("directory masquerading as CF output");

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
            "--dry-run",
        ])
        .output()
        .expect("preview invalid target");

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["mode"], "preview");
    assert_eq!(envelope["data"]["provider_dispatched"], false);
    assert_eq!(envelope["steps"][0]["name"], "resolve target");
    assert!(envelope["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("is a directory")));
    assert!(!calls.exists());
    assert!(!work.exists());
    assert!(output.is_dir());
}

#[test]
fn missing_file_infobase_is_unavailable_before_designer_dispatch() {
    let (_dir, config, base, calls) = setup("DESIGNER");
    fs::remove_file(base.join("ib/1Cv8.1CD")).expect("remove infobase file");
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

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "environment_unavailable");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "unavailable"
    );
    assert!(envelope["data"]["selection"]["candidates"][0]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("1Cv8.1CD")));
    assert!(!calls.exists());
    assert!(!output.exists());
}

#[test]
fn malformed_server_connection_is_unavailable_before_provider_dispatch() {
    let (dir, config, base, calls) = setup("DESIGNER");
    let yaml = fs::read_to_string(&config).expect("config");
    fs::write(
        &config,
        yaml.replace(
            &format!("File={}", base.join("ib").display()),
            "not a connection",
        ),
    )
    .expect("malformed connection config");
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

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "environment_unavailable");
    assert!(envelope["data"]["selection"]["candidates"][0]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("expected non-empty File=")));
    assert!(!calls.exists());
    assert!(!output.exists());
    assert!(!dir.path().join("work/logs/mcp/actions.log").exists());
}

#[test]
fn malformed_config_keeps_the_typed_infobase_failure_payload() {
    let dir = temp_workspace();
    let config = dir.path().join("v8project.yaml");
    fs::write(&config, "workPath: [unterminated").expect("malformed config");
    let output = dir.path().join("main.cf");

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

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["command"], "infobase.configuration.export");
    assert_eq!(envelope["data"]["selection"]["provider"], Value::Null);
    assert_eq!(
        envelope["data"]["selection"]["candidates"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["target_state"], "unchanged");
    assert_eq!(envelope["data"]["execution"]["status"], "failed");
    assert_eq!(envelope["steps"][0]["name"], "configuration load");
    assert!(!output.exists());
}

fn hold_workspace_lock(work: &Path) {
    fs::create_dir_all(work).expect("work");
    fs::write(
        work.join(".v8-runner.workspace.lock"),
        format!(
            "{{\"tool\":\"v8-runner\",\"pid\":{},\"owner_id\":\"test-owner\",\"created_at\":\"2026-09-02T00:00:00Z\"}}",
            std::process::id()
        ),
    )
    .expect("workspace lock");
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
    let (dir, config, base, calls) = setup("DESIGNER");
    let output = base.join("dist/sales.cfe");
    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--log-level",
            "debug",
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
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["artifact_kind"], "cfe");
    assert_eq!(envelope["data"]["published"], true);
    assert_eq!(envelope["data"]["target_state"], "created");
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["implementation"],
        "implemented"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "ready"
    );
    assert_eq!(envelope["data"]["execution"]["status"], "succeeded");
    assert_eq!(envelope["steps"].as_array().map(Vec::len), Some(2));
    assert_eq!(fs::read(&output).expect("published cfe"), b"payload");
    let argv = fs::read_to_string(calls).expect("calls");
    assert!(argv.contains("/DumpDBCfg"));
    assert!(argv.contains("-Extension SalesAddon"));
    let action_log =
        fs::read_to_string(dir.path().join("work/logs/mcp/actions.log")).expect("action log");
    assert!(action_log.contains("starting command under workspace lock"));
    assert!(!action_log.contains("command finished successfully"));
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
fn dt_uses_designer_when_ibcmd_is_preferred_but_not_implemented() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_ibcmd(&platform.join("ibcmd"), &calls);
    write_designer(&platform.join("1cv8"), &calls);
    write_config(&config, &base, &work, "IBCMD", &platform);
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
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["published"], true);
    let argv = fs::read_to_string(calls).expect("calls");
    assert!(argv.contains("/DumpIB"));
    assert!(!argv.contains("infobase dump"));
}

#[test]
fn missing_preferred_ibcmd_falls_back_to_ready_designer_before_dispatch() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_designer(&platform.join("1cv8"), &calls);
    write_config(&config, &base, &work, "IBCMD", &platform);
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
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["provider"],
        "ibcmd-process"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "unavailable"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][1]["readiness"],
        "ready"
    );
    assert!(fs::read_to_string(calls)
        .expect("calls")
        .contains("/DumpCfg"));
}

#[test]
fn missing_preferred_designer_falls_back_to_ready_ibcmd_before_dispatch() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_ibcmd(&platform.join("ibcmd"), &calls);
    write_config(&config, &base, &work, "DESIGNER", &platform);
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

    assert!(command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["selection"]["provider"], "ibcmd-process");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["provider"],
        "designer-batch"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "unavailable"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][1]["readiness"],
        "ready"
    );
    assert!(fs::read_to_string(calls)
        .expect("calls")
        .contains("config save"));
}

#[test]
fn ibcmd_only_environment_cannot_dump_dt_until_capability_is_implemented() {
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

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "environment_unavailable");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["implementation"],
        "experimental"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "not_checked"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][1]["readiness"],
        "unavailable"
    );
    assert!(!calls.exists());
    assert!(!output.exists());
}

#[test]
fn thin_client_alone_is_not_reported_as_designer_ready() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_designer(&platform.join("1cv8c"), &calls);
    write_config(&config, &base, &work, "DESIGNER", &platform);
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

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "environment_unavailable");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "unavailable"
    );
    assert!(!calls.exists());
    assert!(!output.exists());
}

#[test]
fn incomplete_ibcmd_server_contract_does_not_block_ready_designer_alternate() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&base).expect("base");
    fs::create_dir_all(&work).expect("work");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_ibcmd(&platform.join("ibcmd"), &calls);
    write_designer(&platform.join("1cv8"), &calls);
    fs::write(
        &config,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: IBCMD\ninfobase:\n  connection: 'Srvr=localhost;Ref=demo'\nsource-set: []\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            platform.display(),
        ),
    )
    .expect("config");
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
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(
        envelope["data"]["selection"]["candidates"][0]["readiness"],
        "unavailable"
    );
    assert_eq!(
        envelope["data"]["selection"]["candidates"][1]["readiness"],
        "ready"
    );
    let argv = fs::read_to_string(calls).expect("calls");
    assert!(argv.contains("/DumpCfg"));
    assert!(!argv.contains("config save"));
}

#[test]
fn complete_server_dbms_contract_is_dispatched_to_ibcmd() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&base).expect("base");
    fs::create_dir_all(&work).expect("work");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_ibcmd(&platform.join("ibcmd"), &calls);
    fs::write(
        &config,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: IBCMD\ninfobase:\n  connection: 'Srvr=cluster;Ref=demo'\n  dbms:\n    kind: PostgreSQL\n    server: db.example.test\n    name: demo_data\nsource-set: []\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            platform.display(),
        ),
    )
    .expect("config");
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
            "database",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run export");

    assert!(command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["selection"]["provider"], "ibcmd-process");
    let argv = fs::read_to_string(calls).expect("calls");
    assert!(argv.contains("--dbms PostgreSQL"));
    assert!(argv.contains("--database-server db.example.test"));
    assert!(argv.contains("--database-name demo_data"));
    assert!(argv.contains("config save"));
    assert!(argv.split_whitespace().any(|argument| argument == "--db"));
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
    assert_eq!(envelope["data"]["selection"]["provider"], "ibcmd-process");
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
    hold_workspace_lock(&base.join("../work"));
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
    assert_eq!(envelope["error"]["code"], "invalid_argument");
    assert_eq!(envelope["data"]["subject"]["kind"], "main");
    assert_eq!(envelope["data"]["selection"]["provider"], Value::Null);
    assert_eq!(
        envelope["data"]["selection"]["candidates"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(envelope["data"]["target_state"], "unchanged");
    assert_eq!(envelope["data"]["execution"]["status"], "failed");
    assert_eq!(
        envelope["data"]["execution"]["errors"][0]["code"],
        "invalid_argument"
    );
    assert!(
        !calls.exists(),
        "invalid request must not dispatch a provider"
    );
}

#[test]
fn clean_before_failure_is_reported_as_workspace_preparation() {
    let (dir, config, base, calls) = setup("DESIGNER");
    fs::write(dir.path().join("work/logs"), "blocks log directory").expect("log blocker");
    let output = base.join("dist/main.cf");

    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "--json-message",
            "--clean-before-execution",
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

    assert_eq!(command.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "runtime_failure");
    assert_eq!(envelope["steps"][0]["name"], "workspace preparation");
    assert_eq!(envelope["steps"][0]["kind"], "prepare_workspace");
    assert!(!calls.exists());
    assert!(!output.exists());
}

#[test]
fn workspace_lock_io_failure_is_reported_as_lock_acquisition() {
    let (dir, config, base, calls) = setup("DESIGNER");
    fs::remove_dir_all(dir.path().join("work")).expect("remove work directory");
    fs::write(dir.path().join("work"), "not a directory").expect("work path blocker");
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

    assert_eq!(command.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "runtime_failure");
    assert_eq!(envelope["steps"][0]["name"], "workspace lock");
    assert_eq!(envelope["steps"][0]["kind"], "prepare_workspace");
    assert!(!calls.exists());
    assert!(!output.exists());
}

#[test]
fn ready_provider_reports_workspace_busy_without_dispatch() {
    let (dir, config, base, calls) = setup("DESIGNER");
    hold_workspace_lock(&base.join("../work"));
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
        .expect("run busy export");

    assert_eq!(command.status.code(), Some(3));
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "workspace_busy");
    assert_eq!(envelope["error"]["kind"], "workspace");
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["target_state"], "unchanged");
    assert_eq!(envelope["data"]["execution"]["status"], "failed");
    assert_eq!(
        envelope["data"]["execution"]["errors"][0]["code"],
        "workspace_busy"
    );
    assert!(!calls.exists());
    assert!(!output.exists());
    assert!(!dir.path().join("work/logs/mcp/actions.log").exists());
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
    assert!(stdout.contains("implementation: implemented"));
    assert!(stdout.contains("readiness: ready"));
    assert!(stdout.contains("artifact kind: dt"));
    assert!(stdout.contains("execution status: succeeded"));
}

#[test]
fn text_output_explains_unavailable_preference_and_ready_alternate() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_designer(&platform.join("1cv8"), &calls);
    write_config(&config, &base, &work, "IBCMD", &platform);
    let output = base.join("dist/main.cf");

    let command = v8_runner_command()
        .args([
            "--config",
            &config.display().to_string(),
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .output()
        .expect("run text export");

    assert!(command.status.success());
    let stdout = String::from_utf8_lossy(&command.stdout);
    assert!(stdout.contains("provider: designer-batch"));
    assert!(stdout.contains("evidence: argv_tested"));
    assert!(stdout.contains("candidate ibcmd-process:"));
    assert!(stdout.contains("implementation=implemented, readiness=unavailable"));
    assert!(stdout.contains("candidate designer-batch:"));
    assert!(stdout.contains("implementation=implemented, readiness=ready"));
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
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["target_state"], "unchanged");
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
fn provider_failure_never_dispatches_a_ready_alternate_after_spawn() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("platform");
    fs::create_dir_all(&platform).expect("platform");
    let calls = dir.path().join("calls.log");
    write_shell_script(
        &platform.join("1cv8"),
        &format!(
            "printf 'designer-failed\\n' >> '{}'; exit 19",
            calls.display()
        ),
    );
    write_ibcmd(&platform.join("ibcmd"), &calls);
    write_config(&config, &base, &work, "DESIGNER", &platform);
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
        .expect("run failed export");

    assert!(!command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["selection"]["provider"], "designer-batch");
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(
        fs::read_to_string(calls).expect("single provider call"),
        "designer-failed\n"
    );
    assert!(!output.exists());
}

#[test]
fn successful_overwrite_reports_replaced_target_state() {
    let (_dir, config, base, _calls) = setup("DESIGNER");
    let output = base.join("dist/main.cf");
    fs::create_dir_all(output.parent().expect("parent")).expect("output parent");
    fs::write(&output, "old package").expect("old target");

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
        .expect("run overwrite");

    assert!(command.status.success());
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["data"]["target_state"], "replaced");
    assert_eq!(fs::read(&output).expect("target"), b"payload");
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
    assert_eq!(envelope["error"]["code"], "invalid_output");
    assert_eq!(envelope["error"]["kind"], "invalid_output");
    assert_eq!(envelope["data"]["published"], false);
    assert_eq!(envelope["data"]["target_state"], "unchanged");
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
    assert_eq!(envelope["data"]["target_state"], "unchanged");
    assert_eq!(envelope["error"]["code"], "timed_out");
    assert_eq!(envelope["error"]["kind"], "interruption");
}

#[test]
fn no_ready_provider_wins_over_workspace_contention_without_side_effects() {
    let dir = temp_workspace();
    let base = dir.path().join("project");
    let work = dir.path().join("work");
    let config = dir.path().join("v8project.yaml");
    let platform = dir.path().join("empty-platform");
    fs::create_dir_all(&base).expect("base");
    fs::create_dir_all(&platform).expect("platform");
    fs::write(
        &config,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: IBCMD\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set: []\ntools:\n  platform:\n    path: '{}'\n    strict: true\n    version: '8.3.27'\n",
            work.display(),
            platform.display()
        ),
    )
    .expect("config");
    hold_workspace_lock(&work);
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

    assert_eq!(command.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&command.stdout).expect("json envelope");
    assert_eq!(envelope["error"]["code"], "environment_unavailable");
    assert_eq!(envelope["error"]["kind"], "environment");
    assert!(!output.parent().expect("output parent").exists());
    assert!(!work.join("platform-logs").exists());
    assert!(
        !work.join("logs/mcp/actions.log").exists(),
        "selection failure must not create the JSON action log"
    );
}
