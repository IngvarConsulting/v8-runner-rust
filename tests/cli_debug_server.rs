//! Команды `debug-server` со стороны командной строки. Сценарий с подделкой системы — в
//! модуле `src/use_cases/managed_stand/debug_server.rs`; здесь то, что видно снаружи: замок,
//! разбор ключей и конверт.

mod support;

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{hold_workspace_lock, temp_workspace, v8_runner_command};

const SESSION: &str = "0123456789abcdef0123456789abcdef";

fn project(dir: &Path) {
    fs::write(
        dir.join("v8project.yaml"),
        "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=ib'\n",
    )
    .expect("project");
}

fn run(dir: &Path, arguments: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .current_dir(dir)
        .env_remove("V8TR_CONFIG")
        .arg("--json-message")
        .args(arguments)
        .output()
        .expect("run");
    let code = output.status.code().expect("code");
    let text = String::from_utf8_lossy(&output.stdout);
    (
        code,
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("not an envelope: {text}")),
    )
}

fn project_with_external_debugger(dir: &Path, url: &str) {
    fs::write(
        dir.join("v8project.yaml"),
        "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .expect("project");
    fs::write(
        dir.join("v8project.local.yaml"),
        format!(
            "infobases:
  origin:
    connection: 'File=ib'
  external:
    connection: 'Srvr=127.0.0.1:1541;Ref=debug3367'
    debug:
      url: '{url}'
      infobase-alias: debug3367
"
        ),
    )
    .expect("local infobases");
}

#[test]
fn prepare_preview_reports_the_selected_route_without_creating_state() {
    let dir = temp_workspace();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("preview witness");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("witness address"));
    project_with_external_debugger(dir.path(), &url);
    let owner = std::process::id().to_string();

    for selected in ["origin", "external"] {
        let (code, answer) = run(
            dir.path(),
            &[
                "--infobase",
                selected,
                "debug-server",
                "prepare",
                "--owner-pid",
                &owner,
                "--dry-run",
            ],
        );
        assert_eq!(code, 0, "{selected}: {answer}");
        assert_eq!(answer["command"], "debug-server prepare");
        support::command_data::assert_data_matches_its_command_form(&answer, selected);
        let data = &answer["data"];
        assert_eq!(data["action"], "planned", "{selected}: {answer}");
        for field in ["url", "release", "pid", "session", "port"] {
            assert!(data[field].is_null(), "{selected}: {answer}");
        }
        assert!(!dir.path().join("work/debug-server").exists());
    }

    let pending = listener.accept();
    assert!(
        matches!(&pending, Err(error) if error.kind() == ErrorKind::WouldBlock),
        "preview must not contact the configured server: {pending:?}"
    );
}

#[test]
fn prepare_requires_owner_pid() {
    let dir = temp_workspace();
    project(dir.path());
    let output = v8_runner_command()
        .current_dir(dir.path())
        .env_remove("V8TR_CONFIG")
        .args(["debug-server", "prepare"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("required arguments"), "{stderr}");
    assert!(!dir.path().join("work/debug-server").exists());
}

#[test]
fn prepare_refuses_an_explicit_external_cluster_debugger_without_falling_back() {
    let dir = temp_workspace();
    hold_workspace_lock(&dir.path().join("work"));
    let owner = std::process::id().to_string();
    let prepare = |url: &str, expected: &str| {
        project_with_external_debugger(dir.path(), url);
        let (code, answer) = run(
            dir.path(),
            &[
                "--infobase",
                "external",
                "debug-server",
                "prepare",
                "--owner-pid",
                &owner,
            ],
        );
        assert_ne!(code, 0, "{answer}");
        assert_eq!(answer["command"], "debug-server prepare");
        assert_eq!(answer["ok"], false, "{answer}");
        assert_eq!(answer["error"]["code"], expected, "{answer}");
        for field in ["url", "release", "pid", "session", "port"] {
            assert!(answer["data"][field].is_null(), "{answer}");
        }
        assert!(!dir.path().join("work/debug-server").exists());
    };

    let unavailable = format!("http://127.0.0.1:{}", support::free_tcp_port());
    prepare(&unavailable, "debug_server_external_unavailable");

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("RDBG witness");
    listener.set_nonblocking(true).expect("nonblocking");
    let url = format!("http://{}", listener.local_addr().expect("witness address"));
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "no request to the RDBG witness");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept RDBG request: {error}"),
            }
        };
        stream.set_nonblocking(false).expect("blocking peer");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bounded request read");
        let mut request = BufReader::new(&stream).lines();
        assert_eq!(
            request.next().expect("request line").expect("read request"),
            "POST /e1crdbg/rdbg?cmd=getDbgTargets HTTP/1.1"
        );
        assert!(
            request.any(|line| line.expect("read header").is_empty()),
            "the request ends at the empty line"
        );
        let body = r##"{"#exception":"{http://v8.1c.ru/8.2/virtual-resource-system}Exception"}"##;
        write!(
            stream,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("RDBG answer");
    });
    prepare(&url, "debug_server_external_target_unverified");
    server.join().expect("RDBG witness");
}

