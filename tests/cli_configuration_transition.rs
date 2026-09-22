#![cfg(unix)]
mod support;
use std::fs;
use support::{temp_workspace, v8_runner_command, write_shell_script};

#[test]
fn apply_reset_preview_does_not_dispatch_or_create_work_path() {
    for command in ["apply", "reset"] {
        let dir = temp_workspace();
        let platform = dir.path().join("platform");
        let calls = dir.path().join("calls");
        write_shell_script(
            &platform.join("1cv8"),
            &format!("echo called >> '{}'", calls.display()),
        );
        let config = dir.path().join("v8project.yaml");
        fs::write(&config, format!("format: DESIGNER\nworkPath: work\ninfobase:\n  connection: 'File=base'\ntools:\n  platform:\n    path: '{}'\nsource-set: []\n", platform.display())).unwrap();
        let mut invocation = v8_runner_command();
        invocation.args([
            "--config",
            config.to_str().unwrap(),
            "--json-message",
            command,
            "--dry-run",
            "--extension",
            "InstalledAddon",
        ]);
        if command == "reset" {
            invocation.arg("--force");
        }
        let output = invocation.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["data"]["provider_dispatched"], false);
        assert_eq!(json["data"]["completed"], false);
        assert!(!calls.exists());
        assert!(!dir.path().join("work").exists());
    }
}

fn setup(
    connection: &str,
    script: &str,
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = temp_workspace();
    let binary = dir.path().join("1cv8");
    let calls = dir.path().join("calls");
    write_shell_script(
        &binary,
        &format!("printf '%s\\n' \"$@\" >> '{}'\n{script}", calls.display()),
    );
    let config = dir.path().join("v8project.yaml");
    fs::write(&config, format!("format: DESIGNER\nworkPath: work\nexecution_timeout: 300\ninfobase:\n  connection: '{connection}'\n  user: TestUser\n  password: TestPassword\ntools:\n  platform:\n    path: '{}'\nsource-set: []\n", binary.display())).unwrap();
    (dir, config, calls)
}
fn invoke(
    config: &std::path::Path,
    command: &str,
    extra: &[&str],
) -> (std::process::ExitStatus, serde_json::Value) {
    let mut invocation = v8_runner_command();
    invocation.args([
        "--config",
        config.to_str().unwrap(),
        "--json-message",
        command,
    ]);
    if command == "reset" {
        invocation.arg("--force");
    }
    let output = invocation.args(extra).output().unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status, json)
}

const PRESENT_EXTENSION: &str = r#"previous=''
for argument in "$@"; do
  if [ "$previous" = /DumpCfg ]; then printf 'cfe payload' > "$argument"; fi
  previous="$argument"
done
exit 0"#;

fn set_timeout(config: &std::path::Path, millis: u64) {
    fs::write(
        config,
        fs::read_to_string(config).unwrap().replace(
            "execution_timeout: 300",
            &format!("execution_timeout: {millis}"),
        ),
    )
    .unwrap();
}

#[test]
fn transitions_use_exact_designer_operation_and_extension_for_file_and_cluster() {
    for (command, flag) in [("apply", "/UpdateDBCfg"), ("reset", "/RollbackCfg")] {
        for connection in ["File=base", "Srvr=server;Ref=database"] {
            let (dir, config, calls) = setup(connection, PRESENT_EXTENSION);
            set_timeout(&config, 3000);
            let (status, json) = invoke(&config, command, &["--extension", "InstalledAddon"]);
            assert!(status.success(), "{json}");
            assert_eq!(json["data"]["provider_dispatched"], true);
            assert_eq!(json["data"]["completed"], true);
            assert_eq!(json["data"]["provider"]["selected"], "designer");
            let args = fs::read_to_string(calls).unwrap();
            let argv: Vec<_> = args.lines().collect();
            let resolved_connection = if connection.starts_with("File=") {
                format!(
                    "File={}",
                    dir.path().canonicalize().unwrap().join("base").display()
                )
            } else {
                connection.to_owned()
            };
            assert_eq!(
                &argv[..9],
                [
                    "DESIGNER",
                    "/DisableStartupDialogs",
                    "/DisableStartupMessages",
                    "/IBConnectionString",
                    &resolved_connection,
                    "/N",
                    "TestUser",
                    "/P",
                    "TestPassword"
                ]
            );
            assert_eq!(
                &argv[argv.len() - 3..],
                [flag, "-Extension", "InstalledAddon"]
            );
            assert!(!args.contains("/LoadCfg"));
            support::command_data::assert_data_matches_a_declared_form(&json, command);
        }
    }
}

