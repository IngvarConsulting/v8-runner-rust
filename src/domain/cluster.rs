//! Ответ команд `cluster`: что стало с кластером объявленной базы — агентом `ragent` с его
//! процессами и сервером администрирования `ras`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::dbms::{ServiceAction, ServiceState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ClusterResult {
    pub ok: bool,
    pub infobase: String,
    pub state: ServiceState,
    pub action: ServiceAction,
    /// PID агента `ragent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// PID сервера администрирования `ras`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ras_pid: Option<u32>,
    /// Версия платформы кластера.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Строка соединения с базой кластера.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// Адрес агента.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Адрес `ras` для `rac`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ras: Option<String>,
    /// Диапазон портов рабочих процессов.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_range: Option<String>,
    /// Каталог реестра кластера.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
    /// Ответил ли `rac cluster list`.
    pub ready: bool,
    /// Создана ли база в этом реестре.
    pub registered: bool,
    /// Запись была, но процесса из неё уже нет.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale_record: bool,
    /// Процессы `rmngr` и `rphost` упавшего агента, ещё живые.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remnant_pids: Vec<u32>,
    /// Чужой процесс на закреплённом порту.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occupant_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_ras_pid: Option<u32>,
    /// Остатки упавшего агента, снятые по их pid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stopped_remnant_pids: Vec<u32>,
    /// PostgreSQL, погашенный вместе с кластером, потому что поднят его шагом.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_dbms_pid: Option<u32>,
    /// `regport` прежнего реестра, если команда завела новый: по `--new-regport` или сама,
    /// потому что закреплённый порт занят чужим процессом.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_regport: Option<u16>,
    pub duration_ms: u64,
    /// Предупреждения идут только в `warnings` конверта: форма `data` их не несёт.
    #[serde(skip)]
    pub warnings: Vec<String>,
}
