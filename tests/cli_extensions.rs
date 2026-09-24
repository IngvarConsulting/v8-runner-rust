#![cfg(unix)]

mod support;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use support::{
    temp_workspace, v8_runner_command, wait_for_received_line, write_shell_script as write_script,
};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const V8_EXTENSION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExtensionNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

fn write_native_edt_project(
    path: &Path,
    project_name: &str,
    nature: &str,
    base_project: Option<&str>,
) {
    fs::create_dir_all(path.join("metadata")).expect("metadata");
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src").join("Configuration")).expect("src");
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{project_name}</name>\n  <natures>\n    <nature>{nature}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    let base_project_line = base_project
        .map(|value| format!("Base-Project: {value}\n"))
        .unwrap_or_default();
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!(
            "{base_project_line}Manifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"
        ),
    )
    .expect("manifest");
    fs::write(
        path.join("src")
            .join("Configuration")
            .join("Configuration.mdo"),
        "<Configuration />\n",
    )
    .expect("configuration marker");
    fs::write(
        path.join("src").join("Configuration").join("Module.bsl"),
        "Procedure Test()\nEndProcedure\n",
    )
    .expect("module marker");
}

fn write_edt_configuration_source(path: &Path, project_name: &str) {
    write_native_edt_project(path, project_name, V8_CONFIGURATION_NATURE, None);
    fs::write(
        path.join("metadata").join("Configuration.xml"),
        "<Configuration />",
    )
    .expect("descriptor");
}

fn write_edt_extension_source(path: &Path, project_name: &str) {
    write_native_edt_project(
        path,
        project_name,
        V8_EXTENSION_NATURE,
        Some("configuration"),
    );
    fs::write(
        path.join("metadata").join("Configuration.xml"),
        "<Configuration><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>",
    )
    .expect("descriptor");
}

fn setup_extensions_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = base_path.join("v8project.yaml");
    let ibcmd_path = dir.path().join("ibcmd");
    let calls_log = dir.path().join("ibcmd.calls.log");

    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::create_dir_all(base_path.join("exts").join("client-mcp")).expect("client_mcp dir");
    fs::create_dir_all(base_path.join("tests")).expect("tests dir");
    fs::create_dir_all(&work_path).expect("work");
    write_edt_configuration_source(&base_path.join("configuration"), "configuration");
    write_edt_extension_source(
        &base_path.join("exts").join("client-mcp"),
        "client-mcp-project",
    );
    write_edt_extension_source(&base_path.join("tests"), "tests-project");
    write_script(
        &ibcmd_path,
        &format!("printf '%s\\n' \"$*\" >> '{}'\nexit 0", calls_log.display()),
    );

    let config = format!(
        "workPath: '{}'\nformat: EDT\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: configuration\n  - name: client_mcp\n    type: EXTENSION\n    path: exts/client-mcp\n  - name: tests\n    type: EXTENSION\n    path: tests\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        dir.path().join("ib").display(),
        ibcmd_path.display(),
    );
    fs::write(&config_path, config).expect("config");

    (dir, config_path, calls_log, ibcmd_path)
}

/// Rewrites the fake `ibcmd` so a read command answers with the measured inventory text
/// while every call is still logged.
fn write_inventory_ibcmd(ibcmd_path: &Path, calls_log: &Path, inventory: &str) {
    write_script(
        ibcmd_path,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\nfor arg in \"$@\"; do last=\"$arg\"; done\ncase \"$*\" in\n  *\"extension list\"*|*\"extension info\"*) printf '%s' '{}' ;;\n  *\" save \"*) printf 'saved database extension' > \"$last\" ;;\n  *\" export \"*) mkdir -p \"$last\"; printf '%s' '<MetaDataObject><Configuration><Properties><Name>Проба</Name><Version/><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose><NamePrefix>Пр_</NamePrefix></Properties></Configuration></MetaDataObject>' > \"$last/Configuration.xml\" ;;\nesac\nexit 0",
            calls_log.display(),
            inventory
        ),
    );
}

const MEASURED_INVENTORY: &str = "name                         : \"Проба\"\nversion                      : \nactive                       : yes\npurpose                      : add-on\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"9hfFb6YVX2OwLKZaL1L69Eq0Vrg=\"\n";

