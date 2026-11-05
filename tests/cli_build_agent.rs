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
    read_or_empty, serve_managed_launches_on_a_reserved_port, write_fake_designer_for_user,
    FakeAgent, Hold, HoldReply, AGENT_PASSWORD,
};
use support::{
    interrupt_at_hold, temp_workspace, v8_runner_command, CRITICAL_INTERRUPTION_DEFERRED,
};

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    designer_args_log: PathBuf,
    sources: PathBuf,
    /// Порт, объявленный в `tools.designer_agent.port`.
    port: u16,
}

fn harness() -> Harness {
    harness_holding(None)
}

/// Стенд, двойник которого держит команду `hold`, пока тест её не отпустит.
fn harness_holding(hold: Option<Hold>) -> Harness {
    harness_holding_password(hold, AGENT_PASSWORD)
}

fn harness_holding_password(hold: Option<Hold>, password: &str) -> Harness {
    harness_holding_credentials(hold, "", password)
}

fn harness_holding_credentials(hold: Option<Hold>, user: &str, password: &str) -> Harness {
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
    let mut agent = FakeAgent::new(
        true,
        commands_log.clone(),
        None,
        base_dir_file.clone(),
        designer_pid_file.clone(),
    );
    agent.hold = hold;
    agent.any_credentials = !user.is_empty() || password != AGENT_PASSWORD;
    agent.connect_log = (password != AGENT_PASSWORD).then_some("previous connection diagnostic");
    // Двойник держит объявленный порт занятым весь прогон и отвечает ключом, который
    // раннер передал агенту.
    let port = serve_managed_launches_on_a_reserved_port(agent, None);
    write_fake_designer_for_user(
        &bin.join("1cv8"),
        &designer_args_log,
        &designer_pid_file,
        &base_dir_file,
        user,
    );
    let config_path = root.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\nproviders:\n  build: agent\n  dump: agent\ninfobase:\n  connection: 'File={ib}'\n  user: {user}\n  password: {password}\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27'\n  designer_agent:\n    port: {port}\n",
            work = work_path.display(),
            ib = root.join("ib").display(),
            user = serde_json::to_string(user).expect("YAML user"),
            password = serde_json::to_string(password).expect("YAML string"),
            platform = root.join("platform").display(),
        ),
    )
    .expect("write config");
    support::memory::remember_base(
        &work_path,
        "origin",
        support::memory::Base::File(&root.join("ib")),
        &[support::memory::Set::configuration("main", &sources)],
    );
    Harness {
        config_path,
        commands_log,
        designer_args_log,
        sources,
        port,
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

/// The platform's diagnostic batch must survive a generic terminal error, with
/// decoded credentials hidden in both the failed envelope and the transcript.
#[test]
fn failed_agent_load_keeps_the_file_reason_and_stage_without_escaped_credentials() {
    let signals = temp_workspace();
    let release = signals.path().join("release");
    fs::write(&release, "").expect("release before dispatch");
    let password = concat!("privatecredential", "\"", "\\", "\n", "pass");
    let harness = harness_holding_password(
        Some(Hold {
            command: "config load-config-from-files".into(),
            started: signals.path().join("started"),
            release,
            reply: HoldReply::JsonMessages(
                r#"[{"type":"log","message":"Reports/Sales/Forms/Main/Ext/Form.xml: Invalid data path Rows.Reference; credential \u0070rivatecredential\"\\\npass","body":{"\u0070rivatecredential\"\\\npass":"private response"}}]
[{"type":"error","error-type":"ConfigFilesError","message":"configuration files rejected"}]
"#,
            ),
        }),
        password,
    );
    let output = v8_runner_command()
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
            "push",
        ])
        .output()
        .expect("push");
    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("failed envelope");
    assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    for expected in [
        "agent load",
        "ConfigFilesError",
        "Reports/Sales/Forms/Main/Ext/Form.xml",
        "Invalid data path Rows.Reference",
        "configuration files rejected",
    ] {
        assert!(message.contains(expected), "missing {expected}: {message}");
    }
    let transcript = fs::read_to_string(
        harness
            .dir
            .path()
            .join("work/logs/platform/build-agent.log"),
    )
    .expect("transcript");
    let all_display = format!(
        "{}{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        transcript
    );
    for private in [password, "privatecredential", "rivatecredential"] {
        assert!(
            !all_display.contains(private),
            "credential leaked: {all_display}"
        );
    }
    assert!(transcript.contains("previous connection diagnostic"));
    assert!(
        !message.contains("previous connection diagnostic"),
        "another command must not contaminate this failure"
    );
    assert!(transcript.contains("Invalid data path Rows.Reference"));
    assert!(transcript.contains("***"));
    assert!(
        !commands(&harness)
            .iter()
            .any(|line| line.starts_with("config update-db-cfg")),
        "a refused load must not apply"
    );
}

