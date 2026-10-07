use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InitResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// Whether an executor got this command's work: a process was started to do it, or the
    /// request's command was handed to a running session. Starting or opening a session and
    /// its own service commands are not work. `false` whenever the executor got none — a
    /// preview, a refusal or interruption before any work, a run with nothing to do, or a
    /// process that could not be started.
    ///
    /// Always present, so an absent field never has to be read as "no work was given".
    pub provider_dispatched: bool,
    pub steps: Vec<InitStep>,
    /// Источник копии у `infobase create --from`: база, с которой снят образ, и путь снимка.
    /// Без `--from` поля нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<InitSource>,
    pub duration_ms: u64,
}

/// База-источник копии и снимок, из которого создана новая база.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InitSource {
    /// Имя базы-источника в местном слое.
    pub infobase: String,
    /// Абсолютный путь образа DT под `workPath`, снятого с источника; под превью — путь,
    /// куда он лёг бы.
    pub snapshot: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct InitStep {
    pub target: String,
    pub action: String,
    pub status: InitStepStatus,
    pub message: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InitStepStatus {
    Ok,
    Skipped,
    Failed,
    /// The step was decided but not performed, because the run is a preview.
    Planned,
}
