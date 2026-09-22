use crate::domain::execution::{
    ExecutionInterruptionDetails, ExecutionInterruptionKind, ExecutionStatus,
};
use crate::platform::process::ProcessInterruptionReason;
use crate::platform::result::PlatformCommandResult;
use crate::support::error::AppError;

use super::context::{CommandName, ExecutionContext, ExecutionInterruption};

pub(crate) fn command_interruption_status(interruption: ExecutionInterruption) -> ExecutionStatus {
    match interruption {
        ExecutionInterruption::Cancelled => ExecutionStatus::Cancelled,
    }
}

pub(crate) fn command_interruption_details(
    interruption: ExecutionInterruption,
    phase: &str,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    command_interruption_details_with_deferred(interruption, phase, false, message)
}

pub(crate) fn deferred_command_interruption_details(
    interruption: ExecutionInterruption,
    phase: &str,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    command_interruption_details_with_deferred(interruption, phase, true, message)
}

pub(crate) fn process_interruption_details(
    interruption: ProcessInterruptionReason,
    phase: &str,
    deferred: bool,
    message: impl Into<String>,
) -> ExecutionInterruptionDetails {
    ExecutionInterruptionDetails::new(process_interruption_kind(interruption), deferred)
        .with_phase(phase)
        .with_message(message)
}

pub(crate) fn deferred_process_interruption_details(
    phase: &str,
    completed_action: &str,
    result: &PlatformCommandResult,
) -> Option<ExecutionInterruptionDetails> {
    result.process.interruption.map(|interruption| {
        process_interruption_details(
            interruption.reason,
            phase,
            true,
            deferred_process_interruption_message(completed_action, interruption.reason),
        )
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

pub(crate) fn command_interruption_message(
    context: &ExecutionContext,
    interruption: ExecutionInterruption,
) -> String {
    format!(
        "{} for command '{}'",
        interruption.message(context.command()),
        context.command().as_str()
    )
}

pub(crate) fn pending_interruption_message(
    context: &ExecutionContext,
    interruption: ExecutionInterruption,
    phase: impl AsRef<str>,
) -> String {
    format!(
        "{} {}",
        command_interruption_message(context, interruption),
        phase.as_ref()
    )
}

pub(crate) fn interruption_before_safe_point_message(
    context: &ExecutionContext,
    interruption: ExecutionInterruption,
    safe_point: impl AsRef<str>,
) -> String {
    pending_interruption_message(
        context,
        interruption,
        format!("before entering {} safe point", safe_point.as_ref()),
    )
}

pub(crate) fn pending_interruption_error(
    context: &ExecutionContext,
    phase: impl AsRef<str>,
) -> Option<AppError> {
    context.interruption().map(|interruption| {
        AppError::Runtime(pending_interruption_message(context, interruption, phase))
    })
}

pub(crate) fn interruption_before_safe_point(
    context: &ExecutionContext,
    safe_point: impl AsRef<str>,
) -> Option<AppError> {
    context.interruption().map(|interruption| {
        AppError::Runtime(interruption_before_safe_point_message(
            context,
            interruption,
            safe_point.as_ref(),
        ))
    })
}

fn command_interruption_details_with_deferred(
    interruption: ExecutionInterruption,
    phase: &str,
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
    use crate::platform::process::ProcessInterruptionReason;
    use crate::use_cases::context::{CommandName, ExecutionInterruption};

    use super::{
        command_interruption_status, deferred_interruption_warning,
        deferred_interruption_warning_for_command, process_interruption_details,
    };

    #[test]
    fn command_interruption_status_preserves_terminal_state() {
        assert_eq!(
            command_interruption_status(ExecutionInterruption::Cancelled),
            crate::domain::execution::ExecutionStatus::Cancelled
        );
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
            "run",
            true,
            "deferred",
        );

        assert!(details.deferred);
        assert_eq!(details.phase.as_deref(), Some("run"));
        assert_eq!(details.message.as_deref(), Some("deferred"));
    }
}
