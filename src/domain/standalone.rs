//! Ответ команд `standalone`: что стало с сервером `ibsrv` объявленной базы.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::dbms::{ServiceAction, ServiceState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct StandaloneResult {
    pub ok: bool,
    pub infobase: String,
    pub state: ServiceState,
    pub action: ServiceAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Строка соединения прямого шлюза.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Адрес SSH-шлюза.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
    pub ready: bool,
    /// Запись была, но процесса из неё уже нет.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale_record: bool,
    /// Чужой процесс на порту записи.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occupant_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_pid: Option<u32>,
    /// PostgreSQL, погашенный вместе с сервером, потому что поднят его шагом.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_dbms_pid: Option<u32>,
    pub duration_ms: u64,
}
