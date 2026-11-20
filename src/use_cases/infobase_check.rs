use std::io::ErrorKind;
use std::time::Duration;

use crate::config::model::AppConfig;
use crate::platform::designer::DesignerDsl;
use crate::platform::file_base_hold::DATABASE_FILE;
use crate::platform::locator::{PlatformVersion, UtilityType, UtilityVersion};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::{AppError, EnvironmentLayer};
use crate::support::temp::private_temp_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};

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
    check_with_designer(config, &designer, version)
}

fn check_with_designer(
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
        let infobase = &config.infobase;
        let secrets = [infobase.user.as_deref(), infobase.password.as_deref()];
        let secrets: Vec<_> = secrets
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        let evidence = super::ibcmd_diagnostics::format_failure_evidence(
            format!("the Designer could not open the infobase (exit code {code})"),
            &result.process.stdout,
            &result.process.stderr,
            result.platform_log.as_deref(),
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
