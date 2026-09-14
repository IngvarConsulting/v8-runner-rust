use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::artifact::ArtifactSet;

/// Shared execution status used by runner and package-like flows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    InvalidOutput,
}

impl ExecutionStatus {
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

/// Shared counters emitted by parsers and execution adapters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct ExecutionMetrics {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub errors: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, u64>,
}

/// Shared timeout budget for execution scenarios.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema)]
pub struct ExecutionTimeouts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
}

/// Structured execution error that can point to related artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExecutionError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<crate::domain::artifact::ArtifactRef>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retryable: bool,
}

impl ExecutionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Vec::new(),
            artifact: None,
            retryable: false,
        }
    }

    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }
}

/// Command-level interruption kind preserved in serialized execution results.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionInterruptionKind {
    Cancelled,
    TimedOut,
}

/// Structured interruption metadata for actual or deferred command-boundary interruptions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExecutionInterruptionDetails {
    pub kind: ExecutionInterruptionKind,
    pub deferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ExecutionInterruptionDetails {
    pub fn new(kind: ExecutionInterruptionKind, deferred: bool) -> Self {
        Self {
            kind,
            deferred,
            phase: None,
            message: None,
        }
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// Stable pipeline vocabulary for significant execution blocks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStepKind {
    Validation,
    ResolveTarget,
    PrepareWorkspace,
    PlatformCommand,
    ParseOutput,
    Publish,
    Cleanup,
    Diagnostics,
    Other,
}

/// Richer step status beyond the legacy boolean `ok`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStepStatus {
    Succeeded,
    Failed,
    Skipped,
    Degraded,
}

impl ExecutionStepStatus {
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Succeeded | Self::Skipped | Self::Degraded)
    }
}

/// A transport-neutral execution step shared by CLI envelopes and use-case payloads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub status: ExecutionStepStatus,
    pub kind: ExecutionStepKind,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ExecutionError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<ArtifactSet>,
}

impl StepResult {
    pub fn new(
        name: impl Into<String>,
        kind: ExecutionStepKind,
        status: ExecutionStepStatus,
        duration_ms: u64,
    ) -> Self {
        Self {
            name: name.into(),
            ok: status.is_ok(),
            status,
            kind,
            duration_ms,
            target: None,
            message: None,
            diagnostics: Vec::new(),
            errors: Vec::new(),
            artifacts: None,
        }
    }

    pub fn succeeded(name: impl Into<String>, kind: ExecutionStepKind, duration_ms: u64) -> Self {
        Self::new(name, kind, ExecutionStepStatus::Succeeded, duration_ms)
    }

    pub fn failed(name: impl Into<String>, kind: ExecutionStepKind, duration_ms: u64) -> Self {
        Self::new(name, kind, ExecutionStepStatus::Failed, duration_ms)
    }

    pub fn degraded(name: impl Into<String>, kind: ExecutionStepKind, duration_ms: u64) -> Self {
        Self::new(name, kind, ExecutionStepStatus::Degraded, duration_ms)
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<String>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn with_errors(mut self, errors: Vec<ExecutionError>) -> Self {
        self.errors = errors;
        self
    }
}

/// Shared execution envelope for runner-like flows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ExecutionOutcome<T> {
    pub status: ExecutionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ExecutionError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<ExecutionMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<ArtifactSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interruptions: Vec<ExecutionInterruptionDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<T>,
}

impl<T> Default for ExecutionOutcome<T> {
    fn default() -> Self {
        Self::new(ExecutionStatus::Succeeded)
    }
}

impl<T> ExecutionOutcome<T> {
    pub fn new(status: ExecutionStatus) -> Self {
        Self {
            status,
            diagnostics: Vec::new(),
            errors: Vec::new(),
            metrics: None,
            artifacts: None,
            interruptions: Vec::new(),
            payload: None,
        }
    }

    pub const fn is_ok(&self) -> bool {
        self.status.is_ok()
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<String>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn with_errors(mut self, errors: Vec<ExecutionError>) -> Self {
        self.errors = errors;
        self
    }

    pub fn with_metrics(mut self, metrics: ExecutionMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn with_artifacts(mut self, artifacts: ArtifactSet) -> Self {
        self.artifacts = Some(artifacts);
        self
    }

    pub fn with_interruptions(mut self, interruptions: Vec<ExecutionInterruptionDetails>) -> Self {
        self.interruptions = interruptions;
        self
    }

    pub fn with_payload(mut self, payload: T) -> Self {
        self.payload = Some(payload);
        self
    }
}
