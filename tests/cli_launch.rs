#![cfg(unix)]

mod support;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use support::{temp_workspace, v8_runner_command, write_shell_script_atomically};

fn write_script(path: &Path) {
    write_shell_script_atomically(path, "sleep 1");
}

fn write_logging_script(path: &Path, args_log: &Path) {
    let staged_log = args_log.with_extension("tmp");
    write_shell_script_atomically(
        path,
        &format!(
            "printf '%s\n' \"$@\" > '{}'\nmv '{}' '{}'\nsleep 1",
            staged_log.display(),
            staged_log.display(),
            args_log.display()
        ),
    );
}

fn write_bounded_logging_script(path: &Path, args_log: &Path) {
    let staged_log = args_log.with_extension("tmp");
    write_shell_script_atomically(
        path,
        &format!(
            "printf '%s\n' \"$@\" > '{}'\nmv '{}' '{}'\nsleep 60",
            staged_log.display(),
            staged_log.display(),
            args_log.display()
        ),
    );
}

fn read_args_log(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(args) = fs::read_to_string(path) {
            if !args.is_empty() {
                return args;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for args log '{}'", path.display())
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    path.exists()
}

struct FakeHttpRequest {
    method: String,
    session_id: Option<String>,
    body: Option<Value>,
}

fn start_fake_mcp_server(tools: &[&str]) -> (u16, JoinHandle<()>) {
    let tools = tools
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<Vec<_>>();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake MCP server");
    let port = listener.local_addr().expect("fake MCP local addr").port();
    let handle = thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("fake MCP nonblocking listener");
        let started = Instant::now();
        let mut initialize_count = 0;
        let mut initialized_notification_seen = false;
        let mut tools_list_seen = false;
        loop {
            let Ok((mut stream, _)) = listener.accept() else {
                assert!(
                    started.elapsed() <= Duration::from_secs(30),
                    "fake MCP server timed out waiting for requests"
                );
                thread::sleep(Duration::from_millis(10));
                continue;
            };
            stream
                .set_nonblocking(false)
                .expect("fake MCP blocking stream");
            let http_request = read_http_json_request(&mut stream);
            if http_request.method == "DELETE" {
                assert_eq!(
                    http_request.session_id.as_deref(),
                    Some("fake-session"),
                    "DELETE must reuse the initialized MCP session"
                );
                let _ = write!(
                    stream,
                    "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                break;
            }
            let request = http_request.body.expect("json rpc body");
            let method = request["method"].as_str().unwrap_or_default();
            let result = match method {
                "initialize" => {
                    assert!(
                        http_request.session_id.is_none(),
                        "initialize must start without a previous MCP session"
                    );
                    json!({
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "serverInfo": { "name": "fake-client-mcp", "version": "1" }
                    })
                }
                "tools/list" => json!({
                    "tools": tools.iter().map(|name| {
                        json!({
                            "name": name,
                            "description": "",
                            "inputSchema": { "type": "object" }
                        })
                    }).collect::<Vec<_>>()
                }),
                "notifications/initialized" => {
                    assert_eq!(
                        http_request.session_id.as_deref(),
                        Some("fake-session"),
                        "notifications/initialized must use the initialized MCP session"
                    );
                    initialized_notification_seen = true;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                _ => json!({}),
            };
            if method == "initialize" {
                initialize_count += 1;
            }
            if method == "tools/list" {
                assert_eq!(
                    http_request.session_id.as_deref(),
                    Some("fake-session"),
                    "tools/list must use the initialized MCP session"
                );
                assert!(
                    initialized_notification_seen,
                    "tools/list must be requested after notifications/initialized"
                );
                tools_list_seen = true;
            }
            let body = serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "result": result,
            }))
            .expect("response json");
            if write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: fake-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .and_then(|_| stream.write_all(&body))
            .is_err()
            {
                continue;
            }
        }
        assert!(
            initialized_notification_seen,
            "fake MCP server expected notifications/initialized"
        );
        assert_eq!(
            initialize_count, 1,
            "readiness polling must reuse one MCP session"
        );
        assert!(tools_list_seen, "fake MCP server expected tools/list");
    });
    (port, handle)
}

struct UnresponsiveEndpoint {
    address: std::net::SocketAddr,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl UnresponsiveEndpoint {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind unresponsive endpoint");
        let address = listener
            .local_addr()
            .expect("unresponsive endpoint address");
        listener
            .set_nonblocking(true)
            .expect("unresponsive endpoint nonblocking");
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            shutdown,
            thread: Some(thread),
        }
    }

    fn port(&self) -> u16 {
        self.address.port()
    }
}

impl Drop for UnresponsiveEndpoint {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_http_json_request(stream: &mut TcpStream) -> FakeHttpRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("read request");
        assert!(read > 0, "request closed before body");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some((method, session_id, body_start, content_length)) = http_body_bounds(&bytes) {
            if bytes.len() >= body_start + content_length {
                let body = if content_length == 0 {
                    None
                } else {
                    Some(
                        serde_json::from_slice(&bytes[body_start..body_start + content_length])
                            .expect("request json"),
                    )
                };
                return FakeHttpRequest {
                    method,
                    session_id,
                    body,
                };
            }
        }
    }
}

