//! Занятый `workPath` отвечает одинаково у всякой команды командной строки, которая берёт
//! его замок: код `workspace_busy`, род `workspace`, шаг `workspace lock`, выход 3.
//!
//! Перечень команд выводится из таблицы листьев (`src/cli/global_flags_expected.in`):
//! все листья, кроме тех, что замка не берут, — они названы ниже поимённо, с причиной.
//! Вызов листа с превью берётся из `tests/support/previews.rs`, остальных — отсюда;
//! лист без вызова роняет сверку состава.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::previews::{with_preview, write_project};
use support::{hold_workspace_lock, temp_workspace, v8_runner_command};

// Состав таблицы листьев, общий с `src/cli/global_flags.rs`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/cli/global_flags_expected.in"
));

/// Листья, которые замка `workPath` не берут, и почему.
const LEAVES_WITHOUT_THE_LOCK: &[(&str, &str)] = &[
    ("version", "печатает версию и ничего не запускает"),
    ("init", "пишет файл проекта, рабочего каталога не трогает"),
    ("config init", "синоним `init`"),
    (
        "mcp serve stdio",
        "сервер; замок берёт каждый вызов инструмента, а не процесс",
    ),
    (
        "mcp serve http",
        "сервер; замок берёт каждый вызов инструмента, а не процесс",
    ),
];

/// Вызов листа, у которого превью нет: путь листа, аргументы и имя команды в ответе.
fn without_preview() -> Vec<(&'static str, Vec<&'static str>, &'static str)> {
    vec![
        (
            "tools download yaxunit",
            vec!["tools", "download", "yaxunit"],
            "tools download",
        ),
        (
            "tools download vanessa",
            vec!["tools", "download", "vanessa"],
            "tools download",
        ),
        (
            "tools download client-mcp",
            vec!["tools", "download", "client-mcp"],
            "tools download",
        ),
        ("test yaxunit all", vec!["test", "yaxunit", "all"], "test"),
        (
            "test yaxunit module",
            vec!["test", "yaxunit", "module", "Demo"],
            "test",
        ),
        ("test va", vec!["test", "va"], "test"),
    ]
}

/// Каждый лист, берущий замок: путь, вызов, имя команды в ответе и его `workPath`.
fn locking_leaves(
    dir: &Path,
) -> Vec<(&'static str, Vec<String>, &'static str, std::path::PathBuf)> {
    let work = dir.join("work");
    let mut rows: Vec<_> = with_preview(dir)
        .into_iter()
        .map(|row| {
            // У `clone` рабочий каталог — `build` проекта, которого ещё нет.
            let work_path = if row.leaf == "clone" {
                row.trace.join("build")
            } else {
                work.clone()
            };
            (row.leaf, row.arguments, row.command, work_path)
        })
        .collect();
    rows.extend(
        without_preview()
            .into_iter()
            .map(|(leaf, arguments, command)| {
                (
                    leaf,
                    arguments.into_iter().map(str::to_owned).collect(),
                    command,
                    work.clone(),
                )
            }),
    );
    rows
}

#[test]
fn every_leaf_taking_the_lock_is_exercised_here() {
    let mut covered: Vec<&str> = locking_leaves(Path::new("."))
        .into_iter()
        .map(|(leaf, ..)| leaf)
        .collect();
    covered.sort_unstable();
    let mut expected: Vec<&str> = LEAVES_WITH_PREVIEW
        .iter()
        .chain(LEAVES_WITHOUT_PREVIEW)
        .copied()
        .filter(|leaf| {
            !LEAVES_WITHOUT_THE_LOCK
                .iter()
                .any(|(unlocked, _)| unlocked == leaf)
        })
        .collect();
    expected.sort_unstable();
    assert_eq!(covered, expected);

    for (unlocked, _) in LEAVES_WITHOUT_THE_LOCK {
        assert!(
            LEAVES_WITH_PREVIEW.contains(unlocked) || LEAVES_WITHOUT_PREVIEW.contains(unlocked),
            "исключение называет лист, которого нет в дереве: {unlocked}"
        );
    }
}

/// Профиль Vanessa и названные им файлы: без них `test va` отказывает проверкой настроек
/// раньше замка. Файл образца кончается разделом `tools`, поэтому строка обработки
/// дописывается в него.
fn declare_vanessa(config_path: &Path) {
    let mut config = fs::read_to_string(config_path).expect("read config");
    config.push_str(
        "  va:\n    epf_path: ./vanessa.epf\ntests:\n  va:\n    params_path: ./va.json\n    profile: smoke\n    profiles:\n      smoke:\n        feature_path: ./features\n",
    );
    fs::write(config_path, config).expect("write config");
    let root = config_path.parent().expect("project root");
    fs::write(root.join("vanessa.epf"), "epf").expect("vanessa processor");
    fs::write(root.join("va.json"), "{}").expect("vanessa params");
    fs::create_dir_all(root.join("features")).expect("features");
}

/// Код выхода и все конверты, напечатанные в stdout, по порядку. Настройки берутся из
/// текущего каталога, как у таблицы превью: `clone` глобальный ключ отвергает.
fn run(dir: &Path, arguments: &[String]) -> (i32, Vec<Value>) {
    let output = v8_runner_command()
        .current_dir(dir)
        .env_remove("V8TR_CONFIG")
        .arg("--json-message")
        .args(arguments)
        .output()
        .expect("run command");
    let envelopes = serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<Value>()
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| {
            panic!(
                "`{}` printed no json: {error}\nstdout: {}\nstderr: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
    (output.status.code().unwrap_or(-1), envelopes)
}

/// Отказ занятого каталога одинаков у всякой команды и печатается одним конвертом.
#[test]
fn every_leaf_taking_the_lock_answers_workspace_busy_on_a_busy_work_path() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), true);
    declare_vanessa(&config_path);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    fs::write(dir.path().join("main.dt"), "dt").expect("snapshot");
    // Файловая база существует: выбор исполнителя у переноса проверяет её до замка.
    fs::create_dir_all(dir.path().join("ib")).expect("infobase dir");
    fs::write(dir.path().join("ib").join("1Cv8.1CD"), "fixture").expect("infobase file");

    let mut wrong = Vec::new();
    for (leaf, arguments, command, work_path) in locking_leaves(dir.path()) {
        hold_workspace_lock(&work_path);
        let (code, envelopes) = run(dir.path(), &arguments);
        let payload = envelopes.first().cloned().unwrap_or(Value::Null);
        let answer = (
            code,
            envelopes.len(),
            payload["command"].as_str(),
            payload["error"]["code"].as_str(),
            payload["error"]["kind"].as_str(),
            payload["steps"].as_array().map(Vec::len),
            payload["steps"][0]["name"].as_str(),
            payload["steps"][0]["status"].as_str(),
            payload["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("already")),
        );
        let expected = (
            3,
            1,
            Some(command),
            Some("workspace_busy"),
            Some("workspace"),
            Some(1),
            Some("workspace lock"),
            Some("failed"),
            true,
        );
        if answer != expected {
            wrong.push(format!("`{leaf}`: {answer:?}\n{payload}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}
