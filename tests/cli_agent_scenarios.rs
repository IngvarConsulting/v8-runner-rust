//! Остальные сценарии через агентский shell Конфигуратора: `make`, экспортное
//! семейство `infobase …`, расширения.
//!
//! Двойник агента (`support::fake_agent`) отказывает файловым параметрам через
//! символическую ссылку, как настоящий агент (замер 15.09.2026), и требует синоним в
//! форме `NStr()` — так проверяется, что раннер пишет файлы прямо в каталог агента,
//! копирует исходники внешних обработок и заворачивает синоним.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use support::command_data::assert_data_matches_one_of;
use support::fake_agent::{
    read_or_empty, start_fake_agent, write_fake_designer, FakeAgent, Hold, HoldReply,
    AGENT_PASSWORD,
};
use support::{
    interrupt_at_hold, temp_workspace, v8_runner_command, AGENT_COMMAND_ABANDONED,
    CRITICAL_INTERRUPTION_DEFERRED, OPERATOR_INTERRUPT_RECEIVED,
};

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    designer_args_log: PathBuf,
    base_dir_file: PathBuf,
}

fn harness_with(providers: &str) -> Harness {
    harness_for(None, providers)
}

/// Стенд с базой по адресу `connection`; без него — файловая база самого стенда.
fn harness_for(connection: Option<&str>, providers: &str) -> Harness {
    harness_holding(connection, providers, None)
}

/// Стенд, двойник которого держит команду `hold`, пока тест её не отпустит.
fn harness_holding(connection: Option<&str>, providers: &str, hold: Option<Hold>) -> Harness {
    let dir = temp_workspace();
    let root = dir.path().to_path_buf();
    let project = root.join("project");
    fs::create_dir_all(project.join("configuration")).expect("configuration");
    fs::write(
        project.join("configuration").join("Configuration.xml"),
        "<Configuration/>",
    )
    .expect("root");
    fs::create_dir_all(project.join("ext")).expect("ext");
    fs::write(
        project.join("ext").join("Configuration.xml"),
        "<Configuration/>",
    )
    .expect("ext root");
    fs::create_dir_all(project.join("tools").join("Alpha")).expect("tools");
    fs::write(
        project.join("tools").join("Alpha.xml"),
        "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
    )
    .expect("alpha descriptor");
    fs::write(
        project.join("tools").join("Alpha").join("Module.bsl"),
        "// module",
    )
    .expect("alpha module");
    let work_path = root.join("work");
    fs::create_dir_all(&work_path).expect("work dir");
    fs::create_dir_all(root.join("ib")).expect("ib");
    fs::write(root.join("ib").join("1Cv8.1CD"), "db").expect("ib file");
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
    let port = start_fake_agent(agent);
    write_fake_designer(
        &bin.join("1cv8"),
        &designer_args_log,
        &designer_pid_file,
        &base_dir_file,
    );
    let connection = connection.map_or_else(
        || format!("File={}", root.join("ib").display()),
        str::to_owned,
    );
    let config_path = root.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {work}\nformat: DESIGNER\nproviders:\n{providers}infobase:\n  connection: '{connection}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\n  - name: Зонд\n    type: EXTENSION\n    path: project/ext\n  - name: tools\n    type: EXTERNAL_DATA_PROCESSORS\n    path: project/tools\ntools:\n  platform:\n    path: {platform}\n    strict: true\n    version: '8.3.27'\n  designer_agent:\n    port: {port}\n",
            work = work_path.display(),
            password = AGENT_PASSWORD,
            platform = root.join("platform").display(),
        ),
    )
    .expect("write config");
    Harness {
        config_path,
        commands_log,
        designer_args_log,
        base_dir_file,
        dir,
    }
}

