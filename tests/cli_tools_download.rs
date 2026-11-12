mod support;

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

/// Прежний глобальный `builder` в тестовых конфигах: `DESIGNER` — Конфигуратор первым,
/// `IBCMD` — `ibcmd` всюду, где у операции есть развилка.
fn providers_yaml(builder: &str) -> &'static str {
    match builder {
        "IBCMD" => "providers:\n  init: ibcmd\n  build: ibcmd\n  dump: ibcmd\n  infobase.configuration.export: ibcmd\n",
        "AGENT" => "providers:\n  push: agent\n",
        "DEFAULT" => "",
        _ => support::DESIGNER_LEADS,
    }
}

fn write_minimal_config(root: &Path) -> PathBuf {
    write_minimal_config_with_builder(root, "DESIGNER")
}

fn write_minimal_config_with_builder(root: &Path, builder: &str) -> PathBuf {
    let base_path = root.join("project");
    let work_path = root.join("work");
    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration");
    fs::create_dir_all(&work_path).expect("work");
    let config_path = root.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "# yaml-language-server: $schema=./docs/schemas/v8project.schema.json\nworkPath: '{}'\nformat: DESIGNER\n{}infobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  edt_cli:\n    path: /tmp/edt\n",
            work_path.display(),
            providers_yaml(builder),
        ),
    )
    .expect("config");
    config_path
}

fn write_config_with_pending_va(root: &Path) -> PathBuf {
    let config_path = write_minimal_config(root);
    let mut config = fs::read_to_string(&config_path).expect("config");
    config.push_str(
        "tests:\n  va:\n    params_path: missing/params.json\n    profile: smoke\n    profiles:\n      smoke:\n        feature_path: missing/features\n",
    );
    fs::write(&config_path, config).expect("pending va config");
    config_path
}

struct FixtureServer {
    address: std::net::SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FixtureServer {
    fn start(root: &Path) -> (Self, u16) {
        Self::start_with_mode(Some(root.to_path_buf()), Duration::ZERO)
    }

    fn start_with_mode(root: Option<PathBuf>, response_delay: Duration) -> (Self, u16) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fixture server");
        let address = listener.local_addr().expect("fixture server address");
        listener
            .set_nonblocking(true)
            .expect("fixture server nonblocking");
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        if !response_delay.is_zero() {
                            thread::sleep(response_delay);
                            if let Err(error) =
                                write_http_response(&mut stream, "200 OK", &[], b"{}")
                            {
                                eprintln!("sleeping fixture response failed: {error}");
                            }
                        } else if let Some(root) = root.as_deref() {
                            if let Err(error) = serve_fixture_request(&mut stream, root) {
                                eprintln!("fixture response failed: {error}");
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (
            Self {
                address,
                shutdown,
                thread: Some(thread),
            },
            address.port(),
        )
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_fixture_request(stream: &mut TcpStream, root: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > 16 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "fixture HTTP request headers exceed 16 KiB",
            ));
        }
    }
    if request.is_empty() {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&request);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    for (prefix, status) in [
        ("/redirect302/", "302 Found"),
        ("/redirect/", "301 Moved Permanently"),
    ] {
        if let Some(suffix) = path.strip_prefix(prefix) {
            let location = format!("/{suffix}");
            return write_http_response(stream, status, &[("Location", location.as_str())], &[]);
        }
    }
    let (relative, query) = path.split_once('?').unwrap_or((path, ""));
    let mut file_path = root.join(relative.trim_start_matches('/'));
    // Список выпусков (`.../releases?per_page=..&page=N`) лежит страницами в каталоге
    // `releases` рядом с `latest`: страница N — файл `page-N.json`.
    if file_path.is_dir() {
        let page = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("page="))
            .unwrap_or("1");
        file_path = file_path.join(format!("page-{page}.json"));
    }
    match fs::read(file_path) {
        Ok(body) => write_http_response(stream, "200 OK", &[], &body),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_http_response(stream, "404 Not Found", &[], b"not found")
        }
        Err(error) => Err(error),
    }
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n",
        body.len()
    )?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n")?;
    stream.write_all(body)
}

fn write_http_fixture(root: &Path, port: u16) {
    write_http_fixture_with_redirect_prefix(root, port, "");
}

