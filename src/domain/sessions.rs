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
    /// файловой базы, отказ СУБД из-за подключений. Отказ — точно из-за сеансов.
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
    /// Номер сеанса у платформы.
    pub id: Option<String>,
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
}
