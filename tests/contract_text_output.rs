//! Человеческая поверхность закреплена `docs/schemas/text-output.json`.
//!
//! Текст читает человек, но предсказуемым он должен быть по той же причине, что и JSON:
//! его кладут в журнал сборки, пересылают в задаче и ищут в нём глазами. Поэтому у него
//! закрытый состав видов строк, знак статуса, который виден без цвета, и один порядок
//! подробностей у всех команд.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn grammar() -> Value {
    let path = repo_root().join("docs/schemas/text-output.json");
    let text = fs::read_to_string(&path).expect("grammar artefact is present");
    serde_json::from_str(&text).expect("grammar is valid json")
}

fn kinds_for(grammar: &Value, stream: &str) -> Vec<(String, Regex)> {
    grammar["streams"][stream]
        .as_array()
        .expect("stream kinds")
        .iter()
        .map(|kind| {
            let kind = kind.as_str().expect("kind name");
            let pattern = grammar["line_kinds"][kind]["pattern"]
                .as_str()
                .expect("kind pattern");
            (
                kind.to_owned(),
                Regex::new(pattern).expect("kind pattern compiles"),
            )
        })
        .collect()
}

fn detail_kind_order(grammar: &Value, detail: &str) -> usize {
    let kinds = grammar["detail_kinds"].as_array().expect("detail kinds");
    // Виды перечислены в порядке печати, и первый подошедший — он и есть: `free`
    // подходит ко всему, поэтому стоит сразу после `fact` и ловит остаток.
    for (index, kind) in kinds.iter().enumerate() {
        let pattern = Regex::new(kind["pattern"].as_str().expect("pattern")).expect("compiles");
        if pattern.is_match(detail) {
            return index;
        }
    }
    unreachable!("`free` matches every non-empty line: {detail}")
}

struct Run {
    arguments: String,
    code: i32,
    stdout: String,
    stderr: String,
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

fn run(config_path: &Path, arguments: &[&str]) -> Run {
    // Окружение оболочки сюда не проходит: заданная у разработчика `FORCE_COLOR` уронила бы
    // проверку перенаправленного вывода, а `NO_COLOR` сделала бы её пустой.
    let output = v8_runner_command()
        .env_remove("FORCE_COLOR")
        .env_remove("NO_COLOR")
        .args(["--config", &config_path.display().to_string()])
        .args(arguments)
        .output()
        .expect("run command");
    Run {
        arguments: arguments.join(" "),
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn previews(artifact: &str, snapshot: &str) -> Vec<Vec<String>> {
    let owned = |parts: &[&str]| parts.iter().map(|part| (*part).to_owned()).collect();
    vec![
        owned(&["version"]),
        owned(&["build", "--dry-run"]),
        owned(&["dump", "--mode", "full", "--dry-run"]),
        owned(&["convert", "--dry-run"]),
        owned(&["make", "--output", artifact, "--dry-run"]),
        owned(&["load", "--path", artifact, "--dry-run"]),
        owned(&["infobase", "create", "--dry-run"]),
        owned(&["launch", "designer", "--dry-run"]),
        owned(&["extensions", "list", "--dry-run"]),
        owned(&[
            "extensions",
            "create",
            "--name",
            "Demo",
            "--name-prefix",
            "Demo",
            "--dry-run",
        ]),
        owned(&["syntax", "designer-config", "--thin-client"]),
        owned(&[
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            artifact,
            "--dry-run",
        ]),
        owned(&["infobase", "dump", "--output", snapshot, "--dry-run"]),
        owned(&["infobase", "restore", "--input", snapshot, "--dry-run"]),
        owned(&["extensions", "info", "--name", ""]),
    ]
}

fn all_runs() -> (tempfile::TempDir, Vec<Run>) {
    let dir = temp_workspace();
    let config_path = write_project(dir.path());
    let artifact = dir.path().join("main.cf").display().to_string();
    let snapshot = dir.path().join("main.dt").display().to_string();

    let runs = previews(&artifact, &snapshot)
        .into_iter()
        .map(|preview| {
            let borrowed: Vec<&str> = preview.iter().map(String::as_str).collect();
            run(&config_path, &borrowed)
        })
        .collect();
    (dir, runs)
}

/// Каждая напечатанная строка — один из объявленных видов. Состав закрыт, поэтому
/// новая форма строки обязана пройти через форму, а не появиться молча.
#[test]
fn every_printed_line_is_a_declared_kind() {
    let grammar = grammar();
    let stdout_kinds = kinds_for(&grammar, "stdout");
    let stderr_kinds = kinds_for(&grammar, "stderr");
    let (_dir, runs) = all_runs();

    for run in &runs {
        for (stream, text, kinds) in [
            ("stdout", &run.stdout, &stdout_kinds),
            ("stderr", &run.stderr, &stderr_kinds),
        ] {
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                assert!(
                    kinds.iter().any(|(_, pattern)| pattern.is_match(line)),
                    "`{}` printed a line to {stream} that no declared kind matches: {line:?}",
                    run.arguments
                );
            }
        }
    }
}

/// Цвет — оформление. Перенаправленный вывод его не несёт, иначе журнал сборки
/// наполняется escape-последовательностями, а поиск по нему перестаёт работать.
#[test]
fn redirected_output_carries_no_escape_sequences() {
    let (_dir, runs) = all_runs();
    for run in &runs {
        for (stream, text) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
            assert!(
                !text.contains('\u{1b}'),
                "`{}` wrote an escape sequence to {stream}: {text:?}",
                run.arguments
            );
        }
    }
}

/// Статус читается знаком. Успешный прогон не печатает знака отказа, и наоборот:
/// иначе подпись узла и код выхода расходятся, а верят обычно подписи.
#[test]
fn the_node_mark_agrees_with_the_exit_code() {
    let grammar = grammar();
    let failed = grammar["node_marks"]["failed"]
        .as_str()
        .expect("failed mark")
        .to_owned();
    let (_dir, runs) = all_runs();

    for run in &runs {
        let marked_failed = run
            .stdout
            .lines()
            .any(|line| line.starts_with(&format!("{failed} ")));
        if run.code == 0 {
            assert!(
                !marked_failed,
                "`{}` exited 0 and still marked a node failed:\n{}",
                run.arguments, run.stdout
            );
        } else {
            assert!(
                marked_failed || !run.stderr.trim().is_empty(),
                "`{}` exited {} and said nothing about it:\n{}",
                run.arguments,
                run.code,
                run.stdout
            );
        }
    }
}

/// Подробности идут одним порядком у всех команд: предмет, ход дела, артефакт,
/// улика, предупреждение, отказ. Читатель ищет проблему всегда в одном месте.
#[test]
fn details_of_a_node_are_printed_in_the_declared_order() {
    let grammar = grammar();
    let detail = Regex::new(
        grammar["line_kinds"]["detail"]["pattern"]
            .as_str()
            .expect("detail pattern"),
    )
    .expect("detail pattern compiles");
    let (_dir, runs) = all_runs();

    for run in &runs {
        let mut previous = 0usize;
        for line in run.stdout.lines() {
            if !detail.is_match(line) {
                previous = 0;
                continue;
            }
            let body = line.trim_start_matches('│').trim_start();
            let order = detail_kind_order(&grammar, body);
            assert!(
                order >= previous,
                "`{}` printed details out of the declared order at {line:?}:\n{}",
                run.arguments,
                run.stdout
            );
            previous = order;
        }
    }
}
