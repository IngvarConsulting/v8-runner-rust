//! Выгрузка через агентский shell Конфигуратора встроенным SSH-клиентом.
//!
//! Харнесс держит двойника агента прямо в процессе теста: SSH-сервер на `russh` с
//! настоящим рукопожатием и аутентификацией по паролю, который отвечает как агент
//! Конфигуратора — JSON-массивами на каждую команду и файлами в каталог пользователя
//! на `dump-config-to-files`. Поддельный `1cv8` записывает ключи запуска, создаёт
//! раскладку `AgentBaseDir` как платформа и живёт до сигнала; порт для него слушает
//! двойник. Так фальсифицируется ровно то, что обещают правила: первая команда,
//! готовность по аутентификации, локальная платформа для управляемого агента, чтение
//! результата с диска, типизированный отказ у недоступной чужой точки входа.
#![cfg(unix)]

mod support;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use serde_json::Value;
use support::{temp_workspace, v8_runner_command, wait_until, write_shell_script};

const AGENT_PASSWORD: &str = "agentpass";

/// Двойник агента: один экземпляр на соединение, общее состояние — через `Arc`.
#[derive(Clone)]
struct FakeAgent {
    accept_password: bool,
    commands_log: PathBuf,
    /// Каталог `AgentBaseDir`: у чужого агента известен заранее, у управляемого его
    /// сообщает поддельный `1cv8` через файл.
    base_dir: Option<PathBuf>,
    base_dir_file: PathBuf,
    designer_pid_file: PathBuf,
    buffers: Arc<Mutex<HashMap<ChannelId, Vec<u8>>>>,
}

impl FakeAgent {
    fn user_dir(&self) -> PathBuf {
        let base = match self.base_dir.as_ref() {
            Some(base) => base.clone(),
            None => PathBuf::from(fs::read_to_string(&self.base_dir_file).unwrap_or_default()),
        };
        base.join("0")
    }

    /// Ответ на одну команду и признак «сессия завершается».
    fn respond(&self, line: &str) -> (String, bool) {
        if let Ok(mut log) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.commands_log)
        {
            use std::io::Write;
            let _ = writeln!(log, "{line}");
        }
        if line.starts_with("options set") {
            return (
                "[\n{\n\"type\": \"success\",\n\"message\": \"\"\n}\n]\n".to_owned(),
                false,
            );
        }
        if line.starts_with("config dump-config-to-files") {
            let dir = line
                .split_whitespace()
                .find_map(|word| word.strip_prefix("--dir="))
                .unwrap_or_default();
            let target = self.user_dir().join(dir);
            fs::create_dir_all(&target).expect("agent output dir");
            fs::write(target.join("Configuration.xml"), "<Configuration/>\n").expect("dump file");
            return (
                r#"[{"type":"log","message":"Выгрузка конфигурации"},{"type":"progress","message":"100"},{"type":"success","message":""}]"#.to_owned() + "\n",
                false,
            );
        }
        if line == "common shutdown" {
            if let Ok(pid) = fs::read_to_string(&self.designer_pid_file) {
                let _ = std::process::Command::new("kill")
                    .arg(pid.trim())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
            return (
                r#"[{"type":"success","message":""}]"#.to_owned() + "\n",
                true,
            );
        }
        (
            r#"[{"type":"error","error-type":"CommandFormatError","message":"Неизвестная команда"}]"#.to_owned() + "\n",
            false,
        )
    }
}

