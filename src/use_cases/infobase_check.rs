use std::io::ErrorKind;
use std::time::Duration;

use crate::config::model::AppConfig;
use crate::platform::designer::DesignerDsl;
use crate::platform::file_base_hold::DATABASE_FILE;
use crate::platform::locator::{PlatformVersion, UtilityType, UtilityVersion};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::{AppError, EnvironmentLayer};
use crate::support::temp::private_temp_dir;
use crate::use_cases::context::{ExecutionContext, ExecutionTransport, InterruptionSafetyClass};

use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

#[derive(Debug, Default, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct ConnectionChecked {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_version: Option<String>,
}

pub(crate) fn check(
    context: &ExecutionContext,
    config: &AppConfig,
) -> Result<ConnectionChecked, UseCaseError> {
    if let Some(error) =
        crate::use_cases::interruption::pending_interruption_error(context, "the infobase check")
    {
        return Err(error.into());
    }
    let connection = config.v8_connection();
    let args = connection.infobase_args();
    if !matches!(args.as_slice(), [switch, _]
        if ["/F", "-F", "/S", "-S", "/IBConnectionString"]
            .iter().any(|allowed| switch.eq_ignore_ascii_case(allowed)))
    {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            "infobase check requires a single address; specify the user and password in the selected infobase settings",
        ));
    }
    if let Some(base) = connection.file_infobase_dir(&config.base_path) {
        let database = base.join(DATABASE_FILE);
        match database.metadata() {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err(AppError::Validation(format!(
                    "{} is not an infobase file",
                    database.display()
                ))
                .into());
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::InfobaseNotFound,
                    "the infobase file does not exist",
                ));
            }
            Err(error) => {
                return Err(AppError::Runtime(format!(
                    "could not inspect {}: {error}",
                    database.display()
                ))
                .into());
            }
        }
    } else if !config.connection_declared() {
        return Err(AppError::Validation("no native infobase address declared".to_owned()).into());
    }

    let mut utilities = PlatformUtilities::from_config(config);
    let location = utilities.locate(UtilityType::V8).map_err(AppError::from)?;
    let Some(UtilityVersion::Platform(version)) = location.version else {
        return Err(AppError::environment_at(
            EnvironmentLayer::Platform,
            "no exact version is known for the selected 1cv8 executable",
        )
        .into());
    };
    let designer = DesignerDsl::new(
        location.path,
        connection,
        utilities.runner_for(UtilityType::V8),
        None,
        context.process_policy(
            InterruptionSafetyClass::GracefulThenKill,
            Some(Duration::from_secs(60)),
        ),
    );
    check_with_designer(context, config, &designer, version)
}

fn check_with_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    designer: &DesignerDsl<'_>,
    version: PlatformVersion,
) -> Result<ConnectionChecked, UseCaseError> {
    let scratch = private_temp_dir(&config.work_path, "infobase-check-")
        .map_err(|error| AppError::Runtime(format!("prepare the infobase check: {error}")))?;
    let result = designer
        .probe_infobase(scratch.path())
        .map_err(|error| match error {
            error @ crate::platform::designer::DesignerError::Spawn(
                crate::platform::process::ProcessError::TimedOut { .. },
            ) => AppError::TimedOut(error.to_string()),
            error => AppError::from(error),
        });
    let cleanup = scratch.close();
    let result = match (result, cleanup) {
        (Ok(result), Ok(())) => result,
        (Ok(_), Err(error)) => {
            return Err(UseCaseError::from(AppError::Runtime(format!(
                "clean up the infobase check: {error}"
            ))))
        }
        (Err(error), cleanup) => {
            let error = UseCaseError::from(error);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => {
                    error.with_context(format!("clean up the infobase check: {cleanup}"))
                }
            });
        }
    };
    if let Err(code) = result.process.outcome() {
        if context.transport() == ExecutionTransport::Cli {
            match classify_login_failure(result.platform_log.as_deref()) {
                UseCaseErrorKind::CredentialsRejected => {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::CredentialsRejected,
                        "the Designer rejected the infobase credentials",
                    ))
                }
                UseCaseErrorKind::InfobaseDesignerBusy => {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::InfobaseDesignerBusy,
                        "another Designer has the infobase open",
                    ))
                }
                _ => {}
            }
        }
        let infobase = &config.infobase;
        let secrets = [infobase.user.as_deref(), infobase.password.as_deref()];
        let secrets: Vec<_> = secrets
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        let log = result.platform_log.as_deref().filter(|log| {
            context.transport() != ExecutionTransport::Cli
                || !probe_lines(log).any(|line| line == SESSIONS)
        });
        let evidence = super::ibcmd_diagnostics::format_failure_evidence(
            format!("the Designer could not open the infobase (exit code {code})"),
            &result.process.stdout,
            &result.process.stderr,
            log,
            None,
        );
        let message = crate::platform::secrets::mask_text(&evidence, &secrets);
        return Err(UseCaseError::new(
            UseCaseErrorKind::ConnectionFailed,
            message.chars().take(2048).collect::<String>(),
        ));
    }
    if DesignerDsl::generation(&result).is_none() {
        return Err(AppError::InvalidOutput(
            "the Designer probe did not return a configuration generation token".to_owned(),
        )
        .into());
    }
    Ok(ConnectionChecked {
        platform_version: Some(version.to_string()),
    })
}