fn write_http_fixture_with_redirects(root: &Path, port: u16, redirects: bool) {
    let prefix = if redirects { "/redirect" } else { "" };
    write_http_fixture_with_redirect_prefix(root, port, prefix);
}

fn write_http_fixture_with_redirect_prefix(root: &Path, port: u16, prefix: &str) {
    let api = root.join("repos");
    write_release(
        &api.join("bia-technologies")
            .join("yaxunit")
            .join("releases"),
        "25.12",
        &format!("http://127.0.0.1:{port}{prefix}/archives/yaxunit.zip"),
        &[(
            "YAxUnit-25.12.cfe",
            &format!("http://127.0.0.1:{port}{prefix}/assets/YAxUnit-25.12.cfe"),
        )],
    );
    write_release(
        &api.join("Pr-Mex")
            .join("vanessa-automation-single")
            .join("releases"),
        "1.2.043.1",
        &format!("http://127.0.0.1:{port}{prefix}/archives/vanessa-source.zip"),
        &[(
            "vanessa-automation-single.1.2.043.1.zip",
            &format!(
                "http://127.0.0.1:{port}{prefix}/assets/vanessa-automation-single.1.2.043.1.zip"
            ),
        )],
    );
    write_release(
        &api.join("1c-neurofish")
            .join("onec-client-mcp-devkit")
            .join("releases"),
        "v0.6.4",
        &format!("http://127.0.0.1:{port}{prefix}/archives/client-mcp.zip"),
        &[(
            "client_mcp.cfe",
            &format!("http://127.0.0.1:{port}{prefix}/assets/client_mcp.cfe"),
        )],
    );

    fs::create_dir_all(root.join("assets")).expect("assets");
    fs::write(root.join("assets").join("YAxUnit-25.12.cfe"), "yaxunit cfe").expect("yax asset");
    fs::write(root.join("assets").join("client_mcp.cfe"), "client cfe").expect("client asset");
    make_zip(
        &root
            .join("assets")
            .join("vanessa-automation-single.1.2.043.1.zip"),
        &[("vanessa-automation-single.epf", "va epf")],
    );

    fs::create_dir_all(root.join("archives")).expect("archives");
    make_zip(
        &root.join("archives").join("yaxunit.zip"),
        &[(
            "bia-technologies-yaxunit/exts/yaxunit/src/Configuration/Configuration.mdo",
            "yaxunit source",
        )],
    );
    make_zip(
        &root.join("archives").join("client-mcp.zip"),
        &[(
            "1c-neurofish-onec-client-mcp-devkit/exts/client-mcp/src/Configuration/Configuration.mdo",
            "client source",
        )],
    );
    make_zip(
        &root.join("archives").join("vanessa-source.zip"),
        &[("unused/readme.txt", "unused")],
    );
}

/// Пишет выпуск как `releases/latest` и как единственный выпуск списка `releases`.
fn write_release(path: &Path, tag: &str, zipball_url: &str, assets: &[(&str, &str)]) {
    fs::create_dir_all(path).expect("release dir");
    let release = release_json(tag, Channel::Stable, zipball_url, assets);
    fs::write(path.join("latest"), &release).expect("release json");
    write_release_page(path, 1, std::slice::from_ref(&release));
}

/// Как выпуск помечен в списке GitHub.
#[derive(Clone, Copy)]
enum Channel {
    Stable,
    PreRelease,
    Draft,
}

fn release_json(tag: &str, channel: Channel, zipball_url: &str, assets: &[(&str, &str)]) -> String {
    let prerelease = matches!(channel, Channel::PreRelease);
    let draft = matches!(channel, Channel::Draft);
    let assets_json = assets
        .iter()
        .map(|(name, url)| format!(r#"{{"name":"{name}","browser_download_url":"{url}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"tag_name":"{tag}","html_url":"https://example.invalid/{tag}","zipball_url":"{zipball_url}","prerelease":{prerelease},"draft":{draft},"assets":[{assets_json}]}}"#
    )
}

fn write_release_page(path: &Path, page: usize, releases: &[String]) {
    fs::create_dir_all(path).expect("release dir");
    fs::write(
        path.join(format!("page-{page}.json")),
        format!("[{}]", releases.join(",")),
    )
    .expect("release page json");
}

fn make_zip(path: &Path, entries: &[(&str, &str)]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("zip parent");
    }
    let mut zip = ZipWriter::new(fs::File::create(path).expect("create ZIP fixture"));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for &(name, value) in entries {
        zip.start_file(name, options)
            .expect("start ZIP fixture entry");
        zip.write_all(value.as_bytes())
            .expect("write ZIP fixture entry");
    }
    zip.finish().expect("finish ZIP fixture");
}

