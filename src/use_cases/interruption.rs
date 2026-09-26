use crate::domain::execution::{
    ExecutionInterruptionDetails, ExecutionInterruptionKind, ExecutionInterruptionPhase,
    ExecutionStatus,
};
use crate::platform::process::ProcessInterruptionReason;
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
/// собранные из одного факта одними словами.
pub(crate) fn deferred_process_interruption(
    phase: ExecutionInterruptionPhase,
    completed_action: &str,
    result: &PlatformCommandResult,
) -> Option<(String, ExecutionInterruptionDetails)> {
    result.process.interruption.map(|interruption| {
        let warning = deferred_process_interruption_message(completed_action, interruption.reason);
        let details =
            process_interruption_details(interruption.reason, phase, true, warning.clone());
        (warning, details)
    })
}

pub(crate) fn deferred_process_interruption_warning(
    completed_action: &str,
    result: &PlatformCommandResult,
) -> Option<String> {
    result.process.interruption.map(|interruption| {
        deferred_process_interruption_message(completed_action, interruption.reason)
    })
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

pub(crate) fn deferred_process_interruption_message(
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
        deferred_interruption_warning, deferred_interruption_warning_for_command,
        process_interruption_details, SafePoint, SafePointCancel,
    };

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
