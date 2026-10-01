//! Исполнителя выбирает матрица, а не вызывающий: у команд нет флага выбора, у
//! инструментов MCP нет такого поля, а снятый ключ `builder` отклоняется с подсказкой.
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

fn write_project(dir: &Path, extra_yaml: &str) -> PathBuf {
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
            "workPath: {}\nformat: DESIGNER\n{extra_yaml}infobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n",
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

/// Ни одна команда не принимает выбор исполнителя аргументом: знание о платформе
/// принадлежит раннеру, а единственное законное место записать его — файл проекта.
#[test]
fn no_command_accepts_a_provider_flag() {
    let commands: [&[&str]; 16] = [
        &["init"],
        &["clone"],
        &["tools", "download"],
        &["infobase", "create"],
        &["extensions"],
        &["extensions", "create"],
        &["build"],
        &["load"],
        &["test"],
        &["dump"],
        &["infobase", "configuration", "export"],
        &["infobase", "dump"],
        &["infobase", "restore"],
        &["convert"],
        &["make"],
        &["launch"],
    ];
    for command in commands {
        let output = v8_runner_command()
            .args(command)
            .arg("--help")
            .output()
            .expect("run help");
        let help = String::from_utf8_lossy(&output.stdout);
        for flag in ["--provider", "--builder", "--backend"] {
            assert!(
                !help.contains(flag),
                "`{}` advertises {flag}:\n{help}",
                command.join(" ")
            );
        }
    }
}

/// Инструменты MCP видят провайдера так же, как CLI: никак. Поверхность закреплена
/// файлом, и поле выбора исполнителя в неё не входит.
#[test]
fn no_mcp_tool_takes_a_provider_field() {
    let text = fs::read_to_string(repo_root().join("docs/schemas/mcp-tools.json"))
        .expect("pinned mcp surface");
    let surface: Value = serde_json::from_str(&text).expect("valid json");
    for (tool, schema) in surface["tools"].as_object().expect("tools map") {
        let properties = schema["properties"].as_object().expect("input properties");
        for forbidden in ["provider", "builder", "backend"] {
            assert!(
                !properties.contains_key(forbidden),
                "tool {tool} takes a `{forbidden}` field"
            );
        }
    }
}

/// Снятый ключ не игнорируется и не переименовывается: конфиг с ним отклоняется, а
/// ошибка называет, что ставить вместо него.
#[test]
fn a_config_with_the_builder_key_is_refused_with_the_replacement_named() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), "builder: IBCMD\n");

    let (code, payload) = run(&config_path, &["build", "--dry-run"]);

    assert_ne!(code, 0, "a removed key must fail validation: {payload}");
    let message = payload["error"]["message"].as_str().expect("error message");
    assert!(message.contains("'builder' is not supported"), "{message}");
    assert!(message.contains("providers.<operation>"), "{message}");
}

/// Переопределение — один исполнитель для одной операции с развилкой. Список,
/// операция без выбора и исполнитель вне матрицы отклоняются до запуска чего-либо.
#[test]
fn a_provider_override_is_a_scalar_for_an_operation_with_a_choice() {
    let cases = [
        (
            "providers:\n  build:\n    - ibcmd\n",
            "a list is not a provider",
            "invalid type: sequence",
        ),
        (
            "providers:\n  load: designer\n",
            "load has one executor",
            "exactly one executor",
        ),
        (
            "providers:\n  build: webinst\n",
            "webinst does not build",
            "does not implement",
        ),
    ];
    for (yaml, why, expected) in cases {
        let dir = temp_workspace();
        let config_path = write_project(dir.path(), yaml);
        let (code, payload) = run(&config_path, &["build", "--dry-run"]);
        assert_ne!(code, 0, "{why}: {payload}");
        let message = payload["error"]["message"].as_str().expect("error message");
        assert!(message.contains(expected), "{why}: {message}");
    }

    let dir = temp_workspace();
    let config_path = write_project(dir.path(), "providers:\n  build: ibcmd\n");
    let (code, payload) = run(&config_path, &["build", "--dry-run"]);
    assert_eq!(code, 0, "a valid override is accepted: {payload}");
}

/// Публикация не входит ни в одну цепочку умолчаний: она меняет веб-сервер вне
/// рабочего каталога и делается только отдельной командой.
#[test]
fn no_default_chain_names_the_publication_provider() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), "");
    for command in [
        vec!["push", "--dry-run"],
        vec!["infobase", "create", "--dry-run"],
        vec!["pull", "--mode", "full", "--dry-run"],
    ] {
        let (code, payload) = run(&config_path, &command);
        assert_eq!(code, 0, "{payload}");
        let text = payload.to_string();
        assert!(
            !text.contains("webinst") && !text.contains("publish"),
            "`{}` mentions publication: {text}",
            command.join(" ")
        );
    }
}
