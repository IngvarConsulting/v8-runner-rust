//! Выгрузка через агентский shell Конфигуратора поверх системного `ssh`.
//!
//! Харнесс подкладывает два скрипта. `1cv8` в агентском режиме записывает свои ключи,
//! создаёт раскладку `AgentBaseDir` как платформа и живёт до сигнала. `ssh` проверяет
//! пароль через askpass, читает команды из открытого stdin по одной и отвечает
//! JSON-массивами; на `dump-config-to-files` пишет файлы в каталог пользователя агента.
//! Так фальсифицируется ровно то, что обещают правила: первая команда, готовность по
//! аутентификации, локальная платформа для управляемого агента, чтение результата с
//! диска, типизированный отказ у недоступной чужой точки входа.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use support::{free_tcp_port, temp_workspace, v8_runner_command, wait_until, write_shell_script};

const AGENT_PASSWORD: &str = "agentpass";

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    ssh_args_log: PathBuf,
    ssh_commands_log: PathBuf,
    designer_args_log: PathBuf,
    designer_pid_file: PathBuf,
    base_dir_file: PathBuf,
    target: PathBuf,
    port: u16,
}

/// Поведение поддельного `ssh`: кто пускает и что отвечает.
#[derive(Clone, Copy)]
enum SshBehaviour {
    /// Аутентификация по паролю из askpass, ответы JSON.
    Agent,
    /// Порт открыт, пароль отвергнут.
    DeniesPassword,
}

fn write_fake_ssh(harness: &Harness, behaviour: SshBehaviour, ssh_path: &Path) {
    let gate = match behaviour {
        SshBehaviour::Agent => format!(
            "pass=$(\"$SSH_ASKPASS\" 'Password:')\nif [ \"$pass\" != '{AGENT_PASSWORD}' ]; then echo 'user@127.0.0.1: Permission denied (password).' >&2; exit 255; fi"
        ),
        SshBehaviour::DeniesPassword => {
            "echo 'user@127.0.0.1: Permission denied (password).' >&2\nexit 255".to_owned()
        }
    };
    let body = format!(
        r#"for arg in "$@"; do printf '[%s]' "$arg"; done >> "{args_log}"
printf '\n' >> "{args_log}"
printf 'askpass=%s require=%s\n' "$SSH_ASKPASS" "$SSH_ASKPASS_REQUIRE" >> "{args_log}"
{gate}
printf 'designer> '
while IFS= read -r line; do
  printf '%s\n' "$line" >> "{commands_log}"
  case "$line" in
    "options set"*) printf '[\n{{\n"type": "success",\n"message": ""\n}}\n]\n' ;;
    "config dump-config-to-files"*)
      dir=${{line#*--dir=}}; dir=${{dir%% *}}
      userdir="$(cat "{base_dir_file}")/0"
      mkdir -p "$userdir/$dir"
      printf '<Configuration/>\n' > "$userdir/$dir/Configuration.xml"
      printf '[{{"type":"log","message":"Выгрузка конфигурации"}},{{"type":"progress","message":"100"}},{{"type":"success","message":""}}]\n' ;;
    "common shutdown") printf '[{{"type":"success","message":""}}]\n'; kill "$(cat "{pid_file}")" 2>/dev/null; exit 0 ;;
    *) printf '[{{"type":"error","error-type":"CommandFormatError","message":"Неизвестная команда"}}]\n' ;;
  esac
done
exit 0"#,
        args_log = harness.ssh_args_log.display(),
        commands_log = harness.ssh_commands_log.display(),
        base_dir_file = harness.base_dir_file.display(),
        pid_file = harness.designer_pid_file.display(),
    );
    write_shell_script(ssh_path, &body);
}

fn write_fake_designer(harness: &Harness, path: &Path) {
    let body = format!(
        r#"printf '%s\n' "$*" >> "{args_log}"
printf '%s\n' "$$" > "{pid_file}"
base=""
port=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "/AgentBaseDir" ]; then base="$arg"; fi
  if [ "$prev" = "/AgentPort" ]; then port="$arg"; fi
  prev="$arg"
done
if [ -z "$base" ] || [ -z "$port" ]; then exit 3; fi
mkdir -p "$base/0"
printf '{{"usersInfo":[{{"name":"","dir":"0"}}]}}' > "$base/agentbasedir.json"
printf '%s' "$base" > "{base_dir_file}"
# Агент слушает порт: раннер узнаёт о готовности по соединению, а не по прозе ssh.
python3 -c 'import socket, sys
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))
s.listen(5)
while True:
    c, _ = s.accept()
    c.close()' "$port" &
listener=$!
trap 'kill $listener 2>/dev/null; exit 0' TERM INT
while :; do sleep 1; done"#,
        args_log = harness.designer_args_log.display(),
        pid_file = harness.designer_pid_file.display(),
        base_dir_file = harness.base_dir_file.display(),
    );
    write_shell_script(path, &body);
}