#[test]
fn extensions_read_is_previewed_because_it_starts_the_platform() {
    let (_dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    write_inventory_ibcmd(&ibcmd_path, &calls_log, MEASURED_INVENTORY);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
            "list",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json envelope");
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false);
    // Nothing was asked of the platform, so no record may be reported.
    assert!(data["extensions"]
        .as_array()
        .expect("extensions")
        .is_empty());
    let plan = data["plan"].as_str().expect("plan");
    assert!(
        plan.contains("would read every installed extension"),
        "{plan}"
    );
    assert!(plan.contains("file infobase"), "{plan}");
    // The subject of the read is a field, not a phrase: a caller fences on it
    // without parsing `plan`.
    assert_eq!(data["requested"], serde_json::json!({"kind": "all"}));
    assert!(!calls_log.exists(), "preview must not dispatch ibcmd");
}

/// `info --dry-run` names the requested extension structurally, the same way a
/// change preview names its `target`: the wording of `plan` is for people.
#[test]
fn extensions_info_preview_names_the_requested_extension_as_data() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
            "info",
            "--name",
            "Проба",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json envelope");
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false);
    assert_eq!(
        data["requested"],
        serde_json::json!({"kind": "named", "name": "Проба"})
    );
    assert!(data["extensions"]
        .as_array()
        .expect("extensions")
        .is_empty());
    assert!(data["plan"].as_str().expect("plan").contains("'Проба'"));
    assert!(!calls_log.exists(), "preview must not dispatch ibcmd");
}

#[test]
fn extension_change_preview_names_the_target_and_changes_nothing() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
            "activate",
            "--name",
            "Проба",
            "--active",
            "no",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Infobase extension change preview"),
        "{stdout}"
    );
    // A preview performed nothing, so the step must not read as done.
    assert!(stdout.contains("-> planned"), "{stdout}");
    assert!(!stdout.contains("-> ok"), "{stdout}");
    assert!(stdout.contains("would deactivate 'Проба'"), "{stdout}");
    assert!(!calls_log.exists(), "preview must not dispatch ibcmd");
}

#[test]
fn extension_preview_never_echoes_the_infobase_password() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace(
            "infobase:\n  connection: '",
            "infobase:\n  user: Админ\n  password: s3cret\n  connection: '",
        ),
    )
    .expect("config with credentials");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
            "list",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(reported.contains("as 'Админ'"), "{reported}");
    assert!(!reported.contains("s3cret"), "{reported}");
    assert!(!calls_log.exists());
}

#[test]
fn extensions_list_reports_the_installed_composition() {
    let (dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    write_inventory_ibcmd(&ibcmd_path, &calls_log, MEASURED_INVENTORY);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
            "list",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json envelope");
    let extensions = envelope["data"]["extensions"]
        .as_array()
        .expect("extensions");
    assert_eq!(extensions.len(), 1);
    // The answer says what was asked, so a caller can pair it with its request.
    assert_eq!(
        envelope["data"]["requested"],
        serde_json::json!({"kind": "all"})
    );
    assert_eq!(extensions[0]["name"], "Проба");
    assert_eq!(extensions[0]["purpose"], "add-on");
    assert_eq!(extensions[0]["active"], true);
    assert_eq!(extensions[0]["name_prefix"], "Пр_");
    // An empty platform field is an absent value, not an empty string.
    assert!(extensions[0].get("version").is_none());
    let calls = fs::read_to_string(calls_log).expect("calls");
    let calls = calls.lines().collect::<Vec<_>>();
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert!(calls[0].contains("extension list"), "{calls:?}");
    assert!(
        calls[1].contains(" save --db --extension Проба"),
        "{calls:?}"
    );
    assert!(calls[2].contains(" export --file="), "{calls:?}");
    assert!(calls[3].contains("extension list"), "{calls:?}");
    let saved = calls[1].split_whitespace().last().expect("saved path");
    assert!(calls[2].contains(&format!("--file={saved}")), "{calls:?}");
    let temp = dir.path().join("work/temp");
    assert_eq!(fs::read_dir(temp).expect("temp root").count(), 0);
}

#[test]
fn extension_inventory_refuses_a_drifted_applied_record() {
    let (dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    let state = dir.path().join("read-count");
    let drifted = MEASURED_INVENTORY.replace("9hfFb6YVX2OwLKZaL1L69Eq0Vrg=", "changed-hash");
    write_script(
        &ibcmd_path,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\nfor arg in \"$@\"; do last=\"$arg\"; done\ncase \"$*\" in\n  *\"extension list\"*) if test -e '{}'; then printf '%s' '{}'; else touch '{}'; printf '%s' '{}'; fi ;;\n  *\" save \"*) printf 'applied' > \"$last\" ;;\n  *\" export \"*) mkdir -p \"$last\"; printf '%s' '<MetaDataObject><Configuration><Properties><Name>Проба</Name><Version/><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose><NamePrefix>Пр_</NamePrefix></Properties></Configuration></MetaDataObject>' > \"$last/Configuration.xml\" ;;\nesac\nexit 0",
            calls_log.display(),
            state.display(),
            drifted,
            state.display(),
            MEASURED_INVENTORY,
        ),
    );
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
            "list",
        ])
        .output()
        .expect("run command");
    assert!(!output.status.success());
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json envelope");
    assert!(envelope["error"]["message"]
        .as_str()
        .expect("error")
        .contains("changed while reading applied prefixes"));
    assert_eq!(
        fs::read_dir(dir.path().join("work/temp"))
            .expect("temp root")
            .count(),
        0
    );
}

