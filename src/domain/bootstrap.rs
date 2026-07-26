use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResult {
    pub ok: bool,
    pub path: PathBuf,
    pub local_path: PathBuf,
    pub gitignore_path: PathBuf,
    pub source_dir: PathBuf,
    pub dump_target_path: PathBuf,
    pub dumped: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub duration_ms: u64,
}