#[test]
fn malformed_and_truncated_agent_replies_keep_their_class_without_raw_secrets() {
    for text in [
        r#"[{"type":"privatecredential\"\\\npass"}]"#,
        r#"[{"type":"error","message":"privatecredential\"\\\npass"}"#,
    ] {
        let signals = temp_workspace();
        let release = signals.path().join("release");
        fs::write(&release, "").expect("release");
        let password = concat!("privatecredential", "\"", "\\", "\n", "pass");
        let harness = harness_holding_password(
            Some(Hold {
                command: "config load-config-from-files".into(),
                started: signals.path().join("started"),
                release,
                reply: HoldReply::Prose { text, close: true },
            }),
            password,
        );
        let (code, payload) = run(&harness, &["push"]);
        assert_ne!(code, 0);
        assert_eq!(payload["error"]["code"], "invalid_output", "{payload}");
        let transcript = fs::read_to_string(
            harness
                .dir
                .path()
                .join("work/logs/platform/build-agent.log"),
        )
        .expect("transcript");
        for display in [payload.to_string(), transcript] {
            assert!(!display.contains("privatecredential"), "{display}");
            assert!(!display.contains(password), "{display}");
        }
        assert!(
            payload["error"]["message"]
                .as_str()
                .unwrap()
                .contains("omitted"),
            "{payload}"
        );
    }
}

#[test]
fn agent_secrets_are_masked_before_the_terminal_excerpt_boundary() {
    let password = concat!("privatecredential", "\"", "\\", "\n", "pass");
    let reply = format!(
        "{}\n",
        serde_json::json!([
            {"type":"log", "message":"Forms/Main/Ext/Form.xml: invalid data path"},
            {"type":"error", "error-type":"ConfigFilesError", "message":format!("{}{password}", "д".repeat(2044))}
        ])
    );
    // The asynchronous fake server needs a static reply. Retain one bounded
    // diagnostic fixture for this integration-test process's lifetime.
    let reply = Box::leak(reply.into_boxed_str());
    let signals = temp_workspace();
    let release = signals.path().join("release");
    fs::write(&release, "").expect("release");
    let harness = harness_holding_password(
        Some(Hold {
            command: "config load-config-from-files".into(),
            started: signals.path().join("started"),
            release,
            reply: HoldReply::JsonMessages(reply),
        }),
        password,
    );
    let (_, payload) = run(&harness, &["push"]);
    assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
    let message = payload["error"]["message"].as_str().unwrap();
    assert!(
        !message.contains("priv"),
        "a secret prefix must not survive the excerpt boundary"
    );
    assert!(message.contains("Forms/Main/Ext/Form.xml"), "{payload}");
    assert!(message.contains("***"));
    let transcript = fs::read_to_string(
        harness
            .dir
            .path()
            .join("work/logs/platform/build-agent.log"),
    )
    .expect("transcript");
    assert!(!transcript.contains("privatecredential"));
}