const CREDENTIALS: &str = "Пользователь ИБ не идентифицирован";
const LOCK: &str = "Ошибка блокировки информационной базы для конфигурирования.";
const DESIGNER: &str = "Возможно, информационная база уже открыта Конфигуратором.";
const SESSIONS: &str = "С этой информационной базой уже работает конфигуратор:";

fn probe_lines(log: &str) -> impl Iterator<Item = &str> {
    log.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

/// Только полные сигнатуры приватного /Out этой пробы (CTR.WIRE.INFOBASE-CHECK-DATA).
fn classify_login_failure(log: Option<&str>) -> UseCaseErrorKind {
    let lines: Vec<_> = probe_lines(log.unwrap_or_default()).collect();
    match lines.as_slice() {
        [CREDENTIALS] => UseCaseErrorKind::CredentialsRejected,
        [LOCK, DESIGNER] => UseCaseErrorKind::InfobaseDesignerBusy,
        [LOCK, DESIGNER, SESSIONS, session] if is_designer_session_row(session) => {
            UseCaseErrorKind::InfobaseDesignerBusy
        }
        _ => UseCaseErrorKind::ConnectionFailed,
    }
}

/// The observed clustered reply has exactly one row with these fields.
fn is_designer_session_row(line: &str) -> bool {
    const FIELDS: [&str; 5] = ["компьютер", "пользователь", "сеанс", "начат", "приложение"];
    let mut values = line.split(", ");
    FIELDS.iter().all(|field| {
        values
            .next()
            .and_then(|value| value.split_once(':'))
            .is_some_and(|(key, value)| {
                key.trim().to_lowercase() == *field && !value.trim().is_empty()
            })
    }) && values.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::{classify_login_failure, UseCaseErrorKind};

    #[test]
    fn only_complete_unambiguous_probe_signatures_are_classified() {
        let credentials = "Пользователь ИБ не идентифицирован";
        let busy = "Ошибка блокировки информационной базы для конфигурирования.\r\nВозможно, информационная база уже открыта Конфигуратором.";
        let heading = "С этой информационной базой уже работает конфигуратор:";
        for (log, expected) in [
            (format!("\u{feff}{credentials}\r\n"), "credentials_rejected"),
            (format!("{busy}\r\n"), "infobase_designer_busy"),
            (
                format!("{busy}\n{heading}\nКомпьютер: test, Пользователь: test, Сеанс: 1, Начат: test, Приложение: test"),
                "infobase_designer_busy",
            ),
            (format!("{busy}\n{heading}\n"), "connection_failed"),
            (format!("{busy}\n{heading}\nОшибка лицензии"), "connection_failed"),
            (
                format!("Ошибка лицензии\n{credentials}"),
                "connection_failed",
            ),
            (format!("{credentials}\n{busy}"), "connection_failed"),
            (
                format!("{busy}\n{heading}\n{credentials}"),
                "connection_failed",
            ),
            (format!("prefix: {credentials}"), "connection_failed"),
        ] {
            assert_eq!(
                classify_login_failure(Some(&log)).execution_step_code(),
                expected,
                "{log}"
            );
        }
        assert_eq!(
            classify_login_failure(None),
            UseCaseErrorKind::ConnectionFailed
        );
    }
}
