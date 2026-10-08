//! Ответ команды `apply`: что из основной конфигурации стало конфигурацией базы данных.

use serde::{Deserialize, Serialize};

use crate::domain::source_set::SourceSetPurpose;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ApplyResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,
    pub ok: bool,
    /// Получил ли исполнитель работу этой команды
    /// (`INV.WIRE.PROVIDER-DISPATCHED-SAYS-WHETHER-AN-EXECUTOR-GOT-WORK`). У превью — `false`.
    pub provider_dispatched: bool,
    /// Наборы по порядку: конфигурация, расширения, внешние; за ними — расширение-инструмент.
    pub steps: Vec<ApplyStep>,
    pub duration_ms: u64,
}

/// Один набор исходников (или расширение-инструмент `tool:<имя>`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct ApplyStep {
    pub source_set: String,
    pub purpose: SourceSetPurpose,
    pub outcome: ApplyOutcome,
    pub message: Option<String>,
    pub duration_ms: u64,
    /// Что стало с записью журнала поколений набора; нет поля — набор не применялся.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<ApplyGeneration>,
}

/// Исход шага. Удача шага — `applied`, `planned` или `skipped`; отдельного `ok` у шага нет.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOutcome {
    /// Применено.
    Applied,
    /// Превью: применилось бы.
    Planned,
    /// Не применяется: внешний набор или расширение, которого нет в базе.
    Skipped,
    /// Применение отказало; команда остановилась на этом шаге.
    Failed,
    /// Не запускался: команда остановилась раньше.
    NotRun,
}

/// Запись журнала поколений набора после применения.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplyGeneration {
    /// Поколение до применения совпало с записью; после него записано то, что ответил
    /// инструмент записи, и снят признак «не применено».
    Recorded,
    /// База ушла от записи до применения: запись не тронута, следующая отправка это назовёт.
    Kept,
    /// Инструмент записи не ответил поколением: запись стёрта, и следующая отправка базу не
    /// сверяет.
    Erased,
    /// Записи этой пары нет: сверять не с чем.
    Unchecked,
}
