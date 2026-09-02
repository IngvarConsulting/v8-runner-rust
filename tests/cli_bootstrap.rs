#![cfg(unix)]

mod support;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use support::{temp_workspace, v8_runner_command, write_shell_script as write_script};

const LOCAL_CONFIG_SCHEMA_MODEL_LINE: &str = "# yaml-language-server: $schema=https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/v8project.local.schema.json";

fn write_designer_dump_script(path: &Path, calls_log: &Path, exit_code: i32) {
    let body = format!(
        r#"args="$*"
printf '%s\n' "$args" >> "{}"
out=""
target=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "/Out" ]; then out="$arg"; fi
  if [ "$prev" = "/DumpConfigToFiles" ]; then target="$arg"; fi
  prev="$arg"
done
if [ -n "$out" ]; then printf 'designer log: %s\n' "$args" > "$out"; fi
if [ "{exit_code}" = "0" ]; then
  mkdir -p "$target"
  printf '<Configuration />\n' > "$target/Configuration.xml"
fi
printf 'stderr: %s\n' "$args" >&2
exit {exit_code}"#,
        calls_log.display()
    );
    write_script(path, &body);
}

fn bootstrap_args<'a>(
    project_dir: &'a Path,
    platform_path: &'a Path,
    connection: &'a str,
) -> Vec<String> {
    vec![
        "bootstrap".to_owned(),
        "--project-dir".to_owned(),
        project_dir.display().to_string(),
        "--connection".to_owned(),
        connection.to_owned(),
        "--platform-version".to_owned(),
        "8.3.27".to_owned(),
        "--platform-path".to_owned(),
        platform_path.display().to_string(),
    ]
}

#[test]
fn bootstrap_empty_dir_creates_config_and_dumps_main_configuration() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            "File=/tmp/source ib",
        ))
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(config.contains("format: DESIGNER"));
    assert!(config.contains("builder: DESIGNER"));
    assert!(config.contains("connection: '/F \"/tmp/source ib\"'"));
    assert!(config.contains("path: 'src/configuration'"));
    assert!(config.contains("version: '8.3.27'"));
    assert!(!config.contains("platform_path"));
    assert!(!config.contains("secret"));

    let local = fs::read_to_string(project_dir.join("v8project.local.yaml")).expect("local");
    assert!(local.starts_with(LOCAL_CONFIG_SCHEMA_MODEL_LINE));
    assert!(local.contains("path: '"));
    assert!(local.contains(platform_path.display().to_string().as_str()));
    let gitignore = fs::read_to_string(project_dir.join(".gitignore")).expect("gitignore");
    assert!(gitignore.lines().any(|line| line == "v8project.local.yaml"));
    assert!(project_dir
        .join("src/configuration/Configuration.xml")
        .exists());

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("/DumpConfigToFiles"));
    assert!(calls.contains("/F /tmp/source ib"));
}

#[test]
fn bootstrap_unquotes_simple_file_connection_path() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            "File=\"/tmp/source ib\"",
        ))
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(config.contains("connection: '/F \"/tmp/source ib\"'"));
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("/F /tmp/source ib"));
    assert!(!calls.contains("\\\"/tmp/source ib\\\""));
}

#[test]
fn bootstrap_json_success_keeps_credentials_in_local_overlay_only() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    let mut args = bootstrap_args(&project_dir, &platform_path, "File=/tmp/source-ib");
    args.splice(
        0..0,
        [
            "--json-message".to_owned(),
            "--log-level".to_owned(),
            "debug".to_owned(),
        ],
    );
    args.extend([
        "--user".to_owned(),
        "Admin".to_owned(),
        "--password".to_owned(),
        "super-secret".to_owned(),
    ]);
    let action_log = dir.path().join("actions.log");

    let output = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &action_log)
        .args(args)
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "bootstrap");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("super-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("super-secret"));

    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(!config.contains("Admin"));
    assert!(!config.contains("super-secret"));
    let local = fs::read_to_string(project_dir.join("v8project.local.yaml")).expect("local");
    assert!(local.contains("user: 'Admin'"));
    assert!(local.contains("password: 'super-secret'"));
    let log = fs::read_to_string(action_log).expect("action log");
    assert!(!log.contains("Admin"));
    assert!(!log.contains("super-secret"));
    assert!(log.contains("/N ***"));
    assert!(log.contains("/P ***"));
}