fn harness() -> Harness {
    harness_with(
        "  make: agent\n  extensions: agent\n  infobase.configuration.export: agent\n  infobase.dump: agent\n  infobase.restore: agent\n",
    )
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

fn agent_user_dir(harness: &Harness) -> PathBuf {
    PathBuf::from(read_or_empty(&harness.base_dir_file)).join("0")
}

/// `make` cf: `dump-cfg` пишет прямо в каталог агента, файл публикуется, каталог
/// агента после команды чист.
#[test]
fn make_cf_through_the_agent_publishes_the_package() {
    let harness = harness();
    let output = harness.dir.path().join("dist").join("release.cf");

    let (code, payload) = run(
        &harness,
        &["artifacts", "--output", &output.display().to_string()],
    );

    assert_eq!(code, 0, "{payload}");
    assert_eq!(fs::read_to_string(&output).expect("package"), "CF:main");
    let lines = commands(&harness);
    assert_eq!(
        lines.get(1).map(String::as_str),
        Some("common connect-ib"),
        "{lines:?}"
    );
    assert!(
        lines
            .get(2)
            .is_some_and(|line| line.starts_with("config dump-cfg --file=make/")
                && line.ends_with("/main.cf")),
        "{lines:?}"
    );
    assert_eq!(lines.last().map(String::as_str), Some("common shutdown"));
    assert_eq!(read_or_empty(&harness.designer_args_log).lines().count(), 1);
    assert!(
        !agent_user_dir(&harness).join("make").exists()
            || fs::read_dir(agent_user_dir(&harness).join("make"))
                .map(|entries| entries.count() == 0)
                .unwrap_or(true),
        "make dir left behind in the agent user dir"
    );
}

/// `make` cfe: имя расширения уходит в `--extension=`.
#[test]
fn make_cfe_through_the_agent_names_the_extension() {
    let harness = harness();
    let output = harness.dir.path().join("dist").join("probe.cfe");

    let (code, payload) = run(
        &harness,
        &[
            "artifacts",
            "--output",
            &output.display().to_string(),
            "--extension",
            "Зонд",
        ],
    );

    assert_eq!(code, 0, "{payload}");
    assert_eq!(fs::read_to_string(&output).expect("package"), "CFE:Зонд");
    let lines = commands(&harness);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("config dump-cfg --file=make/")
                && line.ends_with(" --extension=Зонд")),
        "{lines:?}"
    );
}

/// `make` epf: исходники копируются в каталог агента (файловый параметр через ссылку
/// агент не разрешает), каждый файл собирается и выгружается обратно для сверки.
#[test]
fn make_epf_through_the_agent_builds_and_verifies_each_external_file() {
    let harness = harness();
    let output = harness.dir.path().join("dist").join("tools");

    let (code, payload) = run(
        &harness,
        &[
            "artifacts",
            "--output",
            &output.display().to_string(),
            "--source-set",
            "tools",
        ],
    );

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        fs::read_to_string(output.join("Alpha.epf")).expect("epf"),
        "EPF:Alpha.xml"
    );
    let lines = commands(&harness);
    let load = lines
        .iter()
        .find(|line| line.starts_with("config load-external-data-processor-or-report-from-files"))
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!(
        load.contains("--file=make/") && load.contains("/src-0/Alpha.xml --ext-file=make/"),
        "{load}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("config dump-external-data-processor-or-report-to-files")),
        "{lines:?}"
    );
    let verified = harness
        .dir
        .path()
        .join("work")
        .join("external-dump")
        .join("tools");
    assert!(
        verified.is_dir(),
        "round-trip descriptor kept under workPath"
    );
    assert!(
        !agent_user_dir(&harness).join("make").exists()
            || fs::read_dir(agent_user_dir(&harness).join("make"))
                .map(|entries| entries.count() == 0)
                .unwrap_or(true)
    );
}

/// Экспорт рабочей конфигурации — `dump-cfg`; конфигурацию базы данных агент
/// экспортировать не умеет, и раннер отказывает при выборе исполнителя, до сессии.
#[test]
fn configuration_export_through_the_agent_handles_working_state_only() {
    let harness = harness();
    let output = harness.dir.path().join("out").join("main.cf");

    let (code, payload) = run(
        &harness,
        &[
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ],
    );

    assert_eq!(code, 0, "{payload}");
    assert_eq!(fs::read_to_string(&output).expect("cf"), "CF:main");
    assert!(
        commands(&harness)
            .iter()
            .any(|line| line.starts_with("config dump-cfg --file=export/")),
        "{:?}",
        commands(&harness)
    );

    let sessions = commands(&harness).len();
    let (code, payload) = run(
        &harness,
        &[
            "infobase",
            "configuration",
            "export",
            "--state",
            "database",
            "--output",
            &harness
                .dir
                .path()
                .join("out")
                .join("db.cf")
                .display()
                .to_string(),
        ],
    );

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "capability", "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("which only designer exports")),
        "{payload}"
    );
    assert_eq!(
        commands(&harness).len(),
        sessions,
        "no session for a refused state"
    );
}

