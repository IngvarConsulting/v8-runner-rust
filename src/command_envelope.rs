use serde::Serialize;

use crate::domain::execution::StepResult;
use crate::domain::next_step::NextStep;
use crate::domain::test::{
    RetainedPaths, TestErrorKind, TestOutputMode, TestReport, TestRunResult, TestTarget,
};

/// Род отказа. Набор закрыт: новый род приходит вместе с версией конверта, а не молча.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Capability,
    Environment,
    Workspace,
    InvalidOutput,
    Interruption,
    Validation,
    Runtime,
    Platform,
    /// Отправка в базу, ушедшую вперёд. Производителя пока нет: его приносит сравнение
    /// поколений.
    #[allow(
        dead_code,
        reason = "заведён под будущего производителя: набор закрыт целиком"
    )]
    NonFastForward,
    /// Памяти о прошлом разе нет. Производителя пока нет: его приносит память по базе.
    #[allow(
        dead_code,
        reason = "заведён под будущего производителя: набор закрыт целиком"
    )]
    NoMemory,
}

/// Код отказа — та же причина подробнее. Набор закрыт вместе с родом.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    CapabilityUnavailable,
    /// Предмет не тот, и другим он не станет.
    Subject,
    /// Не для этой цели.
    Target,
    /// Пока не умеет.
    Soon,
    EnvironmentUnavailable,
    WorkspaceBusy,
    InvalidOutput,
    Cancelled,
    TimedOut,
    InvalidArgument,
    UnsupportedValue,
    RuntimeFailure,
    PlatformFailure,
    /// Пара к роду `non_fast_forward`; производителя пока нет.
    #[allow(
        dead_code,
        reason = "заведён под будущего производителя: набор закрыт целиком"
    )]
    NonFastForward,
    /// Пара к роду `no_memory`; производителя пока нет.
    #[allow(
        dead_code,
        reason = "заведён под будущего производителя: набор закрыт целиком"
    )]
    NoMemory,
}

impl ErrorKind {
    /// Имя рода на проводе. Нужно сторожу закрытого набора, который сверяет типы со схемой.
    #[allow(dead_code, reason = "читает сторож закрытого набора")]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Capability => "capability",
            Self::Environment => "environment",
            Self::Workspace => "workspace",
            Self::InvalidOutput => "invalid_output",
            Self::Interruption => "interruption",
            Self::Validation => "validation",
            Self::Runtime => "runtime",
            Self::Platform => "platform",
            Self::NonFastForward => "non_fast_forward",
            Self::NoMemory => "no_memory",
        }
    }

    /// Весь закрытый набор — для сторожа, который сверяет его со схемой.
    #[allow(dead_code, reason = "читает сторож закрытого набора")]
    pub const ALL: &'static [Self] = &[
        Self::Capability,
        Self::Environment,
        Self::Workspace,
        Self::InvalidOutput,
        Self::Interruption,
        Self::Validation,
        Self::Runtime,
        Self::Platform,
        Self::NonFastForward,
        Self::NoMemory,
    ];
}

impl ErrorCode {
    /// Имя кода на проводе. Читает сторож закрытого набора: у самого конверта код едет
    /// перечислением, а не строкой.
    #[allow(dead_code, reason = "читает сторож закрытого набора")]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::Subject => "subject",
            Self::Target => "target",
            Self::Soon => "soon",
            Self::EnvironmentUnavailable => "environment_unavailable",
            Self::WorkspaceBusy => "workspace_busy",
            Self::InvalidOutput => "invalid_output",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::InvalidArgument => "invalid_argument",
            Self::UnsupportedValue => "unsupported_value",
            Self::RuntimeFailure => "runtime_failure",
            Self::PlatformFailure => "platform_failure",
            Self::NonFastForward => "non_fast_forward",
            Self::NoMemory => "no_memory",
        }
    }

    /// Весь закрытый набор — для сторожа, который сверяет его со схемой.
    #[allow(dead_code, reason = "читает сторож закрытого набора")]
    pub const ALL: &'static [Self] = &[
        Self::CapabilityUnavailable,
        Self::Subject,
        Self::Target,
        Self::Soon,
        Self::EnvironmentUnavailable,
        Self::WorkspaceBusy,
        Self::InvalidOutput,
        Self::Cancelled,
        Self::TimedOut,
        Self::InvalidArgument,
        Self::UnsupportedValue,
        Self::RuntimeFailure,
        Self::PlatformFailure,
        Self::NonFastForward,
        Self::NoMemory,
    ];
}

