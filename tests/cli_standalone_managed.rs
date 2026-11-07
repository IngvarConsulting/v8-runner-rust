//! Команды `standalone` со стороны командной строки. Сценарий с подделкой машины — в
//! `src/use_cases/managed_stand/standalone.rs`.

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::{hold_workspace_lock, temp_workspace, v8_runner_command};

const MANAGED: &str = "infobases:\n  origin:\n    standalone:\n      managed:\n        name: demo\n        db-path: build/ib\n";

fn project(dir: &Path, local: &str) {
    fs::write(
        dir.join("v8project.yaml"),
        "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .expect("project");
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::write(dir.join("src/Configuration.xml"), "<MetaDataObject/>").expect("marker");
    fs::write(dir.join("v8project.local.yaml"), local).expect("local");
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

#[test]
fn status_answers_while_another_command_holds_the_work_path() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    hold_workspace_lock(&dir.path().join("work"));
    let (code, answer) = run(dir.path(), &["standalone", "status"]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["command"], "standalone status");
    assert_eq!(answer["data"]["state"], "not_running");
}

#[test]
fn a_working_command_without_the_server_is_refused_and_names_the_next_command() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(dir.path(), &["extensions", "list"]);
    assert_ne!(code, 0, "{answer}");
    assert_eq!(answer["error"]["kind"], "validation", "{answer}");
    assert!(answer.to_string().contains("standalone start"), "{answer}");
}

#[test]
fn start_without_an_infobase_names_infobase_create() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(dir.path(), &["standalone", "start"]);
    assert_ne!(code, 0, "{answer}");
    let text = answer.to_string();
    assert!(
        text.contains("infobase create") || text.contains("Windows"),
        "{text}"
    );
}

#[test]
fn a_base_without_a_managed_server_is_refused() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:\n  origin:\n    connection: 'Srvr=srv;Ref=demo'\n",
    );
    let (code, answer) = run(dir.path(), &["standalone", "status"]);
    assert_ne!(code, 0, "{answer}");
    assert!(
        answer.to_string().contains("standalone.managed"),
        "{answer}"
    );
}

#[test]
fn a_preview_of_infobase_create_goes_for_the_file_base_of_the_server() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (_, answer) = run(dir.path(), &["infobase", "create", "--dry-run"]);
    assert!(
        !answer.to_string().contains("not running"),
        "the server is not needed to create its base: {answer}"
    );
}
