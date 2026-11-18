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
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=ib'\n  web:\n    url: 'http://localhost/demo'\ntools:\n  platform:\n    path: '{}'\n",
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

/// Приёмка 1: в плане клиента ключи HTTP-отладки с адресом, в ответе — тот же адрес; клиент
/// не запускается.
#[test]
fn a_debug_launch_preview_attaches_the_client_to_the_debug_server() {
    let dir = temp_workspace();
    project(dir.path());
    for mode in ["thin", "thick", "ordinary"] {
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

/// Приёмка 2: Конфигуратор и веб-клиент с `--debug`, неверный адрес и ключи отладки в
/// `--raw-key` — отказ `invalid_argument` без запуска.
#[test]
fn a_debug_launch_is_refused_where_it_cannot_work() {
    let dir = temp_workspace();
    project(dir.path());
    let cases: [&[&str]; 6] = [
        &["launch", "designer", "--debug", URL],
        &["launch", "web", "--debug", URL],
        &["launch", "thin", "--debug", "127.0.0.1:12961"],
        &["launch", "thin", "--debug", "http://127.0.0.1"],
        &[
            "launch",
            "thin",
            "--debug",
            "http://127.0.0.1:12961/e1crdbg",
        ],
        &[
            "launch",
            "thin",
            "--debug",
            URL,
            "--raw-key",
            "/DEBUGGERURL",
        ],
    ];
    for arguments in cases {
        let (code, answer) = run(dir.path(), arguments);
        assert_ne!(code, 0, "{arguments:?}: {answer}");
        assert_eq!(
            answer["error"]["code"], "invalid_argument",
            "{arguments:?}: {answer}"
        );
    }
}