#[test]
fn unknown_extension_failure_never_retries_on_main_configuration() {
    for command in ["apply", "reset"] {
        let (_dir, config, calls) = setup("File=base", "exit 7");
        set_timeout(&config, 3000);
        let (status, json) = invoke(&config, command, &["--extension", "Unknown"]);
        assert!(!status.success());
        assert_eq!(json["data"]["provider_dispatched"], true);
        assert_eq!(json["data"]["completed"], false);
        assert_eq!(json["data"]["status"], "failed");
        let args = fs::read_to_string(calls).unwrap();
        assert_eq!(args.lines().filter(|line| *line == "DESIGNER").count(), 1);
        assert!(args.ends_with("-Extension\nUnknown\n"));
    }
}

#[test]
fn busy_workspace_blocks_execution_but_not_read_only_preview() {
    for command in ["apply", "reset"] {
        let (dir, config, calls) = setup("File=base", "exit 0");
        let work = dir.path().join("work");
        fs::create_dir(&work).unwrap();
        let lock = format!("{{\"tool\":\"v8-runner\",\"pid\":{},\"owner_id\":\"test\",\"created_at\":\"2026-09-22T00:00:00Z\"}}", std::process::id());
        fs::write(work.join(".v8-runner.workspace.lock"), &lock).unwrap();
        let (status, _) = invoke(&config, command, &[]);
        assert!(!status.success());
        let (status, json) = invoke(&config, command, &["--dry-run"]);
        assert!(status.success(), "{json}");
        assert_eq!(
            fs::read_to_string(work.join(".v8-runner.workspace.lock")).unwrap(),
            lock
        );
        assert!(!calls.exists());
    }
}

#[test]
fn invalid_extension_and_clean_preview_refuse_before_writes() {
    for command in ["apply", "reset"] {
        for args in [
            vec!["--extension", ""],
            vec!["--dry-run", "--clean-before-execution"],
        ] {
            let (dir, config, calls) = setup("File=base", "exit 0");
            let output = v8_runner_command()
                .args(["--config", config.to_str().unwrap(), command])
                .args(if command == "reset" {
                    vec!["--force"]
                } else {
                    vec![]
                })
                .args(args)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(!calls.exists());
            assert!(!dir.path().join("work").exists());
        }
    }
}

#[test]
fn critical_transition_preserves_completed_success_after_deferred_timeout() {
    for command in ["apply", "reset"] {
        let (_dir, config, _calls) = setup("File=base", "sleep 1\nexit 0");
        let (status, json) = invoke(&config, command, &[]);
        assert!(status.success(), "{json}");
        assert_eq!(json["data"]["completed"], true);
        assert_eq!(json["data"]["status"], "succeeded");
        assert_eq!(json["data"]["interruption"]["deferred"], true);
        assert_eq!(json["data"]["interruption"]["kind"], "timed_out");
    }
}

#[test]
fn reset_requires_explicit_force_before_config_loading() {
    let output = v8_runner_command()
        .args(["reset", "--dry-run"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("required arguments"));
}

#[test]
fn unsupported_provider_cannot_bypass_transition_capability_matrix() {
    for command in ["apply", "reset"] {
        let (dir, config, calls) = setup("File=base", "exit 0");
        let mut yaml = fs::read_to_string(&config).unwrap();
        yaml.push_str(&format!("providers:\n  {command}: ibcmd\n"));
        fs::write(&config, yaml).unwrap();
        let (status, json) = invoke(&config, command, &["--dry-run"]);
        assert!(!status.success(), "{json}");
        assert_eq!(json["error"]["code"], "invalid_argument");
        assert!(!calls.exists());
        assert!(!dir.path().join("work").exists());
    }
}

#[test]
fn standalone_transition_is_explicitly_unavailable_without_process_or_writes() {
    for command in ["apply", "reset"] {
        let (dir, config, calls) = setup("File=base", "exit 0");
        let yaml = fs::read_to_string(&config).unwrap().replace(
            "  connection: 'File=base'",
            "  standalone:\n    gate: localhost:1543\n    exchange: sftp",
        );
        fs::write(&config, yaml).unwrap();
        let (status, json) = invoke(&config, command, &["--dry-run"]);
        assert!(!status.success(), "{json}");
        assert_eq!(json["error"]["code"], "capability_unavailable");
        assert_eq!(json["data"]["provider_dispatched"], false);
        assert!(!calls.exists());
        assert!(!dir.path().join("work").exists());
    }
}

#[test]
fn deferred_timeout_keeps_failed_main_transition_failed_and_dispatched() {
    for (command, flag) in [("apply", "/UpdateDBCfg"), ("reset", "/RollbackCfg")] {
        let (_dir, config, calls) = setup("File=base", "sleep 1\nexit 19");
        let (status, json) = invoke(&config, command, &[]);
        assert!(!status.success(), "{json}");
        assert_eq!(json["data"]["completed"], false);
        assert_eq!(json["data"]["provider_dispatched"], true);
        assert_eq!(json["data"]["status"], "failed");
        assert_eq!(json["data"]["interruption"]["deferred"], true);
        assert_eq!(json["data"]["interruption"]["kind"], "timed_out");
        let argv = fs::read_to_string(calls).unwrap();
        assert_eq!(argv.lines().last(), Some(flag));
        assert!(!argv.lines().any(|argument| argument == "-Extension"));
        assert_eq!(
            argv.lines()
                .filter(|argument| *argument == "DESIGNER")
                .count(),
            1
        );
        support::command_data::assert_data_matches_a_declared_form(&json, command);
    }
}

fn assert_text_transition_reports_deferred_timeout(exit_code: i32) {
    for command in ["apply", "reset"] {
        let (_dir, config, _calls) = setup("File=base", &format!("sleep 1\nexit {exit_code}"));
        let mut invocation = v8_runner_command();
        invocation.args(["--config", config.to_str().unwrap(), "--no-color", command]);
        if command == "reset" {
            invocation.arg("--force");
        }
        let output = invocation.output().unwrap();
        assert_eq!(output.status.success(), exit_code == 0);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            text.contains("timeout") && text.contains("unsafe interruption was not performed"),
            "{text}"
        );
        if exit_code == 0 {
            assert!(text.contains(&format!("{command} completed")), "{text}");
            assert!(!text.contains("failed"), "{text}");
        } else {
            assert!(text.contains("platform exit code 7"), "{text}");
            assert!(!text.contains(&format!("{command} completed")), "{text}");
        }
    }
}