#[test]
fn extension_inventory_refuses_unattested_prefix_and_cleans_private_snapshots() {
    for failure in ["save", "export", "descriptor"] {
        let (dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
        let descriptor = if failure == "descriptor" {
            "<MetaDataObject><Configuration><Properties><Name>Wrong</Name><Version/><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose><NamePrefix>Пр_</NamePrefix></Properties></Configuration></MetaDataObject>"
        } else {
            "<MetaDataObject><Configuration><Properties><Name>Проба</Name><Version/><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose><NamePrefix>Пр_</NamePrefix></Properties></Configuration></MetaDataObject>"
        };
        write_script(
            &ibcmd_path,
            &format!(
                "printf '%s\\n' \"$*\" >> '{}'\nfor arg in \"$@\"; do last=\"$arg\"; done\ncase \"$*\" in\n  *\"extension list\"*) printf '%s' '{}' ;;\n  *\" save \"*) if test '{}' = save; then printf '%s' 'Pwd=secret' >&2; exit 7; fi; printf 'applied' > \"$last\" ;;\n  *\" export \"*) if test '{}' = export; then printf '%s' 'Pwd=secret' >&2; exit 7; fi; mkdir -p \"$last\"; printf '%s' '{}' > \"$last/Configuration.xml\" ;;\nesac\nexit 0",
                calls_log.display(),
                MEASURED_INVENTORY,
                failure,
                failure,
                descriptor,
            ),
        );
        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
                "extensions",
                "list",
            ])
            .output()
            .expect("run command");
        assert!(!output.status.success(), "{failure}: {output:?}");
        let envelope: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("json envelope");
        let message = envelope["error"]["message"].as_str().expect("error");
        assert!(!message.contains("Pwd=secret"), "{failure}: {message}");
        assert_eq!(
            fs::read_dir(dir.path().join("work/temp"))
                .expect("temp root")
                .count(),
            0,
            "{failure}"
        );
    }
}

#[test]
fn extension_inventory_does_not_echo_initial_platform_credentials() {
    let (_dir, config_path, _calls_log, ibcmd_path) = setup_extensions_project();
    write_script(&ibcmd_path, "printf '%s' 'Pwd=secret' >&2\nexit 17");
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
            "list",
        ])
        .output()
        .expect("run command");
    assert!(!output.status.success());
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json envelope");
    let message = envelope["error"]["message"].as_str().expect("error");
    assert!(message.contains("exit code 17"), "{message}");
    assert!(!message.contains("Pwd=secret"), "{message}");
}

/// Пустое имя и имя, не являющееся идентификатором 1С, отклоняются до того, как
/// раннер пойдёт в базу: цена ошибки не должна включать запуск утилиты.
#[test]
fn extensions_info_rejects_a_name_that_is_not_an_identifier_before_touching_the_infobase() {
    for name in ["", "   ", "имя с пробелом", "1начинается-с-цифры"] {
        let (_dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
        write_inventory_ibcmd(&ibcmd_path, &calls_log, MEASURED_INVENTORY);

        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--no-color",
                "extensions",
                "info",
                "--name",
                name,
            ])
            .output()
            .expect("run command");

        let reported = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "name {name:?} must be refused: {reported}"
        );
        assert!(
            !calls_log.exists(),
            "name {name:?} must be refused before the platform is called: {reported}"
        );
    }
}

#[test]
fn extensions_info_refuses_a_reply_about_another_extension() {
    let (_dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    write_inventory_ibcmd(&ibcmd_path, &calls_log, MEASURED_INVENTORY);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
            "info",
            "--name",
            "Другая",
        ])
        .output()
        .expect("run command");

    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{reported}");
    assert!(reported.contains("Другая"), "{reported}");
}

