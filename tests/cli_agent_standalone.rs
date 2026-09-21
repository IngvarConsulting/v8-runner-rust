//! Автономный сервер как цель: раннер подключается к его SSH-шлюзу, ничего не
//! запуская, и обменивается файлами только объявленным каналом — каталогом
//! пользователя шлюза (`DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE`).
//!
//! Двойник шлюза — тот же `support::fake_agent` с фиксированным каталогом пользователя
//! и именованным логином, как у настоящего `ibsrv` (замер 15.09.2026).
//!
//! Набор идёт и под Windows: поддельная утилита платформы здесь не нужна — раннер к
//! шлюзу подключается, ничего не запуская, а двойник шлюза это обычный сервер на
//! `russh`. Под Windows набор заодно проверяет то, чего не видно под unix: пути на
//! стороне цели собираются через `/`, хотя локальные приходят с `\\`.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::fake_agent::{
    fingerprint_of, fingerprint_with, random_host_key, read_or_empty,
    start_fake_agent_with_host_key, FakeAgent, AGENT_PASSWORD,
};
use support::{temp_workspace, v8_runner_command};

struct Harness {
    dir: tempfile::TempDir,
    config_path: PathBuf,
    commands_log: PathBuf,
    user_dir: PathBuf,
    port: u16,
    /// Что объявлено в `infobase.standalone.host-fingerprint`, если объявлено.
    declared_fingerprint: Option<String>,
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
    harness_with_channel(Channel::Dir)
}

/// Канал обмена двойника: каталог пользователя шлюза на машине раннера или SFTP.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    Dir,
    Sftp,
    /// SFTP только на чтение — как у живого шлюза `ibsrv` 8.3.27.
    SftpReadOnly,
}

fn harness_with_channel(channel: Channel) -> Harness {
    harness_with(channel, random_host_key(), |_| None)
}

fn harness_with(
    channel: Channel,
    host_key: russh::keys::PrivateKey,
    declared: impl Fn(&russh::keys::PrivateKey) -> Option<String>,
) -> Harness {
    let declared_fingerprint = declared(&host_key);
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
    let mut gate = FakeAgent::gate(commands_log.clone(), GATE_USER, user_dir.clone());
    gate.sftp_read_only = channel == Channel::SftpReadOnly;
    let port = start_fake_agent_with_host_key(gate, host_key);
    let harness = Harness {
        config_path: root.join("v8project.yaml"),
        commands_log,
        user_dir,
        port,
        declared_fingerprint,
        dir,
    };
    let infobase = match channel {
        Channel::Dir => standalone_infobase(&harness),
        Channel::Sftp | Channel::SftpReadOnly => sftp_infobase(&harness),
    };
    write_config(&harness, &infobase, "");
    harness
}

fn sftp_infobase(harness: &Harness) -> String {
    format!(
        "  user: {GATE_USER}\n  password: '{password}'\n  standalone:\n    gate: 127.0.0.1:{port}\n{fingerprint}    exchange: sftp\n",
        password = AGENT_PASSWORD,
        port = harness.port,
        fingerprint = harness.declared_fingerprint.as_deref().map_or_else(
            String::new,
            |value| format!("    host-fingerprint: '{value}'\n")
        ),
    )
}

/// Объявленный отпечаток закрепляет шлюз: тот же ключ пускают, чужой — нет.
///
/// Раньше ключ хоста принимался любой, и подменивший адрес получал бы учётные данные
/// пользователя базы: их раннер отправляет сразу после рукопожатия.
#[test]
fn a_declared_fingerprint_lets_the_gate_through() {
    let harness = harness_with(Channel::Sftp, random_host_key(), |key| {
        Some(fingerprint_of(key))
    });

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_eq!(code, 0, "{payload}");
}

/// Отпечаток сверяется тем алгоритмом, каким записан.
///
/// Отпечатки разных алгоритмов не равны никогда, поэтому сверка, всегда считавшая
/// `SHA256`, читала бы объявленный `SHA512` как подменённый ключ — и объявивший его
/// не смог бы подключиться вовсе.
#[test]
fn a_declared_fingerprint_is_compared_with_its_own_algorithm() {
    let harness = harness_with(Channel::Sftp, random_host_key(), |key| {
        Some(fingerprint_with(key, russh::keys::ssh_key::HashAlg::Sha512))
    });

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_eq!(code, 0, "{payload}");
}

#[test]
fn a_gate_that_presents_another_key_is_refused_by_name() {
    // Шлюз держит свой ключ, а в конфигурации назван отпечаток другого — ровно то,
    // что увидел бы владелец при подмене адреса.
    let someone_else = fingerprint_of(&random_host_key());
    let harness = harness_with(Channel::Sftp, random_host_key(), |_| {
        Some(someone_else.clone())
    });

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    let message = payload["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        message.contains("host key"),
        "the refusal says what was wrong: {message}"
    );
    assert!(
        message.contains(&someone_else),
        "the refusal names what was expected: {message}"
    );
}

