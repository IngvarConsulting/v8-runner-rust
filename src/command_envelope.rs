use serde::Serialize;

use crate::domain::execution::StepResult;
use crate::domain::test::{
    RetainedPaths, TestErrorKind, TestOutputMode, TestReport, TestRunResult, TestTarget,
};

/// Structured business error metadata carried by machine-readable command envelopes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EnvelopeError {
    pub code: String,
    pub kind: String,
    pub message: String,
}

impl EnvelopeError {
    pub fn new(
        code: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// Shared machine-readable command payload for CLI JSON and MCP structured content.
#[derive(Debug, Clone, Serialize)]
pub struct Envelope<T: Serialize> {
    pub ok: bool,
    pub command: String,
    pub duration_ms: u64,
    pub data: T,
    pub warnings: Vec<String>,
    pub steps: Vec<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EnvelopeError>,
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(command: impl Into<String>, duration_ms: u64, data: T) -> Self {
        Self {
            ok: true,
            command: command.into(),
            duration_ms,
            data,
            warnings: vec![],
            steps: vec![],
            error: None,
        }
    }

    pub fn err(command: impl Into<String>, duration_ms: u64, data: T) -> Self {
        Self {
            ok: false,
            command: command.into(),
            duration_ms,
            data,
            warnings: vec![],
            steps: vec![],
            error: None,
        }
    }

    pub fn with_error(mut self, error: EnvelopeError) -> Self {
        self.error = Some(error);
        self
    }
}

/// Shared JSON data projection for test command envelopes.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct TestEnvelopeData {
    pub ok: bool,
    pub target: TestTarget,
    pub mode: TestOutputMode,
    pub error_kind: Option<TestErrorKind>,
    pub diagnostics: Vec<String>,
    pub retained_paths: Option<RetainedPaths>,
    pub report: Option<TestReport>,
    pub execution: crate::domain::execution::ExecutionOutcome<TestReport>,
}

impl TestEnvelopeData {
    pub fn from_result(result: &TestRunResult) -> Self {
        let execution = &result.execution;
        Self {
            ok: execution.is_ok(),
            target: result.target.clone(),
            mode: result.mode.clone(),
            error_kind: execution
                .errors
                .first()
                .and_then(|error| TestErrorKind::from_code(&error.code)),
            diagnostics: execution.diagnostics.clone(),
            retained_paths: execution
                .artifacts
                .as_ref()
                .and_then(RetainedPaths::from_artifact_set),
            report: execution.payload.clone(),
            execution: execution.clone(),
        }
    }
}

pub fn test_envelope(result: &TestRunResult) -> Envelope<TestEnvelopeData> {
    Envelope {
        ok: result.execution.is_ok(),
        command: "test".to_owned(),
        duration_ms: result.duration_ms,
        warnings: result.warnings.clone(),
        steps: result.steps.clone(),
        error: None,
        data: TestEnvelopeData::from_result(result),
    }
}
