use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::config::model::AppConfig;
use crate::domain::launch::{LaunchMode, LaunchResult, McpReadinessResult};
use crate::domain::runner::LaunchOptions;
use crate::platform::enterprise::{
    build_launch_args, normalize_launch_payload_path, LaunchClientMode,
};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRequest;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, ExecutionInterruption};
use crate::use_cases::launch_keys::vanessa_enterprise_launch_keys;
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::request::{
    ClientMcpAddonRequest, ClientMcpMode, ClientMcpOptionsRequest, EnterpriseLaunchTarget,
    LaunchRequest as LaunchArgs, LaunchTargetRequest,
};
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};
use crate::use_cases::tool_extension;
use reqwest::blocking::Client;
use reqwest::header::HeaderValue;
use serde_json::{json, Value};
use tracing::debug;

const LAUNCH_STARTUP_PROBE: Duration = Duration::from_millis(250);
const MCP_READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MCP_READY_REQUEST_TIMEOUT: Duration = Duration::from_millis(300);
const MCP_READY_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_ENDPOINT_PATH: &str = "/mcp";
const VANESSA_MCP_TOOLS: &[&str] = &[
    "load_features",
    "open_feature_file",
    "run_scenario",
    "get_test_results",
    "connect_test_client",
];

struct McpProbeSession {
    session_id: Option<HeaderValue>,
}

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &LaunchArgs,
) -> UseCaseResult<LaunchResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        target = ?args.target,
        "executing launch use case"
    );
    let (mode, utility, client_mode) = match args.target {
        LaunchTargetRequest::Designer => (
            LaunchMode::Designer,
            UtilityType::V8,
            LaunchClientMode::Designer,
        ),
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThinClient) => {
            (LaunchMode::Thin, UtilityType::V8C, LaunchClientMode::Thin)
        }
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThickClient) => {
            (LaunchMode::Thick, UtilityType::V8, LaunchClientMode::Thick)
        }
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::OrdinaryApplication) => (
            LaunchMode::Ordinary,
            UtilityType::V8,
            LaunchClientMode::Ordinary,
        ),
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { mode }) => {
            client_mcp_launch_shape(mode)
        }
    };

    if let Some(interruption) = context.interruption() {
        return Err(UseCaseFailure::without_payload(AppError::Runtime(format!(
            "{} for command '{}'",
            interruption_message(interruption),
            context.command().as_str()
        ))));
    }

    let launch = effective_launch_options(config, args)
        .map_err(|error| UseCaseFailure::without_payload(error))?;
    let readiness_url = client_mcp_readiness_url(config, args)
        .map_err(|error| UseCaseFailure::without_payload(error))?;
    let additional_launch_keys = effective_enterprise_launch_keys(config, args, &launch);
    let mut utilities = PlatformUtilities::from_config(config);
    let location = utilities
        .locate(utility)
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;
    let process_args = build_launch_args(
        client_mode,
        &config.v8_connection(),
        &additional_launch_keys,
        &launch,
    );

    debug!("[Запуск] Приложение: {}", mode_label(args.target));
    log_live_stage("launch: start", "[Launch] starting client process");
    let spawned = utilities
        .runner_for(utility)
        .spawn(&ProcessRequest {
            program: location.path.clone(),
            args: process_args,
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: Some(LAUNCH_STARTUP_PROBE),
        })
        .map_err(|error| UseCaseFailure::without_payload(AppError::from(error)))?;

    let mut result = LaunchResult {
        ok: true,
        mode,
        pid: Some(spawned.pid),
        binary: spawned.binary.clone(),
        message: Some(launch_message(config, args, &spawned.binary, spawned.pid)),
        mcp_readiness: None,
    };
    if let Some(url) = readiness_url {
        let required_tools = required_mcp_tools(args);
        match wait_for_mcp_readiness(context, &url, required_tools) {
            Ok(readiness) => {
                result.mcp_readiness = Some(readiness);
            }
            Err(readiness) => {
                let message = readiness
                    .message
                    .clone()
                    .unwrap_or_else(|| "MCP endpoint did not become ready".to_owned());
                result.ok = false;
                result.message = Some(format!(
                    "Launched {} via {} (pid {}) but {message}",
                    mode_label(args.target),
                    spawned.binary.display(),
                    spawned.pid
                ));
                result.mcp_readiness = Some(readiness);
                return Err(UseCaseFailure::with_payload(
                    AppError::Runtime(message),
                    result,
                ));
            }
        }
    }
    Ok(result)
}