fn http_body_bounds(bytes: &[u8]) -> Option<(String, Option<String>, usize, usize)> {
    let header_end = bytes.windows(4).position(|window| window == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let method = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_default()
        .to_owned();
    let session_id = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
        .map(|(_, value)| value.trim().to_owned());
    let content_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    Some((method, session_id, header_end + 4, content_length))
}

fn prepend_config(path: &Path, prefix: &str) {
    let config = fs::read_to_string(path).expect("config");
    fs::write(path, format!("{prefix}{config}")).expect("config");
}

fn insert_client_mcp_config(path: &Path, body: &str) {
    let config = fs::read_to_string(path).expect("config");
    let updated = if config.contains("tools:\n  client_mcp:\n") {
        config.replace(
            "tools:\n  client_mcp:\n",
            &format!("tools:\n  client_mcp:\n{body}"),
        )
    } else {
        config.replace(
            "tools:\n  platform:",
            &format!("tools:\n  client_mcp:\n{body}  platform:"),
        )
    };
    fs::write(path, updated).expect("config");
}

fn canonical_path_string(path: &Path) -> String {
    fs::canonicalize(path)
        .expect("canonical path")
        .to_string_lossy()
        .into_owned()
}

fn write_config(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    platform_version: Option<&str>,
) {
    let mut config = format!(
        "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        platform_path.display(),
    );
    if let Some(platform_version) = platform_version {
        config.push_str(&format!("    version: '{}'\n", platform_version));
    }

    fs::write(path, config).expect("config");
}

/// Конфиг с объявленным клиентским адресом: файловая цель, у которой есть оба адреса.
fn write_config_with_web_url(
    path: &Path,
    work_path: &Path,
    platform_path: &Path,
    url: &str,
    extra: &str,
) {
    fs::write(
        path,
        format!(
            "workPath: '{work}'\nformat: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\n  web:\n    url: '{url}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{platform}'\n{extra}",
            work = work_path.display(),
            platform = platform_path.display(),
        ),
    )
    .expect("config");
}

/// Рабочее место с тонким клиентом и объявленным клиентским адресом.
fn setup_web_project(url: &str, extra: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");

    fs::create_dir_all(dir.path().join("project")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"));
    write_script(&install_dir.join("bin").join("1cv8c"));
    write_config_with_web_url(&config_path, &work_path, &install_dir, url, extra);

    (dir, config_path, install_dir)
}

fn launch_json(config_path: &Path, arguments: &[&str]) -> Value {
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
        ])
        .args(arguments)
        .output()
        .expect("run command");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn planned_args(payload: &Value) -> Vec<String> {
    payload["data"]["plan"]["args"]
        .as_array()
        .unwrap_or_else(|| panic!("no planned args: {payload}"))
        .iter()
        .map(|arg| arg.as_str().unwrap_or_default().to_owned())
        .collect()
}

fn setup_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    setup_project_with_thin_script("sleep 1")
}

fn setup_project_with_failing_thin_binary() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"));
    write_false_executable(&install_dir.join("bin").join("1cv8c"));
    write_config(&config_path, &base_path, &work_path, &install_dir, None);

    (dir, config_path, install_dir, work_path)
}

fn setup_project_with_thin_script(
    thin_script: &str,
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"));
    write_shell_script_atomically(&install_dir.join("bin").join("1cv8c"), thin_script);
    write_config(&config_path, &base_path, &work_path, &install_dir, None);

    (dir, config_path, install_dir, work_path)
}

fn write_false_executable(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent");
    }
    symlink("/usr/bin/false", path).expect("false symlink");
}

fn setup_versioned_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let root_path = dir.path().join("platform-root");
    let version = root_path.join("8.3.25.1234");
    let config_path = dir.path().join("v8project.yaml");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&version.join("bin").join("1cv8"));
    write_script(&version.join("bin").join("1cv8c"));
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &root_path,
        Some("8.3.25.1234"),
    );

    (dir, config_path, version, work_path)
}

fn setup_mcp_va_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    setup_mcp_va_project_with_work_name("work")
}

fn setup_mcp_va_project_with_work_name(
    work_name: &str,
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    setup_mcp_va_project_with_options(work_name, &[])
}

fn setup_mcp_va_project_with_options(
    work_name: &str,
    additional_launch_keys: &[&str],
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join(work_name);
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");
    let args_log = install_dir.join("mcp-va.args.log");
    let va_epf = dir.path().join("va").join("vanessa-automation.epf");
    let va_params = dir.path().join("cfg").join("va-base.json");
    let features_dir = dir.path().join("features").join("smoke");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::create_dir_all(va_epf.parent().expect("va dir")).expect("va dir");
    fs::create_dir_all(va_params.parent().expect("cfg dir")).expect("cfg dir");
    fs::create_dir_all(&features_dir).expect("features");
    fs::write(&va_epf, "epf").expect("epf");
    fs::write(&va_params, "{\n  \"existing\": true\n}\n").expect("params");
    fs::write(features_dir.join("login.feature"), "Feature: Login\n").expect("feature");
    write_script(&install_dir.join("bin").join("1cv8c"));
    write_logging_script(&install_dir.join("bin").join("1cv8"), &args_log);

    let additional_launch_keys_block = if additional_launch_keys.is_empty() {
        String::new()
    } else {
        format!(
            "  enterprise:\n    additional-launch-keys:\n{}",
            additional_launch_keys
                .iter()
                .map(|key| format!("      - '{}'\n", key))
                .collect::<String>()
        )
    };
    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\ntests:\n  va:\n    params_path: '{}'\n    profile: smoke\n    profiles:\n      smoke:\n        feature_path: '{}'\n        features_to_run:\n          - login\n        filter_tags:\n          - '@smoke'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  client_mcp:\n    port: 9874\n  va:\n    epf_path: '{}'\n  platform:\n    path: '{}'\n{}",
        work_path.display(),
        va_params.display(),
        features_dir.display(),
        va_epf.display(),
        install_dir.display(),
        additional_launch_keys_block,
    );
    fs::write(&config_path, config).expect("config");

    (dir, config_path, install_dir, args_log)
}

#[test]
fn launch_json_returns_pid_and_selected_binary() {
    let (_dir, config_path, install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let data = &payload["data"];
    assert_eq!(payload["ok"], true);
    assert_eq!(data["mode"], "thin");
    assert_eq!(
        data["binary"].as_str().expect("binary"),
        canonical_path_string(&install_dir.join("bin").join("1cv8c"))
    );
    assert!(data["pid"].as_u64().expect("pid") > 0);
}

#[test]
fn launch_dry_run_json_returns_a_plan_without_dispatching_the_client() {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");
    let args_log = dir.path().join("thin.args.log");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"));
    write_logging_script(&install_dir.join("bin").join("1cv8c"), &args_log);
    write_config(&config_path, &base_path, &work_path, &install_dir, None);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let data = &payload["data"];
    assert_eq!(payload["ok"], true);
    assert_eq!(data["mode"], "thin");
    assert_eq!(data["provider_dispatched"], false);
    assert!(data["pid"].is_null());
    assert_eq!(
        data["plan"]["program"].as_str().expect("planned program"),
        canonical_path_string(&install_dir.join("bin").join("1cv8c"))
    );
    let planned: Vec<&str> = data["plan"]["args"]
        .as_array()
        .expect("planned args")
        .iter()
        .map(|arg| arg.as_str().expect("arg"))
        .collect();
    assert_eq!(planned.first(), Some(&"ENTERPRISE"));
    assert!(planned.contains(&"/DisableStartupDialogs"));
    assert!(
        !wait_for_file(&args_log, Duration::from_millis(500)),
        "preview must not dispatch the client process"
    );
}

#[test]
fn launch_dry_run_text_masks_credentials_and_says_nothing_was_dispatched() {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"));
    write_script(&install_dir.join("bin").join("1cv8c"));
    write_config(&config_path, &base_path, &work_path, &install_dir, None);
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace(
            "infobase:\n  connection: 'File=/tmp/ib'\n",
            "infobase:\n  connection: 'File=/tmp/ib'\n  user: Admin\n  password: s3cret\n",
        ),
    )
    .expect("config with credentials");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "launch",
            "thin",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Launch preview completed successfully"));
    assert!(stdout.contains("provider dispatched: false"));
    assert!(stdout.contains("/N Admin"));
    assert!(stdout.contains("/P ***"));
    assert!(!stdout.contains("s3cret"), "{stdout}");
}

