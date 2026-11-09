//! Живой агент Конфигуратора через командную строку (#577): команда берёт агента у сторожа,
//! настоящего процесса `v8-runner agent keeper`, а тот держит двойника агента
//! (`support::fake_agent`) между командами.
//!
//! У каждого стенда свой каталог состояния учётной записи (`V8_RUNNER_STATE_DIR`): параллельные
//! тесты не делят предел живых агентов. В конце стенд снимает своих сторожей и их агентов.
#![cfg(unix)]

mod support;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use support::fake_agent::{
    process_is_alive, read_or_empty, serve_managed_launches_on_a_reserved_port,
    write_fake_designer, FakeAgent, Hold, HoldReply, AGENT_PASSWORD,
};
use support::{interrupt_at_hold, temp_workspace, v8_runner_command, AGENT_COMMAND_ABANDONED};

/// Каталог состояния стенда: короткий путь в `/tmp`, потому что путь сокета сторожа ограничен.
struct State(tempfile::TempDir);

impl State {
    fn new() -> Self {
        Self(
            tempfile::Builder::new()
                .prefix("v8rl")
                .tempdir_in("/tmp")
                .expect("state dir"),
        )
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    /// Записи сторожей этого каталога состояния.
    fn records(&self) -> Vec<Value> {
        let Ok(keys) = fs::read_dir(self.path().join("agents")) else {
            return Vec::new();
        };
        keys.flatten()
            .filter_map(|key| fs::read(key.path().join("agent").join("record.json")).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }

    /// Живые сторожа этого каталога состояния.
    fn keepers(&self) -> Vec<Value> {
        self.records()
            .into_iter()
            .filter(|record| {
                record["pid"]
                    .as_u64()
                    .is_some_and(|pid| process_is_alive(pid as u32))
            })
            .collect()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for record in self.records() {
            for pid in [&record["pid"], &record["agent"]["pid"]] {
                if let Some(pid) = pid.as_u64() {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &pid.to_string()])
                        .status();
                }
            }
        }
    }
}

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    designer_args_log: PathBuf,
    designer_pid_file: PathBuf,
    state: PathBuf,
}

/// Стенд с файловой базой `ib` (своей, если не назван), своим двойником агента и каталогом
/// состояния `state`. `agent_yaml` — добавка к `tools.designer_agent`.
fn harness(state: &State, ib: Option<&Path>, agent_yaml: &str, hold: Option<Hold>) -> Harness {
    let dir = temp_workspace();
    let root = dir.path().to_path_buf();
    let project = root.join("project").join("configuration");
    fs::create_dir_all(&project).expect("configuration");
    fs::write(project.join("Configuration.xml"), "<Configuration/>").expect("root");
    fs::create_dir_all(root.join("work")).expect("work dir");
    let ib = ib.map_or_else(
        || {
            let ib = root.join("ib");
            fs::create_dir_all(&ib).expect("ib");
            fs::write(ib.join("1Cv8.1CD"), "db").expect("ib file");
            ib
        },
        Path::to_path_buf,
    );
    let bin = root.join("platform").join("8.3.27.2074").join("bin");
    fs::create_dir_all(&bin).expect("platform dir");
    let commands_log = root.join("agent-commands.log");
    let base_dir_file = root.join("base-dir.txt");
    let designer_pid_file = root.join("designer.pid");
    let designer_args_log = root.join("designer-args.log");
    let mut agent = FakeAgent::new(
        true,
        commands_log.clone(),
        None,
        base_dir_file.clone(),
        designer_pid_file.clone(),
    );
    agent.hold = hold;
    let port = serve_managed_launches_on_a_reserved_port(agent, None);
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
            "workPath: {work}\nformat: DESIGNER\nproviders:\n  infobase.configuration.export: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27.2074'\n  designer_agent:\n    port: {port}\n{agent_yaml}",
            work = root.join("work").display(),
            ib = ib.display(),
            password = AGENT_PASSWORD,
            platform = root.join("platform").display(),
        ),
    )
    .expect("write config");
    Harness {
        dir,
        config_path,
        commands_log,
        designer_args_log,
        designer_pid_file,
        state: state.path().to_path_buf(),
    }
}

impl Harness {
    fn ib(&self) -> PathBuf {
        self.dir.path().join("ib")
    }

    fn command(&self, arguments: &[&str]) -> std::process::Command {
        let mut command = v8_runner_command();
        command
            .env("V8_RUNNER_STATE_DIR", &self.state)
            .args([
                "--config",
                &self.config_path.display().to_string(),
                "--json-message",
            ])
            .args(arguments);
        command
    }

