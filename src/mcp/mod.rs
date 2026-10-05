/// Per-call MCP service context.
pub mod context;
/// MCP-specific EDT syntax execution over the shared actor.
/// MCP-facing error and result contracts.
pub mod error;
/// Thin port used by the MCP service layer to call use cases.
pub mod port;
/// MCP-facing request DTOs.
pub mod request;
/// MCP stdio and HTTP transport adapters.
pub mod server;
/// MCP-facing service layer over transport-neutral use cases.
pub mod service;
/// Runtime telemetry for MCP admission and shared EDT lifecycle.
pub mod telemetry;
