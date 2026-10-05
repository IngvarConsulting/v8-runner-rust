//! Один вызов инструмента MCP из синхронного теста.
//!
//! Командная строка просит полную выгрузку только с согласием (`pull --force`); выгрузку,
//! которая сначала спрашивает систему контроля версий, сегодня просит MCP — у него согласия
//! взять неоткуда. Тесты сторожа замены идут через этот вызов.

use std::path::Path;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::Value;

use super::v8_runner_binary;

/// Ответ инструмента: признак ошибки и структурированный конверт.
pub struct ToolAnswer {
    pub is_error: bool,
    pub envelope: Value,
}

/// Запускает `mcp serve stdio` с файлом настроек, вызывает инструмент и закрывает сервер.
pub fn call_tool(config: &Path, tool: &str, arguments: Value) -> ToolAnswer {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let transport = TokioChildProcess::new(
            tokio::process::Command::new(v8_runner_binary()).configure(|command| {
                command
                    .arg("--config")
                    .arg(config)
                    .args(["mcp", "serve", "stdio"]);
            }),
        )
        .expect("spawn stdio transport");
        let client = ().serve(transport).await.expect("connect rmcp client");
        let response = client
            .peer()
            .call_tool(
                CallToolRequestParams::new(tool.to_owned())
                    .with_arguments(serde_json::from_value(arguments).expect("tool arguments")),
            )
            .await
            .expect("call tool");
        client.cancel().await.expect("cancel client");
        ToolAnswer {
            is_error: response.is_error == Some(true),
            envelope: response.structured_content.expect("structured content"),
        }
    })
}
