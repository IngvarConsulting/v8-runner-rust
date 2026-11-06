//! Команды `dbms` со стороны командной строки. Сценарий с подделкой утилит — в
//! `src/use_cases/managed_stand/tests.rs`; здесь то, что видно снаружи: загрузка конфигурации,
//! замок и конверт.

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::{hold_workspace_lock, temp_workspace, v8_runner_command};

const MANAGED: &str = "infobases:\n  origin:\n    cluster:\n      managed:\n        name: demo\n    dbms:\n      managed: {}\n      name: demo_data\n      locale: ru\n";

fn project(dir: &Path, local: &str) {
    fs::write(
        dir.join("v8project.yaml"),
        "workPath: work\nformat: DESIGNER\nsource-set: []\n",
    )
    .expect("project");
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
    let (code, answer) = run(dir.path(), &["dbms", "status"]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["command"], "dbms status");
    assert_eq!(answer["data"]["state"], "not_running");
}

#[test]
fn stop_waits_for_the_lock_and_says_so() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    hold_workspace_lock(&dir.path().join("work"));
    let (code, answer) = run(dir.path(), &["dbms", "stop"]);
    assert_ne!(code, 0, "{answer}");
    assert_eq!(answer["error"]["code"], "workspace_busy", "{answer}");
}

#[test]
fn status_needs_no_build_and_opens_no_base() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(dir.path(), &["dbms", "status"]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["data"]["provider_dispatched"], false, "{answer}");
    assert!(!dir.path().join("work/infobases").exists());
}

#[test]
fn status_has_no_preview() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let output = v8_runner_command()
        .current_dir(dir.path())
        .env_remove("V8TR_CONFIG")
        .args(["--dry-run", "dbms", "status"])
        .output()
        .expect("run");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("--dry-run"), "{text}");
}

#[test]
fn a_connection_string_is_refused() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(
        dir.path(),
        &["--infobase", "Srvr=srv;Ref=demo", "dbms", "status"],
    );
    assert_ne!(code, 0, "{answer}");
    assert_eq!(answer["error"]["kind"], "validation", "{answer}");
}

#[test]
fn an_infobase_without_a_managed_dbms_is_refused() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:\n  origin:\n    connection: 'Srvr=srv;Ref=demo'\n",
    );
    let (code, answer) = run(dir.path(), &["dbms", "status"]);
    assert_ne!(code, 0, "{answer}");
    let message = answer["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobase.dbms.managed"), "{answer}");
}

#[test]
fn an_access_field_next_to_managed_is_refused_by_its_name() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:\n  origin:\n    cluster:\n      managed:\n        name: demo\n    dbms:\n      managed: {}\n      server: db\n",
    );
    let (code, answer) = run(dir.path(), &["dbms", "status"]);
    assert_ne!(code, 0, "{answer}");
    let message = answer["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobase.dbms.server"), "{answer}");
}

#[test]
fn an_empty_managed_key_means_the_same_as_an_empty_map() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:\n  origin:\n    cluster:\n      managed:\n        name: demo\n    dbms:\n      managed:\n",
    );
    let (code, answer) = run(dir.path(), &["dbms", "status"]);
    assert_eq!(code, 0, "{answer}");
}

#[test]
fn a_working_command_on_a_managed_server_is_refused_and_a_neighbour_keeps_working() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:
  origin:
    connection: 'Srvr=srv;Ref=demo'
  stand:
    cluster:
      managed:
        name: demo
    dbms:
      managed: {}
",
    );
    fs::write(
        dir.path().join("v8project.yaml"),
        "workPath: work
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
",
    )
    .expect("project");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::write(
        dir.path().join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("marker");
    let (code, answer) = run(dir.path(), &["--infobase", "stand", "extensions", "list"]);
    assert_ne!(code, 0, "{answer}");
    let message = answer["error"]["message"].as_str().expect("message");
    assert!(message.contains("cluster.managed"), "{answer}");
    let (_, neighbour) = run(dir.path(), &["--dry-run", "extensions", "list"]);
    let text = neighbour.to_string();
    assert!(!text.contains("cluster.managed"), "{text}");
}

#[cfg(not(windows))]
#[test]
fn other_systems_refuse_start_as_the_environment() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(dir.path(), &["dbms", "start"]);
    assert_ne!(code, 0, "{answer}");
    assert_eq!(
        answer["error"]["code"], "environment_unavailable",
        "{answer}"
    );
}
