//! Выгрузка через агентский shell Конфигуратора встроенным SSH-клиентом.
//!
//! Двойник агента — настоящий SSH-сервер в процессе теста (`support::fake_agent`),
//! поддельный `1cv8` записывает ключи запуска и раскладку `AgentBaseDir`. Так
//! фальсифицируется ровно то, что обещают правила: первая команда, готовность по
//! аутентификации, локальная платформа для управляемого агента, чтение результата с
//! диска, типизированный отказ у недоступной чужой точки входа, а также учёт
//! поколения: неизменившаяся конфигурация не выгружается.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use support::fake_agent::{
    fingerprint_of, process_is_alive, random_host_key, read_or_empty,
    start_fake_agent_with_host_key, write_fake_designer, write_host_key_file, FakeAgent,
    AGENT_PASSWORD,
};
use support::{temp_workspace, v8_runner_command, wait_until};

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

/// Проект с версионной раскладкой платформы: строгий поиск не уходит за её пределы.
/// `agent` — принимает ли двойник пароль; `None` — на порту никто не слушает.
fn harness(with_designer: bool, agent: Option<bool>, attach: bool) -> Harness {
    harness_with(with_designer, agent, attach, random_host_key(), |_| {
        String::new()
    })
}

/// То же, но ключ хоста двойника и добавка к `tools.designer_agent` — от вызывающего.
/// Так проверяется сверка ключа: двойник держит один ключ, конфигурация называет другой.
fn harness_with(
    with_designer: bool,
    agent: Option<bool>,
    attach: bool,
    host_key: russh::keys::PrivateKey,
    agent_extra: impl Fn(&russh::keys::PrivateKey) -> String,
) -> Harness {
    let extra_yaml = agent_extra(&host_key);
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
    let designer_args_log = root.join("designer-args.log");
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
        Some(accept_password) => start_fake_agent_with_host_key(
            FakeAgent::new(
                accept_password,
                commands_log.clone(),
                attached_base.clone(),
                base_dir_file.clone(),
                designer_pid_file.clone(),
            ),
            host_key,
        ),
        None => support::free_tcp_port(),
    };
    if with_designer {
        write_fake_designer(
            &bin.join("1cv8"),
            &designer_args_log,
            &designer_pid_file,
            &base_dir_file,
        );
    }
    let agent_yaml = if let Some(base) = attached_base.as_ref() {
        format!(
            "    attach: 127.0.0.1:{port}\n    base-dir: {}\n{extra_yaml}",
            base.display()
        )
    } else {
        format!("    port: {port}\n{extra_yaml}")
    };
    let config_path = root.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\nproviders:\n  dump: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27'\n  designer_agent:\n{agent_yaml}",
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
        designer_pid_file,
        base_dir_file,
        target,
        dir,
        port,
    }
}

