use std::fmt;

use crate::domain::next_step::NextStep;
use crate::platform::process::WorkGiven;
use crate::support::error::{AppError, CancelledAt, CapabilityReason};

const VALIDATION_EXIT_CODE: i32 = 2;
const RUNTIME_EXIT_CODE: i32 = 3;
const PLATFORM_EXIT_CODE: i32 = 4;

/// Stable use-case error class used by transport adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseCaseErrorKind {
    /// Операция здесь не выполняется; причина уточняет, почему именно.
    Capability(CapabilityReason),
    Environment,
    WorkspaceBusy,
    InvalidOutput,
    /// Отмена оператором; место остановки говорит, оборвана ли работа исполнителя.
    Cancelled(CancelledAt),
    TimedOut,
    Validation,
    Runtime,
    Platform,
}

impl UseCaseErrorKind {
    /// Maps the error kind to the CLI exit code.
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Capability(_) => VALIDATION_EXIT_CODE,
            Self::Environment => VALIDATION_EXIT_CODE,
            Self::WorkspaceBusy => RUNTIME_EXIT_CODE,
            Self::InvalidOutput | Self::Cancelled(_) | Self::TimedOut => PLATFORM_EXIT_CODE,
            Self::Validation => VALIDATION_EXIT_CODE,
            Self::Runtime => RUNTIME_EXIT_CODE,
            Self::Platform => PLATFORM_EXIT_CODE,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Capability(_) => "capability unavailable",
            Self::Environment => "environment unavailable",
            Self::WorkspaceBusy => "workspace busy",
            Self::InvalidOutput => "invalid output",
            Self::Cancelled(_) => "cancelled",
            Self::TimedOut => "timed out",
            Self::Validation => "validation error",
            Self::Runtime => "runtime error",
            Self::Platform => "platform error",
        }
    }
}

/// Transport-neutral error metadata returned by use cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCaseError {
    kind: UseCaseErrorKind,
    message: String,
    /// Шаг, которым вызывающий выходит из отказа. Живёт здесь и только здесь: конверт его
    /// печатает, а транспорт не выдумывает.
    next: Option<Box<NextStep>>,
}

impl UseCaseError {
    /// Creates a new use-case error.
    pub fn new(kind: UseCaseErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            next: None,
        }
    }

    /// Называет шаг, которым вызывающий выходит из отказа.
    #[must_use]
    pub fn with_next(mut self, next: NextStep) -> Self {
        self.next = Some(Box::new(next));
        self
    }

    /// Шаг, названный отказом, если он есть.
    pub fn next(&self) -> Option<&NextStep> {
        self.next.as_deref()
    }

    /// Где отмена остановила команду, если отказ — отмена: по нему ответ с записью о
    /// прерывании называет фазу, а штамп сверяет отметку работы.
    pub const fn cancellation(&self) -> Option<CancelledAt> {
        match self.kind {
            UseCaseErrorKind::Cancelled(at) => Some(at),
            _ => None,
        }
    }

    /// Returns the error kind.
    pub const fn kind(&self) -> UseCaseErrorKind {
        self.kind
    }

    /// Returns the message without the prefixed kind label.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the CLI exit code associated with this error kind.
    pub const fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }
}

impl fmt::Display for UseCaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.label(), self.message)
    }
}

impl From<AppError> for UseCaseError {
    fn from(value: AppError) -> Self {
        // Отмена одна для всех: где бы её ни заметили — на безопасной точке, в снятом
        // процессе, в брошенной команде агента, — род отказа у неё `cancelled`.
        let cancelled_at = value.cancellation();
        let error = Self::classified(value);
        match cancelled_at {
            Some(at) => Self {
                kind: UseCaseErrorKind::Cancelled(at),
                ..error
            },
            None => error,
        }
    }
}