#[test]
fn extensions_create_delete_and_activate_reach_the_platform_verbs() {
    for (arguments, expected) in [
        (
            vec![
                "create",
                "--name",
                "Проба",
                "--name-prefix",
                "Пр_",
                "--purpose",
                "add-on",
            ],
            vec![
                "extension create",
                "--name Проба",
                "--name-prefix Пр_",
                "--purpose add-on",
            ],
        ),
        (
            vec!["delete", "--name", "Проба"],
            vec!["extension delete", "--name Проба"],
        ),
        (
            vec!["activate", "--name", "Проба", "--active", "no"],
            vec!["extension update", "--name Проба", "--active no"],
        ),
    ] {
        let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
        let mut command = v8_runner_command();
        command.args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
        ]);
        command.args(&arguments);
        let output = command.output().expect("run command");

        assert!(output.status.success(), "{arguments:?}");
        let calls = fs::read_to_string(calls_log).expect("calls");
        for fragment in expected {
            assert!(calls.contains(fragment), "{fragment} missing from {calls}");
        }
    }
}

#[test]
fn extensions_command_updates_all_extension_properties() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("│"));
    assert!(stdout.contains("◌ client_mcp: disable_safety"));
    assert!(stdout.contains("updating extension properties"));
    assert!(stdout.contains("│   безопасный режим"));
    assert!(stdout.contains("◌ tests: disable_safety"));
    assert!(stdout.contains("● Extension properties updated successfully"));
    assert_eq!(stdout.matches("◌ client_mcp: disable_safety").count(), 1);
    assert!(!stdout.contains("[Расширения]"));

    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("extension update"));
    assert!(calls.contains("--name client_mcp"));
    assert!(calls.contains("--name tests"));
    assert!(calls.contains("--safe-mode no"));
    assert!(calls.contains("--unsafe-action-protection no"));
}

#[test]
fn extensions_command_streams_stage_before_pipeline_finishes() {
    let (_dir, config_path, _calls_log, ibcmd_path) = setup_extensions_project();
    // The second extension blocks for far longer than the command needs to start, so the
    // window in which the first stage must appear does not compete with process startup.
    // The child is killed once the assertions hold, so the suite does not wait it out.
    write_script(
        &ibcmd_path,
        "case \"$*\" in\n  *\"--name tests\"*) sleep 30 ;;\nesac\nexit 0",
    );

    let mut command = v8_runner_command();
    command
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn command");
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let saw_first_stage = wait_for_received_line(
        &rx,
        Duration::from_secs(15),
        Duration::from_millis(50),
        |line| line.contains("◌ client_mcp: disable_safety"),
    );

    let still_running = child.try_wait().expect("try wait").is_none();
    let _ = child.kill();
    let _ = child.wait();

    assert!(saw_first_stage, "first extension stage was not streamed");
    assert!(
        still_running,
        "process finished before the delayed second extension"
    );
}

#[test]
fn extensions_command_filters_by_requested_source_set_names() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "extensions",
            "--name",
            "client_mcp",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("--name client_mcp"));
    assert!(!calls.contains("--name tests"));
}

#[test]
fn extensions_command_json_failure_reports_operation_target_and_exit_code() {
    let (_dir, config_path, _calls_log, ibcmd_path) = setup_extensions_project();
    write_script(&ibcmd_path, "echo 'cannot update extension' >&2\nexit 17");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["data"]["steps"][0]["ok"], false);
    assert!(payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message")
        .contains("extension update failed for extension 'client_mcp' with exit code 17"));
    assert!(payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message")
        .contains("stderr: cannot update extension"));
}

#[test]
fn extensions_command_json_failure_without_payload_keeps_machine_readable_error() {
    let (_dir, config_path, _calls_log, _ibcmd_path) = setup_extensions_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "extensions",
            "--name",
            "missing",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "extensions");
    assert_eq!(payload["duration_ms"], 0);
    assert_eq!(
        payload["error"]["code"],
        serde_json::Value::String("invalid_argument".to_owned())
    );
    assert!(payload["error"]["message"]
        .as_str()
        .expect("message")
        .contains("unknown extension source-set 'missing'"));
    assert!(payload["data"]["message"]
        .as_str()
        .expect("data message")
        .contains("unknown extension source-set 'missing'"));
}

#[test]
fn installed_extension_can_be_configured_without_an_extension_source_set() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let config = fs::read_to_string(&config_path).expect("config");
    let start = config.find("  - name: client_mcp\n").expect("extensions");
    let end = config.find("tools:\n").expect("tools");
    fs::write(
        &config_path,
        format!("{}{}", &config[..start], &config[end..]),
    )
    .expect("main-only config");

    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "extensions",
            "--installed-name",
            "YaXUnit",
        ])
        .output()
        .expect("configure installed extension");

    assert!(output.status.success(), "{output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["provider_dispatched"], true);
    assert_eq!(payload["data"]["steps"].as_array().expect("steps").len(), 1);
    assert_eq!(payload["data"]["steps"][0]["target"], "YaXUnit");
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert_eq!(calls.lines().count(), 1, "{calls}");
    assert!(calls.contains("extension update"), "{calls}");
    assert!(calls.contains("--name YaXUnit"), "{calls}");
    assert!(calls.contains("--safe-mode no"), "{calls}");
    assert!(calls.contains("--unsafe-action-protection no"), "{calls}");
}

