//! Глобальные ключи: лист дерева команд либо исполняет ключ, либо отвергает его с
//! названной причиной. Молчаливое согласие — ложь вызывающему
//! (`INV.CLI.A-LEAF-WITHOUT-A-PREVIEW-REFUSES-THE-PREVIEW-KEY`).
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};
use tempfile::TempDir;

// Состав таблицы листьев, общий с `src/cli/global_flags.rs`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/cli/global_flags_expected.in"
));

/// Листья без превью: минимальный вызов, доходящий до отказа, и путь листа, который
/// отказ обязан назвать.
const WITHOUT_PREVIEW: &[(&[&str], &str)] = &[
    (&["version"], "version"),
    (
        &[
            "clone",
            "--connection",
            "File=/tmp/ib",
            "--platform-version",
            "8.3.24",
        ],
        "clone",
    ),
    (&["init"], "init"),
    (&["config", "init"], "config init"),
    (&["tools", "download", "yaxunit"], "tools download yaxunit"),
    (&["tools", "download", "vanessa"], "tools download vanessa"),
    (
        &["tools", "download", "client-mcp"],
        "tools download client-mcp",
    ),
    (&["test", "yaxunit", "all"], "test yaxunit all"),
    (
        &["test", "yaxunit", "module", "ОбщийМодуль"],
        "test yaxunit module",
    ),
    (&["test", "va"], "test va"),
    (&["check"], "check"),
    (&["check", "designer-config"], "check designer-config"),
    (&["check", "designer-modules"], "check designer-modules"),
    (&["check", "edt"], "check edt"),
    (&["mcp", "serve", "stdio"], "mcp serve stdio"),
    (&["mcp", "serve", "http"], "mcp serve http"),
];

/// Листья, которые базы не выбирают вовсе.
const IGNORING_THE_BASE: &[(&[&str], &str)] = &[
    (&["version"], "version"),
    (
        &[
            "clone",
            "--connection",
            "File=/tmp/ib",
            "--platform-version",
            "8.3.24",
        ],
        "clone",
    ),
];

/// Листья, которые базу объявляют сами.
const DECLARING_THE_BASE: &[(&[&str], &str)] =
    &[(&["init"], "init"), (&["config", "init"], "config init")];

struct Project {
    dir: TempDir,
    config_path: PathBuf,
    /// Журнал поддельного тонкого клиента: превью его не зовёт, и файла не появляется.
    client_calls: PathBuf,
}

/// Проект с поддельной платформой: превью её не зовёт, и журнал вызовов это показывает.
fn project() -> Project {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let platform = dir.path().join("platform");
    fs::create_dir_all(dir.path().join("project")).expect("sources");
    fs::create_dir_all(&work_path).expect("work");
    let client_calls = dir.path().join("1cv8c.calls.log");
    write_shell_script(&platform.join("bin").join("1cv8"), "exit 0");
    write_shell_script(
        &platform.join("bin").join("1cv8c"),
        &format!(
            "printf '%s\\n' \"$*\" >> \"{}\"\nexit 0",
            client_calls.display()
        ),
    );
    let config_path = dir.path().join("v8project.yaml");
    fs::write(&config_path, project_file(&work_path, &platform)).expect("config");
    fs::write(
        dir.path().join("v8project.local.yaml"),
        "infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n",
    )
    .expect("local overlay");
    Project {
        dir,
        config_path,
        client_calls,
    }
}

fn project_file(work_path: &Path, platform: &Path) -> String {
    format!(
        "workPath: '{}'\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        platform.display()
    )
}

impl Project {
    fn run(&self, arguments: &[&str]) -> Output {
        v8_runner_command()
            .current_dir(self.dir.path())
            .arg("--config")
            .arg(&self.config_path)
            .arg("--json-message")
            .args(arguments)
            .output()
            .expect("run v8-runner")
    }
}

fn reported(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_leaf_without_a_preview_refuses_the_key_and_names_itself() {
    for (arguments, leaf) in WITHOUT_PREVIEW {
        let output = v8_runner_command()
            .args(arguments.iter())
            .arg("--dry-run")
            .output()
            .expect("run v8-runner");
        let reported = reported(&output);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} отвергается как отказ проверки: {reported}"
        );
        assert!(
            reported.contains("has no preview"),
            "{arguments:?}: {reported}"
        );
        // Отказ называет сам лист, а не корень: вызывающий видит, о какой команде речь.
        assert!(
            reported.contains(&format!("`{leaf}`")),
            "{arguments:?}: {reported}"
        );
    }
}

