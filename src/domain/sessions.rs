//! Отказ `infobase_sessions`: базу держат сеансы платформы.
//!
//! Форма общая у всех команд, которые так отказывают (`push`, `apply` по #211 и
//! `infobase create --recreate` по #597): что за база, какие сеансы, может ли раннер их
//! завершить и стоит ли блокировка начала сеансов.

use serde::{Deserialize, Serialize};

/// Поле `error.sessions` отказа `infobase_sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InfobaseSessions {
    /// База: строка соединения без учётных данных.
    pub base: String,
    /// Сеансы базы. `null` — признак занятости есть, а перечня раннер не получил.
    pub list: Option<Vec<SessionEntry>>,
    /// Среди сеансов есть сеанс с этой машины.
    pub from_this_machine: bool,
    /// Может ли раннер завершить сеансы для этой базы и команды.
    pub terminable: bool,
    /// Блокировка начала сеансов, уже стоящая на базе.
    pub lock: SessionLock,
}

/// Один сеанс базы.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionEntry {
    pub computer: String,
    /// Устойчивый код приложения платформы (`Designer`, `1CV8C`, `BackgroundJob`…); `null` —
    /// название не распознано.
    pub application: Option<String>,
    /// Название приложения, как его написала платформа.
    pub application_title: String,
    pub started: String,
    pub user: Option<String>,
    pub id: Option<String>,
}

/// Блокировка начала сеансов.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionLock {
    pub state: SessionLockState,
    /// Сообщение блокировки, если она стоит и раннер его прочёл.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionLockState {
    /// Блокировки нет.
    None,
    /// Блокировку поставил этот раннер.
    Own,
    /// Блокировку поставил кто-то другой.
    Foreign,
    /// Раннер не может узнать, стоит ли блокировка.
    Unknown,
}

impl SessionLock {
    /// Раннер не может узнать, стоит ли блокировка.
    pub const fn unknown() -> Self {
        Self {
            state: SessionLockState::Unknown,
            message: None,
        }
    }
}
