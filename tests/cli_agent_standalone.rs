//! Автономный сервер как цель: раннер подключается к его SSH-шлюзу, ничего не
//! запуская, и обменивается файлами только объявленным каналом — каталогом
//! пользователя шлюза (`DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE`).
//!
//! Двойник шлюза — тот же `support::fake_agent` с фиксированным каталогом пользователя
//! и именованным логином, как у настоящего `ibsrv` (замер 15.09.2026).
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::fake_agent::{read_or_empty, start_fake_agent, FakeAgent, AGENT_PASSWORD};
use support::{temp_workspace, v8_runner_command};

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    user_dir: PathBuf,
    port: u16,
}

const GATE_USER: &str = "agent";

fn write_config(harness: &Harness, infobase: &str, extra: &str) {
    let root = harness.dir.path();
    // Платформы на машине раннера нет вовсе: пустой каталог в `tools.platform.path`
    // закрывает и системный поиск `1cv8`, чтобы установленная платформа не скрыла
    // лишнюю зависимость.
    let no_platform = root.join("no-platform");
    fs::create_dir_all(&no_platform).expect("empty platform dir");
    let tools = if extra.contains("tools:") {
        extra.replacen(
            "tools:\n",
            &format!(
                "tools:\n  platform:\n    path: {}\n    strict: true\n",
                no_platform.display()
            ),
            1,
        )
    } else {
        format!(
            "{extra}tools:\n  platform:\n    path: {}\n    strict: true\n",
            no_platform.display()
        )
    };
    fs::write(
        &harness.config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\ninfobase:\n{infobase}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\n  - name: Зонд\n    type: EXTENSION\n    path: project/ext\n{tools}",
            work = root.join("work").display(),
        ),
    )
    .expect("write config");
}

fn standalone_infobase(harness: &Harness) -> String {
    format!(
        "  user: {GATE_USER}\n  password: '{password}'\n  standalone:\n    gate: 127.0.0.1:{port}\n    exchange:\n      dir: {dir}\n",
        password = AGENT_PASSWORD,
        port = harness.port,
        dir = harness.user_dir.display(),
    )
}