/// Structured business error metadata carried by machine-readable command envelopes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct EnvelopeError {
    pub code: ErrorCode,
    pub kind: ErrorKind,
    pub message: String,
    /// Шаг, которым вызывающий выходит из отказа. Проза остаётся человеку.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<NextStep>,
    /// Поколение конфигурации базы; заполняется отказом `non_fast_forward`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_generation: Option<String>,
    /// Поколение, записанное после прошлого обмена; заполняется тем же отказом.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_generation: Option<String>,
}

impl EnvelopeError {
    pub fn new(code: ErrorCode, kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            code,
            kind,
            message: message.into(),
            next: None,
            base_generation: None,
            local_generation: None,
        }
    }

    /// Называет шаг, которым вызывающий выходит из отказа.
    #[must_use]
    pub fn with_next(mut self, next: Option<NextStep>) -> Self {
        self.next = next;
        self
    }
}

/// Shared machine-readable command payload for CLI JSON and MCP structured content.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Envelope<T: Serialize> {
    pub ok: bool,
    pub command: String,
    pub duration_ms: u64,
    pub data: T,
    pub warnings: Vec<String>,
    pub steps: Vec<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EnvelopeError>,
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(command: impl Into<String>, duration_ms: u64, data: T) -> Self {
        Self {
            ok: true,
            command: command.into(),
            duration_ms,
            data,
            warnings: vec![],
            steps: vec![],
            error: None,
        }
    }

    pub fn err(command: impl Into<String>, duration_ms: u64, data: T) -> Self {
        Self {
            ok: false,
            command: command.into(),
            duration_ms,
            data,
            warnings: vec![],
            steps: vec![],
            error: None,
        }
    }

    pub fn with_error(mut self, error: EnvelopeError) -> Self {
        self.error = Some(error);
        self
    }
}

/// Shared JSON data projection for test command envelopes.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct TestEnvelopeData {
    pub ok: bool,
    pub target: TestTarget,
    pub mode: TestOutputMode,
    pub error_kind: Option<TestErrorKind>,
    pub diagnostics: Vec<String>,
    pub retained_paths: Option<RetainedPaths>,
    pub report: Option<TestReport>,
    pub execution: crate::domain::execution::ExecutionOutcome<TestReport>,
}

impl TestEnvelopeData {
    pub fn from_result(result: &TestRunResult) -> Self {
        let execution = &result.execution;
        Self {
            ok: execution.is_ok(),
            target: result.target.clone(),
            mode: result.mode.clone(),
            error_kind: execution
                .errors
                .first()
                .and_then(|error| TestErrorKind::from_code(&error.code)),
            diagnostics: execution.diagnostics.clone(),
            retained_paths: execution
                .artifacts
                .as_ref()
                .and_then(RetainedPaths::from_artifact_set),
            report: execution.payload.clone(),
            execution: execution.clone(),
        }
    }
}

pub fn test_envelope(result: &TestRunResult) -> Envelope<TestEnvelopeData> {
    Envelope {
        ok: result.execution.is_ok(),
        command: "test".to_owned(),
        duration_ms: result.duration_ms,
        warnings: result.warnings.clone(),
        steps: result.steps.clone(),
        error: None,
        data: TestEnvelopeData::from_result(result),
    }
}

