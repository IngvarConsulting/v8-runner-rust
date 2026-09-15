//! Квитанция о выборе исполнителя — одна форма у всех операций.
//!
//! Она объясняет принятое решение и не предлагает другого: кто выбран, откуда взялся
//! выбор — умолчание матрицы или ключ `providers.*` с именем файла — и кого пропустили
//! с какой причиной. Здесь она проверяется не у экспортного семейства, где появилась
//! первой, а у остальных команд.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn write_project(dir: &Path, utilities: &[&str], providers_yaml: &str) -> PathBuf {
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
    // Версионная раскладка: строгий поиск с версией не уходит за пределы каталога, а
    // версию читает из имени корня установки — так отсутствие утилиты доказуемо.
    let bin = install_dir.join("8.3.27.2074").join("bin");
    fs::create_dir_all(&bin).expect("platform dir");
    for utility in utilities {
        write_shell_script(&bin.join(utility), "exit 0");
    }
    fs::write(dir.join("main.cf"), "cf").expect("artifact");

    let config_path = dir.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {}\nformat: DESIGNER\n{providers_yaml}infobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n    strict: true\n    version: '8.3.27'\n",
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

/// Каждая операция с исполнителем называет его в квитанции — и превью тоже.
#[test]
fn every_operation_with_an_executor_answers_with_a_receipt() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), &["1cv8", "ibcmd"], "");
    let artifact = dir.path().join("main.cf").display().to_string();

    let expectations: Vec<(Vec<&str>, &str)> = vec![
        (vec!["build", "--dry-run"], "designer"),
        (vec!["dump", "--mode", "full", "--dry-run"], "designer"),
        (vec!["init", "--dry-run"], "designer"),
        (vec!["make", "--output", &artifact, "--dry-run"], "designer"),
        (vec!["load", "--path", &artifact, "--dry-run"], "designer"),
        (
            vec!["syntax", "designer-config", "--thin-client"],
            "designer",
        ),
        (vec!["extensions", "list", "--dry-run"], "ibcmd"),
    ];
    for (command, expected) in expectations {
        let (_code, payload) = run(&config_path, &command);
        let receipt = &payload["data"]["provider"];
        assert_eq!(
            receipt["selected"],
            expected,
            "`{}` named the wrong executor: {payload}",
            command.join(" ")
        );
        assert_eq!(
            receipt["origin"]["kind"],
            "default",
            "`{}` did not say the choice was the default: {payload}",
            command.join(" ")
        );
    }
}

/// Умолчание — цепочка: без Конфигуратора сборка идёт через `ibcmd`, и квитанция
/// называет пропущенного с причиной.
#[test]
fn a_default_chain_reports_who_was_skipped_and_why() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), &["ibcmd"], "");

    let (code, payload) = run(&config_path, &["build", "--dry-run"]);

    assert_eq!(code, 0, "{payload}");
    let receipt = &payload["data"]["provider"];
    assert_eq!(receipt["selected"], "ibcmd");
    assert_eq!(receipt["skipped"][0]["provider"], "designer");
    assert!(receipt["skipped"][0]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("not ready")));
}

/// Переопределение называет файл, из которого пришло, и не откатывается.
#[test]
fn an_override_names_its_file_and_never_falls_back() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), &["1cv8"], "providers:\n  build: ibcmd\n");

    let (code, payload) = run(&config_path, &["build", "--dry-run"]);

    assert_ne!(
        code, 0,
        "an override must not fall back to the ready Designer: {payload}"
    );
    let receipt = &payload["data"]["provider"];
    assert_eq!(receipt["selected"], Value::Null);
    assert_eq!(receipt["origin"]["kind"], "override");
    assert_eq!(receipt["origin"]["file"], "v8project.yaml");
    assert_eq!(receipt["skipped"][0]["provider"], "ibcmd");
}

/// Локальный слой тоже переопределяет, и квитанция отличает его от проектного файла.
#[test]
fn a_local_override_is_attributed_to_the_local_file() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), &["1cv8", "ibcmd"], "");
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "providers:\n  build: ibcmd\n",
    )
    .expect("local overlay");

    let (code, payload) = run(&config_path, &["build", "--dry-run"]);

    assert_eq!(code, 0, "{payload}");
    let receipt = &payload["data"]["provider"];
    assert_eq!(receipt["selected"], "ibcmd");
    assert_eq!(receipt["origin"]["file"], "v8project.local.yaml");
}