#[test]
fn mixed_extension_selectors_preserve_exact_names_and_deduplicate_in_dispatch_order() {
    let (_dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    write_script(
        &ibcmd_path,
        &format!(
            "printf '%s\\n' \"$@\" >> '{}'\nprintf '%s\\n' '__END_CALL__' >> '{}'\nexit 0",
            calls_log.display(),
            calls_log.display(),
        ),
    );
    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "extensions",
            "--installed-name",
            "YaXUnit",
            "--name",
            "tests",
            "--installed-name",
            "tests",
            "--installed-name",
            "YaXUnit",
            "--installed-name",
            "yaXUnit",
            "--installed-name",
            " Тестовое расширение ",
        ])
        .output()
        .expect("configure mixed targets");

    assert!(output.status.success(), "{output:?}");
    let calls = fs::read_to_string(calls_log).expect("calls");
    let targets: Vec<&str> = calls
        .split("__END_CALL__\n")
        .filter(|call| !call.is_empty())
        .map(|call| {
            let args: Vec<_> = call.lines().collect();
            assert!(args.windows(2).any(|pair| pair == ["extension", "update"]));
            let name_arg = args
                .iter()
                .position(|arg| *arg == "--name")
                .expect("name flag");
            args[name_arg + 1]
        })
        .collect();
    assert_eq!(
        targets,
        ["tests", "YaXUnit", "yaXUnit", " Тестовое расширение "]
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let reported: Vec<_> = payload["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|step| step["target"].as_str().expect("target"))
        .collect();
    assert_eq!(reported, targets);
}

#[test]
fn invalid_configured_selector_in_a_mixed_request_fails_before_clean_or_platform() {
    let (dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    let work_path = dir.path().join("work");
    fs::create_dir_all(work_path.join("logs")).expect("logs");
    let sentinel = work_path.join("logs").join("existing.log");
    fs::write(&sentinel, "preserve prior diagnostics").expect("log");
    fs::remove_file(ibcmd_path).expect("remove utility to check validation order");
    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "--clean-before-execution",
            "extensions",
            "--installed-name",
            "YaXUnit",
            "--name",
            "missing",
        ])
        .output()
        .expect("reject invalid source-set");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["error"]["code"], "invalid_argument");
    assert!(payload["error"]["message"]
        .as_str()
        .expect("message")
        .contains("unknown extension source-set 'missing'"));
    assert_eq!(
        fs::read_to_string(sentinel).expect("preserved log"),
        "preserve prior diagnostics"
    );
    assert!(!calls_log.exists());
    assert!(!work_path.join(".v8-runner.workspace.lock").exists());
}

#[test]
fn invalid_installed_names_never_dispatch_or_clean() {
    for invalid in ["", "   ", "bad\nname", "bad\tname", "-unsafe"] {
        let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
        let sentinel = dir.path().join("work/logs/existing.log");
        fs::create_dir_all(sentinel.parent().expect("logs parent")).expect("logs");
        fs::write(&sentinel, "keep").expect("log");
        let output = v8_runner_command()
            .arg("--config")
            .arg(&config_path)
            .args([
                "--json-message",
                "--clean-before-execution",
                "extensions",
                "--installed-name",
                "YaXUnit",
            ])
            .arg(format!("--installed-name={invalid}"))
            .output()
            .expect("reject invalid installed name");
        assert_eq!(
            output.status.code(),
            Some(2),
            "name={invalid:?}: {output:?}"
        );
        assert!(!calls_log.exists(), "name={invalid:?}");
        assert_eq!(fs::read_to_string(sentinel).expect("preserved log"), "keep");
    }
}

#[test]
fn installed_extension_platform_failure_keeps_target_and_exit_code_in_json() {
    let (_dir, config_path, _calls_log, ibcmd_path) = setup_extensions_project();
    write_script(
        &ibcmd_path,
        "echo 'installed extension unavailable' >&2\nexit 17",
    );
    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "extensions",
            "--installed-name",
            "YaXUnit",
        ])
        .output()
        .expect("failed installed extension update");

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["data"]["provider_dispatched"], true);
    assert_eq!(payload["data"]["steps"][0]["target"], "YaXUnit");
    assert_eq!(payload["data"]["steps"][0]["ok"], false);
    let message = payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(
        message.contains("extension 'YaXUnit' with exit code 17"),
        "{message}"
    );
    assert!(
        message.contains("stderr: installed extension unavailable"),
        "{message}"
    );
}