fn assert_vanessa_download_without_running_infobase(local_yaml: Option<&str>, extra_args: &[&str]) {
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    let local_path = dir.path().join("v8project.local.yaml");
    let project_yaml = "format: DESIGNER\nworkPath: work\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/configuration\n";
    fs::write(&config_path, project_yaml).expect("project config");
    if let Some(local_yaml) = local_yaml {
        fs::write(&local_path, local_yaml).expect("local config");
    }
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .arg("--config")
        .arg(&config_path)
        .args(extra_args)
        .args(["--json-message", "tools", "download", "vanessa"])
        .output()
        .expect("download Vanessa");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).expect("JSON response");
    assert_eq!(response["data"]["destinations"][0]["tag"], "1.2.043.1");
    let epf_path = Path::new("work")
        .join("tools")
        .join("vanessa-automation-single.epf");
    assert_eq!(
        fs::read(dir.path().join(&epf_path)).expect("downloaded EPF"),
        b"va epf"
    );
    let actual: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(local_path).expect("local config")).expect("local YAML");
    let mut expected: serde_yaml::Value =
        serde_yaml::from_str(local_yaml.unwrap_or("{}")).expect("initial local YAML");
    expected["tools"]["va"]["epf_path"] = serde_yaml::to_value(epf_path).expect("EPF YAML path");
    assert_eq!(actual, expected);
    assert_eq!(
        fs::read_to_string(config_path).expect("project config"),
        project_yaml
    );
    assert!(
        !dir.path().join(".local").exists(),
        "managed service state must be absent"
    );
}

#[test]
fn tools_download_vanessa_without_an_origin() {
    assert_vanessa_download_without_running_infobase(None, &[]);
}

#[test]
fn tools_download_vanessa_with_a_stopped_managed_cluster() {
    let local_yaml = "infobases:\n  origin:\n    cluster:\n      managed:\n        name: toolsbootstrap\n    dbms:\n      managed: {}\n      name: toolsbootstrap\n      locale: ru_RU\ntools:\n  edt_cli:\n    path: pending-edt\n";
    assert_vanessa_download_without_running_infobase(Some(local_yaml), &[]);
    assert_vanessa_download_without_running_infobase(Some(local_yaml), &["--infobase", "origin"]);
}

#[test]
fn tools_download_vanessa_validates_declared_and_explicit_infobases() {
    let dir = temp_workspace();
    let config_path = dir.path().join("v8project.yaml");
    fs::write(&config_path, "format: DESIGNER\n").expect("project config");
    let local_path = dir.path().join("v8project.local.yaml");
    let malformed = "infobases:\n  origin:\n    connection: 'ws=https://example.invalid/base'\n";
    let cases: &[(&str, &[&str], &str)] = &[
        (malformed, &[], "web-client address"),
        (
            "infobases:\n  other:\n    connection: 'ws=https://example.invalid/base'\n",
            &[],
            "web-client address",
        ),
        (
            "{}",
            &["--infobase", "missing"],
            "infobase 'missing' is not declared",
        ),
        (
            "{}",
            &["--infobase", "ws=https://example.invalid/base"],
            "web-client address",
        ),
    ];
    for &(local_yaml, args, expected_message) in cases {
        fs::write(&local_path, local_yaml).expect("local config");
        let output = v8_runner_command()
            .env("V8TR_GITHUB_API_BASE_URL", "http://127.0.0.1:0")
            .arg("--config")
            .arg(&config_path)
            .args(args)
            .args(["--json-message", "tools", "download", "vanessa"])
            .output()
            .expect("run Vanessa download command");
        assert_eq!(output.status.code(), Some(2));
        let response: Value = serde_json::from_slice(&output.stdout).expect("JSON response");
        assert_eq!(response["error"]["code"], "invalid_argument");
        assert!(
            response["error"]["message"]
                .as_str()
                .expect("error message")
                .contains(expected_message),
            "{response}"
        );
        assert_eq!(
            fs::read_to_string(&local_path).expect("local config"),
            local_yaml
        );
    }
}

