use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Сообщение удачной выгрузки, которой нечего сообщить.
///
/// Его же рендерер отличает от настоящего предупреждения, поэтому фраза живёт одним
/// значением: разъехавшись, они сделали бы безоблачную выгрузку предупреждением.
pub const DUMP_SUCCESS_MESSAGE: &str = "dump completed successfully";

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DumpResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// Получил ли исполнитель работу этой команды: запущен процесс, который её выполняет,
    /// либо работающей сессии отдана команда запроса. Подъём и открытие сессии, в том числе
    /// запуск её процесса, и её служебные команды работой не считаются. `false`, если работы
    /// не было: превью, отказ или прерывание до передачи работы, процесс, который не удалось
    /// запустить. Поле есть всегда, поэтому отсутствие работы не выводится из отсутствия
    /// значения.
    pub provider_dispatched: bool,
    /// `true` when the platform reported the configuration generation unchanged since the
    /// last recorded build or dump and nothing was dumped.
    #[serde(default)]
    pub up_to_date: bool,
    pub source_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selectors: Option<Vec<DumpSelectorResult>>,
    pub mode: DumpMode,
    pub target_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct DumpSelectorResult {
    pub requested: String,
    pub normalized: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DumpMode {
    Full,
    Incremental,
    Partial,
}
