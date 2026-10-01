//! Секция `cluster` в секции базы: адрес сервера администрирования и два уровня
//! администраторов над пользователем базы
//! (`DEC.2026-09-21.THE-CLUSTER-SECTION-HOLDS-RAS-AND-TWO-ADMIN-LEVELS`).
//!
//! Щуп — `launch thin --dry-run`: он не запускает клиент, а план называет, что клиенту
//! передано; учётные данные кластера и агента в него попасть не должны. Что отказ
//! валидации не доходит до платформы, доказывает `contract_config_boundary`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};
use tempfile::TempDir;

struct Project {
    dir: TempDir,
    config_path: PathBuf,
}

impl Project {
    fn write_local(&self, body: &str) {
        fs::write(self.dir.path().join("v8project.local.yaml"), body).expect("local overlay");
    }

    fn run_json(&self, command: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config_path)
            .arg("--json-message")
            .args(command)
            .output()
            .expect("run v8-runner")
    }

    fn run_text(&self, command: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config_path)
            .args(command)
            .output()
            .expect("run v8-runner")
    }
}

/// Проектный файл без секции базы: базы объявляет местный слой.
fn project() -> Project {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let platform = dir.path().join("platform");
    fs::create_dir_all(dir.path().join("project")).expect("sources");
    fs::create_dir_all(&work_path).expect("work");
    write_shell_script(&platform.join("bin").join("1cv8"), "exit 0");
    write_shell_script(&platform.join("bin").join("1cv8c"), "exit 0");
    let config_path = dir.path().join("v8project.yaml");
    fs::write(&config_path, project_file(&work_path, &platform)).expect("config");
    Project { dir, config_path }
}

fn project_file(work_path: &Path, platform: &Path) -> String {
    format!(
        "workPath: '{}'\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        platform.display()
    )
}

const LAUNCH_PREVIEW: &[&str] = &["launch", "thin", "--dry-run"];

/// Секция `origin` со всеми тремя уровнями: пользователь базы, администратор кластера,
/// агент центрального сервера и его администратор.
const THREE_LEVELS: &str = "infobases:\n  origin:\n    connection: 'Srvr=srv:1541;Ref=demo'\n    user: ib-admin\n    password: ib-secret\n    cluster:\n      ras: srv:1545\n      user: cluster-admin\n      password: cluster-secret\n      agent:\n        address: srv:1540\n        user: agent-admin\n        password: agent-secret\n";

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn planned_args(payload: &Value) -> Vec<String> {
    payload["data"]["plan"]["args"]
        .as_array()
        .unwrap_or_else(|| panic!("planned args in {payload}"))
        .iter()
        .map(|arg| arg.as_str().expect("arg").to_owned())
        .collect()
}

fn refusal_message(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(2),
        "a config refusal exits with 2: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(output);
    assert_eq!(payload["ok"], false);
    payload["data"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("refusal message in {payload}"))
        .to_owned()
}

/// Три уровня лежат в местном слое порознь, и клиенту уходит только уровень базы:
/// план тонкого клиента несёт `/N` пользователя базы и ничего из секции `cluster`.
#[test]
fn the_three_credential_levels_lie_in_the_local_layer_side_by_side() {
    let project = project();
    project.write_local(THREE_LEVELS);

    let output = project.run_json(LAUNCH_PREVIEW);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["warnings"], Value::Array(Vec::new()), "{payload}");
    let args = planned_args(&payload);
    let user = args
        .iter()
        .position(|arg| arg == "/N")
        .unwrap_or_else(|| panic!("/N in {args:?}"));
    assert_eq!(args[user + 1], "ib-admin", "{args:?}");
    let plan = payload.to_string();
    for stranger in [
        "cluster-admin",
        "cluster-secret",
        "agent-admin",
        "agent-secret",
        "srv:1545",
        "srv:1540",
    ] {
        assert!(
            !plan.contains(stranger),
            "{stranger} leaked into the thin client plan: {plan}"
        );
    }
}

/// У файловой базы кластера нет: секция рядом с `File=` — отказ валидации в обоих
/// режимах вывода (`INV.CONFIG.A-CLUSTER-SECTION-IS-REJECTED-OUTSIDE-A-CLUSTER-BASE`).
#[test]
fn a_cluster_section_next_to_a_file_base_is_refused() {
    let project = project();
    project.write_local(
        "infobases:\n  origin:\n    connection: 'File=/tmp/origin-ib'\n    cluster:\n      ras: srv:1545\n",
    );

    let message = refusal_message(&project.run_json(LAUNCH_PREVIEW));
    assert!(
        message.contains("infobase.cluster is not allowed for a file infobase"),
        "{message}"
    );

    let output = project.run_text(LAUNCH_PREVIEW);
    assert_eq!(output.status.code(), Some(2));
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        printed.contains("infobase.cluster is not allowed for a file infobase"),
        "{printed}"
    );
}

/// Адрес сервера администрирования — `host[:port]`; отказ называет ключ и формы.
#[test]
fn a_malformed_ras_address_is_refused_naming_the_key() {
    let project = project();
    project.write_local(
        "infobases:\n  origin:\n    connection: 'Srvr=srv:1541;Ref=demo'\n    cluster:\n      ras: ':1545'\n",
    );

    let message = refusal_message(&project.run_json(LAUNCH_PREVIEW));

    assert!(message.contains("infobase.cluster.ras"), "{message}");
    assert!(message.contains("`host:port`"), "{message}");
    assert!(message.contains("':1545'"), "{message}");
}