impl server::Server for FakeAgent {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for FakeAgent {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        // Настоящий агент слушает порт только после старта процесса; двойник слушает
        // заранее, поэтому сессию он принимает лишь после того, как поддельный `1cv8`
        // записал свою раскладку.
        if self.base_dir.is_none() {
            let started = std::time::Instant::now();
            while !self.base_dir_file.exists() && started.elapsed() < Duration::from_secs(20) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        // Правило агента: база без пользователей принимает пустой логин и пустую или
        // настроенную пару; любое другое имя отвергается.
        if self.accept_password && user.is_empty() && password == AGENT_PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.buffers
            .lock()
            .expect("buffers")
            .insert(channel.id(), Vec::new());
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        // Приглашение до JSON-режима: раннер обязан его пропустить, а не разбирать.
        session.data(channel, "designer> ".as_bytes().to_vec())?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let lines = {
            let mut buffers = self.buffers.lock().expect("buffers");
            let buffer = buffers.entry(channel).or_default();
            buffer.extend_from_slice(data);
            let mut lines = Vec::new();
            while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=end).collect::<Vec<_>>();
                lines.push(String::from_utf8_lossy(&line).trim_end().to_owned());
            }
            lines
        };
        for line in lines {
            let (reply, closing) = self.respond(&line);
            session.data(channel, reply.into_bytes())?;
            if closing {
                session.eof(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }
}

/// Поднимает двойника на свободном порту в отдельном потоке; живёт до конца теста.
fn start_fake_agent(agent: FakeAgent) -> u16 {
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fake agent runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind fake agent");
            port_tx
                .send(listener.local_addr().expect("addr").port())
                .expect("port");
            let seed: [u8; 32] = rand::random();
            let key = russh::keys::PrivateKey::new(
                russh::keys::ssh_key::private::KeypairData::Ed25519(
                    russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&seed),
                ),
                "fake-agent",
            )
            .expect("host key");
            let config = Arc::new(server::Config {
                auth_rejection_time: Duration::from_millis(50),
                auth_rejection_time_initial: Some(Duration::ZERO),
                inactivity_timeout: Some(Duration::from_secs(120)),
                keys: vec![key],
                ..server::Config::default()
            });
            let mut agent = agent;
            let _ = agent.run_on_socket(config, &listener).await;
        });
    });
    port_rx.recv().expect("fake agent port")
}

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    designer_args_log: PathBuf,
    designer_pid_file: PathBuf,
    base_dir_file: PathBuf,
    target: PathBuf,
    port: u16,
}

fn write_fake_designer(harness: &Harness, path: &Path) {
    let body = format!(
        r#"printf '%s\n' "$*" >> "{args_log}"
printf '%s\n' "$$" > "{pid_file}"
base=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "/AgentBaseDir" ]; then base="$arg"; fi
  prev="$arg"
done
if [ -z "$base" ]; then exit 3; fi
mkdir -p "$base/0"
printf '{{"usersInfo":[{{"name":"","dir":"0"}}]}}' > "$base/agentbasedir.json"
printf '%s' "$base" > "{base_dir_file}"
trap 'exit 0' TERM INT
while :; do sleep 1; done"#,
        args_log = harness.designer_args_log.display(),
        pid_file = harness.designer_pid_file.display(),
        base_dir_file = harness.base_dir_file.display(),
    );
    write_shell_script(path, &body);
}

