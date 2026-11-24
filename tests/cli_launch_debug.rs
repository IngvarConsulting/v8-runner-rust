//! `launch --debug <url>`: клиент предприятия запускается подключённым к серверу отладки.
//! Ключи клиента и отказы видны в превью; живой запуск — в описании PR #758.

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command};

const URL: &str = "http://127.0.0.1:12961";

/// Проект с файловой базой и поддельным тонким клиентом и Конфигуратором.
fn project(dir: &Path) {
    project_with(dir, "");
}

/// То же, с дополнительными строками секции `tools`.
fn project_with(dir: &Path, tools: &str) {
    let bin = dir.join("platform").join("bin");
    fs::create_dir_all(&bin).expect("bin");
    for name in ["1cv8c", "1cv8"] {
        let client = bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        fs::write(&client, "").expect("client");
        #[cfg(unix)]
        support::make_executable(&client);
    }
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::write(dir.join("src/Configuration.xml"), "<MetaDataObject/>").expect("set");
    fs::write(
        dir.join("v8project.yaml"),
        format!(
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=ib'\n  web:\n    url: 'http://localhost/demo'\ntools:\n  platform:\n    path: '{}'\n{tools}",
            dir.join("platform").display()
        ),
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

#[test]
fn a_web_debug_preview_preserves_the_publication_url_and_replaces_old_debug_keys() {
    let dir = temp_workspace();
    project(dir.path());
    let config = dir.path().join("v8project.yaml");
    let base = "http://localhost/demo/sub/?lang=ru&flag&dEbUg=tcp#e1cib/list/Test";
    let yaml = fs::read_to_string(&config).unwrap();
    fs::write(&config, yaml.replace("http://localhost/demo", base)).unwrap();

    let (code, answer) = run(
        dir.path(),
        &["launch", "web", "--web-debug", URL, "--dry-run"],
    );
    assert_eq!(code, 0, "{answer}");
    let data = &answer["data"];
    assert_eq!(data["debug"]["url"], URL);
    assert_eq!(data["provider_dispatched"], false);
    let launch_url = url::Url::parse(data["url"].as_str().unwrap()).unwrap();
    assert_eq!(launch_url.path(), "/demo/sub/");
    assert_eq!(launch_url.fragment(), Some("e1cib/list/Test"));
    assert!(launch_url.query().unwrap().starts_with("lang=ru&flag&"));
    let params: Vec<_> = launch_url.query_pairs().collect();
    assert_eq!(params.len(), 4);
    assert_eq!(params[2], ("Debug".into(), "http,attach".into()));
    assert_eq!(params[3], ("DebuggerURL".into(), URL.into()));

    let (code, plain) = run(dir.path(), &["launch", "web", "--dry-run"]);
    assert_eq!(code, 0, "{plain}");
    assert_eq!(plain["data"]["url"], base);
    assert!(plain["data"].get("debug").is_none());
}

/// Приёмка 1: в плане клиента ключи HTTP-отладки с адресом, в ответе — тот же адрес; клиент
/// не запускается.
#[test]
fn a_debug_launch_preview_attaches_the_client_to_the_debug_server() {
    let dir = temp_workspace();
    project(dir.path());
    for mode in ["thin", "thick", "ordinary", "mcp"] {
        let (code, answer) = run(dir.path(), &["launch", mode, "--debug", URL, "--dry-run"]);
        assert_eq!(code, 0, "{answer}");
        let data = &answer["data"];
        assert_eq!(data["debug"]["url"], URL, "{answer}");
        assert_eq!(data["provider_dispatched"], false);
        let args: Vec<&str> = data["plan"]["args"]
            .as_array()
            .expect("plan args")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let at = args
            .iter()
            .position(|arg| *arg == "/DEBUG")
            .expect("/DEBUG");
        assert_eq!(
            args[at..at + 5],
            ["/DEBUG", "-http", "-attach", "/DEBUGGERURL", URL],
            "{mode}"
        );
    }
    // Без ключа отладки в плане нет.
    let (_, plain) = run(dir.path(), &["launch", "thin", "--dry-run"]);
    assert!(plain["data"].get("debug").is_none(), "{plain}");
    assert!(!plain.to_string().contains("/DEBUGGERURL"));
}

/// Приёмка 2: Конфигуратор и веб-клиент с `--debug`, адрес не вида `http://<хост>:<порт>`
/// и ключи отладки рядом с `--debug` — отказ `invalid_argument` без запуска; сообщение
/// называет свою причину.
#[test]
fn a_debug_launch_is_refused_where_it_cannot_work() {
    let dir = temp_workspace();
    project(dir.path());
    let cases: [(&[&str], &str); 11] = [
        (&["launch", "designer", "--debug", URL], "Designer"),
        (&["launch", "web", "--debug", URL], "web client"),
        (
            &["launch", "thin", "--debug", "127.0.0.1:12961"],
            "http://<host>:<port>",
        ),
        (
            &["launch", "thin", "--debug", "https://127.0.0.1:12961"],
            "http://<host>:<port>",
        ),
        (
            &["launch", "thin", "--debug", "http://127.0.0.1"],
            "http://<host>:<port>",
        ),
        (
            &[
                "launch",
                "thin",
                "--debug",
                "http://127.0.0.1:12961/e1crdbg",
            ],
            "http://<host>:<port>",
        ),
        (
            &["launch", "thin", "--debug", "http://127.0.0.1:12961?x"],
            "http://<host>:<port>",
        ),
        (
            &["launch", "thin", "--debug", "http://127.0.0.1:12961#x"],
            "http://<host>:<port>",
        ),
        (
            &[
                "launch",
                "thin",
                "--debug",
                "http://:secret@127.0.0.1:12961",
            ],
            "http://<host>:<port>",
        ),
        (
            &[
                "launch",
                "thin",
                "--debug",
                URL,
                "--raw-key",
                "/DEBUGGERURL",
            ],
            "--raw-key",
        ),
        (
            &["launch", "thin", "--debug", URL, "--raw-key", "/DEBUG"],
            "--raw-key",
        ),
    ];
    for (arguments, reason) in cases {
        let (code, answer) = run(dir.path(), arguments);
        assert_ne!(code, 0, "{arguments:?}: {answer}");
        assert_eq!(
            answer["error"]["code"], "invalid_argument",
            "{arguments:?}: {answer}"
        );
        let message = answer["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains(reason), "{arguments:?}: {message}");
        assert!(!message.contains("secret"), "{message}");
    }
}

/// Ключи отладки в `tools.enterprise.additional_launch_keys` рядом с `--debug` дали бы клиенту
/// второй сервер отладки — отказ.
#[test]
fn a_debugger_in_the_configured_launch_keys_is_refused_next_to_debug() {
    let dir = temp_workspace();
    project_with(
        dir.path(),
        "  enterprise:
    additional-launch-keys: ['/DEBUGGERURL http://127.0.0.1:1550']
",
    );
    let (code, answer) = run(dir.path(), &["launch", "thin", "--debug", URL, "--dry-run"]);
    assert_ne!(code, 0, "{answer}");
    assert_eq!(answer["error"]["code"], "invalid_argument", "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("additional-launch-keys")),
        "{answer}"
    );
}

#[test]
fn external_web_debug_checks_the_descriptor_and_rdbg_without_republishing() {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;

    let dir = support::temp_workspace();
    project(dir.path());
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let publisher_url = format!("http://{}", listener.local_addr().unwrap());
    let ide_url = "http://localhost:1561";
    let descriptor = format!(
        r#"<point xmlns="http://v8.1c.ru/8.2/virtual-resource-system"><debug enable="true" protocol="http" url="{publisher_url}"/></point>"#
    );
    let publication = dir.path().join("external");
    std::fs::create_dir(&publication).unwrap();
    let vrd = publication.join("default.vrd");
    std::fs::write(&vrd, &descriptor).unwrap();
    let config = dir.path().join("v8project.yaml");
    let yaml = fs::read_to_string(&config).unwrap();
    fs::write(&config, yaml.replace(
        "    url: 'http://localhost/demo'",
        &format!("    url: 'http://localhost:8080/demo/?lang=ru'\n    dir: external\n    debugger-url: {publisher_url}"),
    )).unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = BufReader::new(&stream).lines().next().unwrap().unwrap();
        assert_eq!(request, "POST /e1crdbg/rdbg?cmd=getDbgTargets HTTP/1.1");
        let body = r##"{"#exception":"{http://v8.1c.ru/8.2/virtual-resource-system}Exception"}"##;
        write!(
            stream,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let run = || {
        support::v8_runner_command()
            .current_dir(dir.path())
            .env_remove("V8TR_CONFIG")
            .args([
                "--json-message",
                "web",
                "publish",
                "--prepare-debug",
                ide_url,
            ])
            .output()
            .unwrap()
    };
    let accepted = run();
    let answer: serde_json::Value = serde_json::from_slice(&accepted.stdout).unwrap();
    assert!(accepted.status.success(), "{answer}");
    assert_eq!(answer["data"]["debug"]["state"], "external_route_verified");
    assert_eq!(answer["data"]["debug"]["ide_url"], ide_url);
    assert_eq!(answer["data"]["debug"]["debugger_url"], publisher_url);
    assert_eq!(answer["data"]["provider_dispatched"], false);
    assert_eq!(std::fs::read_to_string(&vrd).unwrap(), descriptor);
    server.join().unwrap();

    let rejected = run();
    let answer: serde_json::Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(!rejected.status.success(), "{answer}");
    assert_eq!(answer["data"]["debug"]["state"], "route_unavailable");
    assert_eq!(answer["error"]["code"], "environment_unavailable");
    assert_eq!(std::fs::read_to_string(vrd).unwrap(), descriptor);

    let (code, rejected) = run_invalid_prepare(dir.path());
    assert_eq!(code, 2, "{rejected}");
    assert_eq!(rejected["error"]["code"], "invalid_argument");
}

#[test]
fn prepare_debug_flag_is_only_available_under_web_publish() {
    let output = v8_runner_command()
        .args(["publish", "--prepare-debug", "http://localhost:1561"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected argument"), "{stderr}");
}

fn run_invalid_prepare(dir: &Path) -> (i32, Value) {
    run(
        dir,
        &[
            "web",
            "publish",
            "--prepare-debug",
            "https://localhost:1561",
        ],
    )
}