    /// `download` рабочей конфигурации через агента: ответ и код выхода.
    fn download(&self) -> (i32, Value) {
        let output = self.dir.path().join("dist").join(format!(
            "{}.cf",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = output.display().to_string();
        let ran = self
            .command(&[
                "download", "main", "--state", "working", "--output", &output,
            ])
            .output()
            .expect("run command");
        let payload: Value = serde_json::from_slice(&ran.stdout).unwrap_or_else(|error| {
            panic!(
                "no json envelope: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&ran.stdout),
                String::from_utf8_lossy(&ran.stderr)
            )
        });
        (ran.status.code().unwrap_or(-1), payload)
    }

    /// Запуски поддельного `1cv8`: по строке на запуск.
    fn launches(&self) -> usize {
        read_or_empty(&self.designer_args_log).lines().count()
    }

    fn commands(&self) -> Vec<String> {
        read_or_empty(&self.commands_log)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn designer_pid(&self) -> u32 {
        read_or_empty(&self.designer_pid_file)
            .trim()
            .parse()
            .expect("designer pid")
    }
}

fn live(payload: &Value) -> &Value {
    &payload["data"]["provider"]["endpoint"]["live"]
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

/// Запрос сторожу его протоколом — построчным JSON по сокету из записи.
fn ask_keeper(record: &Value, request: &Value) -> Value {
    let channel = record["agent"]["channel"].as_str().expect("keeper channel");
    let mut stream = std::os::unix::net::UnixStream::connect(channel).expect("keeper socket");
    writeln!(stream, "{request}").expect("request");
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .expect("keeper answer");
    serde_json::from_str(line.trim_end()).expect("keeper answer json")
}

/// Вторая команда берёт живого агента: `1cv8` поднят один раз, а между командами агент
/// отключён от базы (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`); квитанция говорит,
/// был ли агент живым (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`).
#[test]
fn a_second_command_reuses_the_live_agent() {
    let state = State::new();
    let harness = harness(&state, None, "", None);

    let (code, first) = harness.download();
    assert_eq!(code, 0, "{first}");
    assert_eq!(
        live(&first),
        &json!({"reused": false, "holders": [], "kept": true}),
        "{first}"
    );
    let (code, second) = harness.download();
    assert_eq!(code, 0, "{second}");
    assert_eq!(
        live(&second),
        &json!({"reused": true, "holders": [], "kept": true}),
        "{second}"
    );
    assert_eq!(harness.launches(), 1, "one 1cv8 for both commands");
    let commands = harness.commands();
    assert_eq!(
        commands
            .iter()
            .filter(|line| line.starts_with("config dump-cfg"))
            .count(),
        2,
        "{commands:?}"
    );
    assert_eq!(
        commands.last().map(String::as_str),
        Some("common disconnect-ib"),
        "the idle agent holds no connection to the infobase: {commands:?}"
    );
    assert!(
        process_is_alive(harness.designer_pid()),
        "the agent lives on"
    );
    assert_eq!(state.keepers().len(), 1);
}

/// С `live: false` команда работает с разовым агентом и снимает живого агента своей рабочей
/// копии (`INV.CONFIG.A-LIVE-AGENT-IS-ON-UNLESS-TURNED-OFF`).
#[test]
fn live_off_removes_the_live_agent_and_works_one_shot() {
    let state = State::new();
    let harness = harness(&state, None, "", None);
    let (code, payload) = harness.download();
    assert_eq!(code, 0, "{payload}");
    let live_agent = harness.designer_pid();
    assert_eq!(state.keepers().len(), 1);

    let config = fs::read_to_string(&harness.config_path).expect("config");
    fs::write(&harness.config_path, format!("{config}    live: false\n")).expect("live off");
    let (code, payload) = harness.download();

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        live(&payload),
        &json!({"reused": false, "holders": [], "kept": false, "reason": "live_agent_off"}),
        "{payload}"
    );
    assert!(
        wait_until(Duration::from_secs(20), || state.keepers().is_empty()),
        "the live agent of the working copy is removed"
    );
    assert!(
        wait_until(Duration::from_secs(10), || !process_is_alive(live_agent)),
        "its agent process is gone"
    );
    assert_eq!(harness.launches(), 2);
    let one_shot = harness.designer_pid();
    assert!(
        wait_until(Duration::from_secs(10), || !process_is_alive(one_shot)),
        "the one-shot agent is removed after the command"
    );
}

/// Сверх предела команда сначала снимает простаивающего агента без держателей, а если снять
/// некого — работает с разовым агентом (`INV.PLATFORM.A-COMMAND-OVER-THE-LIMIT-USES-A-ONE-SHOT-AGENT`).
#[test]
fn over_the_limit_an_idle_agent_gives_way_and_a_held_one_does_not() {
    let state = State::new();
    let first = harness(&state, None, "    live-max: 1\n", None);
    let (code, payload) = first.download();
    assert_eq!(code, 0, "{payload}");
    let first_agent = first.designer_pid();

    // Простаивающий агент без держателей уступает место.
    let second = harness(&state, None, "    live-max: 1\n", None);
    let (code, payload) = second.download();
    assert_eq!(code, 0, "{payload}");
    assert_eq!(live(&payload)["kept"], true, "{payload}");
    assert!(
        wait_until(Duration::from_secs(10), || !process_is_alive(first_agent)),
        "the idle agent gave way"
    );
    assert_eq!(state.keepers().len(), 1);

    // Агент с держателем места не уступает: третья команда работает с разовым агентом.
    let keeper = state.keepers().pop().expect("the second keeper");
    let until = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 600_000;
    let held = ask_keeper(
        &keeper,
        &json!({"op": "hold", "holder": {"kind": "ttl", "id": "test", "until_ms": until}}),
    );
    assert_eq!(held["frame"], "holders", "{held}");
    let third = harness(&state, None, "    live-max: 1\n", None);
    let (code, payload) = third.download();
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        live(&payload),
        &json!({"reused": false, "holders": [], "kept": false, "reason": "limit"}),
        "{payload}"
    );
    assert!(
        process_is_alive(second.designer_pid()),
        "the held agent stays"
    );
    let one_shot = third.designer_pid();
    assert!(
        wait_until(Duration::from_secs(10), || !process_is_alive(one_shot)),
        "the one-shot agent is removed"
    );
}

/// Команда, прерванная посреди запроса агенту, снимает живого агента
/// (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
#[test]
fn an_interrupted_command_removes_the_live_agent() {
    let state = State::new();
    let marks = temp_workspace();
    let hold = Hold {
        command: "config dump-cfg".to_owned(),
        started: marks.path().join("started"),
        release: marks.path().join("release"),
        reply: HoldReply::Normal,
    };
    let (started, release) = (hold.started.clone(), hold.release.clone());
    let harness = harness(&state, None, "", Some(hold));
    let output = harness.dir.path().join("dist").join("main.cf");
    let output = output.display().to_string();

    let (code, payload) = interrupt_at_hold(
        harness.command(&[
            "download", "main", "--state", "working", "--output", &output,
        ]),
        &marks.path().join("actions.log"),
        &started,
        &release,
        AGENT_COMMAND_ABANDONED,
    );

    assert_ne!(code, 0, "{payload}");
    assert_eq!(live(&payload)["kept"], false, "{payload}");
    assert_eq!(live(&payload)["reason"], "interrupted", "{payload}");
    let agent = harness.designer_pid();
    assert!(
        wait_until(Duration::from_secs(30), || state.keepers().is_empty()
            && !process_is_alive(agent)),
        "the interrupted live agent is removed"
    );
}

/// Агент, подключённый к базе сессией команды, — отказ `infobase_busy` команде другой рабочей
/// копии на той же базе; простаивающий агент к базе не подключён и не мешает
/// (`INV.PLATFORM.ANOTHER-LAUNCH-MODE-WAITS-ONLY-FOR-A-BUSY-AGENT`).
#[test]
fn a_busy_live_agent_holds_the_infobase_and_an_idle_one_does_not() {
    let state = State::new();
    let marks = temp_workspace();
    let hold = Hold {
        command: "config dump-cfg".to_owned(),
        started: marks.path().join("started"),
        release: marks.path().join("release"),
        reply: HoldReply::Normal,
    };
    let (started, release) = (hold.started.clone(), hold.release.clone());
    let owner = harness(&state, None, "", Some(hold));
    let other = harness(&state, Some(&owner.ib()), "", None);
    let output = owner.dir.path().join("dist").join("main.cf");
    let mut running = owner
        .command(&[
            "download",
            "main",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the owner");
    struct Release<'a>(&'a Path);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            let _ = fs::write(self.0, "");
        }
    }
    let guard = Release(&release);
    assert!(
        wait_until(Duration::from_secs(60), || started.exists()),
        "the owner's command reached the agent"
    );

    let (code, payload) = other.download();
    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["code"], "infobase_busy", "{payload}");
    assert!(
        payload.to_string().contains("config dump-cfg"),
        "the refusal names the agent's command: {payload}"
    );
    assert_eq!(other.launches(), 0, "the refused command launched nothing");

    drop(guard);
    assert!(running.wait().expect("owner").success());
    let (code, payload) = other.download();
    assert_eq!(
        code, 0,
        "an idle live agent does not hold the infobase: {payload}"
    );
}