fn client_mcp_readiness_url(
    config: &AppConfig,
    args: &LaunchArgs,
) -> Result<Option<String>, AppError> {
    let Some(client_mcp) = args.client_mcp.as_ref() else {
        return Ok(None);
    };
    if !client_mcp.wait_ready {
        return Ok(None);
    }
    let Some(port) = client_mcp.port.or(config.tools.client_mcp.port) else {
        return Err(AppError::Validation(
            "launch mcp --wait-ready requires --mcp-port or tools.client_mcp.port".to_owned(),
        ));
    };
    Ok(Some(format!("http://127.0.0.1:{port}{MCP_ENDPOINT_PATH}")))
}

fn required_mcp_tools(args: &LaunchArgs) -> &'static [&'static str] {
    if is_client_mcp_va_launch(args) {
        VANESSA_MCP_TOOLS
    } else {
        &[]
    }
}

fn wait_for_mcp_readiness(
    context: &ExecutionContext,
    url: &str,
    required_tools: &[&str],
) -> Result<McpReadinessResult, McpReadinessResult> {
    let timeout = context
        .remaining_budget()
        .filter(|budget| !budget.is_zero())
        .unwrap_or(MCP_READY_DEFAULT_TIMEOUT);
    let deadline = Instant::now() + timeout;
    let client = Client::builder()
        .timeout(MCP_READY_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| readiness_failure(url, Vec::new(), required_tools, error.to_string()))?;
    let mut last_message = "MCP endpoint did not become ready".to_owned();
    let mut last_tools = Vec::new();
    let mut last_missing = required_tools
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<Vec<_>>();
    let mut session: Option<McpProbeSession> = None;

    loop {
        if Instant::now() >= deadline {
            if let Some(session) = session.take() {
                delete_mcp_session(&client, url, session.session_id.as_ref());
            }
            return Err(readiness_failure_with_missing(
                url,
                last_tools,
                last_missing,
                last_message,
            ));
        }
        if let Some(interruption) = context.interruption() {
            let message = format!(
                "{} while waiting for MCP readiness",
                interruption_message(interruption)
            );
            if let Some(session) = session.take() {
                delete_mcp_session(&client, url, session.session_id.as_ref());
            }
            return Err(readiness_failure_with_missing(
                url,
                last_tools,
                last_missing,
                message,
            ));
        }

        if session.is_none() {
            match initialize_mcp_session(&client, url) {
                Ok(probe_session) => session = Some(probe_session),
                Err(message) => {
                    last_message = format!("MCP endpoint did not become ready at {url}: {message}");
                    last_tools = Vec::new();
                    last_missing = required_tools
                        .iter()
                        .map(|tool| (*tool).to_owned())
                        .collect();
                    sleep_until_next_mcp_probe(deadline);
                    continue;
                }
            }
        }

        let active_session = session.as_ref().expect("initialized MCP session");
        match list_mcp_tools(&client, url, active_session.session_id.as_ref()) {
            Ok(tools) => {
                let missing = missing_required_tools(&tools, required_tools);
                if missing.is_empty() {
                    if let Some(session) = session.take() {
                        delete_mcp_session(&client, url, session.session_id.as_ref());
                    }
                    return Ok(McpReadinessResult {
                        ok: true,
                        url: url.to_owned(),
                        tools,
                        missing_tools: Vec::new(),
                        message: Some("MCP endpoint is ready".to_owned()),
                    });
                }
                last_message = format!(
                    "Vanessa MCP tools were not registered: missing {}",
                    missing.join(", ")
                );
                last_tools = tools;
                last_missing = missing;
            }
            Err(message) => {
                last_message = format!("MCP endpoint did not become ready at {url}: {message}");
                last_tools = Vec::new();
                last_missing = required_tools
                    .iter()
                    .map(|tool| (*tool).to_owned())
                    .collect();
                if let Some(session) = session.take() {
                    delete_mcp_session(&client, url, session.session_id.as_ref());
                }
            }
        }

        sleep_until_next_mcp_probe(deadline);
    }
}