/// Превью называет выбранный бинарник и составленную строку аргументов — то же,
/// что ушло бы в запуск. Иначе одобрять план пришлось бы вслепую.
#[test]
fn launch_dry_run_json_names_the_program_and_the_arguments_it_would_run() {
    let (_dir, config_path, install_dir, _work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["provider_dispatched"], false);

    let program = payload["data"]["plan"]["program"]
        .as_str()
        .expect("preview names the program");
    assert!(
        program.contains(&install_dir.display().to_string()),
        "preview must name the located binary, found {program}"
    );
    let args = payload["data"]["plan"]["args"]
        .as_array()
        .expect("preview names the arguments");
    assert!(
        !args.is_empty(),
        "preview must name the arguments it would pass"
    );
}

#[test]
fn launch_text_includes_binary_pid_and_cleans_platform_logs() {
    let (_dir, config_path, install_dir, work_path) = setup_project();
    let logs_dir = work_path.join("logs").join("platform");
    fs::create_dir_all(&logs_dir).expect("logs dir");
    let stale_log = logs_dir.join("stale.log");
    fs::write(&stale_log, "old log").expect("stale log");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "--clean-before-execution",
            "launch",
            "designer",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Launch completed successfully"));
    assert!(stdout.contains("mode: конфигуратор"));
    assert!(stdout.contains("[status] Launched конфигуратор via"));
    assert!(stdout.contains(
        install_dir
            .join("bin")
            .join("1cv8")
            .to_string_lossy()
            .as_ref()
    ));
    assert!(stdout.contains("pid"));
    assert!(!stale_log.exists());
}

#[test]
fn launch_designer_accepts_positional_mode() {
    let (_dir, config_path, install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "designer",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["mode"], "designer");
    assert_eq!(
        payload["data"]["binary"].as_str().expect("binary"),
        canonical_path_string(&install_dir.join("bin").join("1cv8"))
    );
}

#[test]
fn launch_thick_uses_v8_binary() {
    let (_dir, config_path, install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thick",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        payload["data"]["binary"].as_str().expect("binary"),
        canonical_path_string(&install_dir.join("bin").join("1cv8"))
    );
}

#[test]
fn launch_json_exposes_platform_resolution_metadata() {
    let (_dir, config_path, version_dir, _work_path) = setup_versioned_project();
    let canonical_version_dir = fs::canonicalize(&version_dir).expect("canonical version dir");
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        payload["data"]["binary"].as_str().expect("binary"),
        canonical_version_dir
            .join("bin")
            .join("1cv8c")
            .to_string_lossy()
    );
    assert_eq!(
        payload["data"]["platform_resolution"]["path"]
            .as_str()
            .expect("resolution path"),
        canonical_version_dir
            .join("bin")
            .join("1cv8c")
            .to_string_lossy()
    );
    assert_eq!(
        payload["data"]["platform_resolution"]["version"],
        "8.3.25.1234"
    );
    assert_eq!(payload["data"]["platform_resolution"]["source"], "explicit");
    assert_eq!(
        payload["data"]["platform_resolution"]["installation_root"]
            .as_str()
            .expect("installation root"),
        canonical_version_dir.to_string_lossy()
    );
}

/// Превью маскировало пароль внутри строки соединения, а отказ настоящего запуска
/// печатал его целиком — в stderr и в журнал действий, который живёт дольше запуска.
/// Читаемым остаётся всё, что не секрет: по отказу должно быть видно, куда шли.
#[test]
fn launch_failure_never_echoes_the_password_inside_the_connection_string() {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");
    let action_log = dir.path().join("actions.log");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"));
    write_false_executable(&install_dir.join("bin").join("1cv8c"));
    write_config(&config_path, &base_path, &work_path, &install_dir, None);
    let config = fs::read_to_string(&config_path).expect("config");
    fs::write(
        &config_path,
        config.replace(
            "connection: 'File=/tmp/ib'",
            "connection: 'Srvr=\"srv:1541\";Ref=ut;Usr=Admin;Pwd=s3cret'",
        ),
    )
    .expect("config with credentials in the connection string");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "--log-level",
            "debug",
            "launch",
            "thin",
        ])
        .env("V8TR_ACTION_LOG_FILE", &action_log)
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exited before startup completed"),
        "{stderr}"
    );
    assert!(!stderr.contains("s3cret"), "{stderr}");
    assert!(stderr.contains("Pwd=***"), "{stderr}");
    assert!(stderr.contains("Ref=ut"), "{stderr}");

    let log = fs::read_to_string(&action_log).expect("action log");
    assert!(!log.contains("s3cret"), "{log}");
    // Без положительного якоря проверка журнала прошла бы и тогда, когда показ
    // команды перестал бы в него попадать вовсе.
    assert!(log.contains("Pwd=***"), "{log}");
}

#[test]
fn launch_fails_when_process_exits_during_startup_probe() {
    let (_dir, config_path, install_dir, _work_path) = setup_project_with_failing_thin_binary();
    let thin = install_dir.join("bin").join("1cv8c");
    let thin_target = fs::read_link(&thin).expect("thin symlink");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "thin",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "status={:?}\nstdout={}\nstderr={}\nsymlink={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        thin_target.display()
    );
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exited before startup completed"));
}

#[test]
fn launch_json_failure_returns_error_envelope_and_exit_code() {
    let (_dir, config_path, install_dir, _work_path) = setup_project_with_failing_thin_binary();
    let thin = install_dir.join("bin").join("1cv8c");
    let thin_target = fs::read_link(&thin).expect("thin symlink");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "status={:?}\nstdout={}\nstderr={}\nsymlink={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        thin_target.display()
    );
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stderr.is_empty());
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "launch");
    assert_eq!(payload["error"]["code"], "platform_failure");
    assert_eq!(payload["error"]["kind"], "platform");
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("exited before startup completed"));
}

