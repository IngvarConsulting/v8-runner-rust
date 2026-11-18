//! Ответ команд `debug-server`: сервер отладки 1С для отладчика — свой `dbgs` раннера или
//! сервер отладки кластера.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::dbms::ServiceAction;
use crate::domain::launch::PlatformResolution;

/// Кто поднял сервер отладки и кто его гасит.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerOwner {
    /// `dbgs`, которого поднял раннер для этого сеанса; `stop` его гасит.
    Runner,
    /// Сервер отладки управляемого кластера (`cluster start --debug`); `stop` его не трогает.
    Cluster,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DebugServerResult {
    pub ok: bool,
    /// `started` или `planned` у `start`; `stopped`, `nothing_to_stop` или `planned` у `stop`.
    pub action: ServiceAction,
    /// Сеанс: его передают в `debug-server stop`. У предварительного просмотра `start` его нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<DebugServerOwner>,
    /// Адрес для `/DEBUGGERURL` клиента и для отладчика: `http://127.0.0.1:<порт>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Процесс `dbgs` раннера; у сервера кластера его нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Процесс, с концом которого `dbgs` выходит сам (`--owner-pid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    /// Установка, из которой взят `dbgs`: та же, из которой запускается тонкий клиент.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_resolution: Option<PlatformResolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
    /// `stop`: раннер знает этот сеанс. `false` — записи нет: сеанс уже остановлен или
    /// запущен из другой рабочей копии.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_session: Option<bool>,
    /// Запись была, но процесса из неё уже нет: сервер вышел сам, например с концом владельца.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale_record: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_pid: Option<u32>,
    pub duration_ms: u64,
    /// Предупреждения идут только в `warnings` конверта: форма `data` их не несёт.
    #[serde(skip)]
    pub warnings: Vec<String>,
}