#[test]
fn tools_download_sources_writes_source_set_and_local_tool_settings() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "tools",
            "download",
            "yaxunit",
            "--sources",
        ])
        .output()
        .expect("run command");
    let repeat = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
            "--sources",
        ])
        .output()
        .expect("run command again");
    let client_mcp = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "client-mcp",
            "--sources",
        ])
        .output()
        .expect("run client-mcp command");
    let vanessa = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "vanessa",
        ])
        .output()
        .expect("run vanessa command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        repeat.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        repeat.status.code(),
        String::from_utf8_lossy(&repeat.stdout),
        String::from_utf8_lossy(&repeat.stderr)
    );
    assert!(
        client_mcp.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        client_mcp.status.code(),
        String::from_utf8_lossy(&client_mcp.stdout),
        String::from_utf8_lossy(&client_mcp.stderr)
    );
    assert!(
        vanessa.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        vanessa.status.code(),
        String::from_utf8_lossy(&vanessa.stdout),
        String::from_utf8_lossy(&vanessa.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "tools download");
    assert_eq!(payload["data"]["tool"], "yaxunit");
    assert_eq!(payload["data"]["mode"], "sources");

    let config = fs::read_to_string(&config_path).expect("config");
    assert!(config.starts_with("# yaml-language-server:"));
    assert!(config.contains("name: tests"));
    assert!(config.contains("type: EXTENSION"));
    assert!(config.contains("path: tests"));
    assert!(dir
        .path()
        .join("tests/src/Configuration/Configuration.mdo")
        .exists());
    assert!(!dir
        .path()
        .join("tests/.v8-runner-tools-download.json")
        .exists());
    assert!(!dir
        .path()
        .join(".tests.v8-runner-tools-download.json")
        .exists());
    assert!(dir
        .path()
        .join(".local/tools/.tests.v8-runner-tools-download.json")
        .exists());
    assert!(dir.path().join(".local/.gitignore").is_file());
    assert!(
        !dir.path().join("build").exists(),
        "no build/ in the project"
    );
    assert!(dir
        .path()
        .join("work/tools/vanessa-automation-single.epf")
        .exists());
    assert!(dir
        .path()
        .join(
            "work/tools/onec-client-mcp-devkit/exts/client-mcp/src/Configuration/Configuration.mdo"
        )
        .exists());
    assert!(!dir
        .path()
        .join("work/tools/onec-client-mcp-devkit/exts/client-mcp/.v8-runner-tools-download.json")
        .exists());
    assert!(dir
        .path()
        .join("work/tools/onec-client-mcp-devkit/exts/.client-mcp.v8-runner-tools-download.json")
        .exists());

    let local = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local");
    assert!(local.contains("epf_path:"));
    assert!(local.contains("epf_path: work/tools/vanessa-automation-single.epf"));
    assert!(local.contains("client_mcp:"));
    assert!(local.contains("source:"));
    assert!(local.contains("path: work/tools/onec-client-mcp-devkit/exts/client-mcp"));
    assert!(local.contains("format: EDT"));
    assert!(!local.contains(&dir.path().display().to_string()));
}

#[test]
fn tools_download_sources_rejects_legacy_tests_markers_outside_build() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    fs::create_dir_all(dir.path().join("tests")).expect("tests");
    fs::write(
        dir.path().join(".tests.v8-runner-tools-download.json"),
        "{}\n",
    )
    .expect("legacy root marker");
    fs::write(
        dir.path().join("tests/.v8-runner-tools-download.json"),
        "{}\n",
    )
    .expect("legacy nested marker");

    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
            "--sources",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("download target already exists and is not managed by v8-runner"));
    assert!(!dir
        .path()
        .join(".local/tools/.tests.v8-runner-tools-download.json")
        .exists());
    assert!(!dir.path().join("build").exists());
}

