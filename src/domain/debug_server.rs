//! Ответ команд `debug-server`: сервер отладки 1С для отладчика — свой `dbgs` раннера или
//! сервер отладки кластера.

use serde::{Deserialize, Serialize};

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

    /// Leases keep the existing token shape, with a version nibble reserved for prepare.
    pub(crate) fn new_prepare() -> Self {
        let random = uuid::Uuid::new_v4().as_u128();
        Self((random & !(0xf << 76)) | (8 << 76))
    }

    pub(crate) fn is_prepare(self) -> bool {
        (self.0 >> 76) & 0xf == 8
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
}

/// Что сделала команда `debug-server stop`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerAction {
    /// `stop`: свой `dbgs` сеанса остановлен.
    Stopped,
    /// `stop`: одна lease освобождена, остальные ещё используют процесс.
    Released,
    /// `stop`: аренда неизвестна или процесс уже завершился.
    NothingToStop,
    /// Предварительный просмотр: ничего не запущено и не остановлено.
    Planned,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DebugServerResult {
    pub ok: bool,
    /// Исход освобождения аренды.
    pub action: DebugServerAction,
    /// Аренда, переданная в `debug-server stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<DebugSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<DebugServerOwner>,
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerPrepareScope {
    Client,
    ClientAndServer,
}

impl DebugServerPrepareScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::ClientAndServer => "client_and_server",
        }
    }
}

/// Who owns the process selected by start. External servers are borrowed.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerPrepareOwner {
    Runner,
    Cluster,
    // Reserved until standalone ownership can be confirmed.
    #[allow(dead_code)]
    Standalone,
    External,
}

impl DebugServerPrepareOwner {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runner => "runner",
            Self::Cluster => "cluster",
            Self::Standalone => "standalone",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DebugServerRelease {
    Stop { session: DebugSession },
}

fn nullable_debug_server_release(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    generator.subschema_for::<Option<DebugServerRelease>>()
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DebugServerPrepareResult {
    Planned,
    Prepared {
        url: String,
        infobase_alias: String,
        scope: DebugServerPrepareScope,
        owner: DebugServerPrepareOwner,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner_pid: Option<u32>,
        #[schemars(required, schema_with = "nullable_debug_server_release")]
        release: Option<DebugServerRelease>,
    },
}

/// Read-only observation of a runner-owned debug lease.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerStatus {
    Ready,
    Unknown,
    Stale,
    Unverified,
}

impl DebugServerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unknown => "unknown",
            Self::Stale => "stale",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DebugServerStatusReason {
    ProcessChanged,
    ProcessGone,
    RecordUnavailable,
    ProcessUnverified,
    ListenerUnverified,
    RdbgUnavailable,
    RecordChanged,
}

impl DebugServerStatusReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcessChanged => "process_changed",
            Self::ProcessGone => "process_gone",
            Self::RecordUnavailable => "record_unavailable",
            Self::ProcessUnverified => "process_unverified",
            Self::ListenerUnverified => "listener_unverified",
            Self::RdbgUnavailable => "rdbg_unavailable",
            Self::RecordChanged => "record_changed",
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DebugServerStatusResult {
    pub session: DebugSession,
    pub status: DebugServerStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<DebugServerStatusReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub infobase_alias: Option<String>,
    pub duration_ms: u64,
}