#[test]
fn an_agent_closed_with_stderr_keeps_the_failure_class_and_omits_raw_bytes() {
    let signals = temp_workspace();
    let release = signals.path().join("release");
    fs::write(&release, "").expect("release");
    let harness = harness_holding(Some(Hold {
        command: "config load-config-from-files".into(),
        started: signals.path().join("started"),
        release,
        reply: HoldReply::StderrThenClose(r#"{"password":"\u0061gentpass"}"#),
    }));
    let output = v8_runner_command()
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
            "push",
        ])
        .output()
        .expect("push");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("envelope");
    assert!(!output.status.success());
    assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
    assert!(payload["error"]["message"]
        .as_str()
        .unwrap()
        .contains("omitted"));
    let transcript = fs::read_to_string(
        harness
            .dir
            .path()
            .join("work/logs/platform/build-agent.log"),
    )
    .expect("transcript");
    let display = format!(
        "{}{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        transcript
    );
    assert!(!display.contains("agentpass"));
    assert!(!display.contains("gentpass"));
}

#[test]
fn overlapping_agent_user_and_password_never_disclose_a_password_suffix() {
    let signals = temp_workspace();
    let release = signals.path().join("release");
    fs::write(&release, "").expect("release");
    let harness = harness_holding_credentials(
        Some(Hold {
            command: "config load-config-from-files".into(),
            started: signals.path().join("started"),
            release,
            reply: HoldReply::JsonMessages(
                r#"[{"type":"log","message":"Forms/Main/Ext/Form.xml: alicePrivateSuffix"}]
[{"type":"error","error-type":"ConfigFilesError","message":"alice could not load files"}]
"#,
            ),
        }),
        "alice",
        "alicePrivateSuffix",
    );
    let output = v8_runner_command()
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
            "push",
        ])
        .output()
        .expect("push");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("envelope");
    assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
    assert!(payload["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Forms/Main/Ext/Form.xml"));
    let transcript = fs::read_to_string(
        harness
            .dir
            .path()
            .join("work/logs/platform/build-agent.log"),
    )
    .expect("transcript");
    let all = format!(
        "{}{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        transcript
    );
    assert!(!all.contains("alicePrivateSuffix"), "{all}");
    assert!(!all.contains("PrivateSuffix"), "{all}");
}

