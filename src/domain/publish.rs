//! Публикация информационной базы на веб-сервере.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::launch::DebuggerUrl;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebDebugState {
    Planned,
    Ready,
    ExternalRouteVerified,
    RouteUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct WebDebugPreparation {
    pub state: WebDebugState,
    /// Address to hand to the IDE debugger, independently of the browser and web server.
    pub ide_url: DebuggerUrl,
    /// Address in default.vrd and in the web client's DebuggerURL startup parameter.
    pub debugger_url: Option<DebuggerUrl>,
    pub publication_changed: bool,
}

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
    /// The executor received work, such as running `webinst` or activating Apache. `false`
    /// in a preview and for an external route check.
    pub provider_dispatched: bool,
    pub action: PublishAction,
    /// Web server named in `infobase.web.server`; `apache24` for `infobase.web.managed`.
    pub server: String,
    /// Virtual directory of the publication.
    pub wsdir: String,
    /// Physical directory the publication is written to; under `localPath` for a managed base.
    pub dir: PathBuf,
    /// Address the published infobase opens at: `infobase.web.url` when declared, the loopback
    /// address of the private Apache for `infobase.web.managed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<WebDebugPreparation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PublishPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
