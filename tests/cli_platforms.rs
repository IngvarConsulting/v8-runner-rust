//! `platforms`: установки платформы и выбор раннера для проекта (Unica #2202).
//!
//! Поддельные установки — каталоги версий с исполняемыми скриптами, которые пишут свои
//! вызовы в журнал: по нему видно, что команда платформу не запускала.
#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::fs::symlink;
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
    // Структурный ответ пишет журнал действий в `workPath/logs`, а заводя `workPath`, раннер
    // кладёт в него `.gitignore` (#624); другого команда не пишет.
    let written: Vec<_> = fs::read_dir(dir.path().join("work"))
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    assert!(
        written
            .iter()
            .all(|name| name == "logs" || name == ".gitignore"),
        "{written:?}"
    );
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

#[test]
fn platforms_inventory_includes_the_canonical_path_installation() {
    let dir = temp_workspace();
    let root = dir.path().join("platforms");
    let calls = dir.path().join("calls.log");
    let version = "8.5.4.4294967295";
    let utilities = ["1cv8", "1cv8c", "ibcmd"];
    let wanted = install(&root, version, &utilities, &calls);
    let older_version = "8.5.4.999999999";
    let older = install(&root, older_version, &["1cv8"], &calls);
    let alias_version = "8.5.1.4294967295";
    let alias = dir.path().join(alias_version);
    fs::create_dir(&alias).expect("alias directory");
    for utility in utilities {
        symlink(wanted.join(utility), alias.join(utility)).expect("utility symlink");
    }
    let path = std::env::join_paths([&older, &alias]).expect("PATH");
    let path = path.to_str().expect("UTF-8 PATH");
    let canonical_root = wanted.canonicalize().expect("canonical version directory");

    for tools in ["", "tools:\n  platform:\n    version: sources\n"] {
        let config = project(dir.path(), "2.22", tools);
        let envelope = platforms(&config, Some(path));
        let data = &envelope["data"];
        for (component, utility) in ["designer", "thin_client", "ibcmd"]
            .into_iter()
            .zip(utilities)
        {
            let selected = &data["selected"][component];
            assert_eq!(selected["version"], version, "{data}");
            assert_eq!(selected["source"], "path", "{data}");
            assert_eq!(selected["chosen_by"], "source_format", "{data}");
            assert_eq!(
                selected["installation_root"],
                canonical_root.to_str().unwrap()
            );
            assert_eq!(
                selected["path"],
                canonical_root.join(utility).to_str().unwrap()
            );
        }

        let installations = data["installations"].as_array().expect("installations");
        let versions: Vec<_> = installations
            .iter()
            .map(|installation| installation["version"].as_str().expect("version"))
            .collect();
        let index = versions
            .iter()
            .position(|&found| found == version)
            .expect("PATH installation");
        let older_index = versions
            .iter()
            .position(|&found| found == older_version)
            .expect("older PATH installation");
        assert!(index < older_index, "{data}");
        assert_eq!(
            versions
                .iter()
                .copied()
                .filter(|&found| found == version)
                .count(),
            1
        );
        assert!(!versions.contains(&alias_version), "{data}");
        let installation = &installations[index];
        assert_eq!(installation["root"], canonical_root.to_str().unwrap());
        assert_eq!(
            installation["components"],
            serde_json::json!({"designer": true, "thin_client": true, "ibcmd": true, "web_extension": false})
        );
    }

    let config = project(
        dir.path(),
        "2.22",
        &format!("tools:\n  platform:\n    path: {}\n", older.display()),
    );
    let envelope = platforms(&config, Some(path));
    let installations = envelope["data"]["installations"]
        .as_array()
        .expect("installations");
    assert_eq!(installations.len(), 1, "{envelope}");
    assert_eq!(installations[0]["version"], older_version, "{envelope}");
    assert!(!calls.exists(), "the platform was started");
}

/// Строки таблицы от минимума до 8.5.4: каждый формат выбирает свой выпуск, а не новейшую
/// установку. Цена этого — и более старая платформа, и компонент только из этого выпуска.
#[test]
fn every_measured_format_chooses_its_release_over_the_newest() {
    let dir = temp_workspace();
    let root = dir.path().join("platforms");
    let calls = dir.path().join("calls.log");
    let releases = [
        ("2.17", "8.3.24.1819"),
        ("2.18", "8.3.25.1633"),
        ("2.19", "8.3.26.1656"),
        ("2.20", "8.3.27.2342"),
        ("2.21", "8.5.1.1529"),
        ("2.22", "8.5.4.1878"),
    ];
    let path = releases
        .iter()
        .rev()
        .map(|(_, version)| {
            install(&root, version, &["1cv8"], &calls)
                .display()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(":");

    for (format, version) in releases {
        let config = project(dir.path(), format, "");
        let envelope = platforms(&config, Some(&path));
        let designer = &envelope["data"]["selected"]["designer"];
        // Выпуск, а не сборка: поиск видит и настоящие установки машины в корнях по умолчанию.
        let release = version.rsplit_once('.').expect("release").0;
        assert!(
            designer["version"]
                .as_str()
                .is_some_and(|found| found.starts_with(&format!("{release}."))),
            "{format}: {envelope}"
        );
        assert_eq!(
            designer["chosen_by"], "source_format",
            "{format}: {envelope}"
        );
    }
    assert!(!calls.exists(), "the platform was started");
}