#[test]
fn agent_diagnostics_keep_the_cause_when_a_password_contains_structural_syntax() {
    for (password, reply) in [
        (
            "prefix /P private",
            r#"[{"type":"log","message":"Form.xml: prefix /P private parser rejected"}]
[{"type":"error","error-type":"ConfigFilesError","message":"load failed"}]
"#,
        ),
        (
            "prefix Pwd=private; suffix",
            r#"[{"type":"log","message":"Form.xml: prefix Pwd=private; suffix; parser rejected"}]
[{"type":"error","error-type":"ConfigFilesError","message":"load failed"}]
"#,
        ),
        (
            "prefix http://alice:private@host suffix",
            r#"[{"type":"log","message":"Form.xml: prefix http://alice:private@host suffix parser rejected"}]
[{"type":"error","error-type":"ConfigFilesError","message":"load failed"}]
"#,
        ),
    ] {
        let signals = temp_workspace();
        let release = signals.path().join("release");
        fs::write(&release, "").expect("release");
        let harness = harness_holding_password(
            Some(Hold {
                command: "config load-config-from-files".into(),
                started: signals.path().join("started"),
                release,
                reply: HoldReply::JsonMessages(reply),
            }),
            password,
        );
        let output = v8_runner_command()
            .args([
                "--config",
                &harness.config_path.display().to_string(),
                "--json-message",
                "push",
            ])
            .output()
            .expect("push");
        let payload: Value = serde_json::from_slice(&output.stdout).expect("envelope");
        assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
        let message = payload["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("Form.xml") && message.contains("parser rejected"),
            "{message}"
        );
        let transcript = fs::read_to_string(
            harness
                .dir
                .path()
                .join("work/logs/platform/build-agent.log"),
        )
        .expect("transcript");
        let display = format!(
            "{}{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            transcript
        );
        assert!(!display.contains(password), "{display}");
        assert!(
            !display.contains("prefix"),
            "a credential fragment survived: {display}"
        );
    }
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
    // Объявленный порт уходит агенту как есть.
    let designer_args = read_or_empty(&harness.designer_args_log);
    assert!(
        designer_args.contains(&format!("/AgentPort {} ", harness.port)),
        "{designer_args}"
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
    // Журнал поколений лежит под памятью базы, запись — на набор.
    let bases: Vec<_> = fs::read_dir(harness.dir.path().join("work/infobases"))
        .expect("base memory")
        .map(|entry| entry.expect("entry").path())
        .collect();
    let [base] = bases.as_slice() else {
        panic!("one remembered base: {bases:?}");
    };
    let ledger: Value = serde_json::from_str(&read_or_empty(&base.join("generation.json")))
        .expect("generation ledger");
    let record = &ledger["main"];
    assert_eq!(record["after"], "build");
    assert_eq!(record["tool"], "agent");
    assert!(record["token"]
        .as_str()
        .is_some_and(|token| token.len() == 40));
}

/// `push --no-apply` через агента грузит без `update-db-cfg`; `apply` применяет командой
/// своей сессии и переносит запись поколения (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`).
#[test]
fn an_agent_push_without_apply_leaves_the_update_to_apply() {
    let harness = harness();

    let (code, payload) = run(&harness, &["push", "--no-apply"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["steps"][0]["applied"], false, "{payload}");
    let lines = commands(&harness);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("config load-config-from-files")),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line == "config update-db-cfg"),
        "{lines:?}"
    );
    let before = lines.len();

    let (code, payload) = run(&harness, &["apply"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    assert_eq!(
        payload["data"]["steps"][0]["outcome"], "applied",
        "{payload}"
    );
    assert_eq!(
        payload["data"]["steps"][0]["generation"], "recorded",
        "{payload}"
    );
    let lines = commands(&harness);
    assert!(
        lines[before..]
            .iter()
            .any(|line| line == "config update-db-cfg"),
        "{lines:?}"
    );
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

/// Инкрементальная выгрузка после сборки видит то же поколение и ничего не выгружает.
#[test]
fn an_incremental_dump_after_a_build_with_an_unchanged_generation_dumps_nothing() {
    let harness = harness();
    let (build, payload) = run(&harness, &["build"]);
    assert_eq!(build, 0, "{payload}");
    // Выгрузка поверх каталога вне системы контроля версий отказывает: каталог зафиксирован.
    let project = harness.sources.parent().expect("project dir");
    fs::write(project.join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
    support::commit_sources(project);

    let (dump, payload) = run(&harness, &["dump"]);

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

/// Прогон сборки, который прерывают, пока двойник держит `command`: отвечает он `reply`.
fn build_interrupted_at(command: &str, reply: HoldReply) -> (i32, Value, Vec<String>) {
    let marks = temp_workspace();
    let (started, release) = (marks.path().join("started"), marks.path().join("release"));
    let harness = harness_holding(Some(Hold {
        command: command.to_owned(),
        started: started.clone(),
        release: release.clone(),
        reply,
    }));
    let mut runner = v8_runner_command();
    runner.args([
        "--config",
        &harness.config_path.display().to_string(),
        "--json-message",
        "build",
    ]);
    let (code, payload) = interrupt_at_hold(
        runner,
        &marks.path().join("actions.log"),
        &started,
        &release,
        CRITICAL_INTERRUPTION_DEFERRED,
    );
    let lines = commands(&harness);
    (code, payload, lines)
}

/// Сообщение шага, который не удался: за ним в ответе идут пропущенные наборы.
fn step_message(payload: &Value) -> String {
    payload["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["ok"] == false)
        .and_then(|step| step["message"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Загрузка, отложившая отмену и потом отказавшая или оборвавшая сессию, остаётся отказом,
/// но отложенную отмену называет первой в тексте шага (#317).
#[test]
fn a_load_that_fails_after_a_deferred_cancellation_names_it() {
    for (reply, failure) in [
        (HoldReply::Error, "agent command failed"),
        (HoldReply::Close, "ended before a reply"),
    ] {
        let (code, payload, _) = build_interrupted_at("config load-config-from-files", reply);

        assert_ne!(code, 0, "{payload}");
        assert_ne!(
            payload["error"]["code"], "cancelled",
            "the failure is the agent's: {payload}"
        );
        let message = step_message(&payload);
        let deferral = message
            .find("load ended after cancellation request")
            .unwrap_or_else(|| panic!("{payload}"));
        let failure = message.find(failure).unwrap_or_else(|| panic!("{payload}"));
        assert!(deferral < failure, "{payload}");
    }
}

/// Обновление базы доведено, хоть отмену и просили; следующая команда — поколение — уже не
/// уходит агенту, и шаг останавливается отменой. Отложенную отмену обновления ответ всё равно
/// называет (#317).
#[test]
fn an_update_that_deferred_the_cancellation_is_named_when_the_generation_is_refused() {
    let (code, payload, lines) = build_interrupted_at("config update-db-cfg", HoldReply::Normal);

    assert_eq!(code, 4, "{payload}");
    assert_eq!(payload["error"]["code"], "cancelled", "{payload}");
    assert!(
        step_message(&payload)
            .contains("update_db_cfg completed successfully after cancellation request"),
        "{payload}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("config generation-id")),
        "no request goes to the agent after the interrupt: {lines:?}"
    );
}

/// Агент спрашивает поколение до загрузки: база ушла вперёд записанного им — отказ
/// `non_fast_forward` до загрузки, агенту загрузка не уходит.
#[test]
fn an_agent_push_into_a_base_that_moved_ahead_is_refused_before_the_load() {
    let harness = harness();
    let (code, payload) = run(&harness, &["push", "--force"]);
    assert_eq!(code, 0, "{payload}");
    let ledger_path = harness
        .dir
        .path()
        .join("work")
        .join("infobases")
        .join("origin")
        .join("generation.json");
    let mut ledger: Value =
        serde_json::from_str(&fs::read_to_string(&ledger_path).expect("ledger")).expect("json");
    assert_eq!(ledger["main"]["tool"], "agent", "{ledger}");
    let base = ledger["main"]["token"].as_str().expect("token").to_owned();
    // Память помнит другое поколение — как если бы базу правили после прошлой отправки.
    ledger["main"]["token"] = Value::String("f".repeat(40));
    fs::write(&ledger_path, ledger.to_string()).expect("ledger");
    fs::write(
        harness.sources.join("Catalogs").join("Items.xml"),
        "<Catalog edited=\"1\"/>",
    )
    .expect("edit");
    let loads_before = commands(&harness)
        .iter()
        .filter(|line| line.starts_with("config load-config-from-files"))
        .count();

    let (code, payload) = run(&harness, &["push"]);

    assert_eq!(code, 3, "{payload}");
    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(
        payload["error"]["base_generation"],
        base.as_str(),
        "{payload}"
    );
    assert_eq!(
        commands(&harness)
            .iter()
            .filter(|line| line.starts_with("config load-config-from-files"))
            .count(),
        loads_before,
        "nothing is loaded"
    );
}

#[test]
fn failed_agent_load_keeps_the_nearest_filename_and_typed_cause_after_long_logs() {
    let reply = format!(
        "{}\n",
        serde_json::json!([
            {"type":"log", "message":"noise".repeat(1400)},
            {"type":"log", "message":"Reports/Sales/Forms/Main/Ext/Form.xml"},
            {"type":"loading-issue", "message":"Invalid data path: Rows.Reference"},
            {"type":"error", "error-type":"ConfigFilesError", "message":""}
        ])
    );
    let reply = Box::leak(reply.into_boxed_str());
    let signals = temp_workspace();
    let release = signals.path().join("release");
    fs::write(&release, "").expect("release");
    let harness = harness_holding(Some(Hold {
        command: "config load-config-from-files".into(),
        started: signals.path().join("started"),
        release,
        reply: HoldReply::JsonMessages(reply),
    }));
    let (code, payload) = run(&harness, &["push"]);
    assert_ne!(code, 0);
    assert_eq!(payload["error"]["code"], "platform_failure", "{payload}");
    let message = payload["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("agent load") && message.contains("ConfigFilesError"),
        "{message}"
    );
    let file = message
        .find("Reports/Sales/Forms/Main/Ext/Form.xml")
        .expect("filename");
    let cause = message
        .find("Invalid data path: Rows.Reference")
        .expect("typed cause");
    assert!(file < cause);
    assert!(message.contains("[truncated]"));
    assert!(!commands(&harness)
        .iter()
        .any(|line| line.starts_with("config update-db-cfg")));
}