fn sleep_until_next_mcp_probe(deadline: Instant) {
    let sleep_for = MCP_READY_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
    if !sleep_for.is_zero() {
        std::thread::sleep(sleep_for);
    }
}

fn initialize_mcp_session(client: &Client, url: &str) -> Result<McpProbeSession, String> {
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "v8-runner",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    let (initialize_response, session_id) = post_json_rpc(client, url, &initialize, None)?;
    if initialize_response.get("error").is_some() {
        return Err(format!(
            "initialize failed: {}",
            initialize_response["error"]
        ));
    }
    send_mcp_initialized(client, url, session_id.as_ref())?;
    Ok(McpProbeSession { session_id })
}

fn list_mcp_tools(
    client: &Client,
    url: &str,
    session_id: Option<&HeaderValue>,
) -> Result<Vec<String>, String> {
    let tools_list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let (tools_response, _) = post_json_rpc(client, url, &tools_list, session_id)?;
    if tools_response.get("error").is_some() {
        return Err(format!("tools/list failed: {}", tools_response["error"]));
    }
    extract_tool_names(&tools_response)
}

fn send_mcp_initialized(
    client: &Client,
    url: &str,
    session_id: Option<&HeaderValue>,
) -> Result<(), String> {
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .json(&initialized);
    if let Some(session_id) = session_id {
        request = request.header("Mcp-Session-Id", session_id.clone());
    }
    let response = request.send().map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().map_err(|error| error.to_string())?;
        return Err(format!(
            "notifications/initialized failed with HTTP {status}: {body}"
        ));
    }
    Ok(())
}

fn delete_mcp_session(client: &Client, url: &str, session_id: Option<&HeaderValue>) {
    let Some(session_id) = session_id else {
        return;
    };
    let _ = client
        .delete(url)
        .header("Mcp-Session-Id", session_id.clone())
        .send();
}

fn post_json_rpc(
    client: &Client,
    url: &str,
    payload: &Value,
    session_id: Option<&HeaderValue>,
) -> Result<(Value, Option<HeaderValue>), String> {
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .json(payload);
    if let Some(session_id) = session_id {
        request = request.header("Mcp-Session-Id", session_id.clone());
    }
    let response = request.send().map_err(|error| error.to_string())?;
    let status = response.status();
    let response_session_id = response.headers().get("Mcp-Session-Id").cloned();
    let is_sse = response
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"));
    if !status.is_success() {
        let body = response.text().map_err(|error| error.to_string())?;
        return Err(format!("HTTP {status}: {body}"));
    }
    let value = if is_sse {
        read_first_sse_json(response)?
    } else {
        let body = response.text().map_err(|error| error.to_string())?;
        parse_json_or_sse(&body)?
    };
    Ok((value, response_session_id))
}

fn parse_json_or_sse(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).or_else(|json_error| {
        for event in body.split("\n\n").filter(|event| !event.trim().is_empty()) {
            if let Some(data) = sse_event_data(event) {
                return serde_json::from_str(&data).map_err(|error| error.to_string());
            }
        }
        Err(json_error.to_string())
    })
}

