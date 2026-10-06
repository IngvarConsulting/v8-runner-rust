use std::future::Future;

use tracing::warn;

use crate::config::model::AppConfig;
use crate::domain::infobase_export::InfobaseTransferPhase;
use crate::use_cases::command_lock::CommandLockGuard;
use crate::use_cases::context::CommandName;
use crate::use_cases::infobase_lock::{acquire_infobase_lock, BaseAccess, InfobaseLock};
use crate::use_cases::infobase_owner::{check_infobase_owner, OwnerCheck};
use crate::use_cases::result::UseCaseError;
#[cfg(test)]
use crate::use_cases::result::UseCaseFailure;
use crate::use_cases::workspace_lock::acquire_workspace_lock;

/// Отказ границы команды: шаг и ошибка. Сценарий не запускался.
///
/// Шаг — `workspace lock`, `infobase lock`, `infobase owner` или `workspace preparation`
/// из словаря фаз домена: адаптер печатает его тем же шагом, что и отказы своих сценариев.
#[derive(Debug)]
pub struct BoundaryRefusal {
    pub phase: InfobaseTransferPhase,
    pub error: UseCaseError,
}

/// Что граница говорит сверх ответа команды: шаг, на котором это замечено, и текст.
///
/// Замок базы, который команде чтения не достался; взятие базы без метки и смена ушедшего
/// владельца; метка, которую команда чтения не прочитала.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryNote {
    pub phase: InfobaseTransferPhase,
    pub message: String,
}

/// Runs an adapter dispatch under the shared workspace lock and, for a command that opens a
/// file infobase, under the infobase lock taken after it and past the owner check.
///
/// Занятый каталог отказывает здесь своим родом `WorkspaceBusy`, занятая база —
/// `InfobaseBusy`, база другой рабочей копии — `InfobaseHeld` для всякой команды: словарь
/// провода выбирает транспорт, а не граница замка. `before_dispatch` идёт под обоими
/// замками и получает то, что граница говорит сверх ответа команды.
pub fn dispatch_with_workspace_lock<TResult>(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
    before_dispatch: impl FnOnce(&[BoundaryNote]) -> Result<(), UseCaseError>,
    run: impl FnOnce() -> TResult,
) -> Result<TResult, BoundaryRefusal> {
    let (_workspace_lock, _infobase_lock, notes) = acquire(config, command, base)?;
    before_dispatch(&notes).map_err(|error| BoundaryRefusal {
        phase: InfobaseTransferPhase::WorkspacePreparation,
        error,
    })?;
    Ok(run())
}

/// Та же граница для асинхронного сценария: замки держатся, пока сценарий не дошёл до
/// конечного состояния, и снимаются вместе с его будущим — раньше, чем вызывающий
/// отпустит что-то своё. Сценарий создаётся уже под замками; то, что граница говорит
/// сверх ответа, уходит в журнал.
pub(crate) async fn dispatch_with_workspace_lock_async<TFuture>(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
    run: impl FnOnce() -> TFuture,
) -> Result<TFuture::Output, UseCaseError>
where
    TFuture: Future,
{
    let (_workspace_lock, _infobase_lock, notes) =
        acquire(config, command, base).map_err(|refusal| refusal.error)?;
    for note in &notes {
        warn!(command = command.as_str(), "{}", note.message);
    }
    Ok(run().await)
}

/// Граница превью: замков нет, метку владельца читают без замка и ничего не пишут. Превью
/// команды записи на базе другой рабочей копии отказывает так же, как отказал бы прогон.
pub fn preview_boundary(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
) -> Result<(), BoundaryRefusal> {
    check_infobase_owner(config, command.as_str(), base, OwnerCheck::Preview)
        .map(drop)
        .map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseOwner,
            error,
        })
}

/// Замок `workPath`, затем замок базы и под ним — чья база. Это единственная точка
/// проверки владельца у команды, которая открывает файловую базу: проверки памяти и
/// поколения идут позже, в сценарии. Порядок сброса обратный: база отпускается раньше
/// каталога.
fn acquire(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
) -> Result<(CommandLockGuard, InfobaseLock, Vec<BoundaryNote>), BoundaryRefusal> {
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
    let mut notes: Vec<BoundaryNote> = infobase_lock
        .warning()
        .map(|message| BoundaryNote {
            phase: InfobaseTransferPhase::InfobaseLock,
            message: message.to_owned(),
        })
        .into_iter()
        .collect();
    let owner_notes = check_infobase_owner(config, command.as_str(), base, OwnerCheck::Run)
        .map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseOwner,
            error,
        })?;
    notes.extend(owner_notes.into_iter().map(|message| BoundaryNote {
        phase: InfobaseTransferPhase::InfobaseOwner,
        message,
    }));
    Ok((workspace_lock, infobase_lock, notes))
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

    /// Чья база — проверка границы: на базе другой рабочей копии команда записи отказывает
    /// на шаге `infobase owner` после обоих замков и раньше сценария, а значит раньше
    /// проверок памяти и поколения, которые идут в нём.
    #[test]
    fn a_base_of_another_copy_stops_the_dispatch_before_the_scenario() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        fs::create_dir_all(&base).expect("base");
        let copy = |name: &str| {
            let root = dir.path().join(name);
            fs::create_dir_all(&root).expect("root");
            fs::write(
                root.join("v8project.local.yaml"),
                format!(
                    "infobases:\n  origin:\n    connection: 'File={}'\n",
                    base.display()
                ),
            )
            .expect("local layer");
            let mut config = sample_config(&root.join("work"));
            config.base_path = fs::canonicalize(&root).expect("canonical root");
            config.infobase =
                crate::config::model::InfobaseConfig::file(format!("File={}", base.display()));
            config.infobase_name = Some("origin".to_owned());
            config
        };
        let first = copy("first");
        let second = copy("second");
        dispatch_with_workspace_lock(
            &first,
            CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || (),
        )
        .expect("the first copy takes the base");
        let ran = Cell::new(false);

        let refusal = dispatch_with_workspace_lock(
            &second,
            CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || ran.set(true),
        )
        .expect_err("a base of another copy");

        assert_eq!(refusal.phase, InfobaseTransferPhase::InfobaseOwner);
        assert_eq!(refusal.error.kind(), UseCaseErrorKind::InfobaseHeld);
        assert_eq!(
            refusal.error.next().map(|next| next.command.as_str()),
            Some("infobase create")
        );
        assert!(!ran.get());
        dispatch_with_workspace_lock(
            &second,
            CommandName::InfobaseDump,
            BaseAccess::Reads,
            |notes| {
                assert!(notes.is_empty(), "{notes:?}");
                Ok(())
            },
            || ran.set(true),
        )
        .expect("a read passes");
        assert!(ran.get());
    }
}
