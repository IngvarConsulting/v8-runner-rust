//! Что обещает превью любой команды: след в журнале действий и отказ до одобрения
//! плана, если платформы нет.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn write_project(dir: &Path, with_platform: bool) -> PathBuf {
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
    fs::create_dir_all(install_dir.join("bin")).expect("platform dir");
    if with_platform {
        write_shell_script(&install_dir.join("bin").join("1cv8"), "exit 0");
        write_shell_script(&install_dir.join("bin").join("ibcmd"), "exit 0");
    }

    // Без платформы поиск обязан отказать, а не уйти в PATH или в корни по умолчанию:
    // строгий режим с версией не даёт локатору найти что-то за пределами каталога.
    let strictness = if with_platform {
        ""
    } else {
        "    strict: true\n    version: '8.3.27'\n"
    };
    let config_path = dir.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n{strictness}",
            work_path.display(),
            dir.join("ib").display(),
            install_dir.display()
        ),
    )
    .expect("write config");
    config_path
}

fn run(config_path: &Path, arguments: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
        ])
        .args(arguments)
        .output()
        .expect("run command");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`{}` printed no json envelope: {error}\nstdout: {}\nstderr: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().unwrap_or(-1), payload)
}

fn previews<'a>(artifact: &'a str) -> Vec<Vec<&'a str>> {
    vec![
        vec!["build", "--dry-run"],
        vec!["dump", "--mode", "full", "--dry-run"],
        vec!["init", "--dry-run"],
        vec!["make", "--output", artifact, "--dry-run"],
        vec!["load", "--path", artifact, "--dry-run"],
        vec!["launch", "designer", "--dry-run"],
    ]
}

/// Превью не прячется: строка о вызове появляется в журнале действий, хотя предмет
/// не меняется. Журнал ведётся под `--json-message` в `workPath/logs/mcp/actions.log`.
#[test]
fn every_preview_leaves_a_line_in_the_action_log() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), true);
    // `load` проверяет, что артефакт существует, раньше, чем строит план.
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    let artifact = dir.path().join("main.cf").display().to_string();
    let log = dir.path().join("work/logs/mcp/actions.log");

    for preview in previews(&artifact) {
        let _ = fs::remove_file(&log);
        let (code, payload) = run(&config_path, &preview);
        // Проверяется след успешного превью: отказ и так оставляет строку об ошибке,
        // и на нём правило доказать нельзя.
        assert_eq!(
            code,
            0,
            "`{}` did not preview: {payload}",
            preview.join(" ")
        );
        let text = fs::read_to_string(&log).unwrap_or_default();
        assert!(
            !text.trim().is_empty(),
            "`{}` left the action log empty: {payload}",
            preview.join(" ")
        );
    }
}

/// Отсутствие платформы отказывает до одобрения плана: превью возвращается после
/// поиска утилиты, и вызывающий узнаёт об этом раньше, чем согласится с планом.
#[test]
fn a_preview_refuses_before_naming_a_plan_when_the_platform_is_missing() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), false);
    let artifact = dir.path().join("main.cf").display().to_string();

    for preview in previews(&artifact) {
        let (code, payload) = run(&config_path, &preview);
        assert_ne!(
            code,
            0,
            "`{}` approved a plan without a platform: {payload}",
            preview.join(" ")
        );
        assert_eq!(payload["ok"], false, "{payload}");
        assert!(
            payload["data"]["plan"].is_null(),
            "`{}` named a plan it cannot run: {payload}",
            preview.join(" ")
        );
    }
}
