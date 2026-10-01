//! Форма конверта ответа закреплена схемой `docs/schemas/command-envelope.schema.json`.
//!
//! Схема — не пересказ структуры, а то, против чего проверяется живой вывод: пока
//! настоящие ответы команд ей соответствуют, обратная совместимость наблюдаема, а её
//! слом виден как падение этой проверки, а не как жалоба потребителя.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn envelope_schema() -> Value {
    let path = repo_root().join("docs/schemas/command-envelope.schema.json");
    let text = fs::read_to_string(&path).expect("envelope schema artefact is present");
    serde_json::from_str(&text).expect("envelope schema is valid json")
}

fn assert_matches_envelope(payload: &Value, context: &str) {
    let schema = envelope_schema();
    let validator = jsonschema::validator_for(&schema).expect("envelope schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(payload)
        .map(|error| format!("{} at {}", error, error.instance_path))
        .collect();
    assert!(
        errors.is_empty(),
        "{context} does not match the pinned envelope form:\n{}",
        errors.join("\n")
    );
}

fn write_project(dir: &Path) -> PathBuf {
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    let install_dir = dir.join("platform");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::write(
        base_path.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&work_path).expect("work dir");
    write_shell_script(&install_dir.join("bin").join("1cv8"), "exit 0");
    write_shell_script(&install_dir.join("bin").join("ibcmd"), "exit 0");

    let config_path = dir.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n",
            work_path.display(),
            dir.join("ib").display(),
            install_dir.display()
        ),
    )
    .expect("write config");
    config_path
}

#[test]
fn a_successful_command_answers_in_the_pinned_envelope_form() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "version",
        ])
        .output()
        .expect("run command");

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_matches_envelope(&payload, "a successful `version` reply");
    assert_eq!(payload["ok"], true);
}

/// Шаги — часть формы конверта, и проверять её надо на команде, которая шаги
/// действительно возвращает: иначе закрытый список полей шага никем не держится.
#[test]
fn a_command_with_steps_keeps_every_step_inside_the_pinned_form() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &dir.path().join("main.cf").display().to_string(),
            "--dry-run",
        ])
        .output()
        .expect("run command");

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_matches_envelope(&payload, "an export preview");
    assert!(
        !payload["steps"].as_array().expect("steps array").is_empty(),
        "the reply must carry steps for this check to mean anything: {payload}"
    );
}

#[test]
fn a_refused_command_answers_in_the_same_envelope_form_with_an_error() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path());

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &dir.path().join("main.txt").display().to_string(),
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success(), "a wrong suffix must be refused");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_matches_envelope(&payload, "a refused export");
    assert_eq!(payload["ok"], false);
    assert!(
        payload["error"]["kind"].is_string(),
        "a refusal names its kind: {payload}"
    );
}