#[test]
fn extensions_parent_preview_is_read_only_and_redacts_credentials() {
    let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let work_path = dir.path().join("work");
    fs::remove_dir_all(&work_path).expect("remove fixture workspace");
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace(
            "infobase:\n  connection: '",
            "infobase:\n  user: Админ\n  password: parent-preview-secret\n  connection: '",
        ),
    )
    .expect("credential config");

    for json in [true, false] {
        let output = v8_runner_command()
            .arg("--config")
            .arg(&config_path)
            .arg(if json { "--json-message" } else { "--no-color" })
            .args(["--log-level", "debug"])
            .args(["extensions", "--installed-name", "YaXUnit", "--dry-run"])
            .output()
            .expect("preview installed extension update");
        assert!(output.status.success(), "{output:?}");
        let reported = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!reported.contains("parent-preview-secret"), "{reported}");
        assert!(reported.contains("YaXUnit"), "{reported}");
        if json {
            let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
            assert_eq!(payload["data"]["provider_dispatched"], false);
            assert_eq!(payload["data"]["steps"].as_array().expect("steps").len(), 1);
            assert_eq!(payload["data"]["steps"][0]["target"], "YaXUnit");
        } else {
            assert!(reported.contains("-> planned"), "{reported}");
            assert!(!reported.contains("-> ok"), "{reported}");
        }
        assert!(!calls_log.exists(), "preview must not launch the platform");
        assert!(
            !work_path.exists(),
            "preview must not create workPath or locks: entries={:?}; output={reported}",
            fs::read_dir(&work_path).map(|entries| entries
                .map(|entry| entry.expect("entry").path())
                .collect::<Vec<_>>())
        );
    }
}

#[test]
fn extensions_parent_preview_bypasses_a_live_workspace_lock_but_apply_does_not() {
    let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let lock_path = dir.path().join("work/.v8-runner.workspace.lock");
    let lock = format!(
        "{{\"tool\":\"v8-runner\",\"pid\":{},\"owner_id\":\"another-owner\",\"created_at\":\"2026-09-13T00:00:00Z\"}}",
        std::process::id()
    );
    fs::write(&lock_path, &lock).expect("live workspace lock");
    for preview in [true, false] {
        let mut command = v8_runner_command();
        command.arg("--config").arg(&config_path).args([
            "--json-message",
            "extensions",
            "--installed-name",
            "YaXUnit",
        ]);
        if preview {
            command.arg("--dry-run");
        }
        let output = command.output().expect("run with live lock");
        assert_eq!(output.status.success(), preview, "{output:?}");
        if !preview {
            let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
            assert!(payload["error"]["message"]
                .as_str()
                .expect("message")
                .contains("workspace"));
        }
        assert!(!calls_log.exists());
        assert_eq!(fs::read_to_string(&lock_path).expect("retained lock"), lock);
    }
}

#[test]
fn extensions_parent_preview_rejects_clean_and_requires_the_platform_utility() {
    for (clean, workspace_exists) in [(true, true), (true, false), (false, true)] {
        let (dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
        let sentinel = dir.path().join("work/logs/existing.log");
        if workspace_exists {
            fs::create_dir_all(sentinel.parent().expect("logs parent")).expect("logs");
            fs::write(&sentinel, "keep").expect("log");
        } else {
            fs::remove_dir_all(dir.path().join("work")).expect("remove fixture workspace");
        }
        if !clean {
            fs::remove_file(ibcmd_path).expect("missing utility");
        }
        let mut command = v8_runner_command();
        command
            .arg("--config")
            .arg(&config_path)
            .arg("--json-message");
        if clean {
            command.arg("--clean-before-execution");
        }
        let output = command
            .args(["extensions", "--installed-name", "YaXUnit", "--dry-run"])
            .output()
            .expect("invalid preview");
        assert!(!output.status.success(), "{output:?}");
        let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(payload["command"], "extensions", "{payload}");
        assert_eq!(payload["ok"], false, "{payload}");
        let message = payload["error"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("missing error message (clean={clean}): {payload}"));
        if clean {
            assert_eq!(output.status.code(), Some(2), "{output:?}");
            assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
            assert_eq!(payload["error"]["kind"], "validation", "{payload}");
            assert!(
                message.contains("preview must not modify workPath"),
                "{message}"
            );
        } else {
            assert!(message.contains("ibcmd"), "{message}");
        }
        assert!(!calls_log.exists());
        if workspace_exists {
            assert_eq!(fs::read_to_string(sentinel).expect("preserved log"), "keep");
        } else {
            assert!(
                !dir.path().join("work").exists(),
                "rejected preview must not create workPath"
            );
        }
    }
}

#[test]
fn init_preview_clean_rejection_uses_the_shared_canonical_error_envelope() {
    let (_dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "--clean-before-execution",
            "infobase",
            "create",
            "--dry-run",
        ])
        .output()
        .expect("reject init preview with clean");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(output.status.code(), Some(2), "{payload}");
    assert_eq!(payload["command"], "infobase create", "{payload}");
    assert_eq!(payload["ok"], false, "{payload}");
    assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(!calls_log.exists(), "rejection must not dispatch ibcmd");
}

