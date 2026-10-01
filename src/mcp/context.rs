use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::use_cases::context::ExecutionTransport;

/// Per-call metadata passed into the MCP service layer.
#[derive(Debug, Clone)]
pub struct McpCallContext {
    transport: ExecutionTransport,
    edt_timeout: Option<Duration>,
    cancellation: CancellationToken,
}

impl McpCallContext {
    /// Creates a new MCP call context for the specified transport.
    pub fn new(transport: ExecutionTransport) -> Self {
        Self {
            transport,
            edt_timeout: None,
            cancellation: CancellationToken::new(),
        }
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

    /// Returns the shared cancellation token for the call.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