/// Файл, в котором лежит порождённая форма конверта.
#[allow(dead_code, reason = "читает сторож свежести артефакта")]
pub const COMMAND_ENVELOPE_SCHEMA_PATH: &str = "docs/schemas/command-envelope.schema.json";

/// Форма конверта, порождённая из типов.
///
/// Раньше файл писался руками, и закрытый состав полей держался на внимательности: набор
/// родов отказа можно было расширить в коде и не заметить, что схема о нём молчит. Теперь
/// схема порождается, а тест сверяет её с файлом — как у форм данных команд.
#[allow(dead_code, reason = "читает сторож свежести артефакта")]
pub fn generated_envelope_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Envelope<serde_json::Value>);
    let mut value = crate::command_data::generated_schema(schema, "command-envelope");
    let object = value.as_object_mut().expect("root schema object");
    object.insert(
        "$id".to_owned(),
        serde_json::Value::String(format!(
            "{}/command-envelope.schema.json",
            crate::command_data::REPOSITORY_RAW_SCHEMA_ROOT
        )),
    );
    object.insert(
        "title".to_owned(),
        serde_json::Value::String("v8-runner command envelope".to_owned()),
    );
    object.insert(
        "description".to_owned(),
        serde_json::Value::String(
            "Форма машинного ответа команды: общая часть конверта, шаги и отказ. Данные команды описывает её собственная форма из docs/schemas/command-data."
                .to_owned(),
        ),
    );
    // `data` у каждой команды своя, и пинает её свой контракт: здесь поле остаётся любым.
    if let Some(properties) = object
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        properties.insert("data".to_owned(), serde_json::Value::Bool(true));
    }
    value
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    fn artifact_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(COMMAND_ENVELOPE_SCHEMA_PATH)
    }

    #[test]
    fn generated_envelope_schema_is_current() {
        let generated = crate::command_data::schema_json_pretty(&generated_envelope_schema());
        if std::env::var_os("UPDATE_ENVELOPE_SCHEMA").is_some() {
            std::fs::write(artifact_path(), &generated).expect("write envelope schema");
        }
        let actual = std::fs::read_to_string(artifact_path()).expect("envelope schema artifact");
        assert_eq!(
            actual, generated,
            "{COMMAND_ENVELOPE_SCHEMA_PATH} is stale; rerun UPDATE_ENVELOPE_SCHEMA=1 cargo test generated_envelope_schema_is_current"
        );
    }

    /// Набор родов и кодов закрыт по кругу: что умеет назвать код, то перечисляет схема, и
    /// наоборот. Порождение схемы само по себе этого не держит — оно лишь повторяет типы.
    #[test]
    fn every_error_kind_and_code_is_named_by_the_schema_and_by_a_table() {
        let schema = generated_envelope_schema();
        let named = |definition: &str| -> Vec<String> {
            crate::support::schema::enum_values(
                schema
                    .pointer(&format!("/$defs/{definition}"))
                    .unwrap_or_else(|| panic!("{definition} is defined by the schema")),
            )
        };

        let kinds = named("ErrorKind");
        let codes = named("ErrorCode");
        for kind in ErrorKind::ALL {
            assert!(kinds.contains(&kind.as_str().to_owned()), "{kind:?}");
        }
        for code in ErrorCode::ALL {
            assert!(codes.contains(&code.as_str().to_owned()), "{code:?}");
        }
        assert_eq!(kinds.len(), ErrorKind::ALL.len());
        assert_eq!(codes.len(), ErrorCode::ALL.len());

        // Соответствие рода отказа паре «код, род» на проводе проверяется целиком и по
        // литералам: иначе можно поменять местами два кода, и набор останется тем же.
        use crate::support::error::{CancelledAt, CapabilityReason};
        use crate::use_cases::result::UseCaseErrorKind;
        let cli: Vec<(UseCaseErrorKind, &str, &str)> = vec![
            (
                UseCaseErrorKind::Capability(CapabilityReason::Unavailable),
                "capability_unavailable",
                "capability",
            ),
            (
                UseCaseErrorKind::Capability(CapabilityReason::Subject),
                "subject",
                "capability",
            ),
            (
                UseCaseErrorKind::Capability(CapabilityReason::Target),
                "target",
                "capability",
            ),
            (
                UseCaseErrorKind::Capability(CapabilityReason::Soon),
                "soon",
                "capability",
            ),
            (
                UseCaseErrorKind::Environment,
                "environment_unavailable",
                "environment",
            ),
            (
                UseCaseErrorKind::WorkspaceBusy,
                "workspace_busy",
                "workspace",
            ),
            (
                UseCaseErrorKind::InvalidOutput,
                "invalid_output",
                "invalid_output",
            ),
            (
                UseCaseErrorKind::Cancelled(CancelledAt::Boundary),
                "cancelled",
                "interruption",
            ),
            (
                UseCaseErrorKind::Cancelled(CancelledAt::Work),
                "cancelled",
                "interruption",
            ),
            (UseCaseErrorKind::TimedOut, "timed_out", "interruption"),
            (
                UseCaseErrorKind::Validation,
                "invalid_argument",
                "validation",
            ),
            (UseCaseErrorKind::Runtime, "runtime_failure", "runtime"),
            (UseCaseErrorKind::Platform, "platform_failure", "platform"),
        ];
        let mut produced_codes: Vec<&str> = Vec::new();
        let mut produced_kinds: Vec<&str> = Vec::new();
        for (kind, code_name, kind_name) in cli {
            let (code, rendered) = crate::cli::output::cli_error_contract(kind);
            assert_eq!(code.as_str(), code_name, "{kind:?}");
            assert_eq!(rendered.as_str(), kind_name, "{kind:?}");
            produced_codes.push(code_name);
            produced_kinds.push(kind_name);
        }

        // У MCP словарь свой и уже: ни одного кода возможности он не выдаёт, но едет тем
        // же конвертом и тем же набором.
        use crate::mcp::error::{McpBusinessErrorKind, McpErrorCode};
        for (code, name) in [
            (McpErrorCode::InvalidArgument, "invalid_argument"),
            (McpErrorCode::UnsupportedValue, "unsupported_value"),
            (McpErrorCode::RuntimeFailure, "runtime_failure"),
            (McpErrorCode::PlatformFailure, "platform_failure"),
            (McpErrorCode::Internal, "runtime_failure"),
        ] {
            let rendered: ErrorCode = code.into();
            assert_eq!(rendered.as_str(), name, "{code:?}");
            produced_codes.push(name);
        }
        for (kind, name) in [
            (McpBusinessErrorKind::Validation, "validation"),
            (McpBusinessErrorKind::Runtime, "runtime"),
            (McpBusinessErrorKind::Platform, "platform"),
        ] {
            let rendered: ErrorKind = kind.into();
            assert_eq!(rendered.as_str(), name, "{kind:?}");
            produced_kinds.push(name);
        }

        // Что не назвала ни одна таблица — перечислено здесь как заведённое под будущего
        // производителя. Иначе набор растёт молча: порождение схемы повторяет типы и само
        // этого не ловит.
        const RESERVED_KINDS: &[&str] = &["non_fast_forward", "no_memory"];
        const RESERVED_CODES: &[&str] = &["non_fast_forward", "no_memory"];
        for kind in ErrorKind::ALL {
            let name = kind.as_str();
            assert_eq!(
                produced_kinds.contains(&name),
                !RESERVED_KINDS.contains(&name),
                "{name}: род либо называется таблицей, либо заведён под будущего производителя"
            );
        }
        for code in ErrorCode::ALL {
            let name = code.as_str();
            assert_eq!(
                produced_codes.contains(&name),
                !RESERVED_CODES.contains(&name),
                "{name}: код либо называется таблицей, либо заведён под будущего производителя"
            );
        }
    }
}
