//! Ответ команд `debug-server`: сервер отладки 1С для отладчика — свой `dbgs` раннера или
//! сервер отладки кластера.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::launch::PlatformResolution;

/// Сеанс сервера отладки: 32 шестнадцатеричных знака в нижнем регистре. Он становится именем
/// каталога сеанса, поэтому иное — отказ разбора, а не путь; верхний регистр читается как тот же
/// сеанс.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DebugSession(u128);

impl DebugSession {
    /// Новый случайный сеанс.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().as_u128())
    }
}

impl Default for DebugSession {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DebugSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl std::str::FromStr for DebugSession {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("a session is the 32 hex digits `debug-server start` answered".to_owned());
        }
        u128::from_str_radix(value, 16)
            .map(Self)
            .map_err(|error| error.to_string())
    }
}

impl Serialize for DebugSession {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DebugSession {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for DebugSession {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DebugSession".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-f]{32}$",
            "description": "Сеанс сервера отладки: его передают в `debug-server stop`."
        })
    }
}

/// Кто поднял сервер отладки и кто его гасит.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerOwner {
    /// `dbgs`, которого поднял раннер для этого сеанса; `stop` его гасит.
    Runner,
    /// Сервер отладки управляемого кластера (`cluster start --debug`); `stop` его не трогает.
    Cluster,
    /// Сервер отладки управляемого автономного сервера (`standalone start --debug`); `stop` его
    /// не трогает.
    Standalone,
}

/// Что сделала команда `debug-server`. Сервер отладки не бывает «уже работающим» и не
/// описывается отчётом: у `debug-server` нет ни `status`, ни повторного `start` того же сеанса.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerAction {
    /// `start`: сервер отвечает, сеанс записан.
    Started,
    /// `stop`: свой `dbgs` сеанса остановлен.
    Stopped,
    /// `stop`: гасить нечего — сеанс неизвестен, сервер вышел сам или принадлежит стенду.
    NothingToStop,
    /// Предварительный просмотр: ничего не запущено и не остановлено.
    Planned,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DebugServerResult {
    pub ok: bool,
    /// `started` или `planned` у `start`; `stopped`, `nothing_to_stop` или `planned` у `stop`.
    pub action: DebugServerAction,
    /// Сеанс: его передают в `debug-server stop`. У предварительного просмотра `start` его нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<DebugSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<DebugServerOwner>,
    /// Имя базы, под которым отладчик видит её цели (`infoBaseAlias` запросов `rdbg`):
    /// `DefAlias` у файловой базы, имя базы в сервере у автономного сервера. У сервера кластера
    /// поля нет: не замерено.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infobase_alias: Option<String>,
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
