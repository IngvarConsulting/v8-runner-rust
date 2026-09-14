use std::path::PathBuf;

use crate::domain::issue::Issue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyntaxCheckStatus {
    Clean,
    IssuesFound,
    ToolFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SyntaxIssueSummary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SyntaxCheckResult {
    pub status: SyntaxCheckStatus,
    pub exit_code: i32,
    pub check_name: String,
    pub issues: Vec<Issue>,
    pub summary: SyntaxIssueSummary,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_read_warning: Option<String>,
}