fn harness() -> Harness {
    let dir = temp_workspace();
    let root = dir.path().to_path_buf();
    let project = root.join("project");
    fs::create_dir_all(project.join("configuration").join("Catalogs")).expect("configuration");
    fs::write(
        project.join("configuration").join("Configuration.xml"),
        "<Configuration/>",
    )
    .expect("root");
    fs::write(
        project
            .join("configuration")
            .join("Catalogs")
            .join("Items.xml"),
        "<Catalog/>",
    )
    .expect("catalog");
    fs::create_dir_all(project.join("ext")).expect("ext");
    fs::write(
        project.join("ext").join("Configuration.xml"),
        "<Configuration/>",
    )
    .expect("ext root");
    fs::create_dir_all(root.join("work")).expect("work dir");
    // Каталог пользователя шлюза — «сторона цели»: лежит отдельно от workPath.
    let user_dir = root.join("server").join("users-data").join(GATE_USER);
    fs::create_dir_all(&user_dir).expect("gate user dir");
    let commands_log = root.join("gate-commands.log");
    let port = start_fake_agent(FakeAgent::gate(
        commands_log.clone(),
        GATE_USER,
        user_dir.clone(),
    ));
    let harness = Harness {
        config_path: root.join("v8project.yaml"),
        commands_log,
        user_dir,
        port,
        dir,
    };
    let infobase = standalone_infobase(&harness);
    write_config(&harness, &infobase, "");
    harness
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

fn error_message(payload: &Value) -> String {
    payload["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Пути в командах шлюза относительны его каталога пользователя, а не `workPath`;
/// результат читается из объявленного канала. Платформы на машине раннера нет вовсе,
/// и никто её не ищет: шлюз держит сам сервер.
#[test]
fn gate_commands_carry_target_side_relative_paths() {
    let harness = harness();
    let target = harness.dir.path().join("project").join("configuration");

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    assert_eq!(
        fs::read_to_string(target.join("Configuration.xml")).expect("published dump"),
        "<Configuration/>\n",
        "dump published from the declared dir"
    );
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
    let dump = lines
        .iter()
        .find(|line| line.starts_with("config dump-config-to-files"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    let dir = dump
        .split_whitespace()
        .find_map(|word| word.strip_prefix("--dir="))
        .expect("--dir");
    assert!(
        !Path::new(dir).is_absolute() && !dir.contains("work"),
        "target-side path must be relative to the gate user dir: {dump}"
    );
    let work_path = harness.dir.path().join("work").display().to_string();
    assert!(
        lines.iter().all(|line| !line.contains(&work_path)),
        "no command may carry the runner's workPath: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line == "common shutdown"),
        "a server the runner did not start is never shut down: {lines:?}"
    );
}

/// Сборка через шлюз: исходники выставляются в каталог пользователя шлюза, загрузка и
/// обновление базы — одна сессия.
#[test]
fn build_through_the_gate_loads_from_the_declared_dir() {
    let harness = harness();

    let (code, payload) = run(&harness, &["build"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["steps"][0]["mode"], "full", "{payload}");
    let lines = commands(&harness);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("config load-config-from-files --dir=build/")),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"config update-db-cfg".to_owned()),
        "{lines:?}"
    );
}

/// `make` и состав расширений идут той же сессией шлюза.
#[test]
fn make_and_extensions_go_through_the_gate() {
    let harness = harness();
    let output = harness.dir.path().join("dist").join("release.cf");

    let (code, payload) = run(
        &harness,
        &["artifacts", "--output", &output.display().to_string()],
    );
    assert_eq!(code, 0, "{payload}");
    assert_eq!(fs::read_to_string(&output).expect("package"), "CF:main");

    let (code, payload) = run(&harness, &["extensions", "list"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["extensions"][0]["name"], "Зонд",
        "{payload}"
    );
}

/// Без объявленного канала обмена шлюз не вызывается: отказ до сессии называет ключ.
#[test]
fn a_standalone_server_without_a_declared_channel_is_refused_before_any_session() {
    let harness = harness();
    write_config(
        &harness,
        &format!(
            "  user: {GATE_USER}\n  password: '{password}'\n  standalone:\n    gate: 127.0.0.1:{port}\n",
            password = AGENT_PASSWORD,
            port = harness.port
        ),
        "",
    );

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        error_message(&payload).contains("infobase.standalone.exchange.dir"),
        "{payload}"
    );
    assert!(commands(&harness).is_empty(), "{:?}", commands(&harness));
}

/// Рабочий каталог раннера не назначается на сторону цели: `workPath` внутри
/// каталога обмена — отказ валидации.
#[test]
fn a_work_path_on_the_target_side_is_refused() {
    let harness = harness();
    let infobase = standalone_infobase(&harness);
    let yaml = fs::read_to_string(&harness.config_path).expect("config");
    let inside_target = harness.user_dir.join("work");
    fs::write(
        &harness.config_path,
        yaml.replace(
            &format!("workPath: {}", harness.dir.path().join("work").display()),
            &format!("workPath: {}", inside_target.display()),
        ),
    )
    .expect("config");
    let _ = infobase;

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        error_message(&payload).contains("workPath stays on the runner's side"),
        "{payload}"
    );
}

/// Ключи запуска агента к автономному серверу не относятся: его никто не поднимает;
/// отказ называет ключи и причину.
#[test]
fn launch_keys_do_not_apply_to_a_standalone_server() {
    let harness = harness();
    let infobase = standalone_infobase(&harness);
    write_config(
        &harness,
        &infobase,
        "tools:\n  designer_agent:\n    port: 1543\n",
    );

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    let message = error_message(&payload);
    assert!(
        message.contains("tools.designer_agent.port")
            && message.contains("never started by the runner"),
        "{payload}"
    );
    assert!(commands(&harness).is_empty());
}

/// Цель объявляется один раз: строка подключения рядом с `standalone` — отказ.
#[test]
fn a_connection_string_next_to_standalone_is_refused() {
    let harness = harness();
    let infobase = format!(
        "  connection: 'File={}'\n{}",
        harness.dir.path().join("ib").display(),
        standalone_infobase(&harness)
    );
    write_config(&harness, &infobase, "");

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert!(
        error_message(&payload).contains("declared once"),
        "{payload}"
    );
}

/// У автономного сервера один исполнитель: ключ `providers.*` — ошибка, а операции без
/// строки в матрице (`load`, `init`) отказывают типизированно и сессии не открывают.
#[test]
fn a_standalone_server_has_one_executor_and_no_load() {
    let harness = harness();
    let infobase = standalone_infobase(&harness);
    write_config(&harness, &infobase, "providers:\n  dump: agent\n");
    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);
    assert_ne!(code, 0, "{payload}");
    assert!(
        error_message(&payload).contains("providers.dump is not allowed"),
        "{payload}"
    );

    write_config(&harness, &infobase, "");
    let artifact = harness.dir.path().join("in.cf");
    fs::write(&artifact, "cf").expect("cf");
    let (code, payload) = run(
        &harness,
        &["load", "--path", &artifact.display().to_string()],
    );
    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        error_message(&payload).contains("Designer provider"),
        "{payload}"
    );
    assert!(commands(&harness).is_empty(), "{:?}", commands(&harness));

    let (code, payload) = run(&harness, &["init"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["steps"][0]["status"], "skipped",
        "{payload}"
    );
    assert!(
        payload["data"]["steps"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("never created by the runner")),
        "{payload}"
    );
    assert!(commands(&harness).is_empty(), "{:?}", commands(&harness));
}

/// Снимок автономного сервера через шлюз не снимается: `dump-ib` роняет `ibsrv` 8.3.27
/// (живой прогон 15.09.2026), поэтому строки нет и отказ приходит до сессии.
#[test]
fn a_standalone_snapshot_is_refused_before_any_session() {
    let harness = harness();
    let dt = harness.dir.path().join("out").join("base.dt");

    let (code, payload) = run(
        &harness,
        &["infobase", "dump", "--output", &dt.display().to_string()],
    );

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "capability", "{payload}");
    assert!(error_message(&payload).contains("standalone"), "{payload}");
    assert!(commands(&harness).is_empty(), "{:?}", commands(&harness));
}

/// Клиент к шлюзу не запускается: у автономного сервера нет строки подключения, его
/// адрес — `infobase.web.url`.
#[test]
fn a_client_launch_against_a_standalone_server_points_at_launch_web() {
    let harness = harness();

    let (code, payload) = run(&harness, &["launch", "thin", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "capability", "{payload}");
    assert!(error_message(&payload).contains("launch web"), "{payload}");
}
