//! Карта `infobases` местного слоя и выбор базы ключом `--infobase`
//! (`DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT`).
//!
//! Щуп — `launch thin --dry-run`: он не запускает клиент, а план называет строку
//! подключения, по которой видно, какая база выбрана.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};
use tempfile::TempDir;

struct Project {
    dir: TempDir,
    config_path: PathBuf,
}

impl Project {
    fn local_path(&self) -> PathBuf {
        self.dir.path().join("v8project.local.yaml")
    }

    fn write_local(&self, body: &str) {
        fs::write(self.local_path(), body).expect("local overlay");
    }

    /// Запуск с `--json-message`; `before` — глобальные ключи перед командой.
    fn run_json(&self, before: &[&str], command: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config_path)
            .arg("--json-message")
            .args(before)
            .args(command)
            .output()
            .expect("run v8-runner")
    }

    fn run_text(&self, before: &[&str], command: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config_path)
            .args(before)
            .args(command)
            .output()
            .expect("run v8-runner")
    }
}

/// Проектный файл без секции базы: базы объявляет местный слой.
fn project() -> Project {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let platform = dir.path().join("platform");
    fs::create_dir_all(dir.path().join("project")).expect("sources");
    fs::create_dir_all(&work_path).expect("work");
    write_shell_script(&platform.join("bin").join("1cv8"), "exit 0");
    write_shell_script(&platform.join("bin").join("1cv8c"), "exit 0");
    let config_path = dir.path().join("v8project.yaml");
    fs::write(&config_path, project_file(&work_path, &platform)).expect("config");
    Project { dir, config_path }
}

fn project_file(work_path: &Path, platform: &Path) -> String {
    format!(
        "workPath: '{}'\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        platform.display()
    )
}

const LAUNCH_PREVIEW: &[&str] = &["launch", "thin", "--dry-run"];

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn planned_args(payload: &Value) -> Vec<String> {
    payload["data"]["plan"]["args"]
        .as_array()
        .unwrap_or_else(|| panic!("planned args in {payload}"))
        .iter()
        .map(|arg| arg.as_str().expect("arg").to_owned())
        .collect()
}

fn refusal_message(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(2),
        "a config refusal exits with 2: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(output);
    assert_eq!(payload["ok"], false);
    payload["data"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("refusal message in {payload}"))
        .to_owned()
}

fn warnings(payload: &Value) -> Vec<String> {
    payload["warnings"]
        .as_array()
        .expect("warnings array")
        .iter()
        .map(|warning| warning.as_str().expect("warning text").to_owned())
        .collect()
}

#[test]
fn origin_is_selected_without_a_flag() {
    let project = project();
    project.write_local(
        "infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n  test:\n    connection: 'File=/tmp/test-ib'\n",
    );

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_eq!(payload["ok"], true);
    let args = planned_args(&payload);
    assert!(
        args.iter().any(|arg| arg.contains("/tmp/origin-ib")),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.contains("/tmp/test-ib")),
        "{args:?}"
    );
    assert!(warnings(&payload).is_empty(), "{payload}");
}

#[test]
fn a_declared_name_is_selected_with_the_flag() {
    let project = project();
    project.write_local(
        "infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n  test:\n    connection: 'File=/tmp/test-ib'\n",
    );

    let output = project.run_json(&["--infobase", "test"], LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = planned_args(&json(&output));
    assert!(
        args.iter().any(|arg| arg.contains("/tmp/test-ib")),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.contains("/tmp/origin-ib")),
        "{args:?}"
    );
}

#[test]
fn a_connection_string_selects_an_ad_hoc_base_even_without_a_local_layer() {
    let project = project();

    let output = project.run_json(&["--infobase", "File=/tmp/ad-hoc-ib"], LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = planned_args(&json(&output));
    assert!(
        args.iter().any(|arg| arg.contains("/tmp/ad-hoc-ib")),
        "{args:?}"
    );
}

#[test]
fn an_ad_hoc_connection_string_must_not_carry_credentials() {
    let project = project();

    for connection in [
        "Srvr=srv;Ref=erp;Usr=Admin;Pwd=secret",
        "/S srv\\erp /N Admin /P secret",
    ] {
        let output = project.run_json(&["--infobase", connection], LAUNCH_PREVIEW);

        let message = refusal_message(&output);
        assert!(message.contains("must not carry credentials"), "{message}");
        assert!(message.contains("infobases.<name>"), "{message}");
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("secret"),
            "the refusal must not echo the password"
        );
    }
}

#[test]
fn an_undeclared_name_is_refused_with_the_declared_names() {
    let project = project();
    project.write_local(
        "infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n  test:\n    connection: 'File=/tmp/test-ib'\n",
    );

    let output = project.run_json(&["--infobase", "prod"], LAUNCH_PREVIEW);

    let message = refusal_message(&output);
    assert!(
        message.contains("infobase 'prod' is not declared"),
        "{message}"
    );
    assert!(message.contains("origin, test"), "{message}");
}