#[test]
fn launch_ordinary_supports_typed_keys_and_filters_reserved_raw_duplicates() {
    let (_dir, config_path, install_dir, _work_path) = setup_project();
    let args_log = install_dir.join("ordinary.args.log");
    write_logging_script(&install_dir.join("bin").join("1cv8"), &args_log);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "ordinary",
            "--c",
            "DoWork",
            "--execute",
            "/tmp/tool.epf",
            "--use-privileged-mode",
            "--output",
            "/tmp/user.out.log",
            "--raw-key",
            "/RunModeOrdinaryApplication",
            "--raw-key",
            "/Out",
            "--raw-key",
            "/tmp/ignored.out.log",
            "--raw-key",
            "/WA-",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let args = read_args_log(&args_log);
    assert!(args.contains("ENTERPRISE"));
    assert!(args.contains("/DisableStartupDialogs"));
    assert_eq!(args.matches("/RunModeOrdinaryApplication").count(), 1);
    assert!(args.contains("/UsePrivilegedMode"));
    assert!(args.contains("/Execute"));
    assert!(args.contains("/tmp/tool.epf"));
    assert!(args.contains("/C\nDoWork\n"));
    assert!(args.contains("/WA-"));
    assert!(args.contains("/tmp/user.out.log"));
    assert!(!args.contains("/tmp/ignored.out.log"));
}

#[test]
fn launch_mcp_va_builds_payload_from_configured_port_and_ordinary_mode() {
    let (_dir, config_path, install_dir, args_log) = setup_mcp_va_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "va",
            "--mode",
            "ordinary",
            "--mcp-config",
            "/tmp/mcp conf.json",
            "--raw-key",
            "/WA-",
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
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["mode"], "mcp");
    assert_eq!(
        payload["data"]["binary"].as_str().expect("binary"),
        canonical_path_string(&install_dir.join("bin").join("1cv8"))
    );

    let args = read_args_log(&args_log);
    assert!(args.contains("ENTERPRISE"));
    assert!(args.contains("/DisableStartupDialogs"));
    assert!(args.contains("/RunModeOrdinaryApplication"));
    assert!(args.contains("/Execute"));
    assert!(args.contains("vanessa-automation.epf"));
    assert!(args.contains("/C\nrunMcp=/tmp/mcp conf.json;mcpPort=9874;VAParams="));
    assert!(!args.contains("StartFeaturePlayer"));
    assert!(args.contains("/TESTMANAGER"));
    assert!(args.contains("/WA-"));
    let params_arg = args
        .lines()
        .find(|line| line.contains("VAParams="))
        .expect("VAParams argument");
    let params_path = params_arg
        .split("VAParams=")
        .nth(1)
        .expect("VAParams path")
        .trim_end_matches('"');
    let params = fs::read_to_string(params_path).expect("runtime params");
    let params_json: Value = serde_json::from_str(&params).expect("runtime params JSON");
    assert_eq!(params_json["existing"], true);
    assert!(params_json["WorkspaceRoot"]
        .as_str()
        .expect("WorkspaceRoot")
        .contains(
            config_path
                .parent()
                .expect("config dir")
                .display()
                .to_string()
                .as_str()
        ));
    assert_eq!(params_json["ОстановкаПриВозникновенииОшибки"], false);
    assert_eq!(params_json["СписокФичДляВыполнения"][0], "login");
    assert_eq!(params_json["СписокТеговОтбор"][0], "smoke");
    assert_eq!(
        params_json["ДелатьЛогВыполненияСценариевВТекстовыйФайл"],
        true
    );
    assert_eq!(params_json["ВыводитьВЛогВыполнениеШагов"], true);
    assert_eq!(params_json["ПодробныйЛогВыполненияСценариев"], 1);
    assert_eq!(params_json["ВыгружатьСтатусВыполненияСценариевВФайл"], true);
    assert!(
        params_json["ПутьКФайлуДляВыгрузкиСтатусаВыполненияСценариев"]
            .as_str()
            .expect("status path")
            .ends_with("/va-status.log")
    );
    assert!(params_json["ИмяФайлаЛогВыполненияСценариев"]
        .as_str()
        .expect("text log path")
        .ends_with("/va-text.log"));
    assert_eq!(
        fs::metadata(params_path)
            .expect("params metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let params_dir = Path::new(params_path).parent().expect("params dir");
    assert_eq!(
        fs::metadata(params_dir)
            .expect("params dir metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn launch_mcp_va_wait_ready_returns_registered_vanessa_tools() {
    let (_dir, config_path, install_dir, args_log) = setup_mcp_va_project();
    prepend_config(&config_path, "execution_timeout: 10000\n");
    let (port, server) = start_fake_mcp_server(&[
        "infobase_info",
        "load_features",
        "open_feature_file",
        "run_scenario",
        "get_test_results",
        "connect_test_client",
    ]);
    write_bounded_logging_script(&install_dir.join("bin").join("1cv8"), &args_log);
    write_bounded_logging_script(&install_dir.join("bin").join("1cv8c"), &args_log);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "va",
            "--mode",
            "ordinary",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
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
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["mode"], "mcp");
    assert_eq!(payload["data"]["mcp_readiness"]["ok"], true);
    assert_eq!(
        payload["data"]["mcp_readiness"]["url"],
        format!("http://127.0.0.1:{port}/mcp")
    );
    let tools = payload["data"]["mcp_readiness"]["tools"]
        .as_array()
        .expect("tools");
    assert!(tools.iter().any(|tool| tool == "load_features"));
    assert!(tools.iter().any(|tool| tool == "run_scenario"));
    assert!(tools.iter().any(|tool| tool == "get_test_results"));
    server.join().expect("fake MCP server exits");
}

#[test]
fn launch_mcp_va_wait_ready_fails_when_vanessa_tools_are_missing() {
    let (_dir, config_path, install_dir, args_log) = setup_mcp_va_project();
    prepend_config(&config_path, "execution_timeout: 15000\n");
    insert_client_mcp_config(&config_path, "    wait_ready_timeout_ms: 5000\n");
    let (port, server) = start_fake_mcp_server(&["infobase_info"]);
    write_bounded_logging_script(&install_dir.join("bin").join("1cv8"), &args_log);
    write_bounded_logging_script(&install_dir.join("bin").join("1cv8c"), &args_log);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "va",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["ok"], false);
    assert_eq!(payload["data"]["mcp_readiness"]["ok"], false);
    let missing_tools = payload["data"]["mcp_readiness"]["missing_tools"]
        .as_array()
        .expect("missing tools");
    assert!(missing_tools.iter().any(|tool| tool == "load_features"));
    assert!(missing_tools.iter().any(|tool| tool == "run_scenario"));
    let error_message = payload["error"]["message"].as_str().expect("error message");
    assert!(
        error_message.contains("Vanessa MCP tools were not registered"),
        "unexpected error message: {error_message}; payload={payload}"
    );
    server.join().expect("fake MCP server exits");
}

#[test]
fn launch_mcp_wait_ready_returns_client_mcp_tools_without_vanessa_requirements() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    prepend_config(&config_path, "execution_timeout: 10000\n");
    let (port, server) = start_fake_mcp_server(&["infobase_info", "query_info"]);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
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
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["mode"], "mcp");
    assert_eq!(payload["data"]["mcp_readiness"]["ok"], true);
    assert_eq!(
        payload["data"]["mcp_readiness"]["url"],
        format!("http://127.0.0.1:{port}/mcp")
    );
    assert_eq!(
        payload["data"]["mcp_readiness"]["missing_tools"]
            .as_array()
            .expect("missing tools")
            .len(),
        0
    );
    let tools = payload["data"]["mcp_readiness"]["tools"]
        .as_array()
        .expect("tools");
    assert!(tools.iter().any(|tool| tool == "infobase_info"));
    assert!(tools.iter().any(|tool| tool == "query_info"));
    server.join().expect("fake MCP server exits");
}