#[test]
fn extensions_parent_preview_validates_designer_work_path_without_mutation() {
    let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let project = config_path.parent().expect("project root");
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace("format: EDT", "format: DESIGNER"),
    )
    .expect("Designer config");
    for source in ["configuration", "exts/client-mcp", "tests"] {
        let source_path = project.join(source);
        let descriptor = fs::read(source_path.join("metadata/Configuration.xml"))
            .expect("configuration descriptor");
        fs::remove_dir_all(&source_path).expect("remove EDT source");
        fs::create_dir_all(&source_path).expect("Designer source directory");
        fs::write(source_path.join("Configuration.xml"), descriptor).expect("Designer descriptor");
    }

    let sentinel = dir.path().join("work/logs/existing.log");
    fs::create_dir_all(sentinel.parent().expect("logs parent")).expect("logs");
    fs::write(&sentinel, "keep").expect("log");
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "keep file").expect("file work path");
    let dangling = dir.path().join("dangling");
    let missing_target = dir.path().join("missing-target");
    std::os::unix::fs::symlink(&missing_target, &dangling).expect("dangling symlink");
    let work_alias = dir.path().join("work-alias");
    std::os::unix::fs::symlink(dir.path().join("work"), &work_alias)
        .expect("valid directory alias");
    let symlink_loop = dir.path().join("symlink-loop");
    std::os::unix::fs::symlink(&symlink_loop, &symlink_loop).expect("symlink loop");

    for (relative_path, valid) in [
        ("work", true),
        ("work-alias/new-work", true),
        ("missing/work", true),
        ("scratch/../parent-work", true),
        ("blocker", false),
        ("scratch/../blocker", false),
        ("blocker/child", false),
        ("dangling", false),
        ("dangling/child", false),
        ("symlink-loop/child", false),
    ] {
        let output = v8_runner_command()
            .arg("--config")
            .arg(&config_path)
            .arg("--workdir")
            .arg(dir.path().join(relative_path))
            .args([
                "--json-message",
                "extensions",
                "--installed-name",
                "YaXUnit",
                "--dry-run",
            ])
            .output()
            .expect("Designer preview");
        let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(output.status.success(), valid, "{relative_path}: {payload}");
        assert_eq!(
            payload["command"], "extensions",
            "{relative_path}: {payload}"
        );
        if valid {
            assert_eq!(payload["data"]["provider_dispatched"], false, "{payload}");
        } else {
            assert_eq!(output.status.code(), Some(2), "{relative_path}: {payload}");
            assert_eq!(payload["error"]["kind"], "validation", "{payload}");
            assert!(
                payload["error"]["message"]
                    .as_str()
                    .expect("validation message")
                    .contains("workPath"),
                "{relative_path}: {payload}"
            );
        }
        assert!(!calls_log.exists(), "preview must not dispatch ibcmd");
        assert_eq!(
            fs::read_to_string(&sentinel).expect("preserved log"),
            "keep"
        );
        assert_eq!(
            fs::read_to_string(&blocker).expect("preserved file"),
            "keep file"
        );
        assert_eq!(
            fs::read_link(&dangling).expect("preserved symlink"),
            missing_target
        );
        for missing in ["missing", "scratch", "parent-work", "missing-target"] {
            assert!(
                !dir.path().join(missing).exists(),
                "preview created {missing}"
            );
        }
        assert!(!dir.path().join("work/new-work").exists());
        assert!(!dir.path().join("work/.v8-runner.workspace.lock").exists());
    }
}