/// `INV.CLI.A-COMMAND-WITHOUT-ORIGIN-NAMES-THE-MISSING-STEP`: без `origin` и без
/// `--infobase` команда отказывает до платформы и называет шаг.
#[test]
fn a_command_without_origin_names_the_missing_step() {
    let project = project();
    project.write_local("infobases:\n  test:\n    connection: 'File=/tmp/test-ib'\n");

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    let message = refusal_message(&output);
    assert!(message.contains("`origin` is not declared"), "{message}");
    assert!(message.contains("--infobase"), "{message}");
    assert!(message.contains("infobases.origin.connection"), "{message}");
    assert!(message.contains("declared: test"), "{message}");
}

/// `INV.CONFIG.AN-INFOBASE-NAME-IS-A-PLAIN-IDENTIFIER`: имя, которое не годится в
/// сегмент пути, отвергается с указанием ключа.
#[test]
fn an_infobase_name_is_a_plain_identifier() {
    let project = project();
    project.write_local(
        "infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n  '../escape':\n    connection: 'File=/tmp/other-ib'\n",
    );

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    let message = refusal_message(&output);
    assert!(message.contains("infobases.../escape"), "{message}");
    assert!(message.contains("plain identifier"), "{message}");
}

#[test]
fn the_map_is_refused_in_the_project_file() {
    let project = project();
    let mut config = fs::read_to_string(&project.config_path).expect("config");
    config.push_str("infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n");
    fs::write(&project.config_path, config).expect("config");

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    let message = refusal_message(&output);
    assert!(
        message.contains("declared only in v8project.local.yaml"),
        "{message}"
    );
}

#[test]
fn both_keys_in_one_file_are_refused() {
    let project = project();
    project.write_local(
        "infobase:\n  connection: 'File=/tmp/old-ib'\ninfobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n",
    );

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    let message = refusal_message(&output);
    assert!(
        message.contains("declares both `infobase` and `infobases`"),
        "{message}"
    );
    assert!(message.contains("v8project.local.yaml"), "{message}");
}

/// Прежний ключ в проектном файле и карта в местном — два файла, одна база: секции
/// сливаются по полям, как сливались до переименования.
#[test]
fn the_project_synonym_merges_with_the_local_map_by_field() {
    let project = project();
    let mut config = fs::read_to_string(&project.config_path).expect("config");
    config.push_str("infobase:\n  connection: 'File=/tmp/project-ib'\n");
    fs::write(&project.config_path, config).expect("config");
    project.write_local("infobases:\n  origin:\n    user: 'Admin'\n");

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    let args = planned_args(&payload);
    assert!(
        args.iter().any(|arg| arg.contains("/tmp/project-ib")),
        "{args:?}"
    );
    assert!(args.contains(&"Admin".to_owned()), "{args:?}");
    let warnings = warnings(&payload);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("`infobase:` in v8project.yaml"),
        "{warnings:?}"
    );
    assert!(warnings[0].contains("infobases.origin"), "{warnings:?}");
}

#[test]
fn the_synonym_in_the_local_layer_is_read_as_origin_and_warned_about() {
    let project = project();
    project.write_local("infobase:\n  connection: 'File=/tmp/local-ib'\n");

    let output = project.run_json(&[], LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert!(planned_args(&payload)
        .iter()
        .any(|arg| arg.contains("/tmp/local-ib")));
    let warnings = warnings(&payload);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("`infobase:` in v8project.local.yaml"),
        "{warnings:?}"
    );
}

/// В тексте предупреждение загрузки — узел `▲ config:` со своей подробностью перед
/// лентой команды (`CTR.CLI.TEXT-OUTPUT`).
#[test]
fn the_synonym_warning_is_a_node_of_its_own_in_text_mode() {
    let project = project();
    let mut config = fs::read_to_string(&project.config_path).expect("config");
    config.push_str("infobase:\n  connection: 'File=/tmp/project-ib'\n");
    fs::write(&project.config_path, config).expect("config");

    let output = project.run_text(&[], LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let node = lines
        .iter()
        .position(|line| line.starts_with("▲ config: "))
        .unwrap_or_else(|| panic!("config node in:\n{stdout}"));
    assert!(
        lines[node + 1].starts_with("│   [warning] `infobase:` in v8project.yaml"),
        "{stdout}"
    );
    assert_eq!(
        lines[node + 2],
        "│",
        "a separator joins the config node to the command's timeline:\n{stdout}"
    );
    assert!(
        lines[node + 3..]
            .iter()
            .any(|line| line.starts_with("● ") || line.starts_with("▲ ")),
        "the command's own node follows:\n{stdout}"
    );
}

/// Сервер MCP грузит конфиг тем же путём: ключ `--infobase` действует и на него.
#[test]
fn mcp_serve_selects_the_infobase_by_the_same_flag() {
    let project = project();
    project.write_local("infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n");

    let output = v8_runner_command()
        .arg("--config")
        .arg(&project.config_path)
        .args(["--infobase", "prod", "mcp", "serve", "stdio"])
        .output()
        .expect("run mcp");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("infobase 'prod' is not declared"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