/// Проект с версионной раскладкой платформы: строгий поиск не уходит за её пределы.
/// `agent` — двойник, который поднимается на порту управляемого агента; `None` —
/// на порту никто не слушает.
fn harness(with_designer: bool, agent: Option<bool>, attach: bool) -> Harness {
    let dir = temp_workspace();
    let root = dir.path().to_path_buf();
    let base_path = root.join("project");
    let work_path = root.join("work");
    let target = base_path.join("configuration");
    fs::create_dir_all(&target).expect("configuration dir");
    fs::write(target.join("old.txt"), "old").expect("old marker");
    fs::create_dir_all(&work_path).expect("work dir");
    let bin = root.join("platform").join("8.3.27.2074").join("bin");
    fs::create_dir_all(&bin).expect("platform dir");

    let commands_log = root.join("agent-commands.log");
    let base_dir_file = root.join("base-dir.txt");
    let designer_pid_file = root.join("designer.pid");
    // Чужой агент уже имеет свою раскладку: карту и каталог пользователя создал не раннер.
    let attached_base = attach.then(|| {
        let base = root.join("attached-base");
        fs::create_dir_all(base.join("0")).expect("attached user dir");
        fs::write(
            base.join("agentbasedir.json"),
            r#"{"usersInfo":[{"name":"","dir":"0"}]}"#,
        )
        .expect("attached map");
        base
    });
    let port = match agent {
        Some(accept_password) => start_fake_agent(FakeAgent {
            accept_password,
            commands_log: commands_log.clone(),
            base_dir: attached_base.clone(),
            base_dir_file: base_dir_file.clone(),
            designer_pid_file: designer_pid_file.clone(),
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }),
        None => support::free_tcp_port(),
    };
    let harness = Harness {
        config_path: root.join("v8project.yaml"),
        commands_log,
        designer_args_log: root.join("designer-args.log"),
        designer_pid_file,
        base_dir_file,
        target,
        dir,
        port,
    };
    if with_designer {
        write_fake_designer(&harness, &bin.join("1cv8"));
    }
    let agent_yaml = if let Some(base) = attached_base.as_ref() {
        format!(
            "    attach: 127.0.0.1:{port}\n    base-dir: {}\n",
            base.display()
        )
    } else {
        format!("    port: {port}\n")
    };
    fs::write(
        &harness.config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\nproviders:\n  dump: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27'\n  designer_agent:\n{agent_yaml}",
            work = work_path.display(),
            ib = root.join("ib").display(),
            password = AGENT_PASSWORD,
            platform = root.join("platform").display(),
        ),
    )
    .expect("write config");
    harness
}

fn run_dump(harness: &Harness, extra: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
            "dump",
        ])
        .args(extra)
        .output()
        .expect("run dump");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "dump printed no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().unwrap_or(-1), payload)
}

fn read_or_empty(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Управляемый агент: раннер поднимает Конфигуратор своими ключами, ведёт сессию
/// встроенным клиентом без псевдотерминала, читает результат с диска и гасит агента.
#[test]
fn managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk() {
    let harness = harness(true, Some(true), false);

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["ok"], true, "{payload}");
    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    assert!(
        harness.target.join("Configuration.xml").is_file(),
        "the agent dump was not published into the target"
    );
    assert!(
        !harness.target.join("old.txt").exists(),
        "a full dump replaces the target, it does not merge into it"
    );

    // Ключи запуска: только агентские плюс адрес базы, без `/N` и `/P`.
    let designer_args = read_or_empty(&harness.designer_args_log);
    assert!(
        designer_args.starts_with("DESIGNER /IBConnectionString File="),
        "{designer_args}"
    );
    for key in [
        "/AgentMode",
        &format!("/AgentPort {}", harness.port),
        "/AgentListenAddress 127.0.0.1",
        "/AgentSSHHostKeyAuto",
        "/AgentBaseDir ",
    ] {
        assert!(
            designer_args.contains(key),
            "missing {key}: {designer_args}"
        );
    }
    assert!(!designer_args.contains("/P "), "{designer_args}");
    let base_dir = read_or_empty(&harness.base_dir_file);
    assert!(
        Path::new(&base_dir).starts_with(harness.dir.path().join("work")),
        "the managed agent must live under workPath: {base_dir}"
    );

    // Порядок команд: JSON-режим первым, выгрузка, завершение агента.
    let commands = read_or_empty(&harness.commands_log);
    let lines: Vec<&str> = commands.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("options set --show-prompt=no --output-format=json"),
        "{commands}"
    );
    assert!(
        lines
            .get(1)
            .is_some_and(|line| line.starts_with("config dump-config-to-files --dir=dump/")),
        "{commands}"
    );
    assert_eq!(lines.last().copied(), Some("common shutdown"), "{commands}");

    // Журнал сессии лежит рядом с журналами платформы.
    assert!(harness
        .dir
        .path()
        .join("work/logs/platform/dump-main-agent.log")
        .is_file());

    // Поднятый процесс не переживает команду.
    let pid: u32 = read_or_empty(&harness.designer_pid_file)
        .trim()
        .parse()
        .expect("designer pid");
    assert!(
        wait_until(Duration::from_secs(5), Duration::from_millis(50), || {
            !process_is_alive(pid)
        }),
        "the managed agent must be stopped after the command"
    );
}