fn read_first_sse_json(mut response: reqwest::blocking::Response) -> Result<Value, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            let body = String::from_utf8_lossy(&bytes);
            return parse_json_or_sse(&body);
        }
        bytes.extend_from_slice(&buffer[..read]);
        while let Some((event_end, separator_len)) = sse_event_bounds(&bytes) {
            let event = String::from_utf8_lossy(&bytes[..event_end]);
            if let Some(data) = sse_event_data(&event) {
                return serde_json::from_str(&data).map_err(|error| error.to_string());
            }
            bytes.drain(..event_end + separator_len);
        }
    }
}

fn sse_event_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    let crlf = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
        (None, None) => None,
    }
}

fn sse_event_data(event: &str) -> Option<String> {
    let data = event
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        None
    } else {
        Some(data)
    }
}

fn extract_tool_names(response: &Value) -> Result<Vec<String>, String> {
    let tools = response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .ok_or_else(|| "tools/list response does not contain result.tools".to_owned())?;
    Ok(tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect())
}

fn missing_required_tools(tools: &[String], required_tools: &[&str]) -> Vec<String> {
    required_tools
        .iter()
        .filter(|required| !tools.iter().any(|tool| tool == *required))
        .map(|tool| (*tool).to_owned())
        .collect()
}

fn readiness_failure(
    url: &str,
    tools: Vec<String>,
    required_tools: &[&str],
    message: String,
) -> McpReadinessResult {
    readiness_failure_with_missing(
        url,
        tools,
        required_tools
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect(),
        message,
    )
}

fn readiness_failure_with_missing(
    url: &str,
    tools: Vec<String>,
    missing_tools: Vec<String>,
    message: String,
) -> McpReadinessResult {
    McpReadinessResult {
        ok: false,
        url: url.to_owned(),
        tools,
        missing_tools,
        message: Some(message),
    }
}

fn launch_message(config: &AppConfig, args: &LaunchArgs, binary: &Path, pid: u32) -> String {
    let mut message = format!(
        "Launched {} via {} (pid {})",
        mode_label(args.target),
        binary.display(),
        pid
    );
    if is_client_mcp_launch(args) {
        if let Some(hint) = tool_extension::client_mcp_build_hint(config) {
            message.push_str("; ");
            message.push_str(hint);
        }
    }
    message
}

fn is_client_mcp_launch(args: &LaunchArgs) -> bool {
    matches!(
        args.target,
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { .. })
    )
}

fn effective_enterprise_launch_keys(
    config: &AppConfig,
    args: &LaunchArgs,
    launch: &LaunchOptions,
) -> Vec<String> {
    if is_client_mcp_va_launch(args) {
        return vanessa_enterprise_launch_keys(
            &config.tools.enterprise.additional_launch_keys,
            launch,
        );
    }
    config.tools.enterprise.additional_launch_keys.clone()
}

fn is_client_mcp_va_launch(args: &LaunchArgs) -> bool {
    args.client_mcp.as_ref().is_some_and(|client_mcp| {
        matches!(
            client_mcp.addon,
            Some(ClientMcpAddonRequest::VanessaAutomation)
        )
    })
}

fn interruption_message(interruption: ExecutionInterruption) -> &'static str {
    match interruption {
        ExecutionInterruption::Cancelled => {
            "execution cancelled before reaching a safe completion point"
        }
        ExecutionInterruption::TimedOut => {
            "execution timeout expired before reaching a safe completion point"
        }
    }
}

fn mode_label(target: LaunchTargetRequest) -> &'static str {
    match target {
        LaunchTargetRequest::Designer => "конфигуратор",
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThinClient) => "тонкий клиент",
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ThickClient) => "толстый клиент",
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::OrdinaryApplication) => {
            "обычное приложение"
        }
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { .. }) => {
            "клиентский MCP-сервер"
        }
    }
}

fn client_mcp_launch_shape(mode: ClientMcpMode) -> (LaunchMode, UtilityType, LaunchClientMode) {
    match mode {
        ClientMcpMode::Thin => (LaunchMode::Mcp, UtilityType::V8C, LaunchClientMode::Thin),
        ClientMcpMode::Thick => (LaunchMode::Mcp, UtilityType::V8, LaunchClientMode::Thick),
        ClientMcpMode::Ordinary => (LaunchMode::Mcp, UtilityType::V8, LaunchClientMode::Ordinary),
    }
}