#[test]
fn the_server_keeps_stdout_for_the_protocol_when_it_refuses_the_preview_key() {
    let output = v8_runner_command()
        .args(["mcp", "serve", "stdio", "--dry-run"])
        .output()
        .expect("run v8-runner");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stdout.is_empty(),
        "stdout занят протоколом: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("has no preview"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_preview_key_means_the_same_before_and_after_the_command() {
    let project = project();

    let after = project.run(&["launch", "thin", "--dry-run"]);
    let before = project.run(&["--dry-run", "launch", "thin"]);

    assert!(after.status.success(), "{}", reported(&after));
    assert!(before.status.success(), "{}", reported(&before));
    let plan = |output: &Output| -> Value {
        serde_json::from_slice::<Value>(&output.stdout).expect("json")["data"]["plan"].clone()
    };
    // План есть, и он один и тот же: иначе равенство сошлось бы на двух пустых ответах.
    assert!(plan(&after)["program"].is_string(), "{}", reported(&after));
    assert!(plan(&after)["args"].is_array(), "{}", reported(&after));
    assert_eq!(plan(&after), plan(&before));
    assert!(!project.client_calls.exists(), "превью клиент не запускает");
}

#[test]
fn a_leaf_that_selects_no_base_refuses_the_base_key() {
    for (arguments, leaf) in IGNORING_THE_BASE {
        let output = v8_runner_command()
            .args(arguments.iter())
            .args(["--infobase", "origin"])
            .output()
            .expect("run v8-runner");
        let reported = reported(&output);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {reported}");
        assert!(
            reported.contains("selects no infobase"),
            "{arguments:?}: {reported}"
        );
        assert!(
            reported.contains(&format!("`{leaf}`")),
            "{arguments:?}: {reported}"
        );
    }
}

/// Лист, который базу объявляет сам, принимает по ключу строку подключения, а имя из
/// карты баз отвергает: имени там ещё нет, объявляет его как раз этот вызов.
#[test]
fn a_leaf_that_declares_the_base_refuses_a_name_from_the_map() {
    for (arguments, leaf) in DECLARING_THE_BASE {
        let output = v8_runner_command()
            .args(arguments.iter())
            .args(["--infobase", "origin"])
            .output()
            .expect("run v8-runner");
        let reported = reported(&output);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {reported}");
        assert!(
            reported.contains("takes a connection string here, not the name `origin`"),
            "{arguments:?}: {reported}"
        );
        assert!(
            reported.contains(&format!("`{leaf}`")),
            "{arguments:?}: {reported}"
        );
    }
}

/// Половина сверки, которой здесь не хватало: лист мог объявить поведение в
/// `src/cli/global_flags.rs` и не получить строки ни в одной проверке. Обе половины
/// сверяются с одним включаемым файлом.
#[test]
fn every_leaf_the_shared_list_names_is_exercised_here() {
    fn covered(table: &[(&[&str], &'static str)]) -> Vec<&'static str> {
        let mut leaves: Vec<&'static str> = table.iter().map(|(_, leaf)| *leaf).collect();
        leaves.sort_unstable();
        leaves
    }
    fn named(paths: &[&'static str]) -> Vec<&'static str> {
        let mut paths: Vec<&'static str> = paths.to_vec();
        paths.sort_unstable();
        paths
    }

    assert_eq!(covered(WITHOUT_PREVIEW), named(LEAVES_WITHOUT_PREVIEW));
    assert_eq!(covered(IGNORING_THE_BASE), named(LEAVES_IGNORING_THE_BASE));
    assert_eq!(
        covered(DECLARING_THE_BASE),
        named(LEAVES_DECLARING_THE_BASE)
    );
}

/// Ключ без значения — не отсутствие ключа: вызывающий что-то назвал, и раннер отвечает.
#[test]
fn an_empty_base_key_is_refused_rather_than_read_as_an_absent_key() {
    for arguments in [
        vec!["version", "--infobase", ""],
        vec!["init", "--infobase", "   "],
    ] {
        let output = v8_runner_command()
            .args(&arguments)
            .output()
            .expect("run v8-runner");
        let reported = reported(&output);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {reported}");
        assert!(
            reported.contains("names no infobase"),
            "{arguments:?}: {reported}"
        );
    }
}
