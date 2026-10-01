//! Публикация информационной базы на веб-сервере.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Что просили у `webinst`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublishAction {
    Publish,
    Delete,
}

impl PublishAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Delete => "delete",
        }
    }
}

/// Команда, которую раннер составил для `webinst`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct PublishPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Результат `publish` и `publish --delete`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct PublishResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    pub ok: bool,
    /// Whether `webinst` was started for this command. `false` in a preview. A refusal before
    /// the start answers the shared refusal form, without this field; a failure after the
    /// start answers this form with `ok: false` and `true`.
    pub provider_dispatched: bool,
    pub action: PublishAction,
    /// Web server named in `infobase.web.server`.
    pub server: String,
    /// Virtual directory of the publication.
    pub wsdir: String,
    /// Physical directory the publication is written to.
    pub dir: PathBuf,
    /// Address the published infobase opens at, when `infobase.web.url` is declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PublishPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
