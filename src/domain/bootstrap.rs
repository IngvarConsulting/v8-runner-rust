use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BootstrapResult {
    pub ok: bool,
    pub path: PathBuf,
    pub local_path: PathBuf,
    pub gitignore_path: PathBuf,
    pub source_dir: PathBuf,
    pub dump_target_path: PathBuf,
    pub dumped: bool,
    /// Получил ли исполнитель работу этой команды: запущен процесс, который её выполняет,
    /// либо работающей сессии отдана команда запроса. Подъём и открытие сессии, в том числе
    /// запуск её процесса, и её служебные команды работой не считаются. `false`, если работы
    /// не было: превью, отказ или прерывание до передачи работы, процесс, который не удалось
    /// запустить. Поле есть всегда, поэтому отсутствие работы не выводится из отсутствия
    /// значения.
    pub provider_dispatched: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub duration_ms: u64,
}
