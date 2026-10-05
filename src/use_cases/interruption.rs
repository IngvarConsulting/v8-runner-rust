use crate::domain::execution::{
    ExecutionInterruptionDetails, ExecutionInterruptionKind, ExecutionInterruptionPhase,
    ExecutionOutcome, ExecutionStatus,
};
use crate::platform::process::{ProcessInterruption, ProcessInterruptionReason};
use crate::platform::result::PlatformCommandResult;
use crate::support::error::{AppError, CancelledAt};

use super::context::{CommandName, ExecutionContext, ExecutionInterruption};

/// Какую безопасную точку заметила команда: текст отмены её называет.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SafePoint<'a> {
    /// Точка команды без уточнения.
    Command,
    /// Точка, которую называет место: «during provider selection», «while waiting for …».
    Named(&'a str),
    /// Точка перед шагом: «before entering <шаг> safe point».
    Before(&'a str),
}

/// Отмена, которую команда заметила на своей безопасной точке. Всё, что ответ о ней
/// говорит, строится отсюда: ошибка — отмена на границе, запись — фаза `command_boundary`,
/// текст называет точку.
pub(crate) struct SafePointCancel {
    interruption: ExecutionInterruption,
    message: String,
}

impl SafePointCancel {
    /// Отмена на безопасной точке `point`, если она пришла.
    #[must_use]
    pub(crate) fn noticed(context: &ExecutionContext, point: SafePoint<'_>) -> Option<Self> {
        context.interruption().map(|interruption| {
            let command = format!(
                "{} for command '{}'",
                interruption_text(interruption),
                context.command().as_str()
            );
            let message = match point {
                SafePoint::Command => command,
                SafePoint::Named(place) => format!("{command} {place}"),
                SafePoint::Before(step) => {
                    format!("{command} before entering {step} safe point")
                }
            };
            Self {
                interruption,
                message,
            }
        })
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn status(&self) -> ExecutionStatus {
        match self.interruption {
            ExecutionInterruption::Cancelled => ExecutionStatus::Cancelled,
        }
    }

    /// Запись о прерывании: безопасная точка, работа команды не оборвана.
    pub(crate) fn record(&self) -> ExecutionInterruptionDetails {
        interruption_record(
            CancelledAt::Boundary,
            ExecutionInterruptionPhase::CommandBoundary,
            self.message.clone(),
        )
    }

    pub(crate) fn into_error(self) -> AppError {
        AppError::Cancelled {
            message: self.message,
            at: CancelledAt::Boundary,
        }
    }
}

/// Прерывание, которое принесла ошибка, как его пишет ответ. Ошибка не отмена — `None`:
/// сигнал, пришедший во время чужого отказа, её не переписывает.
pub(crate) fn cancellation_record(
    error: &AppError,
    work_phase: ExecutionInterruptionPhase,
    message: impl Into<String>,
) -> Option<ExecutionInterruptionDetails> {
    error
        .cancellation()
        .map(|at| interruption_record(at, work_phase, message))
}

/// Запись об отмене, остановившей команду в `at`. Безопасная точка — фаза
/// `command_boundary`, где бы её ни проверили; оборванная работа исполнителя — `work_phase`,
/// фаза места вызова.
pub(crate) fn interruption_record(
    at: CancelledAt,
    work_phase: ExecutionInterruptionPhase,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    let phase = match at {
        CancelledAt::Boundary => ExecutionInterruptionPhase::CommandBoundary,
        CancelledAt::Work => work_phase,
    };
    command_interruption_details_with_deferred(
        ExecutionInterruption::Cancelled,
        phase,
        false,
        message,
    )
}

pub(crate) fn deferred_command_interruption_details(
    interruption: ExecutionInterruption,
    phase: ExecutionInterruptionPhase,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    command_interruption_details_with_deferred(interruption, phase, true, message)
}

pub(crate) fn process_interruption_details(
    interruption: ProcessInterruptionReason,
    phase: ExecutionInterruptionPhase,
    deferred: bool,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    ExecutionInterruptionDetails::new(process_interruption_kind(interruption), deferred)
        .with_phase(phase)
        .with_message(message)
}

/// Прерывание, которое процесс отложил и пережил: предупреждение и запись о прерывании,
/// собранные из одного факта одними словами. Факт — то же поле, что у результата процесса:
/// у удачи его берут из результата, у отказа — из `CommandFailure`.
pub(crate) fn deferred_process_interruption(
    phase: ExecutionInterruptionPhase,
    completed_action: &str,
    interruption: Option<ProcessInterruption>,
) -> Option<(String, ExecutionInterruptionDetails)> {
    interruption.map(|interruption| {
        let warning = deferred_process_interruption_message(completed_action, interruption.reason);
        let details =
            process_interruption_details(interruption.reason, phase, true, warning.clone());
        (warning, details)
    })
}

/// Предупреждения об отменах, которые критические команды шага отложили и пережили. Шаг
/// получает учёт от [`collecting_deferrals`] и отмечает каждую команду, как только она
/// кончилась, удачей или отказом: так предупреждение не теряют ни её собственный отказ, ни
/// следующая команда, ни безопасная точка.
#[derive(Debug, Default)]
pub(crate) struct Deferrals(Vec<String>);

impl Deferrals {
    /// Команда процесса платформы кончилась; удачу или отказ называет её исход.
    pub(crate) fn note_result(&mut self, action: &str, result: &PlatformCommandResult) {
        let end = match result.process.outcome() {
            Ok(()) => CommandEnd::Succeeded,
            Err(_code) => CommandEnd::Failed,
        };
        self.note(action, end, result.process.interruption);
    }

    /// Команда агента кончилась удачей или отказом.
    pub(crate) fn note_outcome<T, E>(
        &mut self,
        action: &str,
        outcome: &Result<T, E>,
        interruption: Option<ProcessInterruption>,
    ) {
        let end = if outcome.is_ok() {
            CommandEnd::Succeeded
        } else {
            CommandEnd::Failed
        };
        self.note(action, end, interruption);
    }

    fn note(&mut self, action: &str, end: CommandEnd, interruption: Option<ProcessInterruption>) {
        if let Some(interruption) = interruption {
            self.0.push(deferred_process_interruption_message(
                &end.words(action),
                interruption.reason,
            ));
        }
    }
}

/// Чем кончилась команда, которая отложила отмену.
#[derive(Debug, Clone, Copy)]
enum CommandEnd {
    Succeeded,
    Failed,
}

impl CommandEnd {
    /// Удачу предупреждение называет удачей, а команду, кончившуюся отказом, — просто
    /// кончившейся.
    fn words(self, action: &str) -> String {
        match self {
            Self::Succeeded => format!("{action} completed successfully"),
            Self::Failed => format!("{action} ended"),
        }
    }
}

/// Шаг с критическими командами: `body` отмечает их в учёте, а предупреждения идут в
/// ответ, чем бы шаг ни кончился. Удача отдаёт их вызывающему для сообщения шага, отказ
/// открывает ими свой текст; род и место отмены у ошибки те же.
pub(crate) fn collecting_deferrals<T>(
    body: impl FnOnce(&mut Deferrals) -> Result<T, AppError>,
) -> Result<(T, Vec<String>), AppError> {
    let mut deferrals = Deferrals::default();
    match body(&mut deferrals) {
        Ok(value) => Ok((value, deferrals.0)),
        Err(error) => Err(named_after(error, &deferrals.0)),
    }
}

/// Сообщение удачи, за которым идут предупреждения об отложенных отменах.
#[must_use]
pub(crate) fn append_warnings(message: String, warnings: &[String]) -> String {
    match (message.is_empty(), warnings.is_empty()) {
        (_, true) => message,
        (true, false) => joined(warnings),
        (false, false) => format!("{message}; {}", joined(warnings)),
    }
}

/// Текст отказа, который открывают предупреждения об отложенных отменах, — для ответа,
/// чей отказ несёт текст, а не ошибку; ошибку открывает [`collecting_deferrals`].
#[must_use]
pub(crate) fn prefix_warnings(warnings: &[String], message: String) -> String {
    if warnings.is_empty() {
        message
    } else {
        format!("{}; {message}", joined(warnings))
    }
}

/// Отказ после отложенных отмен: их предупреждения открывают его текст теми же словами,
/// что [`prefix_warnings`]. Род и место отмены у ошибки те же.
fn named_after(error: AppError, warnings: &[String]) -> AppError {
    if warnings.is_empty() {
        error
    } else {
        error.with_context(joined(warnings))
    }
}

/// Предупреждения одной строкой — тем же разделителем, что у `with_context`.
fn joined(warnings: &[String]) -> String {
    warnings.join("; ")
}

/// Отложенное прерывание — в форму с итогом исполнения: запись с `deferred: true` и
/// предупреждение, собранные из одного факта одними словами.
pub(crate) fn record_deferral<T>(
    phase: ExecutionInterruptionPhase,
    action: &str,
    interruption: Option<ProcessInterruption>,
    execution: &mut ExecutionOutcome<T>,
    warnings: &mut Vec<String>,
) {
    if let Some((warning, details)) = deferred_process_interruption(phase, action, interruption) {
        execution.interruptions.push(details);
        warnings.push(warning);
    }
}

/// Отказ команды платформы вместе с прерыванием, которое она отложила и пережила. Ошибку
/// он отдаёт только вместе с ним, так что ответ называет отложенную отмену и у неудачи:
/// оператор просил остановить, и ответ говорит, почему его не послушали.
#[derive(Debug)]
#[must_use]
pub(crate) struct CommandFailure {
    error: AppError,
    interruption: Option<ProcessInterruption>,
}

impl CommandFailure {
    /// Отказ, случившийся после того, как команда отложила прерывание.
    pub(crate) fn after(error: AppError, interruption: Option<ProcessInterruption>) -> Self {
        Self {
            error,
            interruption,
        }
    }

    /// Отказ, которому откладывать было нечего: он пришёл раньше критической команды или
    /// у сессии, которая её не ведёт.
    pub(crate) fn without_deferral(error: AppError) -> Self {
        Self::after(error, None)
    }

    /// Ошибка для формы с итогом исполнения: запись об отложенном прерывании и
    /// предупреждение идут в ответ раньше неё.
    #[must_use]
    pub(crate) fn record_into<T>(
        self,
        phase: ExecutionInterruptionPhase,
        action: &str,
        execution: &mut ExecutionOutcome<T>,
        warnings: &mut Vec<String>,
    ) -> AppError {
        record_deferral(phase, action, self.interruption, execution, warnings);
        self.error
    }

    /// Ошибка без записи о прерывании: отложенную отмену, если она была, называет её
    /// текст теми же словами, что у шага.
    #[must_use]
    pub(crate) fn into_error(self, action: &str) -> AppError {
        let mut deferrals = Deferrals::default();
        deferrals.note(action, CommandEnd::Failed, self.interruption);
        named_after(self.error, &deferrals.0)
    }
}

pub(crate) fn deferred_interruption_warning(
    completed_action: &str,
    interruption: ExecutionInterruption,
) -> String {
    format_deferred_interruption_warning(
        completed_action,
        command_interruption_reason(interruption),
        None,
    )
}

pub(crate) fn deferred_interruption_warning_for_command(
    completed_action: &str,
    command: CommandName,
    interruption: ExecutionInterruption,
) -> String {
    format_deferred_interruption_warning(
        completed_action,
        command_interruption_reason(interruption),
        Some(command),
    )
}

/// Ошибка отмены на безопасной точке, названной местом `place`, если отмена пришла.
#[must_use]
pub(crate) fn pending_interruption_error(
    context: &ExecutionContext,
    place: impl AsRef<str>,
) -> Option<AppError> {
    SafePointCancel::noticed(context, SafePoint::Named(place.as_ref()))
        .map(SafePointCancel::into_error)
}

/// Ошибка отмены перед безопасной точкой шага `step`, если отмена пришла.
#[must_use]
pub(crate) fn interruption_before_safe_point(
    context: &ExecutionContext,
    step: impl AsRef<str>,
) -> Option<AppError> {
    SafePointCancel::noticed(context, SafePoint::Before(step.as_ref()))
        .map(SafePointCancel::into_error)
}

/// Предупреждение об отмене, которую команда отложила до конца успешной операции.
pub(crate) fn deferred_interruption_warning_after(
    context: &ExecutionContext,
    completed_action: &str,
) -> Option<String> {
    context.interruption().map(|interruption| {
        deferred_interruption_warning_for_command(completed_action, context.command(), interruption)
    })
}

fn command_interruption_details_with_deferred(
    interruption: ExecutionInterruption,
    phase: ExecutionInterruptionPhase,
    deferred: bool,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    ExecutionInterruptionDetails::new(command_interruption_kind(interruption), deferred)
        .with_phase(phase)
        .with_message(message)
}

fn command_interruption_kind(interruption: ExecutionInterruption) -> ExecutionInterruptionKind {
    match interruption {
        ExecutionInterruption::Cancelled => ExecutionInterruptionKind::Cancelled,
    }
}

fn process_interruption_kind(interruption: ProcessInterruptionReason) -> ExecutionInterruptionKind {
    match interruption {
        ProcessInterruptionReason::Cancelled => ExecutionInterruptionKind::Cancelled,
        ProcessInterruptionReason::TimedOut => ExecutionInterruptionKind::TimedOut,
    }
}

fn interruption_text(interruption: ExecutionInterruption) -> &'static str {
    match interruption {
        ExecutionInterruption::Cancelled => {
            "execution cancelled before reaching a safe completion point"
        }
    }
}

fn command_interruption_reason(interruption: ExecutionInterruption) -> &'static str {
    match interruption {
        ExecutionInterruption::Cancelled => "cancellation request",
    }
}