/// Выгрузка базы — `dump-ib` в каталог агента и перенос в цель.
#[test]
fn infobase_dump_through_the_agent_publishes_the_dt() {
    let harness = harness();
    let output = harness.dir.path().join("out").join("base.dt");

    let (code, payload) = run(
        &harness,
        &[
            "infobase",
            "dump",
            "--output",
            &output.display().to_string(),
        ],
    );

    assert_eq!(code, 0, "{payload}");
    assert_eq!(fs::read_to_string(&output).expect("dt"), "DT");
    assert!(
        commands(&harness)
            .iter()
            .any(|line| line.starts_with("infobase-tools dump-ib --file=export/")),
        "{:?}",
        commands(&harness)
    );
}

/// Загрузка базы — DT подкладывается в каталог агента жёсткой ссылкой или копией;
/// агент после загрузки сам рвёт соединение, и это не ошибка.
#[test]
fn infobase_restore_through_the_agent_survives_the_agent_closing_the_session() {
    let harness = harness();
    let input = harness.dir.path().join("transfer").join("base.dt");
    fs::create_dir_all(input.parent().unwrap()).expect("transfer");
    fs::write(&input, "payload-42").expect("dt");

    let (code, payload) = run(
        &harness,
        &[
            "infobase",
            "restore",
            "--input",
            &input.display().to_string(),
            "--replace",
        ],
    );

    assert_eq!(code, 0, "{payload}");
    let lines = commands(&harness);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("infobase-tools restore-ib --file=restore/")),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"restored: payload-42".to_owned()),
        "{lines:?}"
    );
    assert!(
        !agent_user_dir(&harness).join("restore").exists()
            || fs::read_dir(agent_user_dir(&harness).join("restore"))
                .map(|entries| entries.count() == 0)
                .unwrap_or(true),
        "the DT copy is withdrawn"
    );
}

