//! Чужие сеансы базы у применения конфигурации: отказ `infobase_sessions` и
//! `infobase_designer_busy` (`error.sessions`) и исход `--sessions force` (`data.sessions`
//! у `push` и `apply`), `INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`.

use serde::{Deserialize, Serialize};

/// Кто держит базу — поле `error.sessions` отказов `infobase_sessions` и
/// `infobase_designer_busy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InfobaseSessions {
    /// Строка соединения базы без учётных данных.
    pub base: String,
    /// Сеансы, как их назвала платформа; `null` — список не получен или не разобран целиком.
    pub list: Option<Vec<SessionEntry>>,
    /// Среди сеансов есть сеанс с этой машины.
    pub from_this_machine: bool,
    /// Может ли раннер завершить сеансы этой базы для этой команды (`--sessions force`).
    pub terminable: bool,
    /// Блокировка начала сеансов, которая уже стоит у базы.
    pub lock: SessionLock,
    /// Откуда известно, что базу держат сеансы.
    pub evidence: SessionsEvidence,
    /// Отказ исполнителя как он есть, когда признак — список сервера (`evidence: server_list`):
    /// база занята, но отказ мог быть и по другой причине. У `executor` — `null`.
    pub executor_error: Option<String>,
}

/// Источник признака «базу держат сеансы».
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionsEvidence {
    /// Улика исполнителя или состояния базы: `error-type` агента, его вопрос, блокировка файлов
    /// файловой базы, отказ СУБД из-за подключений, у пересоздания — сеансы в `rac`: другой
    /// причины отказа нет. Отказ — точно из-за сеансов.
    Executor,
    /// Улики исполнителя нет, а собственный список сервера (`rac session list`, `ibcmd session
    /// list`) называет чужие живые сеансы: база занята, но причина отказа может быть другой.
    ServerList,
}

/// Один сеанс из списка платформы.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SessionEntry {
    /// Компьютер сеанса.
    pub computer: String,
    /// Устойчивый код приложения; `null` — название не из таблицы известных.
    pub application: Option<ApplicationCode>,
    /// Название приложения, как его написала платформа.
    pub application_title: String,
    /// Начало сеанса, местное время базы: `ГГГГ-ММ-ДДTчч:мм:сс`.
    pub started: String,
    /// Пользователь; текст платформы у файловой базы его не называет.
    pub user: Option<String>,
    /// UUID сеанса, когда его сообщил сервер.
    pub id: Option<String>,
    /// Номер сеанса у платформы.
    pub number: Option<u32>,
    pub from_this_machine: bool,
    /// Сеанс не завершается через `sessions terminate --all`.
    pub excluded: bool,
}

pub fn from_this_machine(computer: &str) -> bool {
    crate::support::machine::host_name().is_some_and(|host| {
        let short = |name: &str| name.split('.').next().unwrap_or(name).to_ascii_lowercase();
        !computer.is_empty() && short(computer) == short(&host)
    })
}

pub fn protected_application(application: &str) -> bool {
    // The platform does not identify which Designer sessions belong to this runner.
    matches!(application, "RAS" | "SystemBackgroundJob" | "Designer")
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SessionsListResult {
    pub infobase: String,
    pub managed_ras: bool,
    pub sessions: Vec<SessionEntry>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SessionsTerminateResult {
    pub infobase: String,
    pub managed_ras: bool,
    pub requested: String,
    pub terminated_session_ids: Vec<String>,
    pub errors: Vec<SessionTerminationError>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SessionTerminationError {
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SessionsLockResult {
    pub infobase: String,
    pub managed_ras: bool,
    pub sessions_denied: bool,
    pub jobs_denied: bool,
    pub changed: bool,
    #[serde(skip)]
    #[schemars(skip)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SessionsResult {
    List(SessionsListResult),
    Terminate(SessionsTerminateResult),
    Lock(SessionsLockResult),
}

/// Устойчивый код приложения платформы.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub enum ApplicationCode {
    /// Конфигуратор.
    Designer,
    /// Тонкий клиент.
    #[serde(rename = "1CV8C")]
    ThinClient,
    /// Толстый клиент.
    #[serde(rename = "1CV8")]
    ThickClient,
    /// Фоновое задание.
    BackgroundJob,
    /// Веб-клиент.
    WebClient,
    /// Внешнее соединение.
    #[serde(rename = "COMConnection")]
    ComConnection,
}

/// Блокировка начала сеансов базы.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SessionLock {
    pub state: SessionLockState,
    /// Сообщение блокировки, если раннер его знает.
    pub message: Option<String>,
}

impl SessionLock {
    /// Блокировки нет.
    pub const fn none() -> Self {
        Self {
            state: SessionLockState::None,
            message: None,
        }
    }

    /// Раннер не может узнать, стоит ли блокировка: автономный сервер и кластер без `rac`.
    pub const fn unknown() -> Self {
        Self {
            state: SessionLockState::Unknown,
            message: None,
        }
    }
}

/// Чья блокировка начала сеансов стоит у базы.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionLockState {
    /// Блокировки нет.
    None,
    /// Блокировка раннера.
    Own,
    /// Чужая блокировка; раннер её не трогает.
    Foreign,
    /// Узнать нельзя.
    Unknown,
}

/// Исход `--sessions force` — поле `sessions` формы `push` и `apply`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SessionsOutcome {
    /// Сеансы, которые завершил ключ платформы, — из списка до завершения; пусто —
    /// завершать было некого; `null` — завершение было, а список платформа не дала.
    pub terminated: Option<Vec<SessionEntry>>,
    /// Блокировка начала сеансов на время применения.
    pub lock: SessionLock,
    /// Передал ли раннер платформе сообщение завершаемым сеансам.
    pub message_delivered: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub managed_ras: bool,
}