fn sftp_lines(harness: &Harness) -> Vec<String> {
    commands(harness)
        .into_iter()
        .filter(|line| line.starts_with("sftp "))
        .collect()
}

/// По SFTP выгрузка приходит к раннеру через шлюз: результат опубликован локально, на
/// стороне сервера следа нет, а каталог пользователя шлюза раннер напрямую не трогал.
#[test]
fn a_full_dump_travels_through_sftp() {
    let harness = harness_with_channel(Channel::Sftp);
    let target = harness.dir.path().join("project").join("configuration");

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_eq!(code, 0, "{payload}\n{:?}", commands(&harness));
    assert_eq!(
        fs::read_to_string(target.join("Configuration.xml")).expect("published dump"),
        "<Configuration/>\n"
    );
    let sftp = sftp_lines(&harness);
    assert!(
        sftp.iter().any(|line| line.starts_with("sftp mkdir dump/")),
        "{sftp:?}"
    );
    assert!(
        sftp.iter().any(|line| line.starts_with("sftp read dump/") && line.ends_with("/Configuration.xml")),
        "{sftp:?}"
    );
    assert!(
        sftp.iter().any(|line| line.starts_with("sftp rmdir dump/")),
        "remote run dir is removed: {sftp:?}"
    );
    assert!(
        fs::read_dir(&harness.user_dir)
            .map(|entries| entries.count() == 0)
            .unwrap_or(true),
        "nothing is left on the server side"
    );
}

