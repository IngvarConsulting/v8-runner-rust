use std::fmt;

use crate::config::loader::ConfigLoadError;
use crate::domain::execution::ExecutionStatus;
use crate::domain::next_step::NextStep;
use crate::domain::platforms::PlatformReleaseRefusal;
use crate::domain::sessions::InfobaseSessions;
use crate::platform::process::{ProcessError, WorkGiven};
use crate::support::error::{AppError, CancelledAt, CapabilityReason, EnvironmentLayer};
use crate::use_cases::interruption::CANCELLED_ERROR_CODE;

const VALIDATION_EXIT_CODE: i32 = 2;
const RUNTIME_EXIT_CODE: i32 = 3;
const PLATFORM_EXIT_CODE: i32 = 4;

/// Почему окружение не готово: причина различает коды внутри рода `environment`, как у
/// рода `capability`. Код шага в `data.execution.errors[]` от неё не зависит.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentReason {
    /// Утилиты нет, не та версия или её версию не прочитать.
    Unavailable,
    /// `tools.platform.version: sources`, а выпуска формата исходников среди установок нет
    /// (`INV.PLATFORM.A-MISSING-RELEASE-OF-THE-SOURCES-FORMAT-IS-REFUSED`).
    PlatformReleaseMissing,
}

/// Stable use-case error class used by transport adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseCaseErrorKind {
    /// Операция здесь не выполняется; причина уточняет, почему именно.
    Capability(CapabilityReason),
    /// Окружение не готово; причина уточняет, чего не хватает.
    Environment(EnvironmentReason),
    WorkspaceBusy,
    /// Файловую базу держит другая команда: занято, как `WorkspaceBusy`, можно повторить.
    InfobaseBusy,
    /// Применению нужен монопольный доступ, а базу держат чужие сеансы
    /// (`INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`).
    InfobaseSessions,
    /// Файловую базу держит чужой Конфигуратор; его раннер не закрывает.
    InfobaseDesignerBusy,
    /// База ушла вперёд записанного поколения: сперва `pull` или перезапись `push --force`.
    NonFastForward,
    /// Памяти о базе у рабочей копии нет: сперва `pull` или перезапись `push --force`.
    UntrackedBase,
    /// `agent hold`: живых агентов учётной записи уже `live-max`, и снять простаивающего
    /// некого (`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`).
    AgentLimit,
    /// `agent hold` при `tools.designer_agent.live: false`.
    LiveAgentOff,
    /// `agent hold` в Job, из которого сторож не может выйти.
    LiveAgentJob,
    /// Правило игнорирования git прячет файл набора: изменения набора берутся у git, и
    /// спрятанный файл не загрузился бы (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`).
    SourceSetIgnoredByGit,
    /// Каталог набора и `workPath` на разных томах: промежуточный каталог выгрузки лежит в
    /// `workPath`, и публикация не была бы переименованием
    /// (`INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME`).
    WorkPathOnAnotherVolume,
    InvalidOutput,
    /// Отмена оператором; место остановки говорит, оборвана ли работа исполнителя.
    Cancelled(CancelledAt),
    TimedOut,
    Validation,
    Runtime,
    Platform,
    /// Второй сигнал бросил ожидание итога команды чужого агента: агент, возможно, ещё её
    /// выполняет (`INV.USE-CASES.AN-ATTACHED-AGENT-ANSWERS-CANCELLED-AFTER-ITS-COMMAND-ENDS`).
    /// Конец не подтверждён, поэтому это сбой платформы, а не отмена.
    AgentCommandAbandoned,
}

