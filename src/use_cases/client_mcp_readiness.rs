use std::io::Read;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::HeaderValue;
use serde_json::{json, Value};

use crate::domain::launch::McpReadinessResult;
use crate::use_cases::context::{ExecutionContext, ExecutionInterruption};

const MCP_READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MCP_READY_REQUEST_TIMEOUT: Duration = Duration::from_millis(300);
const MCP_READY_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_ENDPOINT_PATH: &str = "/mcp";

pub(in crate::use_cases) const VANESSA_MCP_TOOLS: &[&str] = &[
    "load_features",
    "open_feature_file",
    "run_scenario",
    "get_test_results",
    "connect_test_client",
];

struct McpProbeSession {
    session_id: Option<HeaderValue>,
}

pub(in crate::use_cases) fn endpoint_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}{MCP_ENDPOINT_PATH}")
}

pub(in crate::use_cases) fn wait_for_readiness(
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
                interruption_message(context, interruption)
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

        let Some(active_session) = session.as_ref() else {
            continue;
        };
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

fn interruption_message(
    context: &ExecutionContext,
    interruption: ExecutionInterruption,
) -> &'static str {
    interruption.message(context.command())
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
        delete_mcp_session(client, url, session_id.as_ref());
        return Err(format!(
            "initialize failed: {}",
            initialize_response["error"]
        ));
    }
    if let Err(message) = send_mcp_initialized(client, url, session_id.as_ref()) {
        delete_mcp_session(client, url, session_id.as_ref());
        return Err(message);
    }
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

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    use crate::use_cases::context::CommandName;

    use super::*;

    #[test]
    fn deletes_session_when_initialized_notification_fails() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake MCP server");
        let port = listener.local_addr().expect("local addr").port();
        let deleted = Arc::new(AtomicBool::new(false));
        let server_deleted = Arc::clone(&deleted);

        let server = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("fake MCP nonblocking listener");
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_http_request(&mut stream);
                        if request.starts_with("DELETE ") {
                            assert!(
                                request.lines().any(|line| line
                                    .eq_ignore_ascii_case("Mcp-Session-Id: fake-session")),
                                "DELETE should include the initialized MCP session id"
                            );
                            server_deleted.store(true, Ordering::SeqCst);
                            write_empty_response(&mut stream, "202 Accepted");
                            break;
                        }
                        if request.contains("\"method\":\"initialize\"")
                            || request.contains("\"method\": \"initialize\"")
                        {
                            write_json_response(
                                &mut stream,
                                "200 OK",
                                Some("fake-session"),
                                &json!({
                                    "jsonrpc": "2.0",
                                    "id": 1,
                                    "result": {
                                        "protocolVersion": "2025-11-25",
                                        "capabilities": {},
                                        "serverInfo": { "name": "fake-client-mcp", "version": "1" }
                                    }
                                }),
                            );
                        } else if request.contains("notifications/initialized") {
                            write_empty_response(&mut stream, "500 Internal Server Error");
                        } else {
                            write_empty_response(&mut stream, "404 Not Found");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() <= deadline,
                            "fake MCP server timed out waiting for DELETE"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("fake MCP accept failed: {error}"),
                }
            }
        });

        let context = ExecutionContext::cli(CommandName::Launch)
            .with_deadline(Some(Instant::now() + Duration::from_millis(350)));

        let readiness = wait_for_readiness(&context, &endpoint_url(port), &[]);

        assert!(readiness.is_err());
        server.join().expect("fake MCP server exits");
        assert!(
            deleted.load(Ordering::SeqCst),
            "client should delete initialized session after notification failure"
        );
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set read timeout");
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let read = stream.read(&mut buffer).expect("read request");
            assert!(read != 0, "request closed before body was read");
            bytes.extend_from_slice(&buffer[..read]);
            let Some(header_end) = find_header_end(&bytes) else {
                continue;
            };
            let content_length = content_length(&bytes[..header_end]);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn find_header_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn content_length(headers: &[u8]) -> usize {
        String::from_utf8_lossy(headers)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or(0)
    }

    fn write_empty_response(stream: &mut TcpStream, status: &str) {
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .expect("write response");
    }

    fn write_json_response(
        stream: &mut TcpStream,
        status: &str,
        session_id: Option<&str>,
        body: &Value,
    ) {
        let body = serde_json::to_vec(body).expect("response json");
        let session_header = session_id
            .map(|session_id| format!("Mcp-Session-Id: {session_id}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{session_header}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .expect("write headers");
        stream.write_all(&body).expect("write body");
    }
}