/// Агенту секция `infobase.dbms` не нужна: подключение `ibcmd` строится только для
/// `ibcmd`, и серверная база без секции обслуживается всем семейством `extensions` —
/// свойствами, составом и его изменением. Превью называет базу в кластере, а не в СУБД.
#[test]
fn a_server_base_without_dbms_serves_extensions_through_the_agent() {
    let harness = harness_for(
        Some("Srvr=127.0.0.1:1541;Ref=demo"),
        "  extensions: agent\n",
    );

    let (code, payload) = run(&harness, &["extensions", "list", "--dry-run"]);
    assert_eq!(code, 0, "{payload}");
    let plan = payload["data"]["plan"].as_str().expect("plan");
    assert!(
        plan.contains("server infobase 'demo' on '127.0.0.1:1541'"),
        "{plan}"
    );
    for arguments in [
        &["extensions", "--dry-run"][..],
        &[
            "extensions",
            "create",
            "--name",
            "Проба",
            "--name-prefix",
            "Пр_",
            "--dry-run",
        ][..],
    ] {
        let (code, payload) = run(&harness, arguments);
        assert_eq!(code, 0, "{payload}");
        let message = payload["data"]["steps"][0]["message"]
            .as_str()
            .expect("step message");
        assert!(
            message.contains("server infobase 'demo' on '127.0.0.1:1541'")
                && message.contains("via the designer agent"),
            "{message}"
        );
    }

    let (code, payload) = run(&harness, &["extensions", "list"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
    let (code, payload) = run(&harness, &["extensions"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
    let (code, payload) = run(
        &harness,
        &[
            "extensions",
            "create",
            "--name",
            "Проба",
            "--name-prefix",
            "Пр_",
        ],
    );
    assert_eq!(code, 0, "{payload}");

    let lines = commands(&harness);
    for expected in [
        "config extensions properties get --all-extensions",
        "config extensions properties set --extension=Зонд --safe-mode=no --unsafe-action-protection=no",
    ] {
        assert!(lines.iter().any(|line| line == expected), "{lines:?}");
    }
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("config extensions create --extension=Проба")),
        "{lines:?}"
    );
}

/// Сессия агента, открытая без команды запроса, работы не даёт. `extensions` без целей
/// подключается к базе служебными командами, ничего не просит и отвечает
/// `provider_dispatched: false`; с целью — просит и отвечает `true` (тест выше).
#[test]
fn an_agent_session_without_a_request_command_gives_no_work() {
    let harness = harness_with("  extensions: agent\n");
    let config = fs::read_to_string(&harness.config_path).expect("config");
    let without_extensions = config.replace(
        "  - name: Зонд\n    type: EXTENSION\n    path: project/ext\n",
        "",
    );
    assert_ne!(
        config, without_extensions,
        "the sample declares an extension set"
    );
    fs::write(&harness.config_path, without_extensions).expect("rewrite config");

    let (code, payload) = run(&harness, &["extensions"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["steps"], serde_json::json!([]), "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], false, "{payload}");
    let lines = commands(&harness);
    assert!(
        lines.iter().any(|line| line == "common connect-ib"),
        "the session was opened, and only service commands went to it: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("config ")),
        "no request command reached the agent: {lines:?}"
    );
}

/// Состав расширений читается из структурного ответа `properties get`.
#[test]
fn extensions_list_and_info_through_the_agent_read_the_structured_reply() {
    let harness = harness();

    let (code, payload) = run(&harness, &["extensions", "list"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["extensions"][0]["name"], "Зонд",
        "{payload}"
    );
    assert_eq!(
        payload["data"]["extensions"][0]["active"], true,
        "{payload}"
    );
    assert_eq!(
        payload["data"]["extensions"][0]["safe_mode"], true,
        "{payload}"
    );
    assert!(payload["data"]["extensions"][0]["name_prefix"].is_null());
    assert_data_matches_one_of(
        &payload["data"],
        "`extensions list`",
        &["extensions-inventory"],
    );
    assert!(
        commands(&harness)
            .contains(&"config extensions properties get --all-extensions".to_owned()),
        "{:?}",
        commands(&harness)
    );

    let (code, payload) = run(&harness, &["extensions", "info", "--name", "Зонд"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["extensions"][0]["name"], "Зонд",
        "{payload}"
    );
    assert!(payload["data"]["extensions"][0]["name_prefix"].is_null());
    assert_data_matches_one_of(
        &payload["data"],
        "`extensions info`",
        &["extensions-inventory"],
    );

    let (code, payload) = run(&harness, &["extensions", "info", "--name", "Нет"]);
    assert_ne!(code, 0, "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("ExtensionNotFound")),
        "{payload}"
    );
    // Агент команду запроса получил и ответил отказом: это отказ после работы, и он
    // отвечает формой чтения, а не общей формой отказа.
    assert_eq!(payload["data"]["ok"], false, "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
    assert_data_matches_one_of(
        &payload["data"],
        "`extensions info` refused by the agent",
        &["extensions-inventory"],
    );
}

/// Отключение безопасного режима — `properties set` на каждое расширение в одной сессии.
#[test]
fn extensions_safety_is_disabled_through_the_agent_in_one_session() {
    let harness = harness();

    let (code, payload) = run(&harness, &["extensions"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["steps"][0]["ok"], true, "{payload}");
    let lines = commands(&harness);
    assert!(
        lines.contains(
            &"config extensions properties set --extension=Зонд --safe-mode=no --unsafe-action-protection=no"
                .to_owned()
        ),
        "{lines:?}"
    );
    assert_eq!(read_or_empty(&harness.designer_args_log).lines().count(), 1);

    let (code, payload) = run(&harness, &["extensions", "info", "--name", "Зонд"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["extensions"][0]["safe_mode"], false,
        "{payload}"
    );
    assert_eq!(
        payload["data"]["extensions"][0]["unsafe_action_protection"], false,
        "{payload}"
    );
}

/// Создание, выключение и удаление: синоним уходит в форме NStr(), как требует агент.
#[test]
fn extensions_create_activate_and_delete_reach_the_agent_verbs() {
    let harness = harness();

    let (code, payload) = run(
        &harness,
        &[
            "extensions",
            "create",
            "--name",
            "Проба",
            "--name-prefix",
            "Пр_",
            "--purpose",
            "add-on",
        ],
    );
    assert_eq!(code, 0, "{payload}");
    let lines = commands(&harness);
    assert!(
        lines.contains(
            &"config extensions create --extension=Проба --name-prefix=Пр_ --synonym=\"ru='Проба'; en='Проба'\" --purpose=add-on"
                .to_owned()
        ),
        "{lines:?}"
    );

    let (code, payload) = run(
        &harness,
        &[
            "extensions",
            "activate",
            "--name",
            "Проба",
            "--active",
            "no",
        ],
    );
    assert_eq!(code, 0, "{payload}");
    let (code, payload) = run(&harness, &["extensions", "info", "--name", "Проба"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["extensions"][0]["active"], false,
        "{payload}"
    );
    assert_eq!(
        payload["data"]["extensions"][0]["purpose"], "add-on",
        "{payload}"
    );

    let (code, payload) = run(&harness, &["extensions", "delete", "--name", "Проба"]);
    assert_eq!(code, 0, "{payload}");
    let (code, payload) = run(&harness, &["extensions", "list"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["extensions"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        1,
        "{payload}"
    );
}

/// `load` через агента не предусмотрен: у агента нет `compare-cfg`, а проба
/// совместимости перед загрузкой обязательна. Строки в матрице нет, и конфиг с
/// `providers.upload: agent` отвергается валидацией — до сессии и до процесса.
#[test]
fn load_through_the_agent_is_refused_without_a_session() {
    let harness = harness_with("  load: agent\n");
    let artifact = harness.dir.path().join("in.cf");
    fs::write(&artifact, "cf").expect("cf");

    let (code, payload) = run(
        &harness,
        &["load", "--path", &artifact.display().to_string()],
    );

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("providers.upload is not allowed")),
        "{payload}"
    );
    assert!(commands(&harness).is_empty(), "{:?}", commands(&harness));
    assert_eq!(read_or_empty(&harness.designer_args_log).lines().count(), 0);
}

const EVERY_AGENT_PROVIDER: &str = "  make: agent\n  extensions: agent\n  infobase.configuration.export: agent\n  infobase.dump: agent\n  infobase.restore: agent\n";

/// Удержание команды `command` двойником: знаки лежат в `marks`.
fn hold(marks: &tempfile::TempDir, command: &str, reply: HoldReply) -> Hold {
    Hold {
        command: command.to_owned(),
        started: marks.path().join("started"),
        release: marks.path().join("release"),
        reply,
    }
}

fn runner(harness: &Harness, arguments: &[&str]) -> std::process::Command {
    let mut command = v8_runner_command();
    command
        .args([
            "--config",
            &harness.config_path.display().to_string(),
            "--json-message",
        ])
        .args(arguments);
    command
}

/// Отмена, которую `restore-ib` отложил, названа и тогда, когда агент потом ответил отказом
/// или оборвал сессию: оператор просил остановить, и ответ говорит, почему его не послушали,
/// как на пути Конфигуратора (#317).
#[test]
fn a_failed_restore_through_the_agent_still_names_the_deferred_cancellation() {
    for reply in [HoldReply::Error, HoldReply::Close] {
        let marks = temp_workspace();
        let held = hold(&marks, "infobase-tools restore-ib", reply);
        let (started, release) = (held.started.clone(), held.release.clone());
        let harness = harness_holding(None, EVERY_AGENT_PROVIDER, Some(held));
        let input = harness.dir.path().join("transfer").join("base.dt");
        fs::create_dir_all(input.parent().unwrap()).expect("transfer");
        fs::write(&input, "payload-42").expect("dt");

        let (code, payload) = interrupt_at_hold(
            runner(
                &harness,
                &[
                    "infobase",
                    "restore",
                    "--input",
                    &input.display().to_string(),
                    "--replace",
                ],
            ),
            &marks.path().join("actions.log"),
            &started,
            &release,
            CRITICAL_INTERRUPTION_DEFERRED,
        );

        assert_ne!(code, 0, "{payload}");
        assert_ne!(
            payload["error"]["code"], "cancelled",
            "the failure is the agent's: {payload}"
        );
        let data = &payload["data"];
        assert_eq!(data["execution"]["status"], "failed", "{payload}");
        let deferred = &data["execution"]["interruptions"][0];
        assert_eq!(deferred["deferred"], true, "{payload}");
        assert_eq!(deferred["kind"], "cancelled", "{payload}");
        assert_eq!(deferred["phase"], "provider_command", "{payload}");
        assert!(
            payload["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .any(|warning| warning
                    .as_str()
                    .unwrap_or_default()
                    .contains("infobase DT restore after cancellation request")),
            "{payload}"
        );
    }
}

/// Загрузка внешней обработки через агента не критическая: она пишет только файл обработки.
/// Отмена её обрывает, как на пути Конфигуратора, и `make` отвечает отменой оборванной работы
/// (#317).
#[test]
fn an_interrupt_cuts_the_external_load_through_the_agent() {
    let marks = temp_workspace();
    let held = hold(
        &marks,
        "config load-external-data-processor-or-report-from-files",
        HoldReply::Normal,
    );
    let (started, release) = (held.started.clone(), held.release.clone());
    let harness = harness_holding(None, EVERY_AGENT_PROVIDER, Some(held));
    let output = harness.dir.path().join("dist").join("tools");

    let (code, payload) = interrupt_at_hold(
        runner(
            &harness,
            &[
                "artifacts",
                "--output",
                &output.display().to_string(),
                "--source-set",
                "tools",
            ],
        ),
        &marks.path().join("actions.log"),
        &started,
        &release,
        AGENT_COMMAND_ABANDONED,
    );

    assert_eq!(code, 4, "{payload}");
    assert_eq!(payload["error"]["code"], "cancelled", "{payload}");
    let execution = &payload["data"]["execution"];
    assert_eq!(execution["status"], "cancelled", "{payload}");
    assert_eq!(
        execution["interruptions"][0]["deferred"], false,
        "{payload}"
    );
    assert_eq!(
        execution["interruptions"][0]["phase"], "provider_command",
        "{payload}"
    );
    assert!(!output.join("Alpha.epf").exists(), "nothing was published");
}

/// Отключение безопасного режима через агента — критическая запись. Отмену, которую она
/// отложила, шаг называет и у удачи, и у отказа агента (#317).
#[test]
fn extensions_safety_through_the_agent_names_the_deferred_cancellation() {
    for (reply, ok, wording) in [
        (
            HoldReply::Normal,
            true,
            "extension properties update completed successfully after cancellation request",
        ),
        (
            HoldReply::Error,
            false,
            "extension properties update ended after cancellation request",
        ),
    ] {
        let marks = temp_workspace();
        let held = hold(&marks, "config extensions properties set", reply);
        let (started, release) = (held.started.clone(), held.release.clone());
        let harness = harness_holding(None, EVERY_AGENT_PROVIDER, Some(held));

        let (code, payload) = interrupt_at_hold(
            runner(&harness, &["extensions"]),
            &marks.path().join("actions.log"),
            &started,
            &release,
            CRITICAL_INTERRUPTION_DEFERRED,
        );

        // Удача после отложенной отмены остаётся удачей, отказ — отказом.
        assert_eq!(code == 0, ok, "{payload}");
        assert_eq!(payload["data"]["ok"], ok, "{payload}");
        let step = &payload["data"]["steps"][0];
        assert_eq!(step["ok"], ok, "{payload}");
        assert!(
            step["message"]
                .as_str()
                .unwrap_or_default()
                .contains(wording),
            "{payload}"
        );
    }
}

/// Изменение состава расширений через агента, отложившее отмену и потом отказавшее или
/// оборвавшее сессию, называет её в сообщении шага (#317).
#[test]
fn an_extension_created_through_the_agent_names_the_deferral_of_its_failure() {
    for (reply, failure) in [
        (HoldReply::Error, "agent command failed"),
        (HoldReply::Close, "ended before a reply"),
    ] {
        let marks = temp_workspace();
        let held = hold(&marks, "config extensions create", reply);
        let (started, release) = (held.started.clone(), held.release.clone());
        let harness = harness_holding(None, EVERY_AGENT_PROVIDER, Some(held));

        let (code, payload) = interrupt_at_hold(
            runner(
                &harness,
                &[
                    "extensions",
                    "create",
                    "--name",
                    "Проба",
                    "--name-prefix",
                    "Пр_",
                ],
            ),
            &marks.path().join("actions.log"),
            &started,
            &release,
            CRITICAL_INTERRUPTION_DEFERRED,
        );

        assert_ne!(code, 0, "{payload}");
        let step = &payload["data"]["steps"][0];
        assert_eq!(step["ok"], false, "{payload}");
        // Отложенная отмена открывает текст отказа, раньше слов агента.
        let message = step["message"].as_str().unwrap_or_default();
        let deferral = message
            .find("extension create ended after cancellation request")
            .unwrap_or_else(|| panic!("{payload}"));
        let failure = message.find(failure).unwrap_or_else(|| panic!("{payload}"));
        assert!(deferral < failure, "{payload}");
    }
}

/// Строки журнала, которых ждут тесты, — те же, что пишет раннер: переименование падает
/// здесь, а не полуминутным ожиданием в каждом тесте.
#[test]
fn the_awaited_log_lines_are_the_runners_own() {
    for (file, line) in [
        ("src/platform/process.rs", CRITICAL_INTERRUPTION_DEFERRED),
        ("src/platform/agent.rs", AGENT_COMMAND_ABANDONED),
        ("src/cli/signal.rs", OPERATOR_INTERRUPT_RECEIVED),
    ] {
        let source =
            fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
                .unwrap_or_else(|error| panic!("{file}: {error}"));
        assert!(
            source.contains(&format!("\"{line}\"")),
            "{file} no longer writes `{line}`"
        );
    }
}
