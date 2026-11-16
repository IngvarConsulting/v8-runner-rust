use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::cli::args::SessionCommand;
use crate::cli::execute::with_cli_workspace_lock;
use crate::cli::output::{failure_envelope, print_command_use_case_error};
use crate::command_envelope::Envelope;
use crate::config::model::AppConfig;
use crate::domain::sessions::SessionsResult;
use crate::output::presenter::Presenter;
use crate::use_cases::context::{CommandName, ExecutionContext};
use crate::use_cases::infobase_lock::BaseAccess;
use crate::use_cases::result::UseCaseError;
use crate::use_cases::sessions::{self, Request, Terminate};

pub(super) const fn command_name(command: &SessionCommand) -> CommandName {
    match command {
        SessionCommand::List => CommandName::SessionsList,
        SessionCommand::Terminate { .. } => CommandName::SessionsTerminate,
        SessionCommand::Deny { .. } => CommandName::SessionsDeny,
        SessionCommand::Allow => CommandName::SessionsAllow,
    }
}

pub(super) fn execute(
    config: &AppConfig,
    command: &SessionCommand,
    presenter: &Presenter,
    clean_before_execution: bool,
    cancellation: CancellationToken,
) -> Result<(), UseCaseError> {
    let name = command_name(command);
    let context = ExecutionContext::cli(name).with_cancellation(cancellation);
    let request = match command {
        SessionCommand::List => Request::List,
        SessionCommand::Terminate { id, message, .. } => Request::Terminate {
            selection: id.map_or(Terminate::All, Terminate::One),
            message: message.as_deref(),
        },
        SessionCommand::Deny {
            message,
            code,
            jobs,
        } => Request::Deny {
            message: message.as_deref(),
            code: code.as_deref(),
            jobs: *jobs,
        },
        SessionCommand::Allow => Request::Allow,
    };
    let access = if matches!(request, Request::List) {
        BaseAccess::Reads
    } else {
        BaseAccess::Writes
    };
    with_cli_workspace_lock(
        config,
        presenter,
        name,
        access,
        clean_before_execution,
        false,
        || {
            let started = Instant::now();
            let duration_ms = || started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
            match sessions::execute(&context, config, request) {
                Ok(result) => {
                    if let SessionsResult::Lock(lock) = &result {
                        presenter.note_leading_warnings(name.as_str(), &lock.warnings);
                    }
                    if presenter.is_json() {
                        presenter.print_envelope(&Envelope::ok(
                            name.as_str(),
                            duration_ms(),
                            result,
                        ));
                    } else {
                        render(presenter, &result);
                    }
                    Ok(())
                }
                Err(failure) => {
                    match failure.payload {
                        Some(result) if presenter.is_json() => presenter.print_envelope(
                            &failure_envelope(name.as_str(), duration_ms(), result, &failure.error),
                        ),
                        Some(result) => {
                            render(presenter, &result);
                            presenter.print_error(&failure.error.to_string());
                        }
                        None => print_command_use_case_error(presenter, name, &failure.error),
                    }
                    Err(failure.error)
                }
            }
        },
    )
}

fn render(presenter: &Presenter, result: &SessionsResult) {
    let (base, managed_ras) = match result {
        SessionsResult::List(result) => (&result.infobase, result.managed_ras),
        SessionsResult::Terminate(result) => (&result.infobase, result.managed_ras),
        SessionsResult::Lock(result) => (&result.infobase, result.managed_ras),
    };
    presenter.print_bare(&format!("Infobase {base}"));
    if managed_ras {
        presenter.print_bare("RAS was started by the runner for this command");
    }
    match result {
        SessionsResult::List(result) => {
            if result.sessions.is_empty() {
                presenter.print_bare("No sessions");
            }
            for session in &result.sessions {
                let state = if session.excluded {
                    " (excluded from --all)"
                } else {
                    ""
                };
                presenter.print_bare(&format!(
                    "{}  {}  {}  {}  {}{state}",
                    session.id.as_deref().unwrap_or_default(),
                    session.user.as_deref().unwrap_or_default(),
                    session.application_title,
                    session.computer,
                    session.started,
                ));
            }
        }
        SessionsResult::Terminate(result) => {
            presenter.print_bare(&format!(
                "Terminated {} sessions",
                result.terminated_session_ids.len()
            ));
            for id in &result.terminated_session_ids {
                presenter.print_bare(id);
            }
            for error in &result.errors {
                presenter.print_error(&format!("{}: {}", error.session_id, error.message));
            }
        }
        SessionsResult::Lock(result) => {
            presenter.print_bare(&format!(
                "New sessions denied: {}. Scheduled jobs denied: {}.",
                result.sessions_denied, result.jobs_denied,
            ));
        }
    }
}
