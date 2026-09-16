//! `publish` и `publish --delete`: публикация базы на веб-сервере через `webinst`.
//!
//! Параметры берутся из `infobase.web`, а не из флагов; развилки у операции нет;
//! у команды есть превью, потому что публикация замещает `default.vrd` целиком.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

struct Project {
    config: PathBuf,
    calls: PathBuf,
    www: PathBuf,
}

fn write_project(dir: &Path, web_yaml: &str) -> Project {
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    let install_dir = dir.join("platform");
    let www = dir.join("www");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::write(
        base_path.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&work_path).expect("work dir");
    fs::create_dir_all(&www).expect("www dir");
    let calls = dir.join("calls.log");
    write_shell_script(&install_dir.join("bin").join("1cv8"), "exit 0");
    write_shell_script(
        &install_dir.join("bin").join("webinst"),
        &format!("printf '%s\\n' \"$@\" >> '{}'\nexit 0", calls.display()),
    );

    let config = dir.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\n{web_yaml}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n",
            work_path.display(),
            dir.join("ib").display(),
            install_dir.display()
        ),
    )
    .expect("write config");
    Project { config, calls, www }
}

fn web_yaml(www: &Path, extra: &str) -> String {
    format!(
        "  web:\n    server: apache24\n    wsdir: demo\n    dir: '{}'\n    url: http://localhost/demo\n{extra}",
        www.display()
    )
}

