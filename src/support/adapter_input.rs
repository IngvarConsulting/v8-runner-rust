use crate::use_cases::request::{DumpModeRequest, LaunchTargetRequest, SyntaxExtensionScope};
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Accepted launch-mode alias set for a specific transport boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchModeAliases {
    Cli,
    Mcp,
}

/// Trims a raw optional string and drops blank values.
pub fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Что не так со значением, пришедшим строкой: оно пустое или не из набора.
///
/// Различие типизировано, потому что код отказа на проводе у этих случаев разный, а
/// восстанавливать его сравнением текста сообщения значит решать прозой.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawValueProblem {
    /// Значение пустое.
    Blank,
    /// Значение не из набора.
    Unsupported,
}

/// Отказ по значению, пришедшему строкой: причина и готовый отказ вызывающему.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawValueError {
    problem: RawValueProblem,
    message: String,
}

impl RawValueError {
    /// Значение не из набора.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(RawValueProblem::Unsupported, message.into())
    }

    fn new(problem: RawValueProblem, message: String) -> Self {
        Self { problem, message }
    }

    /// Что не так со значением.
    pub const fn problem(&self) -> RawValueProblem {
        self.problem
    }

    /// Текст отказа вызывающему.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<RawValueError> for UseCaseError {
    /// Род у такого отказа один — проверка входа; различает случаи код, а не род.
    fn from(value: RawValueError) -> Self {
        Self::new(UseCaseErrorKind::Validation, value.message)
    }
}

/// Trims a required raw string and rejects blank values.
pub fn normalize_required_string(
    value: &str,
    field_name: &'static str,
) -> Result<String, RawValueError> {
    normalize_optional_string(Some(value)).ok_or_else(|| {
        RawValueError::new(
            RawValueProblem::Blank,
            format!("{field_name} must not be blank"),
        )
    })
}

/// Parses a required CLI dump mode.
pub fn parse_required_dump_mode(raw: &str) -> Result<DumpModeRequest, RawValueError> {
    let mode = normalize_required_string(raw, "dump mode")?;
    parse_normalized_dump_mode(&mode)
}

/// Parses an optional dump mode while preserving the caller-selected default when omitted.
pub fn parse_optional_dump_mode(
    raw: Option<&str>,
    default_mode: DumpModeRequest,
) -> Result<DumpModeRequest, RawValueError> {
    match normalize_optional_string(raw) {
        Some(mode) => parse_normalized_dump_mode(&mode),
        None => Ok(default_mode),
    }
}

fn parse_normalized_dump_mode(mode: &str) -> Result<DumpModeRequest, RawValueError> {
    match mode.to_ascii_uppercase().as_str() {
        "FULL" => Ok(DumpModeRequest::Full),
        "INCREMENTAL" => Ok(DumpModeRequest::Incremental),
        "PARTIAL" => Ok(DumpModeRequest::Partial),
        _ => Err(RawValueError::new(
            RawValueProblem::Unsupported,
            format!("unsupported dump mode: {mode}"),
        )),
    }
}

/// Parses a launch mode according to the current transport alias contract.
pub fn parse_launch_target(
    raw: &str,
    field_name: &'static str,
    aliases: LaunchModeAliases,
) -> Result<LaunchTargetRequest, RawValueError> {
    let normalized = normalize_required_string(raw, field_name)?.to_lowercase();
    let mode = match aliases {
        LaunchModeAliases::Cli => match normalized.as_str() {
            "designer" => Some(LaunchTargetRequest::designer()),
            "thin" => Some(LaunchTargetRequest::thin_client()),
            "thick" => Some(LaunchTargetRequest::thick_client()),
            "ordinary" => Some(LaunchTargetRequest::ordinary_application()),
            "mcp" => Some(LaunchTargetRequest::client_mcp()),
            "web" => Some(LaunchTargetRequest::web()),
            _ => None,
        },
        LaunchModeAliases::Mcp => match normalized.as_str() {
            "designer" | "configurator" | "1cv8" | "конфигуратор" => {
                Some(LaunchTargetRequest::designer())
            }
            "thin"
            | "thin-client"
            | "thin client"
            | "thin_client"
            | "tc"
            | "1cv8c"
            | "тонкий клиент"
            | "тонкий" => Some(LaunchTargetRequest::thin_client()),
            "thick"
            | "thick-client"
            | "thick client"
            | "thick_client"
            | "толстый клиент"
            | "толстый" => Some(LaunchTargetRequest::thick_client()),
            _ => None,
        },
    };

    mode.ok_or_else(|| {
        RawValueError::new(
            RawValueProblem::Unsupported,
            format!("unsupported launch {field_name}: {raw}"),
        )
    })
}

