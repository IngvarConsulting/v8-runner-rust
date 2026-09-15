//! Сборка через агентский shell Конфигуратора: одна сессия на команду.
//!
//! Двойник агента (`support::fake_agent`) принимает загрузку только из каталога, в
//! котором видит исходники, — так проверяется, что раннер выставил их агенту ссылкой.
//! Загрузка и обновление базы идут в одной сессии; сборка без изменений сессию не
//! открывает; изменённый файл грузится частично со списком; после удачной загрузки
//! поколение записано, и выгрузка после сборки ничего не выгружает.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use support::fake_agent::{
    read_or_empty, start_fake_agent, write_fake_designer, FakeAgent, AGENT_PASSWORD,
};
use support::{temp_workspace, v8_runner_command};

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    designer_args_log: PathBuf,
    sources: PathBuf,
}

fn harness() -> Harness {
    let dir = temp_workspace();
    let root = dir.path().to_path_buf();
    let sources = root.join("project").join("configuration");
    let work_path = root.join("work");
    fs::create_dir_all(sources.join("Catalogs")).expect("sources");
    fs::write(sources.join("Configuration.xml"), "<Configuration/>").expect("root");
    fs::write(sources.join("Catalogs").join("Items.xml"), "<Catalog/>").expect("catalog");
    fs::create_dir_all(&work_path).expect("work dir");
    let bin = root.join("platform").join("8.3.27.2074").join("bin");
    fs::create_dir_all(&bin).expect("platform dir");

    let commands_log = root.join("agent-commands.log");
    let base_dir_file = root.join("base-dir.txt");
    let designer_pid_file = root.join("designer.pid");
    let designer_args_log = root.join("designer-args.log");
    let port = start_fake_agent(FakeAgent::new(
        true,
        commands_log.clone(),
        None,
        base_dir_file.clone(),
        designer_pid_file.clone(),
    ));
    write_fake_designer(
        &bin.join("1cv8"),
        &designer_args_log,
        &designer_pid_file,
        &base_dir_file,
    );
    let config_path = root.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\nproviders:\n  build: agent\n  dump: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27'\n  designer_agent:\n    port: {port}\n",
            work = work_path.display(),
            ib = root.join("ib").display(),
            password = AGENT_PASSWORD,
            platform = root.join("platform").display(),
        ),
    )
    .expect("write config");
    Harness {
        config_path,
        commands_log,
        designer_args_log,
        sources,
        dir,
    }
}

fn run(harness: &Harness, arguments: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
        ])
        .args(arguments)
        .output()
        .expect("run command");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`{}` printed no json envelope: {error}\nstdout: {}\nstderr: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().unwrap_or(-1), payload)
}

fn commands(harness: &Harness) -> Vec<String> {
    read_or_empty(&harness.commands_log)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Загрузка исходников и обновление базы — два шага одного разговора; после удачной
/// загрузки записано поколение.
#[test]
fn a_managed_build_loads_and_updates_in_one_session_and_records_the_generation() {
    let harness = harness();

    let (code, payload) = run(&harness, &["build"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    assert_eq!(payload["data"]["steps"][0]["mode"], "full", "{payload}");
    let lines = commands(&harness);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("options set --show-prompt=no --output-format=json"),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(1).map(String::as_str),
        Some("common connect-ib"),
        "{lines:?}"
    );
    assert!(
        lines.get(2).is_some_and(|line| line
            .starts_with("config load-config-from-files --dir=build/")
            && line.ends_with("--update-config-dump-info")),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(3).map(String::as_str),
        Some("config update-db-cfg"),
        "{lines:?}"
    );
    assert_eq!(
        lines.get(4).map(String::as_str),
        Some("config generation-id"),
        "{lines:?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("common shutdown"),
        "{lines:?}"
    );
    assert_eq!(
        read_or_empty(&harness.designer_args_log).lines().count(),
        1,
        "one agent process per command"
    );
    let ledger = harness.dir.path().join("work/agent/generation/main.json");
    let record: Value =
        serde_json::from_str(&read_or_empty(&ledger)).expect("generation ledger record");
    assert_eq!(record["after"], "build");
    assert!(record["token"]
        .as_str()
        .is_some_and(|token| token.len() == 40));
}

/// Сборка без изменений не поднимает агента и не открывает сессию.
#[test]
fn a_build_without_changes_opens_no_session() {
    let harness = harness();
    let (first, payload) = run(&harness, &["build"]);
    assert_eq!(first, 0, "{payload}");
    let commands_after_first = commands(&harness).len();

    let (second, payload) = run(&harness, &["build"]);

    assert_eq!(second, 0, "{payload}");
    assert_eq!(payload["data"]["steps"][0]["mode"], "skipped", "{payload}");
    assert_eq!(commands(&harness).len(), commands_after_first);
    assert_eq!(read_or_empty(&harness.designer_args_log).lines().count(), 1);
}

/// Изменённый файл грузится частично, со списком путей относительно каталога загрузки.
#[test]
fn a_changed_file_loads_partially_with_a_list_file() {
    let harness = harness();
    let (first, payload) = run(&harness, &["build"]);
    assert_eq!(first, 0, "{payload}");
    fs::write(
        harness.sources.join("Catalogs").join("Items.xml"),
        "<Catalog changed='1'/>",
    )
    .expect("change");

    let (second, payload) = run(&harness, &["build"]);

    assert_eq!(second, 0, "{payload}");
    assert_eq!(
        payload["data"]["steps"][0]["mode"]["partial"]["file_count"], 1,
        "{payload}"
    );
    let lines = commands(&harness);
    assert!(
        lines.iter().any(
            |line| line.starts_with("config load-config-from-files --dir=build/")
                && line.contains(" --partial --list-file=build/")
        ),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line == "list: Catalogs/Items.xml"),
        "{lines:?}"
    );
}

/// Выгрузка после сборки видит то же поколение и ничего не выгружает.
#[test]
fn a_dump_after_a_build_with_an_unchanged_generation_dumps_nothing() {
    let harness = harness();
    let (build, payload) = run(&harness, &["build"]);
    assert_eq!(build, 0, "{payload}");

    let (dump, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_eq!(dump, 0, "{payload}");
    assert_eq!(payload["data"]["up_to_date"], true, "{payload}");
    assert!(
        payload["data"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("since the last build")),
        "{payload}"
    );
    assert!(
        !commands(&harness)
            .iter()
            .any(|line| line.starts_with("config dump-config-to-files")),
        "nothing may be dumped for an unchanged generation"
    );
    assert!(
        harness.sources.join("Catalogs").join("Items.xml").is_file(),
        "a skipped dump leaves the sources alone"
    );
}