/// Сборка по SFTP: исходники уходят на сторону сервера через шлюз, список частичной
/// загрузки — тоже; после команды на сервере пусто.
#[test]
fn a_build_travels_through_sftp() {
    let harness = harness_with_channel(Channel::Sftp);

    let (code, payload) = run(&harness, &["build"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["steps"][0]["mode"], "full", "{payload}");
    let sftp = sftp_lines(&harness);
    assert!(
        sftp.iter()
            .any(|line| line.starts_with("sftp write build/")
                && line.ends_with("/Configuration.xml")),
        "{sftp:?}"
    );
    assert!(
        commands(&harness)
            .iter()
            .any(|line| line.starts_with("config load-config-from-files --dir=build/")),
        "{:?}",
        commands(&harness)
    );
    assert!(
        fs::read_dir(&harness.user_dir)
            .map(|entries| entries.count() == 0)
            .unwrap_or(true),
        "nothing is left on the server side"
    );
}

/// Частичная сборка по SFTP не возит набор целиком: на сервер уходят корневые описатели,
/// изменённые файлы и список — и ничего больше.
#[test]
fn a_partial_build_through_sftp_ships_only_the_changed_files() {
    let harness = harness_with_channel(Channel::Sftp);
    let (code, payload) = run(&harness, &["build"]);
    assert_eq!(code, 0, "{payload}");
    let seen_before = commands(&harness).len();
    fs::write(
        harness
            .dir
            .path()
            .join("project/configuration/Catalogs/Items.xml"),
        "<Catalog changed='1'/>",
    )
    .expect("change");

    let (code, payload) = run(&harness, &["build"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["steps"][0]["mode"]["partial"]["file_count"], 1,
        "{payload}"
    );
    let writes: Vec<String> = commands(&harness)
        .into_iter()
        .skip(seen_before)
        .filter(|line| line.starts_with("sftp write "))
        .map(|line| line.rsplit('/').next().unwrap_or_default().to_owned())
        .collect();
    let mut expected = vec![
        "Configuration.xml".to_owned(),
        "Items.xml".to_owned(),
        "00-main.list.txt".to_owned(),
    ];
    let mut actual = writes.clone();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected, "{writes:?}");
}

/// Инкрементальная выгрузка по SFTP не возит цель целиком: на сервер уходит только
/// опись выгрузки, обратно приходят изменённые файлы поверх локальной цели.
#[test]
fn an_incremental_dump_through_sftp_sends_only_the_dump_info() {
    let harness = harness_with_channel(Channel::Sftp);
    let target = harness.dir.path().join("project").join("configuration");
    fs::write(target.join("ConfigDumpInfo.xml"), "<ConfigDumpInfo/>").expect("dump info");
    fs::write(target.join("Untouched.xml"), "<Keep/>").expect("untouched");

    let (code, payload) = run(&harness, &["dump", "--mode", "incremental"]);

    assert_eq!(code, 0, "{payload}");
    let sftp = sftp_lines(&harness);
    let writes: Vec<&String> = sftp
        .iter()
        .filter(|line| line.starts_with("sftp write "))
        .collect();
    assert_eq!(writes.len(), 1, "{sftp:?}");
    assert!(writes[0].ends_with("/ConfigDumpInfo.xml"), "{sftp:?}");
    assert!(
        commands(&harness).iter().any(|line| line
            .starts_with("config dump-config-to-files --dir=target/")
            && line.ends_with("--update")),
        "{:?}",
        commands(&harness)
    );
    assert!(
        target.join("updated.txt").is_file(),
        "changed files merged into the target"
    );
    assert!(
        target.join("Untouched.xml").is_file(),
        "files the server did not touch stay"
    );
}

/// Шлюз, чей SFTP не принимает запись (живой `ibsrv` 8.3.27): выгрузка и `make`
/// работают — они только забирают файлы; сборка отказывает типизированно, назвав
/// канал, а не падает где-то посередине.
#[test]
fn a_read_only_sftp_gate_serves_downloads_and_refuses_uploads() {
    let harness = harness_with_channel(Channel::SftpReadOnly);
    let output = harness.dir.path().join("dist").join("release.cf");

    let (code, payload) = run(
        &harness,
        &["artifacts", "--output", &output.display().to_string()],
    );
    assert_eq!(code, 0, "{payload}");
    assert_eq!(fs::read_to_string(&output).expect("package"), "CF:main");

    let (code, payload) = run(&harness, &["build"]);
    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "platform", "{payload}");
    assert!(
        error_message(&payload).contains("sftp exchange"),
        "{payload}"
    );
    assert!(
        !commands(&harness)
            .iter()
            .any(|line| line.starts_with("config load-config-from-files")),
        "nothing is loaded when the sources could not be delivered: {:?}",
        commands(&harness)
    );
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
        error_message(&payload).contains("infobase.standalone.exchange"),
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

/// Секция `standalone` первична, строка рядом с ней — адрес прямого шлюза: конфиг
/// принимается, команда идёт через SSH-шлюз, как раньше, а строку никто не читает — об
/// этом предупреждает загрузчик, пока исполнителя по прямому шлюзу нет (#205).
#[test]
fn a_direct_gate_address_next_to_the_standalone_section_is_accepted_but_not_used_yet() {
    let harness = harness();
    let infobase = format!(
        "  connection: 'Srvr=127.0.0.1:1541;Ref=demo'\n{}",
        standalone_infobase(&harness)
    );
    write_config(&harness, &infobase, "");

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_eq!(code, 0, "{payload}");
    let commands = commands(&harness);
    assert!(
        !commands.is_empty(),
        "the gate served the command: {payload}"
    );
    assert!(
        commands.iter().all(|command| {
            !command.contains("Srvr=")
                && !command.contains("Ref=")
                && !command.contains("1541\\demo")
        }),
        "the direct gate address never reaches the gate in any form: {commands:?}"
    );
    let warnings = payload["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("declared but not used yet")),
        "{warnings:?}"
    );
}

/// Файлового адреса у автономного сервера нет: `File=` рядом с `standalone` — отказ до
/// первого обращения к шлюзу.
#[test]
fn a_file_address_next_to_the_standalone_section_is_refused() {
    let harness = harness();
    let infobase = format!(
        "  connection: 'File={}'\n{}",
        harness.dir.path().join("ib").display(),
        standalone_infobase(&harness)
    );
    write_config(&harness, &infobase, "");

    let (code, payload) = run(&harness, &["dump", "--mode", "full"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        error_message(&payload).contains("Srvr=<host[:port]>;Ref=<name>"),
        "{payload}"
    );
    assert!(commands(&harness).is_empty(), "no gate session was opened");
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

/// Прямой шлюз автономного сервера раннер пока не использует (#205): тонкий клиент идёт
/// по клиентскому адресу без всякого ключа. Не объявлен адрес — отказ называет именно его, а не платформу: платформы
/// на этой машине нет вовсе, и до её поиска дело не доходит.
#[test]
fn a_thin_client_against_a_standalone_server_asks_for_the_web_address() {
    let harness = harness();

    let (code, payload) = run(&harness, &["launch", "thin", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        error_message(&payload).contains("infobase.web.url"),
        "{payload}"
    );
}

/// Второй путь открыли только тонкому клиенту. Остальные режимы против автономной цели
/// отказывают ровно как до его появления: прямой шлюз пока не используется (#205), а по
/// клиентскому адресу ходит только тонкий.
#[test]
fn a_non_thin_mode_against_a_standalone_server_is_still_refused() {
    let harness = harness();

    for mode in [
        vec!["launch", "designer", "--dry-run"],
        vec!["launch", "thick", "--dry-run"],
        vec!["launch", "ordinary", "--dry-run"],
        vec!["launch", "mcp", "--mode", "thick", "--dry-run"],
    ] {
        let (code, payload) = run(&harness, &mode);

        assert_ne!(code, 0, "{mode:?}: {payload}");
        assert_eq!(
            payload["error"]["kind"], "capability",
            "{mode:?}: {payload}"
        );
        assert!(
            error_message(&payload).contains("launch web"),
            "{mode:?}: {payload}"
        );
    }
}

/// Прямой шлюз автономной цели раннер пока не использует, поэтому просить его — ошибка
/// конфигурации, а не пустой запуск.
#[test]
fn via_connection_against_a_standalone_server_is_refused() {
    let harness = harness();

    let (code, payload) = run(
        &harness,
        &["launch", "thin", "--via", "connection", "--dry-run"],
    );

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    assert!(
        error_message(&payload).contains("not used by the runner yet"),
        "{payload}"
    );
}
