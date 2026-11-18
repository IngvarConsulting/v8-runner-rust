//! Команды `debug-server` со стороны командной строки. Сценарий с подделкой системы — в
//! модуле `src/use_cases/managed_stand/debug_server.rs`; здесь то, что видно снаружи: замок,
//! разбор ключей и конверт.

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::{hold_workspace_lock, temp_workspace, v8_runner_command};

const SESSION: &str = "0123456789abcdef0123456789abcdef";

fn project(dir: &Path) {
    fs::write(
        dir.join("v8project.yaml"),
        "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=ib'\n",
    )
    .expect("project");
}

fn run(dir: &Path, arguments: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .current_dir(dir)
        .env_remove("V8TR_CONFIG")
        .arg("--json-message")
        .args(arguments)
        .output()
        .expect("run");
    let code = output.status.code().expect("code");
    let text = String::from_utf8_lossy(&output.stdout);
    (
        code,
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("not an envelope: {text}")),
    )
}

/// Отладчик гасит сервер при отсоединении, даже если в это время идёт `push`: `stop` замка
/// `workPath` не ждёт (`INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK`).
#[test]
fn stop_answers_while_another_command_holds_the_work_path() {
    let dir = temp_workspace();
    project(dir.path());
    fs::create_dir_all(dir.path().join("work")).expect("work");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::write(
        dir.path().join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("set");
    hold_workspace_lock(&dir.path().join("work"));
    let (code, answer) = run(dir.path(), &["debug-server", "stop", SESSION]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["command"], "debug-server stop");
    assert_eq!(answer["data"]["action"], "nothing_to_stop");
    assert_eq!(answer["data"]["known_session"], false);
    assert_eq!(answer["data"]["session"], SESSION);
}

/// Сеанс становится именем каталога: иное, чем 32 шестнадцатеричных знака, отвергает разбор.
#[test]
fn a_session_that_is_not_hex_is_refused_by_the_parser() {
    let dir = temp_workspace();
    project(dir.path());
    let output = v8_runner_command()
        .current_dir(dir.path())
        .env_remove("V8TR_CONFIG")
        .args(["debug-server", "stop", "../../outside"])
        .output()
        .expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value"), "{stderr}");
}