/// Отладчик гасит сервер при отсоединении, даже если в это время идёт `push`: `stop` замка
/// `workPath` не ждёт (`INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK`).
#[test]
fn stop_answers_while_another_command_holds_the_work_path() {
    let dir = temp_workspace();
    project(dir.path());
    fs::create_dir_all(dir.path().join("work")).expect("work");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::write(
        dir.path().join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("set");
    hold_workspace_lock(&dir.path().join("work"));
    let (code, answer) = run(dir.path(), &["debug-server", "stop", SESSION]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["command"], "debug-server stop");
    assert_eq!(answer["data"]["action"], "nothing_to_stop");
    assert_eq!(answer["data"]["known_session"], false);
    assert_eq!(answer["data"]["session"], SESSION);
}

/// Проверка другого сеанса отвечает из текущего workPath, даже если он заблокирован.
#[test]
fn status_reports_an_unknown_session_without_waiting_for_the_work_path() {
    let dir = temp_workspace();
    project(dir.path());
    fs::create_dir_all(dir.path().join("work")).expect("work");
    hold_workspace_lock(&dir.path().join("work"));

    let (code, answer) = run(dir.path(), &["debug-server", "status", SESSION]);
    assert_eq!(code, 0, "{answer}");
    assert_eq!(answer["command"], "debug-server status");
    support::command_data::assert_data_matches_its_command_form(&answer, "unknown debug session");
    assert_eq!(answer["data"]["status"], "unknown");
    assert_eq!(answer["data"]["session"], SESSION);
    for field in ["reason", "pid", "url", "port", "infobase_alias"] {
        assert!(answer["data"].get(field).is_none(), "{answer}");
    }
    assert!(!dir.path().join("work/debug-server").exists());
}

/// Сеанс становится именем каталога: иное, чем 32 шестнадцатеричных знака, отвергает разбор.
#[test]
fn a_session_that_is_not_hex_is_refused_by_the_parser() {
    let dir = temp_workspace();
    project(dir.path());
    let output = v8_runner_command()
        .current_dir(dir.path())
        .env_remove("V8TR_CONFIG")
        .args(["debug-server", "stop", "../../outside"])
        .output()
        .expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value"), "{stderr}");
}

/// Платформа с поддельным `dbgs` из `tests/fixtures/fake-dbgs.rs` рядом с тонким клиентом;
/// возвращает каталог установки.
fn platform_with_a_fake_debug_server(dir: &Path) -> PathBuf {
    let install = dir.join("platform");
    let bin = install.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let exe = std::env::consts::EXE_SUFFIX;
    let client = bin.join(format!("1cv8c{exe}"));
    fs::write(&client, "").expect("client");
    #[cfg(unix)]
    support::make_executable(&client);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-dbgs.rs");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let built = Command::new(rustc)
        .args(["--edition", "2021", "-C", "opt-level=0", "-o"])
        .arg(bin.join(format!("dbgs{exe}")))
        .arg(source)
        .output()
        .expect("rustc");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    install
}

/// Отвечает ли по адресу обработчик `rdbg` отказом 400.
fn rdbg_status(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .write_all(
            b"POST /e1crdbg/rdbg?cmd=getDbgTargets HTTP/1.1
Host: 127.0.0.1
Content-Type: application/xml
Content-Length: 0
Connection: close

",
        )
        .ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    answer.lines().next().map(str::to_owned)
}

/// Приёмка 1 и 2 на настоящем запуске службы: раннер запускает `dbgs` рядом с тонким
/// клиентом, читает его адрес из файла `--notify`, ждёт ответа `rdbg`, и сервер переживает
/// команду; `stop` его гасит, повторный `stop` ничего не трогает.
#[test]
fn start_runs_the_debug_server_beside_the_client_and_stop_ends_it() {
    let dir = temp_workspace();
    let install = platform_with_a_fake_debug_server(dir.path());
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::write(
        dir.path().join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("set");
    fs::write(
        dir.path().join("v8project.yaml"),
        format!(
            "workPath: work
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
infobase:
  connection: 'File=ib'
tools:
  platform:
    path: '{}'
",
            install.display()
        ),
    )
    .expect("project");
    let (code, started) = run(dir.path(), &["debug-server", "start"]);
    // `cargo test` на Windows держит тесты в Job, из которого выйти нельзя: сервер умер бы с
    // командой, и раннер отказывает, ничего не оставив. Полный путь идёт на Linux и macOS.
    if cfg!(windows)
        && started["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Job"))
    {
        assert_eq!(
            started["error"]["code"], "environment_unavailable",
            "{started}"
        );
        let sessions = dir.path().join("work").join("debug-server");
        let left = fs::read_dir(&sessions)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(left, 0, "a refused start leaves no session");
        return;
    }
    assert_eq!(code, 0, "{started}");
    let data = &started["data"];
    assert_eq!(data["action"], "started", "{started}");
    assert_eq!(data["owner"], "runner");
    let port = u16::try_from(data["port"].as_u64().expect("port")).expect("u16");
    assert!((10_000..20_000).contains(&port), "{port}");
    assert_eq!(data["url"], format!("http://127.0.0.1:{port}"));
    assert!(
        data["platform_resolution"]["path"]
            .as_str()
            .is_some_and(|path| path.contains("dbgs")),
        "{started}"
    );
    // Команда вышла, сервер работает.
    let status = rdbg_status(port).expect("the server outlived the command");
    assert!(status.contains("400"), "{status}");
    let session = data["session"].as_str().expect("session").to_owned();
    #[cfg(target_os = "linux")]
    let checks = [
        run(dir.path(), &["debug-server", "status", &session]),
        run(dir.path(), &["debug-server", "status", &session]),
    ];

    let (code, stopped) = run(dir.path(), &["debug-server", "stop", &session]);
    assert_eq!(code, 0, "{stopped}");
    assert_eq!(stopped["data"]["action"], "stopped", "{stopped}");
    assert_eq!(stopped["data"]["stopped_pid"], data["pid"]);
    assert!(rdbg_status(port).is_none(), "the server is stopped");

    let (code, again) = run(dir.path(), &["debug-server", "stop", &session]);
    assert_eq!(code, 0, "{again}");
    assert_eq!(again["data"]["action"], "nothing_to_stop");
    assert_eq!(again["data"]["known_session"], false);

    #[cfg(target_os = "linux")]
    for (code, checked) in checks {
        assert_eq!(code, 0, "{checked}");
        assert_eq!(checked["data"]["status"], "ready", "{checked}");
        for field in ["pid", "port", "url"] {
            assert_eq!(checked["data"][field], data[field], "{checked}");
        }
    }
}
