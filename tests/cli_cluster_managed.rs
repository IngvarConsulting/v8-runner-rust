//! Команды `cluster` со стороны командной строки. Сценарий с подделкой машины —
//! `src/use_cases/managed_stand/cluster.rs`.

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::{hold_workspace_lock, temp_workspace, v8_runner_command};

const MANAGED: &str = "infobases:\n  origin:\n    cluster:\n      managed:\n        name: demo\n    dbms:\n      managed: {}\n      name: demo_data\n";

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
    let (code, answer) = run(dir.path(), &["cluster", "status"]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["command"], "cluster status");
    assert_eq!(answer["data"]["state"], "not_running");
}

#[test]
fn push_without_the_cluster_is_refused_and_names_cluster_start() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(dir.path(), &["push"]);
    assert_ne!(code, 0, "{answer}");
    assert_eq!(answer["error"]["kind"], "validation", "{answer}");
    assert!(answer.to_string().contains("cluster start"), "{answer}");
}

#[test]
fn infobase_create_without_the_cluster_names_cluster_start() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(dir.path(), &["infobase", "create"]);
    assert_ne!(code, 0, "{answer}");
    assert!(answer.to_string().contains("cluster start"), "{answer}");
}

#[test]
fn status_has_no_preview() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let output = v8_runner_command()
        .current_dir(dir.path())
        .env_remove("V8TR_CONFIG")
        .args(["--dry-run", "cluster", "status"])
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
        &["--infobase", "Srvr=srv;Ref=demo", "cluster", "status"],
    );
    assert_ne!(code, 0, "{answer}");
    assert_eq!(answer["error"]["kind"], "validation", "{answer}");
}

#[test]
fn a_base_without_a_managed_cluster_is_refused() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:\n  origin:\n    connection: 'Srvr=srv;Ref=demo'\n",
    );
    let (code, answer) = run(dir.path(), &["cluster", "status"]);
    assert_ne!(code, 0, "{answer}");
    let message = answer["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobase.cluster.managed"), "{answer}");
}

#[test]
fn a_declared_connection_next_to_the_managed_cluster_is_refused() {
    let dir = temp_workspace();
    project(
        dir.path(),
        "infobases:\n  origin:\n    connection: 'Srvr=srv;Ref=demo'\n    cluster:\n      managed:\n        name: demo\n",
    );
    let (code, answer) = run(dir.path(), &["cluster", "status"]);
    assert_ne!(code, 0, "{answer}");
    assert!(
        answer.to_string().contains("infobase.connection"),
        "{answer}"
    );
}

#[test]
fn start_on_other_systems_is_refused_and_on_windows_needs_the_platform() {
    let dir = temp_workspace();
    project(dir.path(), MANAGED);
    let (code, answer) = run(
        dir.path(),
        &["--dry-run", "cluster", "start", "--new-regport"],
    );
    // Превью ничего не поднимает: только план.
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["data"]["action"], "planned", "{answer}");
}