impl UseCaseErrorKind {
    /// Maps the error kind to the CLI exit code.
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Capability(_) => VALIDATION_EXIT_CODE,
            Self::Environment(_) => VALIDATION_EXIT_CODE,
            Self::LiveAgentOff | Self::LiveAgentJob | Self::AgentLimit => VALIDATION_EXIT_CODE,
            Self::WorkspaceBusy
            | Self::InfobaseBusy
            | Self::InfobaseSessions
            | Self::InfobaseDesignerBusy => RUNTIME_EXIT_CODE,
            Self::NonFastForward | Self::UntrackedBase => RUNTIME_EXIT_CODE,
            Self::InvalidOutput | Self::Cancelled(_) | Self::TimedOut => PLATFORM_EXIT_CODE,
            Self::Validation | Self::SourceSetIgnoredByGit | Self::WorkPathOnAnotherVolume => {
                VALIDATION_EXIT_CODE
            }
            Self::Runtime => RUNTIME_EXIT_CODE,
            Self::Platform | Self::AgentCommandAbandoned => PLATFORM_EXIT_CODE,
        }
    }

    /// Род отказа по виду ошибки — единственное отображение [`AppError`] в род. Из рода
    /// выводятся и пара «код, род» конверта, и код шага в `data.execution.errors[]`.
    pub(crate) const fn of(error: &AppError) -> Self {
        match error {
            AppError::CapabilityUnavailable(refusal) => Self::Capability(refusal.reason),
            // Утилиты нет или не та версия — дело окружения, а не сбой платформы: поставьте,
            // и заработает (`INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE`). Нет выпуска
            // формата исходников при `sources` — та же среда, но свой код.
            AppError::EnvironmentUnavailable(_)
            | AppError::PlatformLocator(_)
            | AppError::PlatformLocatorContext { .. } => {
                Self::Environment(if error.platform_release().is_some() {
                    EnvironmentReason::PlatformReleaseMissing
                } else {
                    EnvironmentReason::Unavailable
                })
            }
            // Стенд базы принадлежит другому проекту общего `localPath`: занято, а не неверно.
            AppError::WorkspaceBusy(_)
            | AppError::Config(ConfigLoadError::StandOfAnotherProject(_))
            | AppError::ConfigContext {
                source: ConfigLoadError::StandOfAnotherProject(_),
                ..
            } => Self::WorkspaceBusy,
            AppError::InfobaseBusy(_) => Self::InfobaseBusy,
            AppError::Refused(refusal) => refusal.kind(),
            AppError::Cancelled { at, .. } => Self::Cancelled(*at),
            AppError::AgentAbandoned(_) => Self::AgentCommandAbandoned,
            AppError::TimedOut(_) => Self::TimedOut,
            AppError::InvalidOutput(_) => Self::InvalidOutput,
            // Путь, который процессу без приставки `\\?\` не передать, — вход команды, а не сбой
            // платформы: процесс не запускался.
            AppError::PlatformProcess(ProcessError::UnexpressiblePath { .. })
            | AppError::PlatformProcessContext {
                source: ProcessError::UnexpressiblePath { .. },
                ..
            }
            | AppError::Validation(_)
            | AppError::ValidationIbcmd(_)
            | AppError::ValidationIbcmdContext { .. }
            | AppError::Config(_)
            | AppError::ConfigContext { .. } => Self::Validation,
            AppError::Runtime(_) | AppError::SourceLoadNotStarted(_) => Self::Runtime,
            AppError::Platform(_)
            | AppError::PlatformDesigner(_)
            | AppError::PlatformDesignerContext { .. }
            | AppError::PlatformProcess(_)
            | AppError::PlatformProcessContext { .. }
            | AppError::PlatformEdt(_)
            | AppError::PlatformEdtContext { .. }
            | AppError::PlatformEdtSession(_)
            | AppError::PlatformEdtSessionContext { .. } => Self::Platform,
        }
    }

    /// Код шага исполнителя: свой словарь, едущий внутри `data.execution.errors[]`.
    ///
    /// Имена совпадают с кодами конверта, но поля разные, и различать их должен код, а не
    /// читатель: конверт стал точнее — у рода `capability` там свой код на каждую причину, —
    /// а шаг остаётся при прежнем словаре, потому что его читает другой потребитель.
    pub(crate) const fn execution_step_code(self) -> &'static str {
        match self {
            Self::Capability(_) => "capability_unavailable",
            Self::Environment(_) => "environment_unavailable",
            Self::WorkspaceBusy => "workspace_busy",
            Self::InfobaseBusy => "infobase_busy",
            Self::InfobaseSessions => "infobase_sessions",
            Self::InfobaseDesignerBusy => "infobase_designer_busy",
            Self::NonFastForward => "non_fast_forward",
            Self::UntrackedBase => "untracked_base",
            Self::AgentLimit => "agent_limit",
            Self::LiveAgentOff => "live_agent_off",
            Self::LiveAgentJob => "live_agent_job",
            Self::SourceSetIgnoredByGit => "source_set_ignored_by_git",
            Self::WorkPathOnAnotherVolume => "work_path_on_another_volume",
            Self::InvalidOutput => "invalid_output",
            Self::Cancelled(_) => CANCELLED_ERROR_CODE,
            Self::TimedOut => "timed_out",
            Self::Validation => "invalid_argument",
            Self::Runtime => "runtime_failure",
            Self::Platform => "platform_failure",
            Self::AgentCommandAbandoned => "agent_command_abandoned",
        }
    }

    /// Статус итога исполнения, который ставит отказ этого рода.
    pub(crate) const fn execution_status(self) -> ExecutionStatus {
        match self {
            Self::InvalidOutput => ExecutionStatus::InvalidOutput,
            Self::TimedOut => ExecutionStatus::TimedOut,
            Self::Cancelled(_) => ExecutionStatus::Cancelled,
            Self::Capability(_)
            | Self::Environment(_)
            | Self::WorkspaceBusy
            | Self::InfobaseBusy
            | Self::InfobaseSessions
            | Self::InfobaseDesignerBusy
            | Self::NonFastForward
            | Self::UntrackedBase
            | Self::AgentLimit
            | Self::LiveAgentOff
            | Self::LiveAgentJob
            | Self::SourceSetIgnoredByGit
            | Self::WorkPathOnAnotherVolume
            | Self::Validation
            | Self::Runtime
            | Self::Platform
            | Self::AgentCommandAbandoned => ExecutionStatus::Failed,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Capability(_) => "capability unavailable",
            Self::Environment(_) => "environment unavailable",
            Self::WorkspaceBusy => "workspace busy",
            Self::InfobaseBusy => "infobase busy",
            Self::InfobaseSessions => "infobase held by other sessions",
            Self::InfobaseDesignerBusy => "infobase open in the Designer",
            Self::NonFastForward => "non-fast-forward",
            Self::UntrackedBase => "the infobase is not tracked by this working copy",
            Self::AgentLimit => "live agent limit",
            Self::LiveAgentOff => "live agent off",
            Self::LiveAgentJob => "live agent impossible in this job",
            Self::SourceSetIgnoredByGit => "source set ignored by git",
            Self::WorkPathOnAnotherVolume => "workPath on another volume",
            Self::InvalidOutput => "invalid output",
            Self::Cancelled(_) => "cancelled",
            Self::TimedOut => "timed out",
            Self::Validation => "validation error",
            Self::Runtime => "runtime error",
            Self::Platform => "platform error",
            Self::AgentCommandAbandoned => "agent command abandoned",
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
    /// Подробность, которую несёт отказ одного рода; у разных родов она своя.
    detail: Option<Box<RefusalDetail>>,
}

/// Подробность отказа: у каждого рода своя, и одна на отказ.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RefusalDetail {
    /// Поколения у отказа `non_fast_forward`: базы сейчас и записанное после прошлого обмена.
    Generations(Generations),
    /// Формат исходников и опись установок у отказа `platform_release_missing`.
    PlatformRelease(PlatformReleaseRefusal),
    /// Кто держит базу у отказов `infobase_sessions` и `infobase_designer_busy`.
    Sessions(InfobaseSessions),
    /// Слой, который отказал, у отказа `environment_unavailable`, когда он определён.
    EnvironmentLayer(EnvironmentLayer),
}