fn process_interruption_reason(interruption: ProcessInterruptionReason) -> &'static str {
    match interruption {
        ProcessInterruptionReason::Cancelled => "cancellation request",
        ProcessInterruptionReason::TimedOut => "timeout",
    }
}

fn deferred_process_interruption_message(
    completed_action: &str,
    interruption: ProcessInterruptionReason,
) -> String {
    format_deferred_interruption_warning(
        completed_action,
        process_interruption_reason(interruption),
        None,
    )
}

fn format_deferred_interruption_warning(
    completed_action: &str,
    reason: &str,
    command: Option<CommandName>,
) -> String {
    match command {
        Some(command) => format!(
            "{completed_action} after {reason} for command '{}' during critical phase; unsafe interruption was not performed",
            command.as_str()
        ),
        None => format!(
            "{completed_action} after {reason} during critical phase; unsafe interruption was not performed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::execution::{ExecutionInterruptionPhase, ExecutionStatus};
    use crate::platform::process::ProcessInterruptionReason;
    use crate::support::error::CancelledAt;
    use crate::use_cases::context::{CommandName, ExecutionContext, ExecutionInterruption};

    use super::{
        append_warnings, collecting_deferrals, deferred_interruption_warning,
        deferred_interruption_warning_for_command, prefix_warnings, process_interruption_details,
        CommandFailure, SafePoint, SafePointCancel,
    };
    use crate::domain::execution::ExecutionOutcome;
    use crate::platform::process::{ProcessInterruption, ProcessResult};
    use crate::platform::result::PlatformCommandResult;
    use crate::support::error::AppError;

    fn finished(exit_code: i32, deferred: bool) -> PlatformCommandResult {
        PlatformCommandResult {
            process: ProcessResult {
                exit_code,
                stdout: String::new(),
                stderr: String::new(),
                interruption: deferred
                    .then(|| ProcessInterruption::deferred(ProcessInterruptionReason::Cancelled)),
            },
            platform_log_path: None,
            platform_log: None,
            platform_log_read_error: None,
        }
    }

    /// Учёт отложенных отмен: удача отдаёт предупреждения вызывающему, отказ открывает ими
    /// свой текст, а род и место отмены у ошибки остаются прежними. Удачу предупреждение
    /// называет удачей, команду, кончившуюся отказом, — кончившейся.
    #[test]
    fn a_step_names_its_deferrals_whatever_it_ends_with() {
        let ((), warnings) = collecting_deferrals(|deferrals| {
            deferrals.note_result("load", &finished(0, true));
            deferrals.note_result("update_db_cfg", &finished(0, false));
            Ok(())
        })
        .expect("the step succeeded");
        let [warning] = warnings.as_slice() else {
            panic!("one deferral: {warnings:?}");
        };
        assert!(
            warning.starts_with(
                "load completed successfully after cancellation request during critical phase"
            ),
            "{warning}"
        );

        let error = collecting_deferrals(|deferrals| -> Result<(), AppError> {
            deferrals.note_result("apply", &finished(17, true));
            Err(AppError::Cancelled {
                message: "stopped at the next safe point".to_owned(),
                at: CancelledAt::Boundary,
            })
        })
        .expect_err("the step stopped");
        assert_eq!(error.cancellation(), Some(CancelledAt::Boundary));
        let message = error.to_string();
        assert!(
            message.contains(
                "apply ended after cancellation request during critical phase; unsafe \
                 interruption was not performed; stopped at the next safe point"
            ),
            "{message}"
        );

        let error = collecting_deferrals(|deferrals| -> Result<(), AppError> {
            deferrals.note_outcome(
                "extension create",
                &Err::<(), ()>(()),
                Some(ProcessInterruption::deferred(
                    ProcessInterruptionReason::TimedOut,
                )),
            );
            Err(AppError::Platform("agent command failed".to_owned()))
        })
        .expect_err("the command failed");
        assert_eq!(
            error.to_string(),
            "platform error: extension create ended after timeout during critical phase; \
             unsafe interruption was not performed; agent command failed"
        );

        let untouched = collecting_deferrals(|_| -> Result<(), AppError> {
            Err(AppError::Runtime("no deferral".to_owned()))
        })
        .expect_err("the step failed");
        assert_eq!(untouched.to_string(), "runtime error: no deferral");
    }

    #[test]
    fn warnings_follow_a_success_and_lead_a_failure_text() {
        let warning = ["deferred".to_owned()];
        assert_eq!(append_warnings("done".to_owned(), &[]), "done");
        assert_eq!(
            append_warnings("done".to_owned(), &warning),
            "done; deferred"
        );
        assert_eq!(append_warnings(String::new(), &warning), "deferred");
        assert_eq!(prefix_warnings(&[], "failed".to_owned()), "failed");
        assert_eq!(
            prefix_warnings(&warning, "failed".to_owned()),
            "deferred; failed"
        );
    }

    /// Отказ команды отдаёт ошибку только вместе с отсрочкой: форма с итогом исполнения
    /// получает запись и предупреждение, текстовая — предупреждение в начале текста.
    #[test]
    fn a_command_failure_hands_out_its_error_only_with_its_deferral() {
        let deferred = Some(ProcessInterruption::deferred(
            ProcessInterruptionReason::Cancelled,
        ));
        let mut execution = ExecutionOutcome::<()>::new(ExecutionStatus::Failed);
        let mut warnings = Vec::new();
        let error = CommandFailure::after(AppError::Platform("agent failed".to_owned()), deferred)
            .record_into(
                ExecutionInterruptionPhase::ProviderCommand,
                "infobase DT restore",
                &mut execution,
                &mut warnings,
            );
        assert_eq!(error.to_string(), "platform error: agent failed");
        let [record] = execution.interruptions.as_slice() else {
            panic!("one record: {:?}", execution.interruptions);
        };
        assert!(record.deferred);
        assert_eq!(
            record.phase,
            Some(ExecutionInterruptionPhase::ProviderCommand)
        );
        let [warning] = warnings.as_slice() else {
            panic!("one warning: {warnings:?}");
        };
        assert!(
            warning.starts_with("infobase DT restore after cancellation request"),
            "{warning}"
        );

        let error = CommandFailure::after(AppError::Platform("agent failed".to_owned()), deferred)
            .into_error("artifact export");
        assert_eq!(
            error.to_string(),
            "platform error: artifact export ended after cancellation request during critical \
             phase; unsafe interruption was not performed; agent failed"
        );

        let error = CommandFailure::without_deferral(AppError::Platform("no".to_owned()))
            .into_error("artifact export");
        assert_eq!(error.to_string(), "platform error: no");
    }

    /// Отмена на безопасной точке — отмена на границе: статус `cancelled`, запись
    /// `command_boundary` без отсрочки, ошибка того же места. Текст называет точку.
    #[test]
    fn a_safe_point_cancel_is_a_cancellation_at_the_boundary() {
        let idle = ExecutionContext::cli(CommandName::Test);
        assert!(SafePointCancel::noticed(&idle, SafePoint::Command).is_none());

        let cancellation = tokio_util::sync::CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Test).with_cancellation(cancellation);
        for (point, tail) in [
            (SafePoint::Command, "for command 'test'"),
            (
                SafePoint::Named("during provider selection"),
                "for command 'test' during provider selection",
            ),
            (
                SafePoint::Before("run"),
                "for command 'test' before entering run safe point",
            ),
        ] {
            let cancel = SafePointCancel::noticed(&context, point).expect("pending cancel");
            assert!(cancel.message().ends_with(tail), "{}", cancel.message());
            assert_eq!(cancel.status(), ExecutionStatus::Cancelled);
            let record = cancel.record();
            assert!(!record.deferred);
            assert_eq!(
                record.phase,
                Some(ExecutionInterruptionPhase::CommandBoundary)
            );
            assert_eq!(
                cancel.into_error().cancellation(),
                Some(CancelledAt::Boundary)
            );
        }
    }

    #[test]
    fn deferred_warning_uses_shared_reason_vocabulary() {
        assert_eq!(
            deferred_interruption_warning(
                "operation completed successfully",
                ExecutionInterruption::Cancelled,
            ),
            "operation completed successfully after cancellation request during critical phase; unsafe interruption was not performed"
        );
        assert_eq!(
            deferred_interruption_warning_for_command(
                "dump publication completed",
                CommandName::Dump,
                ExecutionInterruption::Cancelled,
            ),
            "dump publication completed after cancellation request for command 'pull' during critical phase; unsafe interruption was not performed"
        );
    }

    #[test]
    fn process_details_preserve_deferred_flag() {
        let details = process_interruption_details(
            ProcessInterruptionReason::Cancelled,
            ExecutionInterruptionPhase::Run,
            true,
            "deferred",
        );

        assert!(details.deferred);
        assert_eq!(details.phase, Some(ExecutionInterruptionPhase::Run));
        assert_eq!(details.message.as_deref(), Some("deferred"));
    }
}