/// Метка прежних версий в `build/` признаётся: уже скачанный `tests` повтор без `--force`
/// оставляет как есть и не отказывает, а `--force` пишет новую метку под `localPath`, не трогая
/// старую (`INV.USE-CASES.DOWNLOADED-TOOLS-LIVE-IN-THE-WORK-PATH`).
#[test]
fn tools_download_sources_recognizes_the_legacy_tests_marker_in_build() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let user_file = dir.path().join("tests/custom.feature");
    fs::create_dir_all(user_file.parent().expect("tests parent")).expect("tests dir");
    fs::write(&user_file, "downloaded earlier").expect("tests file");
    let legacy = dir
        .path()
        .join("build/.tests.v8-runner-tools-download.json");
    fs::create_dir_all(legacy.parent().expect("build")).expect("build");
    fs::write(
        &legacy, "{}
",
    )
    .expect("legacy marker");

    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);
    let run = |force: bool| {
        let mut command = v8_runner_command();
        command
            .env(
                "V8TR_GITHUB_API_BASE_URL",
                format!("http://127.0.0.1:{port}"),
            )
            .args([
                "--config",
                &config_path.display().to_string(),
                "tools",
                "download",
                "yaxunit",
                "--sources",
            ]);
        if force {
            command.arg("--force");
        }
        command.output().expect("run command")
    };

    let repeat = run(false);
    assert!(
        repeat.status.success(),
        "stdout={}
stderr={}",
        String::from_utf8_lossy(&repeat.stdout),
        String::from_utf8_lossy(&repeat.stderr)
    );
    assert_eq!(
        fs::read_to_string(&user_file).expect("kept"),
        "downloaded earlier"
    );

    let forced = run(true);
    assert!(
        forced.status.success(),
        "stdout={}
stderr={}",
        String::from_utf8_lossy(&forced.stdout),
        String::from_utf8_lossy(&forced.stderr)
    );
    assert!(dir
        .path()
        .join(".local/tools/.tests.v8-runner-tools-download.json")
        .is_file());
    assert!(legacy.is_file(), "the legacy marker is left alone");
}

#[test]
fn tools_download_repairs_pending_vanessa_configuration() {
    let dir = temp_workspace();
    let config_path = write_config_with_pending_va(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "vanessa",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let local = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local");
    assert!(local.contains("epf_path:"));
    assert!(local.contains("epf_path: work/tools/vanessa-automation-single.epf"));
    assert!(!local.contains(&dir.path().display().to_string()));
}

/// Выпуски Vanessa для выбора (#160): флаг latest (файл `latest`) стоит на 1.2.043.1; в
/// списке есть большие обычные выпуски, pre-release и черновик. Первая страница полная,
/// поэтому вторая тоже читается; на ней — наибольший pre-release 1.2.045.7, обычный
/// 1.2.043.42 (числом больше 1.2.043.9) и черновик 1.2.046.1.
fn write_vanessa_release_list(server_root: &Path, port: u16) {
    let releases_dir = server_root
        .join("repos")
        .join("Pr-Mex")
        .join("vanessa-automation-single")
        .join("releases");
    let vanessa_release = |tag: &str, channel: Channel| {
        let name = format!("vanessa-automation-single.{tag}.zip");
        let url = format!("http://127.0.0.1:{port}/assets/{name}");
        make_zip(
            &server_root.join("assets").join(&name),
            &[("vanessa-automation-single.epf", &format!("va epf {tag}"))],
        );
        release_json(
            tag,
            channel,
            &format!("http://127.0.0.1:{port}/archives/vanessa-source.zip"),
            &[(name.as_str(), url.as_str())],
        )
    };
    let mut first_page = vec![
        vanessa_release("1.2.043.1", Channel::Stable),
        vanessa_release("1.2.043.9", Channel::Stable),
        vanessa_release("1.2.044.1", Channel::PreRelease),
    ];
    first_page.extend((0..97).map(|minor| {
        release_json(
            &format!("1.1.{minor}"),
            Channel::Stable,
            "http://127.0.0.1:1/unused.zip",
            &[],
        )
    }));
    write_release_page(&releases_dir, 1, &first_page);
    write_release_page(
        &releases_dir,
        2,
        &[
            vanessa_release("1.2.046.1", Channel::Draft),
            vanessa_release("1.2.045.7", Channel::PreRelease),
            vanessa_release("1.2.043.42", Channel::Stable),
        ],
    );
}

/// Скачивает Vanessa с поддельного API и возвращает выбранный тег и содержимое EPF.
fn download_vanessa_from_release_list(extra_args: &[&str]) -> (String, String) {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);
    write_vanessa_release_list(&server_root, port);

    let config = config_path.display().to_string();
    let mut args = vec![
        "--config",
        config.as_str(),
        "--json-message",
        "tools",
        "download",
        "vanessa",
    ];
    args.extend_from_slice(extra_args);
    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args(&args)
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    let tag = payload["data"]["destinations"][0]["tag"]
        .as_str()
        .unwrap_or_else(|| panic!("destination tag in {payload}"))
        .to_owned();
    let epf = fs::read_to_string(dir.path().join("work/tools/vanessa-automation-single.epf"))
        .expect("downloaded epf");
    (tag, epf)
}