/// Оба поколения отказа «база ушла вперёд».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generations {
    /// Поколение базы сейчас.
    pub base: String,
    /// Поколение, записанное после прошлого обмена.
    pub local: String,
}

impl UseCaseError {
    /// Creates a new use-case error.
    pub fn new(kind: UseCaseErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            next: None,
            detail: None,
        }
    }

    /// Кто держит базу у отказов `infobase_sessions` и `infobase_designer_busy`.
    pub fn sessions(&self) -> Option<&InfobaseSessions> {
        match self.detail.as_deref()? {
            RefusalDetail::Sessions(sessions) => Some(sessions),
            RefusalDetail::Generations(_)
            | RefusalDetail::PlatformRelease(_)
            | RefusalDetail::EnvironmentLayer(_) => None,
        }
    }

    /// Слой окружения, который отказал, у отказа `environment_unavailable`. У отказа другого
    /// рода — в том числе отмены, ставшей поверх него, — слоя нет, что бы ни лежало в подробности.
    pub fn environment_layer(&self) -> Option<EnvironmentLayer> {
        if self.kind != UseCaseErrorKind::Environment(EnvironmentReason::Unavailable) {
            return None;
        }
        match self.detail.as_deref()? {
            RefusalDetail::EnvironmentLayer(layer) => Some(*layer),
            RefusalDetail::Generations(_)
            | RefusalDetail::PlatformRelease(_)
            | RefusalDetail::Sessions(_) => None,
        }
    }

    /// Называет, кто держит базу.
    #[must_use]
    pub fn with_sessions(mut self, sessions: InfobaseSessions) -> Self {
        self.detail = Some(Box::new(RefusalDetail::Sessions(sessions)));
        self
    }

    /// Формат исходников и опись установок отказа `platform_release_missing`.
    pub fn platform_release(&self) -> Option<&PlatformReleaseRefusal> {
        match self.detail.as_deref()? {
            RefusalDetail::PlatformRelease(refusal) => Some(refusal),
            RefusalDetail::Generations(_)
            | RefusalDetail::Sessions(_)
            | RefusalDetail::EnvironmentLayer(_) => None,
        }
    }

    /// Оба поколения отказа «база ушла вперёд».
    pub fn generations(&self) -> Option<&Generations> {
        match self.detail.as_deref()? {
            RefusalDetail::Generations(generations) => Some(generations),
            RefusalDetail::PlatformRelease(_)
            | RefusalDetail::Sessions(_)
            | RefusalDetail::EnvironmentLayer(_) => None,
        }
    }

    /// Называет оба поколения отказа «база ушла вперёд».
    #[must_use]
    pub fn with_generations(mut self, generations: Generations) -> Self {
        self.detail = Some(Box::new(RefusalDetail::Generations(generations)));
        self
    }

    /// Тот же отказ с другим текстом: род, шаг и подробность — поколения, опись выпуска,
    /// сеансы, слой окружения — остаются.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Уточняет текст отказа тем, что было до него; род, шаг и подробность остаются.
    #[must_use]
    pub fn with_context(mut self, context: impl AsRef<str>) -> Self {
        self.message = format!("{}; {}", context.as_ref(), self.message);
        self
    }

    /// Дописывает к тексту отказа то, что из него следует; род, шаг и подробность остаются.
    #[must_use]
    pub fn followed_by(mut self, tail: impl AsRef<str>) -> Self {
        self.message = format!("{}; {}", self.message, tail.as_ref());
        self
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
        if let AppError::Refused(refusal) = value {
            return *refusal;
        }
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
    /// Отказ по ошибке: род — от [`UseCaseErrorKind::of`], текст — без метки рода; отмену
    /// поверх него узнаёт `From`.
    fn classified(value: AppError) -> Self {
        let kind = UseCaseErrorKind::of(&value);
        let platform_release = value
            .platform_release()
            .map(|missing| RefusalDetail::PlatformRelease(missing.refusal()));
        debug_assert_eq!(
            kind == UseCaseErrorKind::Environment(EnvironmentReason::PlatformReleaseMissing),
            platform_release.is_some(),
            "the code platform_release_missing and its inventory come together"
        );
        let layer = value.environment_layer();
        debug_assert!(
            layer.is_none()
                || kind == UseCaseErrorKind::Environment(EnvironmentReason::Unavailable),
            "only environment_unavailable names a layer"
        );
        let detail = platform_release
            .or_else(|| layer.map(RefusalDetail::EnvironmentLayer))
            .map(Box::new);
        let message = match value {
            AppError::CapabilityUnavailable(refusal) => refusal.message,
            AppError::Refused(refusal) => refusal.message,
            AppError::EnvironmentUnavailable(refusal) => refusal.message,
            AppError::WorkspaceBusy(message)
            | AppError::InfobaseBusy(message)
            | AppError::Cancelled { message, .. }
            | AppError::TimedOut(message)
            | AppError::InvalidOutput(message)
            | AppError::Validation(message)
            | AppError::Runtime(message)
            | AppError::SourceLoadNotStarted(message)
            | AppError::Platform(message)
            | AppError::AgentAbandoned(message) => message,
            AppError::ValidationIbcmd(error) => error.to_string(),
            AppError::PlatformDesigner(error) => error.to_string(),
            AppError::PlatformLocator(error) => error.to_string(),
            AppError::PlatformProcess(error) => error.to_string(),
            AppError::PlatformEdt(error) => error.to_string(),
            AppError::PlatformEdtSession(error) => error.to_string(),
            AppError::Config(error) => error.to_string(),
            AppError::ValidationIbcmdContext { context, source } => format!("{context}; {source}"),
            AppError::PlatformDesignerContext { context, source } => format!("{context}; {source}"),
            AppError::PlatformLocatorContext { context, source } => format!("{context}; {source}"),
            AppError::PlatformProcessContext { context, source } => format!("{context}; {source}"),
            AppError::PlatformEdtContext { context, source } => format!("{context}; {source}"),
            AppError::PlatformEdtSessionContext { context, source } => {
                format!("{context}; {source}")
            }
            AppError::ConfigContext { context, source } => format!("{context}; {source}"),
        };
        Self {
            detail,
            ..Self::new(kind, message)
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
    crate::domain::dump::PullAllResult,
    crate::domain::convert::ConvertResult,
    crate::domain::extensions::ExtensionsResult,
    crate::domain::extensions::ExtensionInventoryResult,
    crate::domain::init::InitResult,
    crate::domain::artifacts::ArtifactsResult,
    crate::domain::artifacts::MakeAllResult,
    crate::domain::infobase_export::DownloadAllResult,
    crate::domain::build::BuildResult,
    crate::domain::apply::ApplyResult,
    crate::domain::reset::ResetResult,
    crate::domain::diff::DiffResult,
    crate::domain::load::LoadResult,
    crate::domain::launch::LaunchResult,
    crate::domain::publish::PublishResult,
    crate::use_cases::full_pull_publication::FullPullReady<'_>,
);

/// The publisher inspects this intermediate receipt before the command can finish.
/// Like the final answer, it takes its dispatch fact only from the platform's mark.
pub(crate) fn stamp_prepared_dispatch<T: CarriesDispatch>(prepared: &mut T, work: &WorkGiven) {
    prepared.stamp_work(work);
}

/// Ставит `provider_dispatched` итогового ответа из отметки работы команды. Сценарии пишут
/// в конструкторах `false`, а вход отдаёт исход через этот штамп; промежуточную квитанцию
/// публикации отмечает `stamp_prepared_dispatch` через того же владельца `stamp_work`.
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
    use super::{
        stamp_dispatch, CarriesDispatch, EnvironmentReason, UseCaseError, UseCaseErrorKind,
        UseCaseFailure,
    };
    use crate::config::loader::ConfigLoadError;
    use crate::platform::designer::DesignerError;
    use crate::platform::edt_session::EdtSessionError;
    use crate::platform::ibcmd::IbcmdError;
    use crate::platform::locator::{
        LocatorError, PlatformVersion, PlatformVersionRequirement, UtilityType,
    };
    use crate::platform::process::{ProcessError, WorkGiven};
    use crate::support::error::{AppError, CancelledAt, CapabilityReason};
    use std::path::PathBuf;

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

    #[test]
    fn a_prepared_full_pull_receipt_takes_dispatch_only_from_the_actual_work_mark() {
        use crate::domain::dump::{DumpMode, ReportedDumpMode};
        let work = WorkGiven::for_command();
        let mut ready = crate::use_cases::full_pull_publication::FullPullReady {
            command: crate::use_cases::context::CommandName::Dump,
            requested_mode: DumpMode::Full,
            mode: ReportedDumpMode::Full,
            extension: None,
            mode_reason: None,
            provider_dispatched: false,
            source_set: "main",
            target: std::path::Path::new("sources"),
            stage: std::path::Path::new("stage"),
            snapshot_prepared: true,
            snapshot_warning: None,
        };
        super::stamp_prepared_dispatch(&mut ready, &work);
        assert!(!ready.provider_dispatched);
        work.mark_work_given();
        super::stamp_prepared_dispatch(&mut ready, &work);
        assert!(ready.provider_dispatched);
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
        assert_eq!(
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable).exit_code(),
            2
        );
        assert_eq!(UseCaseErrorKind::WorkspaceBusy.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::InfobaseBusy.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::NonFastForward.exit_code(), 3);
        assert_eq!(UseCaseErrorKind::UntrackedBase.exit_code(), 3);
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

    /// Снятый процесс, конец которого не подтверждён, — сбой платформы, а не отмена.
    #[test]
    fn an_interruption_whose_end_is_unconfirmed_is_a_platform_failure() {
        let error = UseCaseError::from(AppError::PlatformProcess(
            ProcessError::InterruptedEndUnconfirmed {
                cmd: "1cv8 DESIGNER".to_owned(),
                source: std::io::Error::other("wait refused"),
            },
        ));

        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert!(error.message().contains("was not confirmed"));
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
        let app_error = AppError::from(IbcmdError::from(
            crate::config::model::InfobaseConfig::file("File=/ib")
                .dbms_access()
                .err()
                .expect("no dbms section"),
        ))
        .with_context("failed to build ibcmd connection");
        assert!(matches!(app_error, AppError::ValidationIbcmdContext { .. }));

        let error = UseCaseError::from(app_error);

        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(error.message().contains("failed to build ibcmd connection"));
        assert!(error.message().contains("infobase.dbms.kind"));
    }

    /// Утилиты нет или не та версия — род `environment` с кодом выхода 2, с уточнением и без:
    /// поставьте её, и заработает. Род `platform` оставлен за сбоем самой платформы.
    #[test]
    fn a_missing_utility_is_an_environment_failure() {
        let missing = || LocatorError::NotFound {
            utility: UtilityType::EdtCli,
            detail: None,
        };
        for app_error in [
            AppError::from(missing()),
            AppError::from(missing()).with_context("failed to resolve 1cedtcli"),
        ] {
            let error = UseCaseError::from(app_error);

            assert_eq!(
                error.kind(),
                UseCaseErrorKind::Environment(EnvironmentReason::Unavailable),
                "{error}"
            );
            assert_eq!(error.exit_code(), 2, "{error}");
            assert!(error.message().contains("was not found"), "{error}");
            assert_eq!(
                error.environment_layer(),
                Some(crate::support::error::EnvironmentLayer::Platform),
                "{error}"
            );
        }
    }

    /// Утилита нашлась, но не той версии, или версию прочитать нельзя — тоже дело окружения:
    /// поставьте подходящую, и заработает.
    #[test]
    fn an_unsuitable_utility_version_is_an_environment_failure() {
        let required = || PlatformVersionRequirement::parse("8.3.25").expect("requirement");
        for locator_error in [
            LocatorError::VersionMismatch {
                utility: UtilityType::Ibcmd,
                path: PathBuf::from("/opt/1cv8/8.3.24.1000/ibcmd"),
                required: required(),
                found: PlatformVersion::parse_strict("8.3.24.1000").expect("version"),
            },
            LocatorError::UnknownVersion {
                utility: UtilityType::Ibcmd,
                path: PathBuf::from("/opt/custom/ibcmd"),
                required: required(),
            },
        ] {
            let error = UseCaseError::from(AppError::from(locator_error));

            assert_eq!(
                error.kind(),
                UseCaseErrorKind::Environment(EnvironmentReason::Unavailable),
                "{error}"
            );
            assert_eq!(error.exit_code(), 2, "{error}");
            assert!(error.message().contains("required"), "{error}");
        }

        let error = UseCaseError::from(AppError::from(LocatorError::BelowMinimumPlatform {
            utility: UtilityType::V8,
            path: PathBuf::from("/opt/1cv8/8.3.23.1000/1cv8"),
            found: PlatformVersion::parse_strict("8.3.23.1000").expect("version"),
        }));
        assert_eq!(
            error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable),
            "{error}"
        );
        assert_eq!(error.exit_code(), 2, "{error}");
        assert!(error.message().contains("8.3.24"), "{error}");
    }

    #[test]
    fn contextual_designer_errors_keep_typed_platform_source() {
        let app_error = AppError::from(DesignerError::StaleLogCleanup {
            path: "/tmp/designer.log".into(),
            source: std::io::Error::other("busy"),
        })
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
        assert!(error
            .message()
            .contains("failed to remove previous designer /Out log"));
    }
}