fn run(config: &Path, arguments: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .args(["--config", &config.display().to_string(), "--json-message"])
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

/// Публикация берёт всё из `infobase.web` и составляет `webinst` по грамматике платформы.
#[test]
fn publish_composes_webinst_from_the_declared_web_section() {
    let dir = temp_workspace();
    let www = dir.path().join("www");
    let project = write_project(dir.path(), &web_yaml(&www, ""));

    let (code, payload) = run(&project.config, &["publish"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["command"], "publish");
    assert_eq!(payload["data"]["ok"], true);
    assert_eq!(payload["data"]["action"], "publish");
    assert_eq!(payload["data"]["server"], "apache24");
    assert_eq!(payload["data"]["wsdir"], "demo");
    assert_eq!(payload["data"]["url"], "http://localhost/demo");
    assert_eq!(payload["data"]["provider_dispatched"], true);
    let argv = fs::read_to_string(&project.calls).expect("webinst was called");
    let argv: Vec<&str> = argv.lines().collect();
    assert_eq!(argv[0], "-publish");
    assert_eq!(argv[1], "-apache24");
    assert_eq!(argv[2], "-wsdir");
    assert_eq!(argv[3], "demo");
    assert_eq!(argv[4], "-dir");
    assert_eq!(argv[5], project.www.display().to_string());
    assert_eq!(argv[6], "-connstr");
    assert!(argv[7].starts_with("File="), "{argv:?}");
}

/// Удаление — отдельный явный ключ, и оно не несёт строки соединения.
#[test]
fn publish_delete_removes_the_publication_without_a_connection_string() {
    let dir = temp_workspace();
    let www = dir.path().join("www");
    let project = write_project(dir.path(), &web_yaml(&www, ""));

    let (code, payload) = run(&project.config, &["publish", "--delete"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["action"], "delete");
    let argv = fs::read_to_string(&project.calls).expect("webinst was called");
    assert!(argv.starts_with("-delete\n"), "{argv}");
    assert!(!argv.contains("-connstr"), "{argv}");
}

/// Превью составляет команду и находит утилиту, но веб-сервер не трогает.
#[test]
fn publish_dry_run_names_the_command_and_touches_nothing() {
    let dir = temp_workspace();
    let www = dir.path().join("www");
    let project = write_project(dir.path(), &web_yaml(&www, ""));

    let (code, payload) = run(&project.config, &["publish", "--dry-run"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], false);
    assert_eq!(payload["data"]["plan"]["args"][0], "-publish");
    assert!(payload["data"]["plan"]["program"]
        .as_str()
        .is_some_and(|program| program.ends_with("webinst")));
    assert!(!project.calls.exists(), "a preview must not run webinst");
}

/// `-connstr` несёт строку соединения целиком, а в ней бывает `Pwd=`. Превью
/// публикации печатало её как есть, хотя превью запуска пароль уже закрывало.
/// Остальные параметры публикации остаются читаемыми: по ним и одобряют план.
#[test]
fn publish_preview_never_echoes_the_password_of_the_connection_string() {
    let dir = temp_workspace();
    let www = dir.path().join("www");
    let project = write_project(dir.path(), &web_yaml(&www, ""));
    let config = fs::read_to_string(&project.config).expect("config");
    fs::write(
        &project.config,
        config.replace(
            &format!("connection: 'File={}'", dir.path().join("ib").display()),
            "connection: 'Srvr=srv:1541;Ref=ut;Usr=Admin;Pwd=s3cret'",
        ),
    )
    .expect("config with a password in the connection string");

    let (code, payload) = run(&project.config, &["publish", "--dry-run"]);

    assert_eq!(code, 0, "{payload}");
    let args = payload["data"]["plan"]["args"].to_string();
    assert!(!args.contains("s3cret"), "{args}");
    assert!(
        args.contains("Srvr=srv:1541;Ref=ut;Usr=Admin;Pwd=***"),
        "{args}"
    );
    assert!(!project.calls.exists(), "a preview must not run webinst");
}

/// Предусловия утилиты называются до запуска, а не после её отказа.
#[test]
fn publish_refuses_before_webinst_when_a_precondition_is_missing() {
    let cases: [(&str, &str); 3] = [
        (
            "  web:\n    server: apache22\n    wsdir: demo\n    dir: '{www}'\n",
            "infobase.web.conf is required to publish on apache22",
        ),
        (
            "  web:\n    server: apache24\n    wsdir: demo\n    dir: '{www}'\n    os-auth: true\n",
            "os-auth is supported only for iis",
        ),
        (
            "  web:\n    server: apache24\n    wsdir: demo\n    dir: '{www}/missing'\n",
            "does not exist or is not a directory",
        ),
    ];
    for (template, expected) in cases {
        let dir = temp_workspace();
        let www = dir.path().join("www");
        let yaml = template.replace("{www}", &www.display().to_string());
        let project = write_project(dir.path(), &yaml);
        let (code, payload) = run(&project.config, &["publish", "--dry-run"]);
        assert_ne!(code, 0, "{template}: {payload}");
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(message.contains(expected), "{template}: {message}");
        assert!(!project.calls.exists(), "{template}: webinst must not run");
    }
}

/// У публикации нет выбора исполнителя: ключ отклоняется как ключ без развилки.
#[test]
fn publish_rejects_a_provider_override_because_there_is_no_choice() {
    let dir = temp_workspace();
    let www = dir.path().join("www");
    let project = write_project(
        dir.path(),
        &format!("{}providers:\n  publish: webinst\n", web_yaml(&www, "")),
    );

    let (code, payload) = run(&project.config, &["publish", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("providers.publish is not allowed"),
        "{message}"
    );
}

/// Без секции `infobase.web` публиковать нечего, и отказ говорит, что объявить.
#[test]
fn publish_without_a_web_section_is_refused() {
    let dir = temp_workspace();
    let project = write_project(dir.path(), "");

    let (code, payload) = run(&project.config, &["publish", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation");
    assert!(payload["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("infobase.web")));
}

/// `ws=…` — адрес клиента, а не канал администрирования: отказ называет замену.
#[test]
fn a_web_connection_string_is_refused_as_an_administrative_channel() {
    let dir = temp_workspace();
    let project = write_project(dir.path(), "");
    let yaml = fs::read_to_string(&project.config).expect("config");
    let yaml = yaml.replace(
        &format!("connection: 'File={}'", dir.path().join("ib").display()),
        "connection: 'ws=http://localhost/demo'",
    );
    fs::write(&project.config, yaml).expect("config");

    let (code, payload) = run(&project.config, &["build", "--dry-run"]);

    assert_ne!(code, 0, "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobase.web.url"), "{message}");
    assert!(
        message.contains("File=") && message.contains("Srvr="),
        "{message}"
    );
}
