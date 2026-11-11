//! Ход долгой команды агента: события в stdout при `--events -` и строки хода в тексте
//! (`INV.USE-CASES.A-LONG-AGENT-COMMAND-SHOWS-ITS-PROGRESS`, `CTR.WIRE.PROGRESS-EVENTS`).
//!
//! Двойник агента — настоящий SSH-сервер в процессе теста (`support::fake_agent`): на выгрузку
//! он отвечает строкой журнала с паролем базы, двумя массивами `progress` и итогом, каждым
//! отдельным массивом. Чужой агент раннер не поднимает, поэтому платформа здесь не нужна.

mod support;

use std::fs;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use support::fake_agent::{
    random_host_key, start_fake_agent_with_host_key, FakeAgent, Hold, HoldReply, AGENT_PASSWORD,
};
use support::{temp_workspace, v8_runner_command};

/// Строка журнала агента, в которой агент повторил пароль базы.
const DUMP_LOG: &str = "Подключение с паролем agentpass выполнено";

struct Harness {
    _dir: tempfile::TempDir,
    root: PathBuf,
    config_path: PathBuf,
    target: PathBuf,
}

fn harness() -> Harness {
    harness_with(|_, _| {})
}

/// То же, но двойника донастраивает тест: `configure` получает корень прогона.
fn harness_with(configure: impl FnOnce(&Path, &mut FakeAgent)) -> Harness {
    assert!(
        DUMP_LOG.contains(AGENT_PASSWORD),
        "the log names the password"
    );
    let dir = temp_workspace();
    let root = dir.path().to_path_buf();
    let target = root.join("project").join("configuration");
    fs::create_dir_all(&target).expect("configuration dir");
    fs::create_dir_all(root.join("work")).expect("work dir");
    let base = root.join("attached-base");
    fs::create_dir_all(base.join("0")).expect("attached user dir");
    fs::write(
        base.join("agentbasedir.json"),
        r#"{"usersInfo":[{"name":"","dir":"0"}]}"#,
    )
    .expect("attached map");
    let mut agent = FakeAgent::new(
        true,
        root.join("agent-commands.log"),
        Some(base.clone()),
        root.join("base-dir.txt"),
        root.join("designer.pid"),
    );
    agent.dump_log = Some(DUMP_LOG);
    configure(&root, &mut agent);
    let port = start_fake_agent_with_host_key(agent, random_host_key());
    let config_path = root.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: '{work}'\nformat: DESIGNER\nproviders:\n  pull: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  designer_agent:\n    attach: 127.0.0.1:{port}\n    base-dir: '{base}'\n",
            work = root.join("work").display(),
            ib = root.join("ib").display(),
            password = AGENT_PASSWORD,
            base = base.display(),
        ),
    )
    .expect("write config");
    Harness {
        _dir: dir,
        root,
        config_path,
        target,
    }
}

fn run(config: &Path, arguments: &[&str]) -> Output {
    v8_runner_command()
        .env_remove("FORCE_COLOR")
        .env_remove("NO_COLOR")
        .args(["--config", &config.display().to_string()])
        .args(arguments)
        .output()
        .expect("run v8-runner")
}