#[test]
fn launch_mcp_wait_ready_fails_when_endpoint_never_starts() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    prepend_config(&config_path, "execution_timeout: 5000\n");
    insert_client_mcp_config(&config_path, "    wait_ready_timeout_ms: 500\n");
    let endpoint = UnresponsiveEndpoint::start();
    let port = endpoint.port();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["data"]["ok"], false);
    assert_eq!(payload["data"]["mcp_readiness"]["ok"], false);
    assert_eq!(
        payload["data"]["mcp_readiness"]["url"],
        format!("http://127.0.0.1:{port}/mcp")
    );
    assert!(payload["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("MCP endpoint did not become ready"));
}

#[test]
fn launch_mcp_wait_ready_text_failure_is_not_rendered_as_success() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    prepend_config(&config_path, "execution_timeout: 5000\n");
    insert_client_mcp_config(&config_path, "    wait_ready_timeout_ms: 500\n");
    let endpoint = UnresponsiveEndpoint::start();
    let port = endpoint.port();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "launch",
            "mcp",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Launch failed"));
    assert!(!stdout.contains("Launch completed successfully"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("runtime error"));
}

#[test]
fn launch_mcp_wait_ready_terminates_process_on_readiness_failure() {
    let marker = temp_workspace();
    let started = marker.path().join("started");
    let terminated = marker.path().join("terminated");
    let script = format!(
        "printf started > '{}'\ntrap 'printf terminated > \"{}\"; exit 0' TERM INT\nwhile true; do sleep 1; done",
        started.display(),
        terminated.display()
    );
    let (_dir, config_path, _install_dir, _work_path) = setup_project_with_thin_script(&script);
    prepend_config(&config_path, "execution_timeout: 15000\n");
    insert_client_mcp_config(&config_path, "    wait_ready_timeout_ms: 5000\n");
    let endpoint = UnresponsiveEndpoint::start();
    let port = endpoint.port();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json failure payload");
    assert!(payload["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("MCP endpoint did not become ready"));
    assert!(
        wait_for_file(&started, Duration::from_secs(10)),
        "launch process should have started before the readiness timeout"
    );
    assert!(
        wait_for_file(&terminated, Duration::from_secs(10)),
        "wait-ready failure should terminate the launched client process"
    );
}

#[test]
fn launch_mcp_wait_ready_uses_configured_wait_timeout() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    prepend_config(&config_path, "execution_timeout: 15000\n");
    insert_client_mcp_config(&config_path, "    wait_ready_timeout_ms: 500\n");
    let endpoint = UnresponsiveEndpoint::start();
    let port = endpoint.port();

    let started = Instant::now();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "--mcp-port",
            &port.to_string(),
            "--wait-ready",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json failure payload");
    assert!(payload["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("MCP endpoint did not become ready"));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "wait-ready should use tools.client_mcp.wait_ready_timeout_ms instead of the global execution_timeout"
    );
}

#[test]
fn thin_external_epf_wait_returns_structured_exit_and_artifacts() {
    let (_dir, config_path, _install_dir, work_path) =
        setup_project_with_thin_script("printf client-stderr >&2\nexit 7");
    let epf = work_path.join("runtime-check.epf");
    let output = work_path.join("runtime.out");
    let stderr = work_path.join("runtime.stderr");
    fs::write(&epf, "epf").expect("epf");

    let command_output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            &epf.display().to_string(),
            "--output",
            &output.display().to_string(),
            "--stderr-output",
            &stderr.display().to_string(),
            "--wait-for-exit",
            "--wait-timeout-ms",
            "5000",
        ])
        .output()
        .expect("run command");

    assert!(
        command_output.status.success(),
        "status={:?}\\nstdout={}\\nstderr={}",
        command_output.status.code(),
        String::from_utf8_lossy(&command_output.stdout),
        String::from_utf8_lossy(&command_output.stderr)
    );
    let payload: Value = serde_json::from_slice(&command_output.stdout).expect("json");
    let wait = &payload["data"]["external_epf_wait"];
    assert!(wait["pid"].as_u64().is_some());
    assert_eq!(wait["execute_path"], epf.display().to_string());
    assert_eq!(wait["exit_code"], 7);
    assert_eq!(wait["timed_out"], false);
    assert_eq!(wait["output_path"], output.display().to_string());
    assert_eq!(wait["stderr_path"], stderr.display().to_string());
    assert_eq!(
        fs::read_to_string(stderr).expect("captured stderr"),
        "client-stderr"
    );
}

#[test]
fn thin_external_epf_wait_timeout_terminates_client_group() {
    let marker = temp_workspace();
    let descendant_pid = marker.path().join("descendant.pid");
    let script = format!(
        "sleep 30 &\nprintf '%s' $! > '{}'\nwait",
        descendant_pid.display()
    );
    let (_dir, config_path, _install_dir, work_path) = setup_project_with_thin_script(&script);
    let epf = work_path.join("runtime-check.epf");
    let output = work_path.join("runtime.out");
    let stderr = work_path.join("runtime.stderr");
    fs::write(&epf, "epf").expect("epf");

    let command_output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            &epf.display().to_string(),
            "--output",
            &output.display().to_string(),
            "--stderr-output",
            &stderr.display().to_string(),
            "--wait-for-exit",
            "--wait-timeout-ms",
            "5000",
        ])
        .output()
        .expect("run command");

    assert!(!command_output.status.success());
    let payload: Value = serde_json::from_slice(&command_output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["kind"], "runtime");
    assert_eq!(payload["data"]["external_epf_wait"]["timed_out"], true);
    assert!(
        wait_for_file(&descendant_pid, Duration::from_secs(10)),
        "client fixture did not publish its descendant pid before the wait timeout"
    );
    let pid = fs::read_to_string(descendant_pid).expect("descendant pid");
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("probe descendant")
            .success(),
        "timeout must terminate the entire client process group"
    );
}