fn effective_launch_options(
    config: &AppConfig,
    args: &LaunchArgs,
) -> Result<LaunchOptions, AppError> {
    let is_client_mcp = matches!(
        args.target,
        LaunchTargetRequest::Enterprise(EnterpriseLaunchTarget::ClientMcp { .. })
    );
    let Some(client_mcp) = args.client_mcp.as_ref() else {
        return if is_client_mcp {
            Err(AppError::Validation(
                "launch mcp requires client_mcp options".to_owned(),
            ))
        } else {
            Ok(args.launch.clone())
        };
    };
    if !is_client_mcp {
        return Err(AppError::Validation(
            "client_mcp options are supported only for launch mcp".to_owned(),
        ));
    }

    let mut launch = args.launch.clone();
    let mut payload = build_client_mcp_payload(client_mcp, config.tools.client_mcp.port);
    if matches!(
        client_mcp.addon,
        Some(ClientMcpAddonRequest::VanessaAutomation)
    ) {
        let va_launch = crate::use_cases::vanessa::prepare_client_mcp_launch(config)?;
        crate::use_cases::vanessa::apply_client_mcp_launch(&mut launch, &mut payload, &va_launch);
    }
    launch.c = Some(payload);
    Ok(launch)
}

fn build_client_mcp_payload(
    options: &ClientMcpOptionsRequest,
    configured_port: Option<u16>,
) -> String {
    let mut payload = match options.config_path.as_deref() {
        Some(path) => format!("runMcp={}", normalize_launch_payload_path(Path::new(path))),
        None => "runMcp".to_owned(),
    };
    if let Some(port) = options.port.or(configured_port) {
        payload.push_str(&format!(";mcpPort={port}"));
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::execute;
    use crate::config::model::{
        AppConfig, BuildConfig, BuilderBackend, EnterpriseToolConfig, PlatformToolConfig,
        SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolExtensionArtifactConfig,
        ToolExtensionConfig, ToolExtensionInput, ToolsConfig,
    };
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::{
        ClientMcpMode, ClientMcpOptionsRequest, LaunchRequest, LaunchTargetRequest,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(path);
    }

    fn read_args_log(path: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(args) = fs::read_to_string(path) {
                return args;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::read_to_string(path).expect("args log")
    }

    fn sample_config(base_path: &Path, work_path: &Path, platform_path: &Path) -> AppConfig {
        AppConfig {
            base_path: base_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            execution_timeout: 300_000,
            format: SourceFormat::Designer,
            builder: BuilderBackend::Designer,
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("."),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(platform_path.to_path_buf()),
                    version: None,
                },
                enterprise: EnterpriseToolConfig::default(),
                edt_cli: Default::default(),
                ..Default::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn thin_launch_app_appends_enterprise_additional_keys() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("thin.args.log");
        let platform_dir = dir.path().join("platform");
        write_script(
            &platform_dir.join("bin").join("1cv8c"),
            &format!("printf '%s\n' \"$@\" > '{}'\nsleep 1", args_log.display()),
        );

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.enterprise.additional_launch_keys = vec!["/TESTMANAGER".to_owned()];

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::thin_client(),
                launch: Default::default(),
                client_mcp: None,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        let args = read_args_log(&args_log);
        assert!(args.contains("ENTERPRISE"));
        assert!(args.contains("/TESTMANAGER"));
    }

    #[cfg(unix)]
    #[test]
    fn designer_launch_app_does_not_append_enterprise_additional_keys() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("designer.args.log");
        let platform_dir = dir.path().join("platform");
        write_script(
            &platform_dir.join("bin").join("1cv8"),
            &format!("printf '%s\n' \"$@\" > '{}'\nsleep 1", args_log.display()),
        );

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.enterprise.additional_launch_keys = vec!["/TESTMANAGER".to_owned()];

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::designer(),
                launch: Default::default(),
                client_mcp: None,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        let args = read_args_log(&args_log);
        assert!(args.contains("DESIGNER"));
        assert!(args.contains("/DisableStartupDialogs"));
        assert!(!args.contains("/TESTMANAGER"));
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_launch_app_uses_enterprise_binary_and_ordinary_mode_key() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("ordinary.args.log");
        let platform_dir = dir.path().join("platform");
        write_script(
            &platform_dir.join("bin").join("1cv8"),
            &format!("printf '%s\n' \"$@\" > '{}'\nsleep 1", args_log.display()),
        );

        let config = sample_config(dir.path(), dir.path(), &platform_dir);

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::ordinary_application(),
                launch: Default::default(),
                client_mcp: None,
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        let args = read_args_log(&args_log);
        assert!(args.contains("ENTERPRISE"));
        assert!(args.contains("/RunModeOrdinaryApplication"));
        assert!(args.contains("/DisableStartupDialogs"));
    }

    #[cfg(unix)]
    #[test]
    fn client_mcp_launch_does_not_prepare_configured_tool_extension() {
        let dir = tempdir().expect("tempdir");
        let args_log = dir.path().join("mcp.args.log");
        let platform_dir = dir.path().join("platform");
        write_script(
            &platform_dir.join("bin").join("1cv8c"),
            &format!("printf '%s\n' \"$@\" > '{}'\nsleep 1", args_log.display()),
        );

        let mut config = sample_config(dir.path(), dir.path(), &platform_dir);
        config.tools.client_mcp.port = Some(9874);
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Artifact(ToolExtensionArtifactConfig {
                path: dir.path().join("client_mcp.cfe"),
            }),
        });

        let result = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest::default()),
            },
        )
        .expect("launch succeeds");

        assert!(result.ok);
        assert!(result
            .message
            .as_deref()
            .expect("message")
            .contains("v8-runner build"));
        let args = read_args_log(&args_log);
        assert!(args.contains("ENTERPRISE"));
        assert!(args.contains("/C\"runMcp;mcpPort=9874\""));
        assert!(!args.contains("/LoadCfg"));
        assert!(!args.contains("-Extension"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_inconsistent_client_mcp_request_state_before_locating_platform() {
        let dir = tempdir().expect("tempdir");
        let platform_dir = dir.path().join("missing-platform");
        let config = sample_config(dir.path(), dir.path(), &platform_dir);

        let missing_options = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: None,
            },
        )
        .expect_err("client_mcp options are required");
        assert!(
            missing_options
                .error
                .to_string()
                .contains("launch mcp requires client_mcp options"),
            "{missing_options:?}"
        );

        let unexpected_options = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::thin_client(),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest::default()),
            },
        )
        .expect_err("client_mcp options are rejected for non-mcp launch");
        assert!(
            unexpected_options
                .error
                .to_string()
                .contains("client_mcp options are supported only for launch mcp"),
            "{unexpected_options:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wait_ready_requires_effective_client_mcp_port_before_locating_platform() {
        let dir = tempdir().expect("tempdir");
        let platform_dir = dir.path().join("missing-platform");
        let config = sample_config(dir.path(), dir.path(), &platform_dir);

        let error = execute(
            &ExecutionContext::cli(CommandName::Launch),
            &config,
            &LaunchRequest {
                target: LaunchTargetRequest::client_mcp_with_mode(ClientMcpMode::Thin),
                launch: Default::default(),
                client_mcp: Some(ClientMcpOptionsRequest {
                    wait_ready: true,
                    ..ClientMcpOptionsRequest::default()
                }),
            },
        )
        .expect_err("wait-ready without port should fail before platform lookup");

        assert!(
            error
                .error
                .to_string()
                .contains("launch mcp --wait-ready requires --mcp-port or tools.client_mcp.port"),
            "{error:?}"
        );
    }
}
