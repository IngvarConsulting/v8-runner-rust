//! `platforms`: установки платформы и выбор раннера для проекта (Unica #2202).
//!
//! Поддельные установки — каталоги версий с исполняемыми скриптами, которые пишут свои
//! вызовы в журнал: по нему видно, что команда платформу не запускала.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command, write_shell_script};

/// Проект формата Конфигуратора, исходники которого в формате `format`.
fn project(dir: &Path, format: &str, tools: &str) -> PathBuf {
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("src");
    fs::write(
        src.join("Configuration.xml"),
        format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"{format}\">\n</MetaDataObject>\n"
        ),
    )
    .expect("Configuration.xml");
    let config = dir.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n{tools}"
        ),
    )
    .expect("config");
    config
}

/// Установка версии `version` с утилитами `utilities` под `root`.
fn install(root: &Path, version: &str, utilities: &[&str], calls: &Path) -> PathBuf {
    let dir = root.join(version);
    for utility in utilities {
        write_shell_script(
            &dir.join(utility),
            &format!("echo '{utility} {version}' >> '{}'", calls.display()),
        );
    }
    dir
}

fn platforms(config: &Path, path: Option<&str>) -> Value {
    let mut command = v8_runner_command();
    command
        .arg("--config")
        .arg(config)
        .args(["--json-message", "platforms"]);
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let output = command.output().expect("run platforms");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_data_matches_its_command_form(&envelope, "platforms");
    envelope
}

/// Предупреждает ли ответ о расхождении формата исходников и платформы. Прочие
/// предупреждения загрузки конфига сюда не относятся.
fn warns_about_the_format(envelope: &Value) -> bool {
    envelope["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .any(|warning| {
            warning
                .as_str()
                .is_some_and(|w| w.contains("sources format"))
        })
}

/// Ответ называет каждую установку с её составом, настройку проекта и выбор по каждой
/// утилите; платформу не запускает и рабочего каталога не заводит.
#[test]
fn platforms_name_every_installation_and_the_choice_without_starting_anything() {
    let dir = temp_workspace();
    let root = dir.path().join("platforms");
    let calls = dir.path().join("calls.log");
    install(&root, "8.3.27.2074", &["1cv8", "ibcmd"], &calls);
    install(&root, "8.5.4.1878", &["1cv8", "1cv8c", "ibcmd"], &calls);
    let config = project(
        dir.path(),
        "2.20",
        &format!("tools:\n  platform:\n    path: {}\n", root.display()),
    );

    let envelope = platforms(&config, None);
    let data = &envelope["data"];

    let versions: Vec<&str> = data["installations"]
        .as_array()
        .expect("installations")
        .iter()
        .map(|installation| installation["version"].as_str().expect("version"))
        .collect();
    assert_eq!(versions, ["8.5.4.1878", "8.3.27.2074"], "{data}");
    assert_eq!(data["installations"][1]["components"]["thin_client"], false);
    assert_eq!(data["installations"][1]["components"]["designer"], true);
    assert_eq!(data["requirement"]["path"], root.display().to_string());
    assert_eq!(data["source_format"]["version"], "2.20");
    assert_eq!(data["source_format"]["release"], "8.3.27");
    // Путь задан: формат исходников выбор не решает, берётся новейшая под путём.
    assert_eq!(data["selected"]["designer"]["version"], "8.5.4.1878");
    assert_eq!(data["selected"]["designer"]["chosen_by"], "newest");
    assert_eq!(data["selected"]["designer"]["source"], "explicit");
    assert!(!warns_about_the_format(&envelope), "{envelope}");
    assert!(!calls.exists(), "the platform was started");
    // Структурный ответ пишет журнал действий в `workPath/logs`; другого команда не пишет.
    let written: Vec<_> = fs::read_dir(dir.path().join("work"))
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    assert!(written.iter().all(|name| name == "logs"), "{written:?}");
}

/// Версию не задали: формат исходников выбирает выпуск, который его пишет, а не
/// новейший. Формат, которого раннер не знает, оставляет новейшую с предупреждением.
#[test]
fn without_a_version_the_sources_format_chooses_the_release() {
    let dir = temp_workspace();
    let root = dir.path().join("platforms");
    let calls = dir.path().join("calls.log");
    let older = install(&root, "8.3.27.2074", &["1cv8"], &calls);
    let newer = install(&root, "8.5.4.1878", &["1cv8"], &calls);
    let path = format!("{}:{}", newer.display(), older.display());
    let config = project(dir.path(), "2.20", "");

    let envelope = platforms(&config, Some(&path));
    let designer = &envelope["data"]["selected"]["designer"];
    assert_eq!(designer["chosen_by"], "source_format", "{envelope}");
    assert!(
        designer["version"]
            .as_str()
            .is_some_and(|version| version.starts_with("8.3.27.")),
        "{envelope}"
    );
    assert!(!warns_about_the_format(&envelope), "{envelope}");

    let config = project(dir.path(), "2.99", "");
    let envelope = platforms(&config, Some(&path));
    assert_eq!(
        envelope["data"]["selected"]["designer"]["chosen_by"], "newest",
        "{envelope}"
    );
    let warnings = envelope["warnings"].as_array().expect("warnings");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().is_some_and(|w| w.contains("2.99"))),
        "{envelope}"
    );
    assert!(!calls.exists(), "the platform was started");
}