/// Чужой агент закрепляется объявленным отпечатком.
#[test]
fn an_attached_agent_that_presents_another_key_is_refused_by_name() {
    let someone_else = fingerprint_of(&random_host_key());
    let expected = someone_else.clone();
    let harness = harness_with(false, Some(true), true, random_host_key(), move |_| {
        format!("    host-fingerprint: '{someone_else}'\n")
    });

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    let message = payload["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains(&expected), "{message}");
    // Учётные данные не ушли: до аутентификации дело не дошло.
    assert!(
        read_or_empty(&harness.commands_log).is_empty(),
        "the agent saw no command: {}",
        read_or_empty(&harness.commands_log)
    );
}

/// Управляемый агент закрепляется тем же файлом, который раннер отдаёт платформе.
///
/// Именно это утверждение несёт решение: раннер не занимает порт `1543`, а подключается
/// к тому, кто ответил. Здесь ответил не тот.
#[test]
fn a_managed_agent_is_pinned_by_the_host_key_file_it_was_given() {
    let harness = harness_with(true, Some(true), false, random_host_key(), |_| {
        String::new()
    });
    let key_file = harness.dir.path().join("host_key");
    write_host_key_file(&key_file, &random_host_key());
    let config = std::fs::read_to_string(&harness.config_path).expect("config");
    std::fs::write(
        &harness.config_path,
        config.replace(
            "  designer_agent:\n",
            &format!("  designer_agent:\n    host-key: {}\n", key_file.display()),
        ),
    )
    .expect("rewrite config");

    let (code, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_ne!(
        code, 0,
        "a key the agent does not hold is refused: {payload}"
    );
    let message = payload["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("host key"), "{message}");
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

fn commands(harness: &Harness) -> Vec<String> {
    read_or_empty(&harness.commands_log)
        .lines()
        .map(str::to_owned)
        .collect()
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
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
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

    // Порядок команд: JSON-режим, подключение к базе, поколение, выгрузка, завершение.
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
    assert_eq!(
        lines.get(2).map(String::as_str),
        Some("config generation-id"),
        "{lines:?}"
    );
    assert!(
        lines
            .get(3)
            .is_some_and(|line| line.starts_with("config dump-config-to-files --dir=dump/")),
        "{lines:?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("common shutdown"),
        "{lines:?}"
    );

    // Журнал сессии лежит рядом с журналами платформы; поколение записано.
    assert!(harness
        .dir
        .path()
        .join("work/logs/platform/dump-main-agent.log")
        .is_file());
    assert!(harness
        .dir
        .path()
        .join("work/agent/generation/main.json")
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

/// Инкрементальная выгрузка обновляет цель на месте через ссылку в каталоге агента.
#[test]
fn incremental_mode_updates_the_target_in_place_through_a_link() {
    let harness = harness(true, Some(true), false);

    let (code, payload) = run_dump(&harness, &["--mode", "incremental"]);

    assert_eq!(code, 0, "{payload}");
    assert!(harness.target.join("Configuration.xml").is_file());
    assert!(
        harness.target.join("updated.txt").is_file(),
        "the agent must have been asked to update the target itself"
    );
    assert!(
        harness.target.join("old.txt").is_file(),
        "an incremental dump merges into the target, it does not replace it"
    );
    let lines = commands(&harness);
    assert!(
        lines.iter().any(
            |line| line.starts_with("config dump-config-to-files --dir=target/")
                && line.ends_with("--update")
        ),
        "{lines:?}"
    );
    let user_dir = PathBuf::from(read_or_empty(&harness.base_dir_file)).join("0");
    assert!(
        !user_dir.join("target").exists()
            || fs::read_dir(user_dir.join("target"))
                .map(|entries| entries.count() == 0)
                .unwrap_or(true),
        "the link is withdrawn after the command"
    );
}

/// Поколение конфигурации, не изменившееся с последней выгрузки, не выгружается снова.
#[test]
fn an_unchanged_generation_is_not_dumped_twice() {
    let harness = harness(true, Some(true), false);

    let (first, payload) = run_dump(&harness, &["--mode", "full"]);
    assert_eq!(first, 0, "{payload}");
    let dumps_after_first = commands(&harness)
        .iter()
        .filter(|line| line.starts_with("config dump-config-to-files"))
        .count();
    assert_eq!(dumps_after_first, 1);

    let (second, payload) = run_dump(&harness, &["--mode", "full"]);

    assert_eq!(second, 0, "{payload}");
    assert_eq!(payload["data"]["up_to_date"], true, "{payload}");
    // Вопрос о поколении — команда запроса: исполнитель работу получил, хоть выгрузки и не было.
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
    assert!(
        payload["data"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("nothing to dump")),
        "{payload}"
    );
    let dumps_after_second = commands(&harness)
        .iter()
        .filter(|line| line.starts_with("config dump-config-to-files"))
        .count();
    assert_eq!(
        dumps_after_second, 1,
        "the second command must ask for the generation and stop there"
    );
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
    let lines = commands(&harness);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("options set --show-prompt=no --output-format=json"),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line == "common shutdown"),
        "the runner must not stop an agent it did not start: {lines:?}"
    );
}

/// Чужой агент, которому команда не дала ни одной команды запроса: сессия открыта, соединение
/// с базой закрыто служебной командой, а работы исполнитель не получил. Закрытие чужого
/// агента снимает отметку работы само — как и завершение управляемого.
#[test]
fn an_attached_agent_released_without_a_request_command_gives_no_work() {
    let harness = harness(false, Some(true), true);
    let config = fs::read_to_string(&harness.config_path).expect("config");
    let with_extensions = config.replace(
        "providers:\n  dump: agent\n",
        "providers:\n  dump: agent\n  extensions: agent\n",
    );
    assert_ne!(config, with_extensions, "the sample names its providers");
    fs::write(&harness.config_path, with_extensions).expect("rewrite config");

    let output = v8_runner_command()
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
            "extensions",
        ])
        .output()
        .expect("run extensions");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");

    assert_eq!(output.status.code(), Some(0), "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], false, "{payload}");
    let lines = commands(&harness);
    assert!(
        lines.iter().any(|line| line == "common disconnect-ib"),
        "the attached agent was released: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("config ")),
        "no request command reached the agent: {lines:?}"
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
    // Агент запущен, но сессия не открылась: запуск процесса сессии работой не считается.
    assert_ne!(payload["data"]["provider_dispatched"], true, "{payload}");
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
    assert_ne!(payload["data"]["provider_dispatched"], true, "{payload}");
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