/// Проект с версионной раскладкой платформы: строгий поиск не уходит за её пределы.
fn harness(with_designer: bool, ssh: Option<SshBehaviour>, agent_yaml: &str) -> Harness {
    harness_on(with_designer, ssh, agent_yaml, free_tcp_port())
}

fn harness_on(
    with_designer: bool,
    ssh: Option<SshBehaviour>,
    agent_yaml: &str,
    port: u16,
) -> Harness {
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

    let harness = Harness {
        config_path: root.join("v8project.yaml"),
        ssh_args_log: root.join("ssh-args.log"),
        ssh_commands_log: root.join("ssh-commands.log"),
        designer_args_log: root.join("designer-args.log"),
        designer_pid_file: root.join("designer.pid"),
        base_dir_file: root.join("base-dir.txt"),
        target,
        dir,
        port,
    };
    if with_designer {
        write_fake_designer(&harness, &bin.join("1cv8"));
    }
    let ssh_path = root.join("fake-ssh");
    if let Some(behaviour) = ssh {
        write_fake_ssh(&harness, behaviour, &ssh_path);
    }
    fs::write(
        &harness.config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\nproviders:\n  dump: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27'\n  designer_agent:\n    ssh: {ssh}\n{agent_yaml}",
            work = work_path.display(),
            ib = root.join("ib").display(),
            password = AGENT_PASSWORD,
            platform = root.join("platform").display(),
            ssh = ssh_path.display(),
            agent_yaml = if agent_yaml.is_empty() {
                format!("    port: {port}\n")
            } else {
                agent_yaml.to_owned()
            },
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
/// через системный `ssh` без псевдотерминала, читает результат с диска и гасит агента.
#[test]
fn managed_agent_dumps_through_the_system_ssh_client_and_reads_the_result_from_disk() {
    let harness = harness(true, Some(SshBehaviour::Agent), "");

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

    // Сессия: `-T`, порт агента, пустой логин базы без пользователей, пароль через askpass.
    let ssh_args = read_or_empty(&harness.ssh_args_log);
    assert!(ssh_args.contains("[-T]"), "{ssh_args}");
    assert!(
        ssh_args.contains(&format!("[-p][{}]", harness.port)),
        "{ssh_args}"
    );
    assert!(ssh_args.contains("[-l][]"), "{ssh_args}");
    assert!(ssh_args.contains("require=force"), "{ssh_args}");

    // Порядок команд: JSON-режим первым, выгрузка, завершение агента.
    let commands = read_or_empty(&harness.ssh_commands_log);
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
    let harness = harness(true, Some(SshBehaviour::Agent), "");

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

/// Готовность агента доказывает аутентификация: открытый порт с отвергнутым паролем —
/// отказ среды с уликой от `ssh`, а не попытка работать дальше.
#[test]
fn a_rejected_password_is_an_environment_refusal_even_though_the_port_answers() {
    let harness = harness(true, Some(SshBehaviour::DeniesPassword), "");

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(
        payload["error"]["code"], "environment_unavailable",
        "{payload}"
    );
    let message = payload["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("session ended before a reply") && message.contains("Permission denied"),
        "{payload}"
    );
    assert!(
        !harness.target.join("Configuration.xml").exists(),
        "nothing may be published after a refused session"
    );
}

/// Агент для файловой цели без платформы на этой машине — отказ на выборе
/// исполнителя: `ssh` не вызывается вовсе.
#[test]
fn a_managed_agent_without_the_local_platform_is_refused_before_ssh_is_started() {
    let harness = harness(false, Some(SshBehaviour::Agent), "");

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
        !harness.ssh_args_log.exists(),
        "ssh must not be started when the local platform is missing"
    );
}

/// Чужая точка входа, которая не отвечает, — типизированный отказ; свой процесс рядом
/// не поднимается.
#[test]
fn an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead() {
    // Свободный порт, на котором никто не слушает: соединение отвергает ОС.
    let port = free_tcp_port();
    let harness = harness_on(
        true,
        Some(SshBehaviour::Agent),
        &format!("    attach: 127.0.0.1:{port}\n    base-dir: /tmp/agent-base\n"),
        port,
    );

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
    assert!(
        !harness.ssh_args_log.exists(),
        "reachability is decided by the socket; ssh is not started for a dead endpoint"
    );
}

/// Ключи двух режимов не смешиваются: `attach` с ключами запуска — ошибка валидации.
#[test]
fn attach_does_not_mix_with_launch_keys() {
    let harness = harness(
        true,
        Some(SshBehaviour::Agent),
        "    attach: 127.0.0.1:2222\n    port: 1600\n",
    );

    let (code, payload) = run_dump(&harness, &["--mode", "full", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("tools.designer_agent.attach")),
        "{payload}"
    );
}
