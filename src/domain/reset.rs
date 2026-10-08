//! Ответ команды `reset`: отброшено ли непринятое одной цели — основной конфигурации или
//! расширения.

use serde::{Deserialize, Serialize};

use crate::domain::source_set::SourceSetPurpose;
use crate::domain::status::GenerationRecordFate;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ResetResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,
    pub ok: bool,
    /// Получил ли исполнитель работу этой команды
    /// (`INV.WIRE.PROVIDER-DISPATCHED-SAYS-WHETHER-AN-EXECUTOR-GOT-WORK`). У превью — `false`.
    pub provider_dispatched: bool,
    /// Набор, чья цель откатывается.
    pub source_set: String,
    pub purpose: SourceSetPurpose,
    pub outcome: ResetOutcome,
    /// Что стало с хеш-памятью набора; нет поля — команда до неё не дошла.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_memory: Option<HashMemoryFate>,
    /// Что стало с записью журнала поколений набора; нет поля — отката не было или отмена
    /// прервала чтение поколения после него (сообщение называет и то и другое).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationRecordFate>,
    pub message: Option<String>,
    pub duration_ms: u64,
}

/// Исход команды.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResetOutcome {
    /// Непринятое отброшено: основная конфигурация (или расширение) равна конфигурации базы
    /// данных.
    Discarded,
    /// Непринятого нет: отката не было, ничего не записано.
    NothingToDiscard,
    /// Превью: ничего не читалось и не запускалось.
    Planned,
    /// Команда отказала; что успело случиться, говорят `hash_memory` и сообщение.
    Failed,
}

/// Хеш-память набора перед откатом.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HashMemoryFate {
    /// Своя хеш-память набора заменена пустой: следующая отправка грузит набор целиком.
    Replaced,
    /// Своей непустой хеш-памяти у набора нет: ничего не записано, новой памяти команда не
    /// создаёт.
    Absent,
}