#[test]
fn bootstrap_preserves_non_secret_connection_attributes() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);

    let output = v8_runner_command()
        .args(bootstrap_args(
            &project_dir,
            &platform_path,
            "File=/tmp/source-ib;Locale=ru",
        ))
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(project_dir.join("v8project.yaml")).expect("config");
    assert!(config.contains("connection: 'File=/tmp/source-ib;Locale=ru'"));
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("/IBConnectionString File=/tmp/source-ib;Locale=ru"));
}

#[test]
fn bootstrap_rejects_existing_targets_without_force() {
    for target in [
        "v8project.yaml",
        "v8project.local.yaml",
        "src/configuration",
    ] {
        let dir = temp_workspace();
        let project_dir = dir.path().join("project");
        let platform_path = dir.path().join("1cv8");
        let calls_log = dir.path().join("calls.log");
        write_designer_dump_script(&platform_path, &calls_log, 0);
        if target.ends_with("configuration") {
            fs::create_dir_all(project_dir.join(target)).expect("target dir");
        } else {
            fs::create_dir_all(&project_dir).expect("project dir");
            fs::write(project_dir.join(target), "existing").expect("target file");
        }

        let output = v8_runner_command()
            .args(bootstrap_args(
                &project_dir,
                &platform_path,
                "File=/tmp/source-ib",
            ))
            .output()
            .expect("run command");

        assert!(!output.status.success(), "target {target}");
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
        assert!(!calls_log.exists());
    }
}

#[test]
fn bootstrap_does_not_write_local_overlay_when_gitignore_update_fails() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    fs::create_dir_all(project_dir.join(".gitignore")).expect("gitignore dir");
    let mut args = bootstrap_args(&project_dir, &platform_path, "File=/tmp/source-ib");
    args.extend([
        "--user".to_owned(),
        "Admin".to_owned(),
        "--password".to_owned(),
        "super-secret".to_owned(),
    ]);

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("gitignore"));
    assert!(!project_dir.join("v8project.local.yaml").exists());
    assert!(!calls_log.exists());
}

#[test]
fn bootstrap_force_overwrites_existing_targets() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 0);
    fs::create_dir_all(project_dir.join("src/configuration")).expect("source dir");
    fs::write(project_dir.join("v8project.yaml"), "existing").expect("config");
    fs::write(project_dir.join("v8project.local.yaml"), "existing").expect("local");
    let mut args = bootstrap_args(&project_dir, &platform_path, "File=/tmp/source-ib");
    args.push("--force".to_owned());

    let output = v8_runner_command()
        .args(args)
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(project_dir
        .join("src/configuration/Configuration.xml")
        .exists());
}

#[test]
fn bootstrap_rejects_embedded_connection_credentials() {
    for connection in [
        "File=/tmp/source-ib;Usr=Admin;Pwd=secret",
        "/F /tmp/source-ib /N Admin /P secret",
        "/S server/ref /N=Admin /P=secret",
    ] {
        let dir = temp_workspace();
        let project_dir = dir.path().join("project");
        let platform_path = dir.path().join("1cv8");
        let output = v8_runner_command()
            .args(bootstrap_args(&project_dir, &platform_path, connection))
            .output()
            .expect("run command");

        assert!(!output.status.success(), "connection {connection}");
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("embedded credentials"));
        assert!(!project_dir.join("v8project.yaml").exists());
    }
}

#[test]
fn bootstrap_rejects_global_config_flag_in_text_mode() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");

    let output = v8_runner_command()
        .args([
            "--config",
            "/definitely/missing/v8project.yaml",
            "bootstrap",
            "--project-dir",
            &project_dir.display().to_string(),
            "--connection",
            "File=/tmp/source-ib",
            "--platform-version",
            "8.3.27",
            "--platform-path",
            &platform_path.display().to_string(),
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not supported for `bootstrap`"));
    assert!(!project_dir.join("v8project.yaml").exists());
}

#[test]
fn bootstrap_rejects_global_config_flag_in_json_mode() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");

    let output = v8_runner_command()
        .args([
            "--config",
            "/definitely/missing/v8project.yaml",
            "--json-message",
            "bootstrap",
            "--project-dir",
            &project_dir.display().to_string(),
            "--connection",
            "File=/tmp/source-ib",
            "--platform-version",
            "8.3.27",
            "--platform-path",
            &platform_path.display().to_string(),
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "bootstrap");
    assert_eq!(payload["error"]["kind"], "validation");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("not supported for `bootstrap`"));
    assert!(!project_dir.join("v8project.yaml").exists());
}

