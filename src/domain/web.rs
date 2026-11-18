//! Ответ команд `web`: что стало с частным Apache кластера объявленной базы.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::dbms::{ServiceAction, ServiceState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct WebResult {
    pub ok: bool,
    pub infobase: String,
    pub state: ServiceState,
    pub action: ServiceAction,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<PathBuf>,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occupant_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_port: Option<u16>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub publication_stale: bool,
    pub duration_ms: u64,
}