#[test]
fn text_transition_reports_deferred_timeout_on_success() {
    assert_text_transition_reports_deferred_timeout(0);
}

#[test]
fn text_transition_reports_deferred_timeout_on_failure() {
    assert_text_transition_reports_deferred_timeout(7);
}

#[test]
fn missing_extension_is_refused_even_when_designer_would_exit_successfully() {
    for command in ["apply", "reset"] {
        let (_dir, config, calls) = setup("File=base", "exit 0");
        let (status, json) = invoke(&config, command, &["--extension", "NoSuchCompatExtension"]);
        assert!(!status.success(), "{json}");
        assert_eq!(json["data"]["completed"], false);
        let recorded = fs::read_to_string(calls).unwrap_or_default();
        assert!(
            !recorded
                .lines()
                .any(|arg| arg == "/UpdateDBCfg" || arg == "/RollbackCfg"),
            "{recorded}"
        );
    }
}

#[test]
fn extension_presence_requires_fresh_nonempty_regular_artifact_and_cleans_probe() {
    for command in ["apply", "reset"] {
        for artifact_action in [": > \"$argument\"", "ln -s \"$0\" \"$argument\"", "exit 1"] {
            let script = format!("previous=''\nfor argument in \"$@\"; do\n  if [ \"$previous\" = /DumpCfg ]; then {artifact_action}; fi\n  previous=\"$argument\"\ndone\nexit 0");
            let (dir, config, calls) = setup("File=base", &script);
            set_timeout(&config, 3000);
            let (status, json) = invoke(&config, command, &["--extension", "InstalledAddon"]);
            assert!(!status.success(), "{json}");
            assert_eq!(json["data"]["completed"], false);
            assert_eq!(json["data"]["provider_dispatched"], true);
            let recorded = fs::read_to_string(calls).unwrap();
            assert!(!recorded
                .lines()
                .any(|arg| arg == "/UpdateDBCfg" || arg == "/RollbackCfg"));
            assert!(!fs::read_dir(dir.path().join("work"))
                .unwrap()
                .any(|item| item
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".extension-presence-")));
        }
    }
}

#[test]
fn cancelled_presence_probe_never_dispatches_critical_transition() {
    for command in ["apply", "reset"] {
        let (dir, config, calls) = setup("File=base", "sleep 10\nexit 0");
        set_timeout(&config, 30000);
        let mut invocation = v8_runner_command();
        invocation.args([
            "--config",
            config.to_str().unwrap(),
            "--json-message",
            command,
            "--extension",
            "InstalledAddon",
        ]);
        if command == "reset" {
            invocation.arg("--force");
        }
        let child = invocation
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        assert!(
            support::wait_for_file(&calls, std::time::Duration::from_secs(10)),
            "probe was not dispatched"
        );
        assert!(std::process::Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success());
        let output = child.wait_with_output().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(!output.status.success(), "{json}");
        assert_eq!(json["data"]["status"], "cancelled");
        assert_eq!(json["data"]["completed"], false);
        assert_eq!(json["data"]["interruption"]["deferred"], false);
        assert_eq!(json["data"]["interruption"]["phase"], "extension_presence");
        let recorded = fs::read_to_string(calls).unwrap();
        assert!(!recorded
            .lines()
            .any(|arg| arg == "/UpdateDBCfg" || arg == "/RollbackCfg"));
        assert!(!fs::read_dir(dir.path().join("work"))
            .unwrap()
            .any(|item| item
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".extension-presence-")));
    }
}
