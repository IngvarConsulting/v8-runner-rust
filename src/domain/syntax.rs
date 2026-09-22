use std::path::PathBuf;

use crate::domain::issue::Issue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyntaxCheckStatus {
    Clean,
    IssuesFound,
    ToolFailed,
    /// Превью: проверка запланирована, но не выполнялась. Остальные три значения —
    /// приговоры конфигурации, и `clean` из превью был бы приговором выдуманным:
    /// платформа конфигурацию не смотрела. Предел знания называется своим значением.
    Planned,
}

/// Чем платформа выполнила проверку. Набор закрыт: прежнее `designer-modules` исчезло
/// вместе с отдельным путём `/CheckModules` — режимы проверки модулей выполняет
/// `/CheckConfig`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CheckName {
    /// `/CheckConfig` Конфигуратора.
    DesignerConfig,
    /// Проверка проекта средствами EDT CLI.
    Edt,
}

impl CheckName {
    /// Имя проверки на проводе; оно же попадает в имя файла журнала платформы.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DesignerConfig => "designer-config",
            Self::Edt => "edt",
        }
    }
}

impl std::fmt::Display for CheckName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SyntaxIssueSummary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SyntaxCheckResult {
    /// Квитанция о выборе исполнителя; `None`, пока выбор не начинался.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<crate::domain::capability::ProviderReceipt>,

    /// `false`, когда прогон остановился на превью и платформу не запускал.
    pub provider_dispatched: bool,

    pub status: SyntaxCheckStatus,
    pub exit_code: i32,
    pub check_name: CheckName,
    pub issues: Vec<Issue>,
    pub summary: SyntaxIssueSummary,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_log_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_read_warning: Option<String>,
    /// Фраза о предмете. Превью называет ею, что было бы выполнено.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
