#![cfg(unix)]

mod support;

use support::v8_runner_command;

#[test]
fn root_help_splits_commands_and_global_options() {
    let output = v8_runner_command()
        .args(["--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Commands:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("Print application version"));
    assert!(stdout.contains("Build configured source-sets into the infobase"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn root_version_flag_prints_application_version() {
    let output = v8_runner_command()
        .args(["--version"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("v8-runner {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn config_init_help_separates_global_and_command_options() {
    let output = v8_runner_command()
        .args(["config", "init", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(!stdout.contains("--file <FILE>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn build_help_exposes_source_set_selector() {
    let output = v8_runner_command()
        .args(["build", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("--source-set <SOURCE_SET>"));
    assert!(stdout.contains("--full-rebuild"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn test_help_exposes_no_build_option() {
    let output = v8_runner_command()
        .args(["test", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--no-build"));
}

#[test]
fn dump_help_clarifies_object_selector_compatibility() {
    let output = v8_runner_command()
        .args(["dump", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("canonical TYPE:NAME selectors"));
    assert!(stdout.contains("legacy TYPE.NAME selectors are accepted for compatibility"));
}

#[test]
fn tools_download_help_exposes_tool_commands() {
    let output = v8_runner_command()
        .args(["tools", "download", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("Commands:"));
    assert!(stdout.contains("yaxunit"));
    assert!(stdout.contains("vanessa"));
    assert!(stdout.contains("client-mcp"));
    assert!(!stdout.contains("--extensions"));
}

#[test]
fn tools_download_extension_help_exposes_sources_flag() {
    let output = v8_runner_command()
        .args(["tools", "download", "yaxunit", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("--sources"));
    assert!(stdout.contains("--force"));
}

#[test]
fn launch_help_uses_output_path_name_and_global_json_selector() {
    let output = v8_runner_command()
        .args(["launch", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--stderr-output <STDERR_OUTPUT>"));
    assert!(stdout.contains("--wait-for-exit"));
    assert!(stdout.contains("--wait-timeout-ms <WAIT_TIMEOUT_MS>"));
    assert!(!stdout.contains("--out <OUT>"));
    assert!(!stdout.contains("--mode <MODE>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn test_help_does_not_expose_direct_launch_wait_options() {
    let output = v8_runner_command()
        .args(["test", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("--c <C>"));
    assert!(!stdout.contains("--execute <EXECUTE>"));
    assert!(!stdout.contains("--output <OUTPUT>"));
    assert!(!stdout.contains("--stderr-output"));
    assert!(!stdout.contains("--wait-for-exit"));
    assert!(!stdout.contains("--wait-timeout-ms"));
}

#[test]
fn make_help_keeps_output_path_under_command_options() {
    let output = v8_runner_command()
        .args(["make", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn convert_help_uses_output_target_root_name() {
    let output = v8_runner_command()
        .args(["convert", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--source-set <SOURCE_SET>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn infobase_configuration_export_help_fixes_the_exact_grammar() {
    let output = v8_runner_command()
        .args(["infobase", "configuration", "export", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--state <STATE>"));
    assert!(stdout.contains("[possible values: working, database]"));
    assert!(stdout.contains("--extension <EXTENSION>"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--dry-run"));
    assert!(!stdout.contains("--provider"));
    assert!(!stdout.contains("--engine"));
}

#[test]
fn infobase_dump_help_calls_dt_a_transfer_file_not_a_backup() {
    let output = v8_runner_command()
        .args(["infobase", "dump", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("not a backup"));
}