/// Инкрементальный режим у агента — полная выгрузка с предупреждением в ответе.
#[test]
fn incremental_mode_through_the_agent_degrades_to_full_and_says_so() {
    let harness = harness(true, Some(true), false);

    let (code, payload) = run_dump(&harness, &["--mode", "incremental"]);

    assert_eq!(code, 0, "{payload}");
    assert!(
        payload["data"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("ran a full export")),
        "{payload}"
    );
    assert!(harness.target.join("Configuration.xml").is_file());
}

/// Чужой агент: раннер ничего не поднимает, подключается по `attach`, читает результат
/// из объявленного `base-dir` и не гасит процесс, который не поднимал.
#[test]
fn an_attached_agent_is_used_without_launching_or_stopping_anything() {
    let harness = harness(true, Some(true), true);

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    assert!(
        harness.target.join("Configuration.xml").is_file(),
        "the attached agent dump was not published into the target"
    );
    assert!(
        !harness.designer_args_log.exists(),
        "an attached agent is never launched by the runner"
    );
    let commands = read_or_empty(&harness.commands_log);
    assert!(
        commands.starts_with("options set --show-prompt=no --output-format=json"),
        "{commands}"
    );
    assert!(
        !commands.contains("common shutdown"),
        "the runner must not stop an agent it did not start: {commands}"
    );
}

/// Готовность агента доказывает аутентификация: открытый порт с отвергнутым паролем —
/// отказ среды, а не попытка работать дальше.
#[test]
fn a_rejected_password_is_an_environment_refusal_even_though_the_port_answers() {
    let harness = harness(true, Some(false), false);

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(
        payload["error"]["code"], "environment_unavailable",
        "{payload}"
    );
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("rejected the credentials")),
        "{payload}"
    );
    assert!(
        !harness.target.join("Configuration.xml").exists(),
        "nothing may be published after a refused session"
    );
}

/// Агент для файловой цели без платформы на этой машине — отказ на выборе
/// исполнителя: к двойнику никто не подключается.
#[test]
fn a_managed_agent_without_the_local_platform_is_refused_before_any_connection() {
    let harness = harness(false, Some(true), false);

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(
        payload["error"]["code"], "environment_unavailable",
        "{payload}"
    );
    let receipt = &payload["data"]["provider"];
    assert!(receipt["selected"].is_null(), "{payload}");
    assert_eq!(receipt["skipped"][0]["provider"], "agent", "{payload}");
    assert!(
        receipt["skipped"][0]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("1cv8")),
        "{payload}"
    );
    assert!(
        !harness.commands_log.exists(),
        "no session may be opened when the local platform is missing"
    );
}

/// Чужая точка входа, которая не отвечает, — типизированный отказ; свой процесс рядом
/// не поднимается.
#[test]
fn an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead() {
    let harness = harness(true, None, true);

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(
        payload["error"]["code"], "environment_unavailable",
        "{payload}"
    );
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unreachable")),
        "{payload}"
    );
    assert!(
        !harness.designer_args_log.exists(),
        "an attached endpoint must never be replaced by a managed launch"
    );
}

/// Ключи двух режимов не смешиваются: `attach` с ключами запуска — ошибка валидации.
#[test]
fn attach_does_not_mix_with_launch_keys() {
    let harness = harness(true, None, false);
    let config = fs::read_to_string(&harness.config_path).expect("config");
    fs::write(
        &harness.config_path,
        config.replace(
            &format!("    port: {}\n", harness.port),
            &format!("    attach: 127.0.0.1:{}\n    port: 1600\n", harness.port),
        ),
    )
    .expect("rewrite config");

    let (code, payload) = run_dump(&harness, &["--mode", "full", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("tools.designer_agent.attach")),
        "{payload}"
    );
}
