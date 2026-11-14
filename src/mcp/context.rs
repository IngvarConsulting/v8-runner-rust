use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::use_cases::context::{CommandLineTarget, ExecutionTransport};

/// Per-call metadata passed into the MCP service layer.
#[derive(Debug, Clone)]
pub struct McpCallContext {
    transport: ExecutionTransport,
    edt_timeout: Option<Duration>,
    cancellation: CancellationToken,
    /// Второй сигнал вызова — повторная отмена того же запроса: перестать ждать команду
    /// чужого агента, которую раннер снять не может.
    abandonment: CancellationToken,
    /// Глобальные ключи, с которыми запущен сервер: совет отказа называет команду строки,
    /// и она должна попасть в тот же проект, ту же базу и тот же рабочий каталог.
    command_line: CommandLineTarget,
    /// Текущий каталог процессов платформы без своего: `workPath/temp` проекта сервера.
    start_dir: Option<std::path::PathBuf>,
}

impl McpCallContext {
    /// Creates a new MCP call context for the specified transport.
    pub fn new(transport: ExecutionTransport) -> Self {
        Self {
            transport,
            edt_timeout: None,
            cancellation: CancellationToken::new(),
            abandonment: CancellationToken::new(),
            command_line: CommandLineTarget::default(),
            start_dir: None,
        }
    }

    /// Процессы платформы без своего текущего каталога начинают в `dir`.
    pub fn with_start_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.start_dir = Some(dir);
        self
    }

    /// Текущий каталог процессов платформы без своего, если он назван.
    pub fn start_dir(&self) -> Option<&std::path::Path> {
        self.start_dir.as_deref()
    }

    /// Creates a stdio MCP call context.
    pub fn stdio() -> Self {
        Self::new(ExecutionTransport::McpStdio)
    }

    /// Creates an HTTP MCP call context.
    pub fn http() -> Self {
        Self::new(ExecutionTransport::McpHttp)
    }

    /// Returns the originating transport.
    pub const fn transport(&self) -> ExecutionTransport {
        self.transport
    }

    /// Attaches an EDT subprocess timeout budget for this call.
    pub fn with_edt_timeout(mut self, edt_timeout: Option<Duration>) -> Self {
        self.edt_timeout = edt_timeout;
        self
    }

    /// Returns the EDT subprocess timeout budget for this call, if any.
    pub const fn edt_timeout(&self) -> Option<Duration> {
        self.edt_timeout
    }

    /// Attaches a shared cancellation token to the call.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Привязывает второй сигнал вызова.
    pub fn with_abandonment(mut self, abandonment: CancellationToken) -> Self {
        self.abandonment = abandonment;
        self
    }

    /// Второй сигнал вызова.
    pub fn abandonment(&self) -> CancellationToken {
        self.abandonment.clone()
    }

    /// Attaches the global keys the server was started with.
    pub fn with_command_line(mut self, command_line: CommandLineTarget) -> Self {
        self.command_line = command_line;
        self
    }

    /// The global keys the server was started with.
    pub const fn command_line(&self) -> &CommandLineTarget {
        &self.command_line
    }

    /// Returns the shared cancellation token for the call.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