#[test]
fn bootstrap_failed_dump_redacts_secrets_in_outputs() {
    let dir = temp_workspace();
    let project_dir = dir.path().join("project");
    let platform_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("calls.log");
    write_designer_dump_script(&platform_path, &calls_log, 17);
    let action_log = dir.path().join("actions.log");
    let mut args = bootstrap_args(&project_dir, &platform_path, "File=/tmp/source-ib");
    args.splice(
        0..0,
        [
            "--json-message".to_owned(),
            "--log-level".to_owned(),
            "debug".to_owned(),
        ],
    );
    args.extend([
        "--user".to_owned(),
        "Admin".to_owned(),
        "--password".to_owned(),
        "super-secret".to_owned(),
    ]);

    let output = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &action_log)
        .args(args)
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("Admin"));
    assert!(!stdout.contains("super-secret"));
    assert!(!stderr.contains("Admin"));
    assert!(!stderr.contains("super-secret"));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "bootstrap");
    assert_eq!(payload["data"]["dumped"], false);
    assert!(payload["data"]["path"]
        .as_str()
        .expect("path")
        .contains("v8project.yaml"));
    assert!(payload["data"]["dump_target_path"]
        .as_str()
        .expect("target")
        .contains("src/configuration"));
    let log = fs::read_to_string(action_log).expect("action log");
    assert!(!log.contains("Admin"));
    assert!(!log.contains("super-secret"));
}

fn write_minimal_config(dir: &Path) -> PathBuf {
    let config_path = dir.join("v8project.yaml");
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");
    config_path
}

#[test]
fn missing_config_in_text_mode_returns_validation_error_on_stderr() {
    let output = v8_runner_command()
        .args(["--config", "/definitely/missing/v8project.yaml", "build"])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("config file not found"));
}

#[test]
fn missing_config_in_json_mode_keeps_error_envelope_shape() {
    let output = v8_runner_command()
        .args([
            "--config",
            "/definitely/missing/v8project.yaml",
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["duration_ms"], 0);
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(
        payload["data"]["message"],
        "config file not found: /definitely/missing/v8project.yaml"
    );
}

#[test]
fn default_config_path_uses_v8project_yaml_from_current_dir() {
    let dir = temp_workspace();
    let _config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "build"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "build");
}

#[test]
fn default_config_path_applies_sibling_local_overlay() {
    let dir = temp_workspace();
    let _config_path = write_minimal_config(dir.path());
    let local_work_path = dir.path().join("local-work");
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "workPath: local-work\n",
    )
    .expect("local overlay");

    let output = v8_runner_command()
        .current_dir(dir.path())
        .args(["--json-message", "build"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "build");
    assert!(local_work_path.exists());
}

#[test]
fn unsupported_main_config_shape_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let mut config = fs::read_to_string(&config_path).expect("config");
    config.push_str("tools:\n  platform:\n    typo: value\n");
    fs::write(&config_path, config).expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("config contains unsupported key or value"));
}

#[test]
fn unsupported_local_overlay_shape_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "tools:\n  platform:\n    typo: value\n",
    )
    .expect("local overlay");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("local config overlay contains unsupported key or value"));
}

#[test]
fn action_logging_failure_in_json_mode_keeps_command_identity() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let log_path = dir.path().join("action-log-as-dir");
    fs::create_dir_all(&log_path).expect("log dir");

    let output = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &log_path)
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["error"]["code"], "runtime_failure");
    assert_eq!(payload["error"]["kind"], "runtime");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("failed to open action log file"));
}

#[test]
fn test_module_pre_dispatch_validation_in_json_mode_keeps_command_identity() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "test",
            "yaxunit",
            "module",
            "   ",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "test");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(
        payload["data"]["message"],
        "test module requires a non-empty module name"
    );
}

#[test]
fn artifacts_pre_dispatch_validation_in_json_mode_keeps_command_identity() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "make",
            "--output",
            &dir.path().join("out.cf").display().to_string(),
            "--source-set",
            "missing",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "make");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(payload["data"]["message"], "unknown source-set 'missing'");
}

#[test]
fn mcp_rejects_clean_before_execution_flag() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--clean-before-execution",
            "mcp",
            "serve",
            "stdio",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--clean-before-execution is not supported for MCP transports"));
}

#[test]
fn legacy_top_level_connection_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\nconnection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("legacy top-level key 'connection'"));
}

#[test]
fn legacy_top_level_credentials_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\ncredentials:\n  user: Admin\n  password: secret\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("legacy top-level key 'credentials'"));
}

#[test]
fn top_level_execution_timeout_seconds_is_rejected_in_json_mode() {
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nexecution_timeout_seconds: 300\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\n",
            work_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "build");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(message.contains("top-level key 'execution_timeout_seconds'"));
    assert!(message.contains("execution_timeout in milliseconds"));
}