#[test]
fn extensions_parent_preview_still_validates_project_sources_without_creating_work_path() {
    let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let work_path = dir.path().join("work");
    fs::remove_dir_all(&work_path).expect("remove fixture workspace");
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace("path: exts/client-mcp", "path: missing-extension-source"),
    )
    .expect("invalid source config");

    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args([
            "--json-message",
            "extensions",
            "--installed-name",
            "YaXUnit",
            "--dry-run",
        ])
        .output()
        .expect("preview with invalid project source");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]["message"]
            .as_str()
            .expect("validation message")
            .contains("missing-extension-source"),
        "{payload}"
    );
    assert!(!calls_log.exists());
    assert!(
        !work_path.exists(),
        "invalid preview must not create workPath"
    );
}

#[test]
fn extensions_parent_preview_rejects_missing_work_path_inside_edt_source_via_symlink() {
    let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let project = config_path.parent().expect("project root");
    let alias = dir.path().join("project-alias");
    std::os::unix::fs::symlink(project, &alias).expect("project alias");
    let work_path = alias.join("configuration/new-work");
    let output = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .arg("--workdir")
        .arg(&work_path)
        .args([
            "--json-message",
            "extensions",
            "--installed-name",
            "YaXUnit",
            "--dry-run",
        ])
        .output()
        .expect("preview with work path overlapping source");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    let message = payload["error"]["message"]
        .as_str()
        .expect("validation message");
    assert!(
        message.contains("overlaps generated work target"),
        "{message}"
    );
    assert!(message.contains("configuration"), "{message}");
    assert!(
        !work_path.exists(),
        "validation must not create workPath inside sources"
    );
    assert!(!calls_log.exists());
}

#[test]
fn extensions_parent_options_with_subcommands_fail_before_clean_or_dispatch() {
    for parent_options in [
        vec!["--name", "tests"],
        vec!["--installed-name", "YaXUnit"],
        vec!["--dry-run"],
    ] {
        let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
        let sentinel = dir.path().join("work/logs/existing.log");
        fs::create_dir_all(sentinel.parent().expect("logs parent")).expect("logs");
        fs::write(&sentinel, "keep").expect("log");
        let output = v8_runner_command()
            .arg("--config")
            .arg(&config_path)
            .args(["--json-message", "--clean-before-execution", "extensions"])
            .args(&parent_options)
            .args(["delete", "--name", "Other"])
            .output()
            .expect("reject parent options with subcommand");

        assert_eq!(
            output.status.code(),
            Some(2),
            "{parent_options:?}: {output:?}"
        );
        assert!(
            !calls_log.exists(),
            "invalid request must never delete Other"
        );
        assert_eq!(fs::read_to_string(sentinel).expect("preserved log"), "keep");
    }
}

#[test]
fn extension_subcommands_accept_global_flags_before_and_after_the_subcommand() {
    let (_dir, config_path, calls_log, ibcmd_path) = setup_extensions_project();
    write_inventory_ibcmd(&ibcmd_path, &calls_log, MEASURED_INVENTORY);
    let output = v8_runner_command()
        .args(["extensions", "--json-message", "list", "--config"])
        .arg(&config_path)
        .output()
        .expect("inventory with global options around subcommand");

    assert!(output.status.success(), "{output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["provider_dispatched"], true);
    assert_eq!(payload["data"]["extensions"][0]["name"], "Проба");
    let calls = fs::read_to_string(calls_log).expect("inventory calls");
    assert!(calls.contains("extension list"), "{calls}");
    assert!(!calls.contains("extension update"), "{calls}");
}

#[test]
fn extensions_preview_and_apply_accept_missing_work_path_with_parent_components() {
    let (dir, config_path, calls_log, _ibcmd_path) = setup_extensions_project();
    let work_path = dir.path().join("work");
    let scratch_path = dir.path().join("scratch");
    fs::remove_dir_all(&work_path).expect("remove fixture workspace");
    let requested_work_path = scratch_path.join("../work");

    for preview in [true, false] {
        let mut command = v8_runner_command();
        command
            .arg("--config")
            .arg(&config_path)
            .arg("--workdir")
            .arg(&requested_work_path)
            .args([
                "--json-message",
                "extensions",
                "--installed-name",
                "YaXUnit",
            ]);
        if preview {
            command.arg("--dry-run");
        }
        let output = command.output().expect("work path with parent components");
        assert!(output.status.success(), "preview={preview}: {output:?}");
        let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(payload["data"]["provider_dispatched"], !preview);
        if preview {
            assert!(
                !scratch_path.exists(),
                "preview must not create transient ancestors"
            );
            assert!(!work_path.exists(), "preview must not create workPath");
            assert!(!calls_log.exists());
        } else {
            assert!(work_path.is_dir(), "apply must prepare workPath");
            assert!(fs::read_to_string(&calls_log)
                .expect("apply calls")
                .contains("--name YaXUnit"));
        }
    }
}