fn shown(output: &Output) -> String {
    format!(
        "code: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// События хода идут в stdout строками NDJSON, пока агент выгружает, и проходят схему;
/// конверт — последняя строка, одна, и событий внутри него нет. `step_index` растёт на
/// единицу, `step_count` его не меньше.
#[test]
fn events_stream_the_agent_progress_and_the_envelope_comes_last_on_one_line() {
    let harness = harness();
    let output = run(
        &harness.config_path,
        &["--json-message", "--events", "-", "pull", "--force"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(output.status.code(), Some(0), "{}", shown(&output));
    assert!(
        harness.target.join("Configuration.xml").is_file(),
        "{}",
        shown(&output)
    );

    let lines: Vec<&str> = stdout.lines().collect();
    let (envelope, events) = lines.split_last().expect("stdout is not empty");
    let envelope: Value = serde_json::from_str(envelope).expect("the last line is the envelope");
    assert_eq!(envelope["ok"], true, "{envelope}");
    assert_eq!(envelope["command"], "pull", "{envelope}");
    assert!(envelope.get("event").is_none(), "{envelope}");
    assert!(
        !lines.last().unwrap().contains(r#""event":"#),
        "the envelope carries no progress: {envelope}"
    );

    let events: Vec<Value> = events
        .iter()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("not json: {line}")))
        .collect();
    for event in &events {
        support::command_data::assert_progress_event_contract(event);
        assert_eq!(event["command"], "pull", "{event}");
    }

    let steps: Vec<&Value> = events
        .iter()
        .filter(|event| event["event"] == "step")
        .collect();
    assert!(
        steps
            .iter()
            .any(|step| step["phase"] == "dump" && step["set"] == "main"),
        "the dump is a step of the set: {}",
        shown(&output)
    );
    for (position, step) in steps.iter().enumerate() {
        let index = step["step_index"].as_u64().expect("step_index");
        assert_eq!(
            index,
            position as u64 + 1,
            "step_index grows by one: {step}"
        );
        assert!(
            step["step_count"].as_u64().expect("step_count") >= index,
            "{step}"
        );
    }

    let agent: Vec<&Value> = events
        .iter()
        .filter(|event| event["event"] == "agent")
        .collect();
    assert!(
        agent.iter().any(|event| event["agent_type"] == "progress"
            && event["text"] == "Выгрузка конфигурации 50%"),
        "the agent progress reached stdout: {}",
        shown(&output)
    );
    let log = agent
        .iter()
        .find(|event| event["agent_type"] == "log")
        .unwrap_or_else(|| panic!("the agent log reached stdout: {}", shown(&output)));
    assert!(
        !log["text"].as_str().unwrap().contains(AGENT_PASSWORD),
        "the infobase password is masked: {log}"
    );
    assert!(!stdout.contains(AGENT_PASSWORD), "{}", shown(&output));
}

/// Событие доходит до stdout, пока агент ещё работает: двойник держит выгрузку, а шаг
/// `dump` уже прочитан из stdout живого процесса. Конверт приходит после, последним.
#[test]
fn events_reach_stdout_while_the_agent_still_works() {
    let harness = harness_with(|root, agent| {
        agent.hold = Some(Hold {
            command: "config dump-config-to-files".to_owned(),
            started: root.join("dump-held"),
            release: root.join("dump-release"),
            reply: HoldReply::Normal,
        });
    });
    let mut child = v8_runner_command()
        .env_remove("FORCE_COLOR")
        .env_remove("NO_COLOR")
        .args(["--config", &harness.config_path.display().to_string()])
        .args(["--json-message", "--events", "-", "pull", "--force"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn v8-runner");
    let stdout = child.stdout.take().expect("piped stdout");
    let (lines_tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut seen = Vec::new();
    loop {
        let line = lines
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|_| panic!("no dump step while the agent works: {seen:?}"));
        let event: Value = serde_json::from_str(&line).expect("an NDJSON line");
        seen.push(line);
        if event["event"] == "step" && event["phase"] == "dump" {
            break;
        }
    }
    assert!(
        support::wait_until(Duration::from_secs(30), Duration::from_millis(50), || {
            harness.root.join("dump-held").exists()
        }),
        "the agent got the dump"
    );
    assert!(
        child.try_wait().expect("child state").is_none(),
        "the step came before the command ended: {seen:?}"
    );

    fs::write(harness.root.join("dump-release"), "").expect("release the dump");
    let status = child.wait().expect("wait for v8-runner");
    seen.extend(lines.iter());
    assert!(status.success(), "{seen:?}");
    let envelope: Value =
        serde_json::from_str(seen.last().expect("the envelope")).expect("envelope json");
    assert_eq!(envelope["ok"], true, "{seen:?}");
}

/// Без ключа stdout прежний: один конверт и никаких событий.
#[test]
fn without_events_stdout_is_one_envelope() {
    let harness = harness();
    let output = run(&harness.config_path, &["--json-message", "pull", "--force"]);
    assert_eq!(output.status.code(), Some(0), "{}", shown(&output));
    let envelope: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("stdout is one envelope: {}", shown(&output)));
    assert_eq!(envelope["ok"], true, "{envelope}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(r#""event":"#),
        "{}",
        shown(&output)
    );
}

/// Текстом ход агента идёт в stdout строками своего вида, под узлом этапа и с маской пароля.
#[test]
fn text_output_shows_the_agent_progress_lines() {
    let harness = harness();
    let output = run(&harness.config_path, &["--no-color", "pull", "--force"]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(output.status.code(), Some(0), "{}", shown(&output));
    assert!(
        stdout
            .lines()
            .any(|line| line == "│     ↳ progress: Выгрузка конфигурации 50%"),
        "{}",
        shown(&output)
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("│     ↳ log: Подключение с паролем ")),
        "{}",
        shown(&output)
    );
    assert!(!stdout.contains(AGENT_PASSWORD), "{}", shown(&output));
}

fn assert_refused(output: &Output, names: &str) {
    assert_eq!(output.status.code(), Some(2), "{}", shown(output));
    assert!(
        output.stdout.is_empty(),
        "stdout stays empty: {}",
        shown(output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--events"), "{}", shown(output));
    assert!(stderr.contains(names), "{}", shown(output));
}

/// Без `--json-message` событиям некуда встать перед конвертом: отказ при старте.
#[test]
fn events_without_json_message_are_refused_at_start() {
    let dir = temp_workspace();
    let config = dir.path().join("v8project.yaml");
    let output = run(&config, &["--events", "-", "pull", "--force"]);
    assert_refused(&output, "--json-message");
}

/// stdout сервера MCP занят протоколом: отказ при старте, текстом в stderr.
#[test]
fn events_are_refused_by_the_mcp_server() {
    let dir = temp_workspace();
    let config = dir.path().join("v8project.yaml");
    let output = run(
        &config,
        &["--json-message", "--events", "-", "mcp", "serve", "stdio"],
    );
    assert_refused(&output, "mcp serve stdio");
}

/// stdout `--publication-stdio` занят его кадрами: отказ при старте, текстом в stderr.
#[test]
fn events_are_refused_next_to_publication_stdio() {
    let dir = temp_workspace();
    let config = dir.path().join("v8project.yaml");
    let stage = dir.path().join("stage").display().to_string();
    let output = run(
        &config,
        &[
            "--events",
            "-",
            "pull",
            "main",
            "--force",
            "--publication-stdio",
            &stage,
        ],
    );
    assert_refused(&output, "--publication-stdio");
}
