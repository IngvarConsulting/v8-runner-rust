//! Ответ команд `dbms`: что стало с PostgreSQL объявленной базы.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// В каком состоянии служба сейчас.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    NotRunning,
    Starting,
    StartedByThisCopy,
    /// Порт записи слушает процесс, которого раннер этой копии не запускал.
    Foreign,
}

/// Что сделала команда.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Started,
    AlreadyRunning,
    Stopped,
    NothingToStop,
    Reported,
    Planned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DbmsResult {
    pub ok: bool,
    pub infobase: String,
    pub state: ServiceState,
    pub action: ServiceAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Значение для `--db-server`: `127.0.0.1 port=<порт>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Путь к файлу пароля. Значения в ответе нет
    /// (`INV.PLATFORM.MANAGED-POSTGRES-PASSWORD-STAYS-OUT-OF-THE-OUTPUT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
    /// Поднята явно, а не шагом сервера; `None`, пока службы нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit: Option<bool>,
    pub ready: bool,
    /// Запись была, но процесса из неё уже нет.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale_record: bool,
    /// Чужой процесс на порту записи.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occupant_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_pid: Option<u32>,
    pub duration_ms: u64,
}
