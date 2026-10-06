use std::future::Future;

use tracing::warn;

use crate::config::model::AppConfig;
use crate::domain::infobase_export::InfobaseTransferPhase;
use crate::use_cases::context::CommandName;
use crate::use_cases::infobase_lock::{acquire_infobase_lock, BaseAccess, InfobaseLock};
use crate::use_cases::result::UseCaseError;
#[cfg(test)]
use crate::use_cases::result::UseCaseFailure;
use crate::use_cases::workspace_lock::{acquire_workspace_lock, CommandLockGuard};

/// Отказ границы команды: шаг и ошибка. Сценарий не запускался.
///
/// Шаг — `workspace lock`, `infobase lock` или `workspace preparation` из словаря фаз
/// домена: адаптер печатает его тем же шагом, что и отказы своих сценариев.
#[derive(Debug)]
pub struct BoundaryRefusal {
    pub phase: InfobaseTransferPhase,
    pub error: UseCaseError,
}

/// Runs an adapter dispatch under the shared workspace lock and, for a command that opens a
/// file infobase, under the infobase lock taken after it.
///
/// Занятый каталог отказывает здесь своим родом `WorkspaceBusy`, занятая база —
/// `InfobaseBusy` для всякой команды: словарь провода выбирает транспорт, а не граница
/// замка. `before_dispatch` идёт под обоими замками и получает предупреждение команды
/// чтения, которой замок базы не достался.
pub fn dispatch_with_workspace_lock<TResult>(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
    before_dispatch: impl FnOnce(Option<&str>) -> Result<(), UseCaseError>,
    run: impl FnOnce() -> TResult,
) -> Result<TResult, BoundaryRefusal> {
    let (_workspace_lock, infobase_lock) = acquire(config, command, base)?;
    before_dispatch(infobase_lock.warning()).map_err(|error| BoundaryRefusal {
        phase: InfobaseTransferPhase::WorkspacePreparation,
        error,
    })?;
    Ok(run())
}

/// Та же граница для асинхронного сценария: замки держатся, пока сценарий не дошёл до
/// конечного состояния, и снимаются вместе с его будущим — раньше, чем вызывающий
/// отпустит что-то своё. Сценарий создаётся уже под замками; предупреждение команды
/// чтения без замка базы уходит в журнал.
pub(crate) async fn dispatch_with_workspace_lock_async<TFuture>(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
    run: impl FnOnce() -> TFuture,
) -> Result<TFuture::Output, UseCaseError>
where
    TFuture: Future,
{
    let (_workspace_lock, infobase_lock) =
        acquire(config, command, base).map_err(|refusal| refusal.error)?;
    if let Some(warning) = infobase_lock.warning() {
        warn!(command = command.as_str(), "{warning}");
    }
    Ok(run().await)
}

/// Замок `workPath`, затем замок базы. Порядок сброса обратный: база отпускается раньше
/// каталога.
fn acquire(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
) -> Result<(CommandLockGuard, InfobaseLock), BoundaryRefusal> {
    let workspace_lock =
        acquire_workspace_lock(config, command.as_str()).map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::WorkspaceLock,
            error: error.into(),
        })?;
    let infobase_lock =
        acquire_infobase_lock(config, command.as_str(), base).map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseLock,
            error: error.into(),
        })?;
    Ok((workspace_lock, infobase_lock))
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
    use crate::domain::infobase_export::InfobaseTransferPhase;
    use crate::support::fs::acquire_advisory_lock;
    use crate::use_cases::context::CommandName;
    use crate::use_cases::infobase_lock::BaseAccess;
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
            infobase: crate::config::model::InfobaseConfig::file(format!(
                "File={}",
                work_path.with_file_name("ib").display()
            )),
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

        let refusal = dispatch_with_workspace_lock(
            &config,
            CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || {
                ran.set(true);
            },
        )
        .expect_err("busy workspace");

        assert_eq!(refusal.phase, InfobaseTransferPhase::WorkspaceLock);
        assert_eq!(refusal.error.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(!ran.get());
        // Замок базы идёт после замка `workPath`: занятый каталог его не трогает.
        assert_eq!(
            fs::read_dir(dir.path())
                .expect("dir")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains("infobase"))
                .count(),
            0
        );
    }

    #[test]
    fn a_held_base_stops_the_dispatch_after_the_workspace_lock() {
        let dir = tempdir().expect("tempdir");
        let first_work = dir.path().join("first");
        let second_work = dir.path().join("second");
        fs::create_dir_all(&first_work).expect("first work");
        fs::create_dir_all(&second_work).expect("second work");
        let mut second = sample_config(&second_work);
        second.infobase = sample_config(&first_work).infobase;
        let ran = Cell::new(false);

        let refusal = dispatch_with_workspace_lock(
            &sample_config(&first_work),
            CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || {
                dispatch_with_workspace_lock(
                    &second,
                    CommandName::Test,
                    BaseAccess::Writes,
                    |_| Ok(()),
                    || ran.set(true),
                )
            },
        )
        .expect("first command holds both locks")
        .expect_err("second command is refused on the base");

        assert_eq!(refusal.phase, InfobaseTransferPhase::InfobaseLock);
        assert_eq!(refusal.error.kind(), UseCaseErrorKind::InfobaseBusy);
        assert!(!ran.get());
    }
}