#[test]
fn thin_external_epf_wait_timeout_overrides_execution_timeout() {
    let (_dir, config_path, _install_dir, work_path) = setup_project_with_thin_script("sleep 5");
    prepend_config(&config_path, "execution_timeout: 100\n");
    let epf = work_path.join("runtime-check.epf");
    let output = work_path.join("runtime.out");
    let stderr = work_path.join("runtime.stderr");
    fs::write(&epf, "epf").expect("epf");

    let started = Instant::now();
    let command_output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            &epf.display().to_string(),
            "--output",
            &output.display().to_string(),
            "--stderr-output",
            &stderr.display().to_string(),
            "--wait-for-exit",
            "--wait-timeout-ms",
            "800",
        ])
        .output()
        .expect("run command");

    assert!(!command_output.status.success());
    assert!(
        started.elapsed() >= Duration::from_millis(650),
        "wait timeout must not be shortened by execution_timeout; elapsed={:?}",
        started.elapsed()
    );
    let payload: Value = serde_json::from_slice(&command_output.stdout).expect("json");
    assert_eq!(payload["data"]["external_epf_wait"]["timed_out"], true);
    assert!(payload["data"]["message"]
        .as_str()
        .expect("message")
        .contains("800ms"));
}

#[test]
fn thin_external_epf_wait_rejects_normalized_raw_reserved_key_before_spawn() {
    let marker = temp_workspace();
    let started = marker.path().join("started");
    let script = format!("printf started > '{}'\nsleep 1", started.display());
    let (_dir, config_path, _install_dir, work_path) = setup_project_with_thin_script(&script);
    let epf = work_path.join("runtime-check.epf");
    fs::write(&epf, "epf").expect("epf");

    let command_output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            &epf.display().to_string(),
            "--output",
            "/tmp/runtime.out",
            "--stderr-output",
            "/tmp/runtime.stderr",
            "--wait-for-exit",
            "--wait-timeout-ms",
            "100",
            "--raw-key",
            "//Out=/tmp/override.out",
        ])
        .output()
        .expect("run command");

    assert!(!command_output.status.success());
    assert!(
        !started.exists(),
        "validation must happen before client spawn"
    );
    assert!(String::from_utf8_lossy(&command_output.stdout)
        .contains("does not support raw /C, /Execute, or /Out"));
}

#[test]
fn thin_external_epf_wait_rejects_whitespace_raw_execute_alias_before_spawn() {
    let marker = temp_workspace();
    let started = marker.path().join("started");
    let script = format!("printf started > '{}'\nsleep 1", started.display());
    let (_dir, config_path, _install_dir, work_path) = setup_project_with_thin_script(&script);
    let epf = work_path.join("runtime-check.epf");
    fs::write(&epf, "epf").expect("epf");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            &epf.display().to_string(),
            "--output",
            "/tmp/runtime.out",
            "--stderr-output",
            "/tmp/runtime.stderr",
            "--wait-for-exit",
            "--wait-timeout-ms",
            "100",
            "--raw-key",
            "/Execute alternate.epf",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert!(
        !started.exists(),
        "validation must happen before client spawn"
    );
}

#[test]
fn thin_external_epf_wait_rejects_attached_c_alias_before_spawn() {
    let marker = temp_workspace();
    let started = marker.path().join("started");
    let script = format!("printf started > '{}'\nsleep 1", started.display());
    let (_dir, config_path, _install_dir, work_path) = setup_project_with_thin_script(&script);
    let epf = work_path.join("runtime-check.epf");
    fs::write(&epf, "epf").expect("epf");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "thin",
            "--execute",
            &epf.display().to_string(),
            "--output",
            "/tmp/runtime.out",
            "--stderr-output",
            "/tmp/runtime.stderr",
            "--wait-for-exit",
            "--wait-timeout-ms",
            "100",
            "--raw-key",
            "/C\"RunUnitTests\"",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert!(
        !started.exists(),
        "validation must happen before client spawn"
    );
}

#[test]
fn launch_mcp_rejects_external_epf_wait_flags() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "mcp",
            "--wait-for-exit",
            "--wait-timeout-ms",
            "100",
            "--stderr-output",
            "/tmp/runtime.stderr",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("supported only for direct `launch thin`")
    );
}

#[test]
fn launch_mcp_va_does_not_duplicate_explicit_testmanager_raw_key() {
    let (_dir, config_path, _install_dir, args_log) =
        setup_mcp_va_project_with_options("work", &["/TESTMANAGER"]);
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "va",
            "--mode",
            "ordinary",
            "--raw-key",
            "/TESTMANAGER",
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

    let args = read_args_log(&args_log);
    let test_manager_count = args
        .split_whitespace()
        .filter(|arg| arg.eq_ignore_ascii_case("/TESTMANAGER"))
        .count();
    assert_eq!(test_manager_count, 1);
}

#[test]
fn launch_mcp_va_adds_testmanager_when_raw_value_matches_name() {
    let (_dir, config_path, _install_dir, args_log) = setup_mcp_va_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "va",
            "--mode",
            "ordinary",
            "--raw-key",
            "/VAUser",
            "--raw-key",
            "TESTMANAGER",
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

    let args = read_args_log(&args_log);
    assert!(args.contains("/VAUser"));
    assert!(args.contains("TESTMANAGER"));
    assert!(args
        .split_whitespace()
        .any(|arg| arg.eq_ignore_ascii_case("/TESTMANAGER")));
}