impl UseCaseError {
    /// Род отказа по виду ошибки; отмену поверх него узнаёт `From`.
    fn classified(value: AppError) -> Self {
        match value {
            AppError::CapabilityUnavailable(refusal) => Self::new(
                UseCaseErrorKind::Capability(refusal.reason),
                refusal.message,
            ),
            AppError::EnvironmentUnavailable(message) => {
                Self::new(UseCaseErrorKind::Environment, message)
            }
            AppError::WorkspaceBusy(message) => Self::new(UseCaseErrorKind::WorkspaceBusy, message),
            AppError::Cancelled { message, at } => {
                Self::new(UseCaseErrorKind::Cancelled(at), message)
            }
            AppError::TimedOut(message) => Self::new(UseCaseErrorKind::TimedOut, message),
            AppError::InvalidOutput(message) => Self::new(UseCaseErrorKind::InvalidOutput, message),
            AppError::Validation(message) => Self::new(UseCaseErrorKind::Validation, message),
            AppError::ValidationIbcmd(error) => {
                Self::new(UseCaseErrorKind::Validation, error.to_string())
            }
            AppError::ValidationIbcmdContext { context, source } => {
                Self::new(UseCaseErrorKind::Validation, format!("{context}; {source}"))
            }
            AppError::Runtime(message) => Self::new(UseCaseErrorKind::Runtime, message),
            AppError::Platform(message) => Self::new(UseCaseErrorKind::Platform, message),
            AppError::PlatformDesigner(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformDesignerContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformLocator(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformProcess(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformLocatorContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformProcessContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformEdt(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformEdtContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::PlatformEdtSession(error) => {
                Self::new(UseCaseErrorKind::Platform, error.to_string())
            }
            AppError::PlatformEdtSessionContext { context, source } => {
                Self::new(UseCaseErrorKind::Platform, format!("{context}; {source}"))
            }
            AppError::Config(error) => Self::new(UseCaseErrorKind::Validation, error.to_string()),
            AppError::ConfigContext { context, source } => {
                Self::new(UseCaseErrorKind::Validation, format!("{context}; {source}"))
            }
        }
    }
}

/// A failed use-case execution with structured payload and transport-neutral error metadata.
#[derive(Debug, Clone)]
pub struct UseCaseFailure<T> {
    pub error: UseCaseError,
    pub payload: Option<T>,
}

impl<T> UseCaseFailure<T> {
    /// Creates a failure that should still be rendered as a structured command payload.
    pub fn with_payload(error: impl Into<UseCaseError>, payload: T) -> Self {
        Self {
            error: error.into(),
            payload: Some(payload),
        }
    }

    /// Creates a failure that should not emit a structured command payload.
    pub fn without_payload(error: impl Into<UseCaseError>) -> Self {
        Self {
            error: error.into(),
            payload: None,
        }
    }

    /// Отказ там, где исполнитель, может быть, уже получил работу команды. После работы —
    /// формой самой команды: из неё и её `provider_dispatched` вызывающий узнаёт, что работа
    /// была. До работы — общей формой отказа, как всякий отказ до начала. Какая из двух,
    /// решает отметка работы команды, а не место вызова: одна и та же ошибка исполнителя
    /// бывает и до запуска, и после него.
    #[must_use]
    pub(crate) fn after_possible_work(
        error: impl Into<UseCaseError>,
        work: &WorkGiven,
        payload: impl FnOnce() -> T,
    ) -> Self {
        if work.given() {
            Self::with_payload(error, payload())
        } else {
            Self::without_payload(error)
        }
    }
}

/// The transport-neutral result contract for use-case execution.
pub type UseCaseResult<T> = Result<T, UseCaseFailure<T>>;

/// Полезная нагрузка любого исхода: сам результат или тот, что несёт отказ.
pub(crate) fn payload_mut<T>(outcome: &mut UseCaseResult<T>) -> Option<&mut T> {
    match outcome {
        Ok(result) => Some(result),
        Err(failure) => failure.payload.as_mut(),
    }
}

/// Форма, несущая `provider_dispatched`. Значение ей ставит только `stamp_dispatch` — из
/// отметки работы команды, а не из значения, решённого по месту.
pub(crate) trait CarriesDispatch {
    fn stamp_work(&mut self, work: &WorkGiven);
}

macro_rules! carries_dispatch {
    ($($ty:ty),* $(,)?) => {
        $(
            impl CarriesDispatch for $ty {
                fn stamp_work(&mut self, work: &WorkGiven) {
                    self.provider_dispatched = work.given();
                }
            }
        )*
    };
}

carries_dispatch!(
    crate::domain::syntax::SyntaxCheckResult,
    crate::domain::bootstrap::BootstrapResult,
    crate::domain::dump::DumpResult,
    crate::domain::convert::ConvertResult,
    crate::domain::extensions::ExtensionsResult,
    crate::domain::extensions::ExtensionInventoryResult,
    crate::domain::init::InitResult,
    crate::domain::artifacts::ArtifactsResult,
    crate::domain::build::BuildResult,
    crate::domain::load::LoadResult,
    crate::domain::launch::LaunchResult,
    crate::domain::publish::PublishResult,
);

/// Ставит `provider_dispatched` ответа из отметки работы команды. Это единственное место, где
/// признак получает значение: сценарии пишут в конструкторах `false`, а вход сценария отдаёт
/// исход через этот штамп, так что ни CLI, ни MCP не видят признака, решённого по месту.
pub(crate) fn stamp_dispatch<T: CarriesDispatch>(
    mut outcome: UseCaseResult<T>,
    work: &WorkGiven,
) -> UseCaseResult<T> {
    // Отказ без формы после работы — ошибка сценария: вызывающий прочёл бы «ничего не
    // запускалось». Такой отказ строит `UseCaseFailure::after_possible_work`, и всякий тест,
    // дошедший до забытого места, падает здесь, как бы оно ни называлось. В сборке без
    // проверок место остаётся видно в журнале.
    if let Err(failure) = &outcome {
        // Оборвать можно только работу, которую исполнитель получил: отмена посреди работы
        // без отметки назвала бы фазу работы, которой не было.
        let unmarked_work =
            failure.error.cancellation() == Some(CancelledAt::Work) && !work.given();
        debug_assert!(
            !unmarked_work,
            "a cancellation that cut the executor's work short needs the work mark: {} ({})",
            failure.error,
            std::any::type_name::<T>()
        );
        if unmarked_work {
            tracing::error!(
                error = %failure.error,
                form = std::any::type_name::<T>(),
                "a cancellation named cut work that the work mark never saw"
            );
        }
        let formless = failure.payload.is_none() && work.given();
        debug_assert!(
            !formless,
            "a failure after the executor got the command's work must answer in the command's form: {} ({})",
            failure.error,
            std::any::type_name::<T>()
        );
        if formless {
            tracing::error!(
                error = %failure.error,
                form = std::any::type_name::<T>(),
                "a failure after the executor got the command's work answered without its form"
            );
        }
    }
    if let Some(payload) = payload_mut(&mut outcome) {
        payload.stamp_work(work);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::{stamp_dispatch, CarriesDispatch, UseCaseError, UseCaseErrorKind, UseCaseFailure};
    use crate::config::loader::ConfigLoadError;
    use crate::platform::designer::DesignerError;
    use crate::platform::edt_session::EdtSessionError;
    use crate::platform::ibcmd::IbcmdError;
    use crate::platform::process::{ProcessError, WorkGiven};
    use crate::support::error::{AppError, CancelledAt, CapabilityReason};

    /// Форма с признаком в миниатюре.
    #[derive(Debug)]
    struct Form {
        provider_dispatched: bool,
    }

    impl CarriesDispatch for Form {
        fn stamp_work(&mut self, work: &WorkGiven) {
            self.provider_dispatched = work.given();
        }
    }

    fn refusal() -> AppError {
        AppError::Runtime("the executor refused".to_owned())
    }

    /// Отказ отвечает формой команды, только когда исполнитель уже получил работу: до неё —
    /// общей формой отказа, после — формой, и штамп ставит в ней `true`.
    #[test]
    fn a_failure_answers_in_the_command_form_only_after_work() {
        let work = WorkGiven::for_command();
        let before: UseCaseFailure<Form> =
            UseCaseFailure::after_possible_work(refusal(), &work, || {
                unreachable!("no form is built before any work")
            });
        assert!(
            before.payload.is_none(),
            "a refusal before any work keeps the shared refusal form"
        );

        work.mark_work_given();
        let after = UseCaseFailure::after_possible_work(refusal(), &work, || Form {
            provider_dispatched: false,
        });
        let stamped = stamp_dispatch(Err::<Form, _>(after), &work);
        assert!(
            matches!(&stamped, Err(failure)
                if failure.payload.as_ref().is_some_and(|form| form.provider_dispatched)),
            "{stamped:?}"
        );
    }

    /// Отказ без формы после работы ловит сам штамп: забытое место падает в любом тесте,
    /// который до него дошёл, как бы оно ни называлось.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "must answer in the command's form")]
    fn the_stamp_catches_a_failure_without_its_form_after_work() {
        let work = WorkGiven::for_command();
        work.mark_work_given();
        let _ = stamp_dispatch(
            Err::<Form, _>(UseCaseFailure::without_payload(refusal())),
            &work,
        );
    }

    /// Всякая отмена — род `cancelled`, где бы её ни заметили: на безопасной точке, в снятом
    /// процессе, в брошенной команде агента или сессии EDT. Место остановки отказ несёт дальше.
    #[test]
    fn every_cancellation_answers_as_cancelled() {
        for (error, at) in [
            (
                AppError::Cancelled {
                    message: "safe point".to_owned(),
                    at: CancelledAt::Boundary,
                },
                CancelledAt::Boundary,
            ),
            (
                AppError::PlatformProcess(ProcessError::Cancelled {
                    cmd: "webinst".to_owned(),
                    delivered: true,
                })
                .with_context("publish"),
                CancelledAt::Work,
            ),
            (
                AppError::from(EdtSessionError::RunningCancelled { delivered: false }),
                CancelledAt::Boundary,
            ),
        ] {
            let error = UseCaseError::from(error);
            assert_eq!(error.kind(), UseCaseErrorKind::Cancelled(at), "{error}");
            assert_eq!(error.exit_code(), 4);
            assert_eq!(error.cancellation(), Some(at), "{error}");
        }
        let timeout = UseCaseError::from(AppError::TimedOut("agent".to_owned()));
        assert_eq!(timeout.kind(), UseCaseErrorKind::TimedOut);
        assert_eq!(timeout.cancellation(), None);
    }

    /// Отмена посреди работы бывает только у команды, чей исполнитель работу получил: штамп
    /// ловит отказ, который назвал бы оборванной работу, которой не было.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "needs the work mark")]
    fn the_stamp_catches_a_cut_work_without_the_work_mark() {
        let work = WorkGiven::for_command();
        let cut = AppError::PlatformProcess(ProcessError::Cancelled {
            cmd: "1cv8 DESIGNER".to_owned(),
            delivered: true,
        });
        let _ = stamp_dispatch(
            Err::<Form, _>(UseCaseFailure::with_payload(
                cut,
                Form {
                    provider_dispatched: false,
                },
            )),
            &work,
        );
    }

    #[test]
    fn use_case_error_kinds_keep_stable_cli_exit_codes() {
        for reason in [
            CapabilityReason::Unavailable,
            CapabilityReason::Subject,
            CapabilityReason::Target,
            CapabilityReason::Soon,
        ] {
            assert_eq!(
                UseCaseErrorKind::Capability(reason).exit_code(),
                2,
                "{reason:?}"
            );
        }
        assert_eq!(UseCaseErrorKind::Environment.exit_code(), 2);
        assert_eq!(UseCaseErrorKind::WorkspaceBusy.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::InvalidOutput.exit_code(), 4);
        for at in [CancelledAt::Boundary, CancelledAt::Work] {
            assert_eq!(UseCaseErrorKind::Cancelled(at).exit_code(), 4, "{at:?}");
        }
        assert_eq!(UseCaseErrorKind::TimedOut.exit_code(), 4);
        assert_eq!(UseCaseErrorKind::Validation.exit_code(), 2);
        assert_eq!(UseCaseErrorKind::Runtime.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::Platform.exit_code(), 4);
    }

    #[test]
    fn process_app_error_normalizes_only_at_adapter_boundary() {
        let error = UseCaseError::from(AppError::PlatformProcess(ProcessError::SpawnFailed {
            cmd: "1cv8c ENTERPRISE".to_owned(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing binary"),
        }));

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert_eq!(error.exit_code(), 4);
        assert!(error.message().contains("failed to spawn process"));
        assert!(error.message().contains("1cv8c ENTERPRISE"));
    }

    #[test]
    fn generic_process_timeout_keeps_the_existing_platform_error_kind() {
        let error = UseCaseError::from(AppError::PlatformProcess(ProcessError::TimedOut {
            cmd: "1cv8 DESIGNER".to_owned(),
            timeout_ms: 100,
        }));

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
    }

    #[test]
    fn contextual_config_errors_stay_validation_errors() {
        let app_error = AppError::Config(ConfigLoadError::NotFound("v8project.yaml".to_owned()))
            .with_context("failed to load project config");
        assert!(matches!(app_error, AppError::ConfigContext { .. }));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(error.message().contains("failed to load project config"));
        assert!(error.message().contains("v8project.yaml"));
    }

    #[test]
    fn contextual_edt_session_errors_stay_platform_errors() {
        let app_error = AppError::from(EdtSessionError::QueueFull)
            .with_context("failed to acquire EDT session");
        assert!(matches!(
            app_error,
            AppError::PlatformEdtSessionContext { .. }
        ));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert!(error.message().contains("failed to acquire EDT session"));
        assert!(error.message().contains("shared EDT queue is full"));
    }

    #[test]
    fn contextual_ibcmd_validation_errors_keep_typed_source() {
        let app_error = AppError::from(IbcmdError::MissingServerDbmsField("kind"))
            .with_context("failed to build ibcmd connection");
        assert!(matches!(app_error, AppError::ValidationIbcmdContext { .. }));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(error.message().contains("failed to build ibcmd connection"));
        assert!(error.message().contains("infobase.dbms.kind"));
    }

    #[test]
    fn contextual_designer_errors_keep_typed_platform_source() {
        let app_error = AppError::from(DesignerError::UtilityNotFound("1cv8".to_owned()))
            .with_context("failed to resolve designer utility");
        assert!(matches!(
            app_error,
            AppError::PlatformDesignerContext { .. }
        ));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert!(error
            .message()
            .contains("failed to resolve designer utility"));
        assert!(error.message().contains("designer utility not found"));
    }
}
