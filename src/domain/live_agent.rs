//! Ответ команд `agent hold`, `agent release`, `agent stop` и `agent status` (#577).

use serde::Serialize;

use crate::domain::capability::LiveAgentHolder;

/// `data` команды `agent hold`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct AgentHoldResult {
    /// Ключ живого агента цели проекта: шестнадцатеричная строка без частей, из которых он
    /// сложен.
    pub key: String,
    /// Взят ли уже живой агент (`true`) или `hold` поднял его сам (`false`).
    pub reused: bool,
    /// Держатели после `hold`, этот среди них.
    pub holders: Vec<LiveAgentHolder>,
    pub keeper_pid: u32,
    pub agent_pid: Option<u32>,
    pub duration_ms: u64,
}

/// `data` команды `agent release`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct AgentReleaseResult {
    /// Ключ живого агента цели проекта; `null`, если живого агента нет.
    pub key: Option<String>,
    /// Была ли отметка этого держателя и снята ли она: `false` — снимать было нечего.
    pub released: bool,
    /// Держатели после `release`.
    pub holders: Vec<LiveAgentHolder>,
    pub duration_ms: u64,
}

/// `data` команды `agent stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct AgentStopResult {
    /// Снятые живые агенты; пусто — снимать было нечего. Агент с `closing: true` ещё
    /// выполняет команду: сторож снимет его, как только она кончится.
    pub stopped: Vec<LiveAgentEntry>,
    /// Агенты, которых `--all` снять не смог: обход идёт дальше, отказ назван здесь.
    pub refused: Vec<AgentStopRefusal>,
    pub duration_ms: u64,
}

/// Живой агент, которого `agent stop --all` не снял.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct AgentStopRefusal {
    pub key: String,
    /// Почему не снят.
    pub message: String,
}

/// `data` команды `agent status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct AgentStatusResult {
    /// Живые агенты учётной записи ОС.
    pub agents: Vec<LiveAgentEntry>,
    pub duration_ms: u64,
}

/// Живой агент учётной записи, как его называет сторож.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct LiveAgentEntry {
    pub key: String,
    /// База агента в каноническом виде: `file:…` или `server:…`; строка подключения иной
    /// формы — только отпечатком `connection:<sha256>`.
    pub infobase: Option<String>,
    /// Агент цели этого проекта (`--config`).
    pub this_project: bool,
    pub keeper_pid: u32,
    pub agent_pid: Option<u32>,
    pub holders: Vec<LiveAgentHolder>,
    /// Команда, которую агент выполняет сейчас или последней в открытой сессии.
    pub busy: Option<String>,
    /// Подключён ли агент к базе сессией команды.
    pub connected: bool,
    /// Сколько агент простаивает, мс; занятый — ноль.
    pub idle_ms: u64,
    /// Сторож уже решил снять агента.
    pub closing: bool,
}
