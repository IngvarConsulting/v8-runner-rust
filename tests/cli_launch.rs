#![cfg(unix)]

mod support;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use support::{free_tcp_port, temp_workspace, v8_runner_command, write_shell_script_atomically};

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

fn write_blocking_logging_script(path: &Path, args_log: &Path, release: &ProcessRelease) {
    let staged_log = args_log.with_extension("tmp");
    write_shell_script_atomically(
        path,
        &format!(
            "printf '%s' \"$$\" > '{}'\nprintf '%s\n' \"$@\" > '{}'\nmv '{}' '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nprintf released > '{}'",
            release.pid_path().display(),
            staged_log.display(),
            staged_log.display(),
            args_log.display(),
            release.path().display(),
            release.completed_path().display()
        ),
    );
}

struct ProcessRelease {
    path: PathBuf,
    completed_path: PathBuf,
    pid_path: PathBuf,
    released: bool,
}

impl ProcessRelease {
    fn new(path: PathBuf) -> Self {
        let completed_path = path.with_extension("completed");
        let pid_path = path.with_extension("pid");
        Self {
            path,
            completed_path,
            pid_path,
            released: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn completed_path(&self) -> &Path {
        &self.completed_path
    }

    fn pid_path(&self) -> &Path {
        &self.pid_path
    }

    fn release_and_wait(mut self) {
        assert!(self.release_process(), "fake 1cv8 process did not stop");
        self.released = true;
    }

    fn release_process(&self) -> bool {
        let _ = fs::write(&self.path, b"release");
        let discovery_deadline = Instant::now() + Duration::from_millis(500);
        let pid = loop {
            if self.completed_path.exists() {
                return true;
            }
            if let Some(pid) = fs::read_to_string(&self.pid_path)
                .ok()
                .and_then(|value| value.trim().parse::<u32>().ok())
            {
                break Some(pid);
            }
            if Instant::now() >= discovery_deadline {
                break None;
            }
            thread::sleep(Duration::from_millis(20));
        };
        let Some(pid) = pid else {
            return true;
        };

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.completed_path.exists() || !process_is_running(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
        if wait_for_process_exit(pid, Duration::from_secs(2)) {
            return true;
        }
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .output();
        wait_for_process_exit(pid, Duration::from_secs(2))
    }
}

impl Drop for ProcessRelease {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.release_process();
        }
    }
}

fn process_is_running(pid: u32) -> bool {
    process_state(pid).is_some_and(|state| !state.starts_with('Z'))
}

fn process_state(pid: u32) -> Option<String> {
    Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|state| {
            let state = state.trim();
            (!state.is_empty()).then(|| state.to_owned())
        })
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_is_running(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    !process_is_running(pid)
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
                write!(stream, "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n",)
                    .expect("write delete response");
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
                    write!(stream, "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n",)
                        .expect("write initialized response");
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
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: fake-session\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .expect("write response headers");
            stream.write_all(&body).expect("write response body");
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
        "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        platform_path.display(),
    );
    if let Some(platform_version) = platform_version {
        config.push_str(&format!("    version: '{}'\n", platform_version));
    }

    fs::write(path, config).expect("config");
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
        "workPath: '{}'\nformat: DESIGNER\nbuilder: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\ntests:\n  va:\n    params_path: '{}'\n    profile: smoke\n    profiles:\n      smoke:\n        feature_path: '{}'\n        features_to_run:\n          - login\n        filter_tags:\n          - '@smoke'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project\ntools:\n  client_mcp:\n    port: 9874\n  va:\n    epf_path: '{}'\n  platform:\n    path: '{}'\n{}",
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
    let release = ProcessRelease::new(install_dir.join("release-1cv8"));
    prepend_config(&config_path, "execution_timeout: 10000\n");
    let (port, server) = start_fake_mcp_server(&[
        "infobase_info",
        "load_features",
        "open_feature_file",
        "run_scenario",
        "get_test_results",
        "connect_test_client",
    ]);
    write_blocking_logging_script(&install_dir.join("bin").join("1cv8"), &args_log, &release);

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
    release.release_and_wait();
    server.join().expect("fake MCP server exits");
}

#[test]
fn launch_mcp_va_wait_ready_fails_when_vanessa_tools_are_missing() {
    let (_dir, config_path, install_dir, args_log) = setup_mcp_va_project();
    let release = ProcessRelease::new(install_dir.join("release-1cv8"));
    prepend_config(&config_path, "execution_timeout: 15000\n");
    insert_client_mcp_config(&config_path, "    wait_ready_timeout_ms: 5000\n");
    let (port, server) = start_fake_mcp_server(&["infobase_info"]);
    write_blocking_logging_script(&install_dir.join("bin").join("1cv8"), &args_log, &release);

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
    assert!(payload["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("Vanessa MCP tools were not registered"));
    release.release_and_wait();
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
    let port = free_tcp_port();

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
    let port = free_tcp_port();

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
    let port = free_tcp_port();

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
    let port = free_tcp_port();

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