/// Без ключа берётся выпуск `releases/latest`, хотя в списке есть и большие обычные
/// выпуски, и pre-release (#160).
#[test]
fn tools_download_vanessa_takes_the_latest_release_without_prerelease_flag() {
    let (tag, epf) = download_vanessa_from_release_list(&[]);
    assert_eq!(tag, "1.2.043.1");
    assert_eq!(epf, "va epf 1.2.043.1");
}

/// С `--prerelease` берётся наибольшая версия с учётом pre-release, со всех страниц
/// списка; черновик пропускается (#160).
#[test]
fn tools_download_vanessa_prerelease_takes_the_highest_version_including_prerelease() {
    let (tag, epf) = download_vanessa_from_release_list(&["--prerelease"]);
    assert_eq!(tag, "1.2.045.7");
    assert_eq!(epf, "va epf 1.2.045.7");
}

/// `--prerelease` есть только у Vanessa: у другого инструмента ключ отвергается разбором
/// аргументов, и ничего не скачивается.
#[test]
fn tools_download_rejects_prerelease_for_other_tools() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let output = v8_runner_command()
        .env("V8TR_GITHUB_API_BASE_URL", "http://127.0.0.1:1")
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "tools",
            "download",
            "yaxunit",
            "--prerelease",
        ])
        .output()
        .expect("run command");

    // Ключ объявлен только у `vanessa`, поэтому отказ даёт разбор аргументов.
    assert_eq!(
        output.status.code(),
        Some(2),
        "yaxunit must reject --prerelease"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected argument"), "{stderr}");
    assert!(!dir.path().join("work/tools").exists());
}

#[test]
fn tools_download_follows_latest_release_and_asset_redirects() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture_with_redirects(&server_root, port, true);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}/redirect"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "client-mcp",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.path().join("work/tools/client_mcp.cfe").exists());
    let local = fs::read_to_string(dir.path().join("v8project.local.yaml")).expect("local");
    assert!(local.contains("artifact:"));
    assert!(local.contains("client_mcp.cfe"));
}

/// Живая сверка формы `tools download`: загрузка идёт с поддельного сервера в этом же
/// процессе, поэтому сеть для неё не нужна.
#[test]
fn tools_download_answers_in_the_form_declared_for_it() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "tools",
            "download",
            "client-mcp",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json envelope");
    assert_eq!(payload["command"], "tools download", "{payload}");
    assert_data_matches_its_command_form(&payload, "`tools download client-mcp`");
}

#[test]
fn tools_download_follows_302_redirects() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture_with_redirect_prefix(&server_root, port, "/redirect302");

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}/redirect302"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "client-mcp",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.path().join("work/tools/client_mcp.cfe").exists());
}

#[test]
fn tools_download_artifacts_keeps_yaxunit_out_of_source_sets() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config = fs::read_to_string(&config_path).expect("config");
    assert!(!config.contains("name: tests"));
    assert!(dir.path().join("work/tools/YAxUnit-25.12.cfe").exists());
    assert!(!dir.path().join("v8project.local.yaml").exists());
}

#[test]
fn tools_download_artifacts_handles_large_assets_without_pipe_deadlock() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);
    fs::write(
        server_root.join("assets").join("YAxUnit-25.12.cfe"),
        vec![b'x'; 8 * 1024 * 1024],
    )
    .expect("large yax asset");

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::metadata(dir.path().join("work/tools/YAxUnit-25.12.cfe"))
            .expect("large asset")
            .len(),
        8 * 1024 * 1024
    );
}