/// Normalizes optional extension targeting for syntax-like requests.
pub fn normalize_extension_scope(
    extension: Option<&str>,
    all_extensions: Option<bool>,
) -> SyntaxExtensionScope {
    let extension = normalize_optional_string(extension);
    let all_extensions = all_extensions.unwrap_or(extension.is_none());
    SyntaxExtensionScope::new(extension, all_extensions)
}

/// Normalizes a single optional EDT project name into the use-case request list.
pub fn normalize_edt_projects(project_name: Option<&str>) -> Vec<String> {
    normalize_optional_string(project_name).map_or_else(Vec::new, |project| vec![project])
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_edt_projects, normalize_extension_scope, parse_launch_target,
        parse_optional_dump_mode, parse_required_dump_mode, LaunchModeAliases, RawValueProblem,
    };
    use crate::use_cases::request::{DumpModeRequest, LaunchTargetRequest, SyntaxExtensionScope};
    use crate::use_cases::result::UseCaseError;
    use crate::use_cases::result::UseCaseErrorKind;

    #[test]
    fn parses_dump_modes_for_cli_and_mcp_defaults() {
        assert_eq!(
            parse_required_dump_mode("incremental").expect("cli dump mode"),
            DumpModeRequest::Incremental
        );
        assert_eq!(
            parse_optional_dump_mode(None, DumpModeRequest::Incremental).expect("default mode"),
            DumpModeRequest::Incremental
        );
        assert_eq!(
            parse_optional_dump_mode(Some(" PARTIAL "), DumpModeRequest::Incremental)
                .expect("partial mode"),
            DumpModeRequest::Partial
        );
    }

    #[test]
    fn parses_launch_modes_for_cli_and_mcp_alias_sets() {
        assert_eq!(
            parse_launch_target("ordinary", "mode", LaunchModeAliases::Cli).expect("cli ordinary"),
            LaunchTargetRequest::ordinary_application()
        );
        assert_eq!(
            parse_launch_target("mcp", "mode", LaunchModeAliases::Cli).expect("cli mcp"),
            LaunchTargetRequest::client_mcp()
        );
        assert_eq!(
            parse_launch_target("Тонкий клиент", "utility_type", LaunchModeAliases::Mcp)
                .expect("mcp alias"),
            LaunchTargetRequest::thin_client()
        );
        let error = parse_launch_target("ordinary", "utility_type", LaunchModeAliases::Mcp)
            .expect_err("ordinary is not published for MCP");
        assert_eq!(error.problem(), RawValueProblem::Unsupported);
        assert_eq!(error.message(), "unsupported launch utility_type: ordinary");
        assert_eq!(
            UseCaseError::from(error).kind(),
            UseCaseErrorKind::Validation
        );
    }

    #[test]
    fn normalizes_extension_scope_and_edt_project_name() {
        assert_eq!(
            normalize_extension_scope(Some(" Ext "), None),
            SyntaxExtensionScope::SingleExtension {
                name: "Ext".to_owned(),
            }
        );
        assert_eq!(
            normalize_extension_scope(None, Some(false)),
            SyntaxExtensionScope::MainConfiguration
        );
        assert_eq!(normalize_edt_projects(Some(" Project ")), vec!["Project"]);
        assert!(normalize_edt_projects(Some("   ")).is_empty());
    }
}