#[test]
fn launch_mcp_rejects_user_managed_c_payload() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "--c",
            "runMcp",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("launch mcp manages /C internally"));
}

#[test]
fn launch_mcp_rejects_user_managed_execute_payload() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "--execute",
            "tool.epf",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("launch mcp manages /C internally"));
}

#[test]
fn launch_mcp_rejects_reserved_raw_payloads() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "--raw-key",
            "/C\"runOther\"",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not support raw /C"));
}

#[test]
fn launch_mcp_rejects_semicolon_in_mcp_config_path() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "--mcp-config",
            "/tmp/conf;mcpPort=1.json",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("must not contain ';'"));
}

#[test]
fn launch_mcp_rejects_zero_mcp_port() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "--mcp-port",
            "0",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--mcp-port must be greater than or equal to 1"));
}

#[test]
fn launch_mcp_va_rejects_semicolon_in_generated_params_path() {
    let (_dir, config_path, _install_dir, _args_log) =
        setup_mcp_va_project_with_work_name("work;bad");
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "mcp",
            "va",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("generated Vanessa params path for launch mcp must not contain ';'"));
}

#[test]
fn launch_non_mcp_rejects_mcp_options() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "launch",
            "thin",
            "--mcp-port",
            "9876",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "--mcp-config, --mcp-port, --mode, --wait-ready, and MCP_SCENARIO are supported only for `launch mcp`"
    ));
}

/// `launch web` открывает объявленный адрес; без адреса — отказ, который называет,
/// откуда адрес берётся. Раннер не выводит его из строки подключения.
#[test]
fn launch_web_without_a_declared_address_is_refused_with_the_reason() {
    let (_dir, config_path, _install_dir, _work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "web",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["error"]["kind"], "validation");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("infobase.web.url"), "{message}");
    assert!(message.contains("publish"), "{message}");
}

/// Превью `launch web` называет открывалку системы и адрес, браузер не трогает.
#[test]
fn launch_web_dry_run_names_the_opener_and_the_address() {
    let (dir, config_path, _install_dir, _work_path) = setup_project();
    let yaml = fs::read_to_string(&config_path).expect("config");
    let yaml = yaml.replace(
        "infobase:\n  connection: 'File=/tmp/ib'\n",
        "infobase:\n  connection: 'File=/tmp/ib'\n  web:\n    url: http://localhost/demo\n",
    );
    fs::write(&config_path, yaml).expect("config");
    let _ = dir;

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "launch",
            "web",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["mode"], "web");
    assert_eq!(payload["data"]["url"], "http://localhost/demo");
    assert_eq!(payload["data"]["provider_dispatched"], false);
    assert!(payload["data"]["platform_resolution"].is_null());
    let args = payload["data"]["plan"]["args"]
        .as_array()
        .expect("plan args");
    assert_eq!(
        args.last().and_then(Value::as_str),
        Some("http://localhost/demo")
    );
}

/// У цели два адреса, и тонкий клиент открывается любым. `--via web` берёт клиентский и
/// передаёт его как ws-соединение.
#[test]
fn a_thin_client_goes_through_the_web_address_when_asked() {
    let (_dir, config_path, install_dir) = setup_web_project("http://localhost/base", "");

    let payload = launch_json(
        &config_path,
        &["launch", "thin", "--via", "web", "--dry-run"],
    );

    assert_eq!(payload["ok"], true, "{payload}");
    assert_eq!(payload["data"]["via"], "web", "{payload}");
    assert_eq!(payload["data"]["url"], "http://localhost/base", "{payload}");
    assert_eq!(
        payload["data"]["plan"]["program"]
            .as_str()
            .expect("program"),
        canonical_path_string(&install_dir.join("bin").join("1cv8c"))
    );
    let args = planned_args(&payload);
    let at = args
        .iter()
        .position(|arg| arg == "/WS")
        .unwrap_or_else(|| panic!("no /WS in {args:?}"));
    assert_eq!(args[at + 1], "http://localhost/base", "{args:?}");
    assert!(
        !args.iter().any(|arg| arg == "/IBConnectionString"),
        "клиентский адрес заменяет административный, а не дополняет: {args:?}"
    );
}

/// Умолчание у файловой цели — административный адрес, и объявленный `web.url` его не
/// подменяет: путь выбирает вид цели, а не наличие публикации.
#[test]
fn a_thin_client_keeps_the_connection_address_by_default() {
    let (_dir, config_path, _install) = setup_web_project("http://localhost/base", "");

    let payload = launch_json(&config_path, &["launch", "thin", "--dry-run"]);

    assert_eq!(payload["data"]["via"], "connection", "{payload}");
    assert!(payload["data"]["url"].is_null(), "{payload}");
    let args = planned_args(&payload);
    assert!(
        args.iter().any(|arg| arg == "/IBConnectionString"),
        "{args:?}"
    );
    assert!(!args.iter().any(|arg| arg == "/WS"), "{args:?}");
}

/// Развилка есть только у тонкого клиента: у остальных режимов адрес один, и ключ,
/// которому нечего выбирать, отвергается, а не игнорируется молча.
#[test]
fn via_is_refused_where_there_is_no_choice() {
    let (_dir, config_path, _install) = setup_web_project("http://localhost/base", "");

    for mode in ["designer", "thick", "ordinary", "web"] {
        let payload = launch_json(&config_path, &["launch", mode, "--via", "web", "--dry-run"]);

        assert_eq!(payload["ok"], false, "{mode}: {payload}");
        assert_eq!(payload["error"]["kind"], "validation", "{mode}: {payload}");
    }
}

