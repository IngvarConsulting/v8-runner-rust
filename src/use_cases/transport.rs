use std::future::Future;

use crate::config::model::AppConfig;
use crate::use_cases::context::CommandName;
use crate::use_cases::result::UseCaseError;
#[cfg(test)]
use crate::use_cases::result::UseCaseFailure;
use crate::use_cases::workspace_lock::{acquire_workspace_lock, WorkspaceLockGuard};

/// Runs an adapter dispatch under the shared workspace lock.
///
/// Занятый каталог отказывает здесь своим родом `WorkspaceBusy` для всякой команды: словарь
/// провода выбирает транспорт, а не граница замка.
pub fn dispatch_with_workspace_lock<TResult>(
    config: &AppConfig,
    command: CommandName,
    before_dispatch: impl FnOnce() -> Result<(), UseCaseError>,
    run: impl FnOnce() -> TResult,
) -> Result<TResult, UseCaseError> {
    let _workspace_lock = acquire(config, command)?;
    before_dispatch()?;
    Ok(run())
}

/// Та же граница для асинхронного сценария: замок держится, пока сценарий не дошёл до
/// конечного состояния, и снимается вместе с его будущим — раньше, чем вызывающий
/// отпустит что-то своё. Сценарий создаётся уже под замком.
pub(crate) async fn dispatch_with_workspace_lock_async<TFuture>(
    config: &AppConfig,
    command: CommandName,
    run: impl FnOnce() -> TFuture,
) -> Result<TFuture::Output, UseCaseError>
where
    TFuture: Future,
{
    let _workspace_lock = acquire(config, command)?;
    Ok(run().await)
}

fn acquire(config: &AppConfig, command: CommandName) -> Result<WorkspaceLockGuard, UseCaseError> {
    acquire_workspace_lock(config, command.as_str()).map_err(UseCaseError::from)
}

/// Maps a use-case failure payload into a transport-specific response while preserving the
/// original transport-neutral error for the adapter boundary.
#[cfg(test)]
pub fn map_failure_response<TPayload, TResponse, FPayload, FFallback>(
    failure: UseCaseFailure<TPayload>,
    payload_mapper: FPayload,
    fallback_response: FFallback,
) -> (UseCaseError, TResponse)
where
    FPayload: FnOnce(TPayload) -> TResponse,
    FFallback: FnOnce(&UseCaseError) -> TResponse,
{
    let error = failure.error;
    let response = match failure.payload {
        Some(payload) => payload_mapper(payload),
        None => fallback_response(&error),
    };
    (error, response)
}

#[cfg(test)]
mod tests {
    use super::{dispatch_with_workspace_lock, map_failure_response};
    use crate::config::model::{
        AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::support::fs::acquire_advisory_lock;
    use crate::use_cases::context::CommandName;
    use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure};
    use crate::use_cases::workspace_lock::workspace_lock_path;
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn sample_config(work_path: &Path) -> AppConfig {
        AppConfig {
            base_path: work_path.join("base"),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("main"),
            }],
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn maps_failure_payload_without_losing_transport_neutral_error() {
        let (error, response) = map_failure_response(
            UseCaseFailure::with_payload(
                UseCaseError::new(UseCaseErrorKind::Runtime, "boom"),
                41_u32,
            ),
            |value| value + 1,
            |_| 0,
        );

        assert_eq!(error.kind(), UseCaseErrorKind::Runtime);
        assert_eq!(error.message(), "boom");
        assert_eq!(response, 42);
    }

    #[test]
    fn dispatch_with_workspace_lock_stops_before_run_when_workspace_is_busy() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = fs::canonicalize(&config.work_path).expect("canonical work");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let ran = Cell::new(false);

        let error = dispatch_with_workspace_lock(
            &config,
            CommandName::Build,
            || Ok(()),
            || {
                ran.set(true);
            },
        )
        .expect_err("busy workspace");

        assert_eq!(error.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(!ran.get());
    }
}