#[test]
fn tools_download_sources_refuses_to_replace_unmanaged_tests_dir() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let user_file = dir.path().join("tests/custom.feature");
    fs::create_dir_all(user_file.parent().expect("tests parent")).expect("tests dir");
    fs::write(&user_file, "user content").expect("user test");

    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
            "--sources",
            "--force",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&user_file).expect("user file"),
        "user content"
    );
}

#[test]
fn tools_download_artifacts_requires_designer_builder() {
    let dir = temp_workspace();
    let config_path = write_minimal_config_with_builder(dir.path(), "IBCMD");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "client-mcp",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("needs the Designer as the push provider"));
}

#[test]
fn tools_download_force_refuses_to_replace_unmanaged_tool_file() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let user_file = dir.path().join("work/tools/vanessa-automation-single.epf");
    fs::create_dir_all(user_file.parent().expect("tools parent")).expect("tools dir");
    fs::write(&user_file, "user epf").expect("user epf");

    let server_root = dir.path().join("server");
    let (_server, port) = FixtureServer::start(&server_root);
    write_http_fixture(&server_root, port);

    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "vanessa",
            "--force",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&user_file).expect("user file"),
        "user epf"
    );
}

/// DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE, at the command boundary.
///
/// Раньше `execution_timeout: 200` обрывал эту загрузку на 200-й миллисекунде. Теперь
/// медленное зеркало дожидаются: обрывает загрузку только тишина в сокете, а отвечающий
/// с задержкой сервер тишиной не является. Провал приходит по содержимому ответа, а не
/// по часам.
#[test]
fn a_slow_mirror_is_waited_for_instead_of_being_cut_off_by_a_command_budget() {
    const RESPONSE_DELAY: Duration = Duration::from_secs(2);

    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let (_server, port) = FixtureServer::start_with_mode(None, RESPONSE_DELAY);

    let started = std::time::Instant::now();
    let output = v8_runner_command()
        .env(
            "V8TR_GITHUB_API_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
        ])
        .output()
        .expect("run command");
    let elapsed = started.elapsed();

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed >= RESPONSE_DELAY,
        "the slow response must be waited for, not cut short; elapsed={elapsed:?}"
    );
    assert!(!output.status.success(), "{combined}");
    assert!(
        !combined.contains("timed out"),
        "the failure must come from the response, not from a clock: {combined}"
    );
}

#[test]
fn tools_download_sources_rejects_conflicting_tests_source_set() {
    let dir = temp_workspace();
    let config_path = write_minimal_config(dir.path());
    let mut config = fs::read_to_string(&config_path).expect("config");
    config = config.replace(
        "tools:\n",
        "  - name: tests\n    type: CONFIGURATION\n    path: custom-tests\ntools:\n",
    );
    fs::write(&config_path, config).expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "tools",
            "download",
            "yaxunit",
            "--sources",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("source-set 'tests' already exists"));
    assert!(!dir.path().join("tests").exists());
}

/// `client_mcp.cfe` регистрируется расширением-инструментом, а его ставит только
/// Конфигуратор: без ключа `providers.push` агент из цепочки `push` при таком расширении
/// выпадает, и скачивание артефакта проходит; ключ, назначивший агента, — отказ.
#[test]
fn tools_download_client_mcp_artifact_follows_the_push_chain_shaped_by_the_tool_extension() {
    for (builder, accepted) in [("DEFAULT", true), ("AGENT", false)] {
        let dir = temp_workspace();
        let config_path = write_minimal_config_with_builder(dir.path(), builder);
        let server_root = dir.path().join("server");
        let (_server, port) = FixtureServer::start(&server_root);
        write_http_fixture(&server_root, port);

        let output = v8_runner_command()
            .env(
                "V8TR_GITHUB_API_BASE_URL",
                format!("http://127.0.0.1:{port}"),
            )
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
                "tools",
                "download",
                "client-mcp",
            ])
            .output()
            .expect("run client-mcp command");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.success(), accepted, "{builder}: {stdout}");
        if !accepted {
            assert!(
                stdout.contains("needs the Designer as the push provider"),
                "{builder}: {stdout}"
            );
        }
    }
}
