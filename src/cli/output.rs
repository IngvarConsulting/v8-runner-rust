use serde::Serialize;

use crate::command_envelope::{Envelope, EnvelopeError, ErrorCode, ErrorKind};
use crate::output::presenter::Presenter;
use crate::support::error::CapabilityReason;
use crate::use_cases::context::CommandName;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

pub fn print_command_error(
    presenter: &Presenter,
    command: &str,
    error: &UseCaseError,
    text_message: &str,
) {
    if presenter.is_json() {
        presenter.print_envelope(&pre_dispatch_error_envelope(command, error));
    } else {
        presenter.print_error(text_message);
    }
}

pub fn print_command_use_case_error(
    presenter: &Presenter,
    command: CommandName,
    error: &UseCaseError,
) {
    print_command_error(presenter, command.as_str(), error, &error.to_string());
}

/// `data` отказа, случившегося до того, как команда начала работу.
///
/// Предмета у такого ответа нет — платформа не запускалась, план не строился, — поэтому
/// форма несёт только текст отказа. Машинная часть причины живёт в `error` конверта.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RefusalData {
    pub message: String,
}

pub fn pre_dispatch_error_envelope(command: &str, error: &UseCaseError) -> Envelope<RefusalData> {
    let data = RefusalData {
        message: error.message().to_owned(),
    };
    failure_envelope(command, 0, data, error)
}

pub fn failure_envelope<T: Serialize>(
    command: impl Into<String>,
    duration_ms: u64,
    data: T,
    error: &UseCaseError,
) -> Envelope<T> {
    with_cli_error(Envelope::err(command, duration_ms, data), error)
}

pub fn with_cli_error<T: Serialize>(envelope: Envelope<T>, error: &UseCaseError) -> Envelope<T> {
    envelope.with_error(cli_envelope_error(error))
}

fn cli_envelope_error(error: &UseCaseError) -> EnvelopeError {
    let (code, kind) = cli_error_contract(error.kind());
    EnvelopeError::new(code, kind, error.message()).with_next(error.next().cloned())
}

/// Род отказа определяет пару «код, род» на проводе. Причина возможности различает коды
/// внутри одного рода: подбор команд на сайте зовёт их `subject`, `target` и `soon`.
pub(crate) const fn cli_error_contract(kind: UseCaseErrorKind) -> (ErrorCode, ErrorKind) {
    match kind {
        UseCaseErrorKind::Capability(reason) => (
            match reason {
                CapabilityReason::Unavailable => ErrorCode::CapabilityUnavailable,
                CapabilityReason::Subject => ErrorCode::Subject,
                CapabilityReason::Target => ErrorCode::Target,
                CapabilityReason::Soon => ErrorCode::Soon,
            },
            ErrorKind::Capability,
        ),
        UseCaseErrorKind::Environment => {
            (ErrorCode::EnvironmentUnavailable, ErrorKind::Environment)
        }
        UseCaseErrorKind::WorkspaceBusy => (ErrorCode::WorkspaceBusy, ErrorKind::Workspace),
        UseCaseErrorKind::InvalidOutput => (ErrorCode::InvalidOutput, ErrorKind::InvalidOutput),
        UseCaseErrorKind::Cancelled(_) => (ErrorCode::Cancelled, ErrorKind::Interruption),
        UseCaseErrorKind::TimedOut => (ErrorCode::TimedOut, ErrorKind::Interruption),
        UseCaseErrorKind::Validation => (ErrorCode::InvalidArgument, ErrorKind::Validation),
        UseCaseErrorKind::Runtime => (ErrorCode::RuntimeFailure, ErrorKind::Runtime),
        UseCaseErrorKind::Platform => (ErrorCode::PlatformFailure, ErrorKind::Platform),
    }
}