/// Адреса нет — отказывает и `launch web`, и тонкий клиент по вебу, одним и тем же текстом:
/// не хватает им одного и того же.
#[test]
fn a_web_launch_without_an_address_is_refused_the_same_way_for_both_paths() {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");
    fs::create_dir_all(dir.path().join("project")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8c"));
    write_config(&config_path, dir.path(), &work_path, &install_dir, None);

    for arguments in [
        vec!["launch", "thin", "--via", "web", "--dry-run"],
        vec!["launch", "web", "--dry-run"],
    ] {
        let payload = launch_json(&config_path, &arguments);

        assert_eq!(payload["ok"], false, "{arguments:?}: {payload}");
        assert_eq!(
            payload["error"]["kind"], "validation",
            "{arguments:?}: {payload}"
        );
        assert!(
            payload["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("infobase.web.url"),
            "{arguments:?}: {payload}"
        );
    }
}

/// Поле `via` есть у каждого режима, а не только там, где был выбор: иначе его
/// отсутствие пришлось бы толковать.
#[test]
fn every_launch_names_the_address_it_used() {
    let (_dir, config_path, _install) = setup_web_project("http://localhost/base", "");

    for (arguments, expected) in [
        (vec!["launch", "designer", "--dry-run"], "connection"),
        (vec!["launch", "thick", "--dry-run"], "connection"),
        (vec!["launch", "web", "--dry-run"], "web"),
        (vec!["launch", "thin", "--via", "web", "--dry-run"], "web"),
    ] {
        let payload = launch_json(&config_path, &arguments);

        assert_eq!(payload["ok"], true, "{arguments:?}: {payload}");
        assert_eq!(payload["data"]["via"], expected, "{arguments:?}: {payload}");
    }
}

/// Пароль из userinfo не показывается нигде, где раннер показывает адрес: ни в плане,
/// ни в поле `url`, ни в сообщении. Имя пользователя остаётся — по нему адрес узнаётся.
#[test]
fn a_client_address_is_reported_without_its_userinfo_password() {
    let (_dir, config_path, _install) = setup_web_project("http://alice:s3cret@localhost/base", "");

    for arguments in [
        vec!["launch", "thin", "--via", "web", "--dry-run"],
        vec!["launch", "web", "--dry-run"],
    ] {
        let payload = launch_json(&config_path, &arguments);
        let rendered = payload.to_string();

        assert!(
            !rendered.contains("s3cret"),
            "{arguments:?} показал пароль: {payload}"
        );
        assert!(
            rendered.contains("alice"),
            "{arguments:?} потерял имя пользователя: {payload}"
        );
        assert_eq!(
            payload["data"]["url"], "http://alice:***@localhost/base",
            "{arguments:?}: {payload}"
        );
    }
}

/// Пользовательские ключи запуска дописываются после наших и своего адреса не отменяют:
/// раннер не теряет `/WS` и не падает, даже когда рядом положили второй адрес.
#[test]
fn additional_launch_keys_do_not_displace_the_web_address() {
    let (_dir, config_path, _install) = setup_web_project(
        "http://localhost/base",
        "  enterprise:\n    additional-launch-keys: ['/IBConnectionString', 'File=/tmp/other']\n",
    );

    let payload = launch_json(
        &config_path,
        &["launch", "thin", "--via", "web", "--dry-run"],
    );

    assert_eq!(payload["ok"], true, "{payload}");
    let args = planned_args(&payload);
    let ws = args
        .iter()
        .position(|arg| arg == "/WS")
        .unwrap_or_else(|| panic!("no /WS in {args:?}"));
    let theirs = args
        .iter()
        .position(|arg| arg == "/IBConnectionString")
        .unwrap_or_else(|| panic!("user key dropped: {args:?}"));
    assert!(ws < theirs, "наш адрес идёт первым: {args:?}");
}

/// У автономной цели `infobase.user` и `infobase.password` — учётные данные SSH-шлюза, а
/// не базы. Тонкий клиент к ней идёт по вебу без всякого ключа и **без** `/N` и `/P`:
/// иначе раннер отдал бы пароль шлюза в командную строку клиента.
#[test]
fn a_standalone_thin_client_carries_the_address_without_the_gate_credentials() {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let exchange = dir.path().join("exchange");
    let config_path = dir.path().join("v8project.yaml");
    fs::create_dir_all(dir.path().join("project")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    fs::create_dir_all(&exchange).expect("exchange");
    write_script(&install_dir.join("bin").join("1cv8c"));
    fs::write(
        &config_path,
        format!(
            "workPath: '{work}'\nformat: DESIGNER\ninfobase:\n  user: gate-user\n  password: gate-secret\n  web:\n    url: 'http://localhost/standalone'\n  standalone:\n    gate: 127.0.0.1:1543\n    exchange:\n      dir: '{exchange}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{platform}'\n",
            work = work_path.display(),
            exchange = exchange.display(),
            platform = install_dir.display(),
        ),
    )
    .expect("config");

    let payload = launch_json(&config_path, &["launch", "thin", "--dry-run"]);

    assert_eq!(payload["ok"], true, "{payload}");
    assert_eq!(
        payload["data"]["via"], "web",
        "умолчание автономной цели — веб: {payload}"
    );
    let args = planned_args(&payload);
    let at = args
        .iter()
        .position(|arg| arg == "/WS")
        .unwrap_or_else(|| panic!("no /WS in {args:?}"));
    assert_eq!(args[at + 1], "http://localhost/standalone", "{args:?}");
    assert!(
        !args.iter().any(|arg| arg == "/N" || arg == "/P"),
        "реквизиты шлюза клиенту не принадлежат: {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.contains("gate-secret")),
        "пароль шлюза не должен попадать в командную строку: {args:?}"
    );
}

/// Маскируется отчёт, а не запуск: в процесс уходит настоящий адрес, иначе клиент никуда
/// не подключится. Проверяется на живом запуске, а не на превью.
#[test]
fn a_real_web_launch_passes_the_unmasked_address_to_the_client() {
    let dir = temp_workspace();
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = dir.path().join("v8project.yaml");
    let args_log = dir.path().join("thin.args.log");
    fs::create_dir_all(dir.path().join("project")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_logging_script(&install_dir.join("bin").join("1cv8c"), &args_log);
    write_config_with_web_url(
        &config_path,
        &work_path,
        &install_dir,
        "http://alice:s3cret@localhost/base",
        "",
    );

    let payload = launch_json(&config_path, &["launch", "thin", "--via", "web"]);

    assert_eq!(payload["ok"], true, "{payload}");
    assert_eq!(payload["data"]["provider_dispatched"], true, "{payload}");
    assert_eq!(payload["data"]["via"], "web", "{payload}");
    assert_eq!(
        payload["data"]["url"], "http://alice:***@localhost/base",
        "отчёт несёт замаскированный адрес: {payload}"
    );
    assert!(
        !payload.to_string().contains("s3cret"),
        "отчёт не должен нести пароль: {payload}"
    );

    let dispatched = read_args_log(&args_log);
    assert!(
        dispatched.contains("http://alice:s3cret@localhost/base"),
        "в процесс обязан уйти настоящий адрес, иначе клиент не подключится: {dispatched:?}"
    );
}
