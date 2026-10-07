use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::capability::ProviderReceipt;

/// Направление перевода: откуда и куда. Направления с пакетом исполняет цепочка строки
/// `convert` матрицы, направления между EDT и XML — `1cedtcli`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConvertDirection {
    EdtToDesigner,
    DesignerToEdt,
    /// Наборы XML платформы — в пакеты `.cf` и `.cfe`.
    DesignerToPackage,
    /// Наборы проекта EDT — в пакеты: сперва `1cedtcli` переводит их в XML.
    EdtToPackage,
    /// Файл пакета `.cf` или `.cfe` — в XML платформы.
    PackageToDesigner,
}

impl ConvertDirection {
    /// Пакет на входе или на выходе: такое направление исполняет цепочка строки `convert`.
    pub const fn involves_a_package(self) -> bool {
        matches!(
            self,
            Self::DesignerToPackage | Self::EdtToPackage | Self::PackageToDesigner
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConvertScope {
    All,
    Single,
    /// Позиционный аргумент назвал файл пакета, а не набор.
    Package,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConvertOutput {
    /// Набор, который переведён; нет — на входе был файл пакета.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_set: Option<String>,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConvertResult {
    pub ok: bool,
    /// Whether an executor got this command's work: a process of the EDT CLI or of `ibcmd` was
    /// started to do it, or the request's command was handed to a running EDT session.
    /// Starting the session and its own service commands are not work. `false` whenever it got
    /// none — a preview, a refusal or interruption before any work, or a process that could
    /// not be started.
    ///
    /// Always present, so an absent field never has to be read as "no work was given". In a
    /// preview `outputs` names what would be written.
    pub provider_dispatched: bool,
    pub direction: ConvertDirection,
    pub scope: ConvertScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_set: Option<String>,
    /// Рабочая область EDT; нет — направление обходится без `1cedtcli`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<PathBuf>,
    pub outputs: Vec<ConvertOutput>,
    /// Выбор исполнителя направления с пакетом; у перевода между EDT и XML выбора нет.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderReceipt>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
