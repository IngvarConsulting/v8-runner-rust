use std::future::Future;

use crate::config::model::AppConfig;
use crate::domain::infobase_export::InfobaseTransferPhase;
use crate::domain::warning::Warning;
use crate::use_cases::command_lock::CommandLockGuard;
use crate::use_cases::context::{CommandName, StandChange};
use crate::use_cases::infobase_lock::{acquire_infobase_lock, BaseAccess, InfobaseLock};
use crate::use_cases::infobase_owner::{check_infobase_owner, OwnerCheck};
use crate::use_cases::result::UseCaseError;
#[cfg(test)]
use crate::use_cases::result::UseCaseFailure;
use crate::use_cases::workspace_lock::{
    acquire_local_lock, acquire_memory_lock, acquire_workspace_lock,
};

/// Отказ границы команды: шаг и ошибка. Сценарий не запускался.
///
/// Шаг — `workspace lock`, `infobase lock`, `infobase owner` или `workspace preparation`
/// из словаря фаз домена: адаптер печатает его тем же шагом, что и отказы своих сценариев.
#[derive(Debug)]
pub struct BoundaryRefusal {
    pub phase: InfobaseTransferPhase,
    pub error: UseCaseError,
}

/// Что граница говорит сверх ответа команды: шаг, на котором это замечено, и предупреждение.
///
/// Замок базы, который команде чтения не достался; запись в базу другой рабочей копии — с
/// кодом `infobase_of_another_copy`; взятие базы без метки и смена ушедшего владельца; метка,
/// которую команда чтения не прочитала.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryNote {
    pub phase: InfobaseTransferPhase,
    pub warning: Warning,
}

impl BoundaryNote {
    pub fn message(&self) -> &str {
        self.warning.message()
    }
}

/// Runs an adapter dispatch under the shared workspace lock and, for a command that opens a
/// file infobase, under the infobase lock taken after it and past the owner check.
///
/// Занятый каталог отказывает здесь своим родом `WorkspaceBusy`, занятая база —
/// `InfobaseBusy` для всякой команды: словарь провода выбирает транспорт, а не граница
/// замка. База другой рабочей копии не отказ, а предупреждение. `before_dispatch` идёт под обоими
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

/// Shared CLI boundary: previews only inspect ownership; execution retains both
/// command locks. The adapter receives the same notes without printing them here.
pub(crate) fn dispatch_with_workspace_boundary<TResult>(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
    preview: bool,
    before_dispatch: impl FnOnce(&[BoundaryNote]) -> Result<(), UseCaseError>,
    run: impl FnOnce() -> TResult,
) -> Result<TResult, BoundaryRefusal> {
    if preview {
        let notes = preview_boundary(config, command, base)?;
        before_dispatch(&notes).map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::WorkspacePreparation,
            error,
        })?;
        Ok(run())
    } else {
        dispatch_with_workspace_lock(config, command, base, before_dispatch, run)
    }
}

/// Та же граница для асинхронного сценария: замки держатся, пока сценарий не дошёл до
/// конечного состояния, и снимаются вместе с его будущим — раньше, чем вызывающий
/// отпустит что-то своё. Сценарий создаётся уже под замками; то, что граница говорит
/// сверх ответа, возвращается рядом с его исходом.
pub(crate) async fn dispatch_with_workspace_lock_async<TFuture>(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
    run: impl FnOnce() -> TFuture,
) -> Result<(TFuture::Output, Vec<BoundaryNote>), UseCaseError>
where
    TFuture: Future,
{
    let (_workspace_lock, _infobase_lock, notes) =
        acquire(config, command, base).map_err(|refusal| refusal.error)?;
    Ok((run().await, notes))
}

/// Граница превью: замков нет, метку владельца читают без замка и ничего не пишут. Превью
/// команды записи на базе другой рабочей копии предупреждает так же, как прогон, а метку,
/// которую не прочитать, называет отказом; превью команды чтения говорит, если метку не
/// прочитать.
pub fn preview_boundary(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
) -> Result<Vec<BoundaryNote>, BoundaryRefusal> {
    check_infobase_owner(config, command.as_str(), base, OwnerCheck::Preview)
        .map(|warnings| {
            warnings
                .into_iter()
                .map(|warning| BoundaryNote {
                    phase: InfobaseTransferPhase::InfobaseOwner,
                    warning,
                })
                .collect()
        })
        .map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseOwner,
            error,
        })
}

/// Замки каталогов команды: память базы снимается раньше `localPath`, `localPath` — раньше
/// `workPath`; порядок полей — порядок сброса.
pub(crate) struct CommandLocks {
    _memory: Option<CommandLockGuard>,
    _local: Option<CommandLockGuard>,
    _workspace: CommandLockGuard,
}

/// Меняет ли команда стенд под `localPath` (`INV.CLI.THE-LOCAL-LOCK-FOLLOWS-THE-WORKPATH-LOCK`).
fn changes_the_stand(config: &AppConfig, command: CommandName) -> bool {
    match command.stand_change() {
        StandChange::Always => true,
        StandChange::Never => false,
        StandChange::WhenManaged => {
            let infobase = &config.infobase;
            // Загрузчик помечает базу стенда памятью `managed:<имя>`, даже когда для создания
            // подставил файловую базу сервера вместо его секции.
            infobase.managed_memory.is_some()
                || infobase
                    .standalone
                    .as_ref()
                    .is_some_and(|standalone| standalone.managed.is_some())
                || infobase
                    .cluster
                    .as_ref()
                    .is_some_and(|cluster| cluster.managed.is_some())
                || infobase
                    .dbms
                    .as_ref()
                    .is_some_and(|dbms| dbms.managed.is_some())
        }
    }
}

/// Замок `workPath`, затем замок базы и под ним — чья база. Это единственная точка
/// проверки владельца у команды, которая открывает файловую базу: проверки памяти и
/// поколения идут позже, в сценарии. Порядок сброса обратный: база отпускается раньше
/// каталога.
fn acquire(
    config: &AppConfig,
    command: CommandName,
    base: BaseAccess,
) -> Result<(CommandLocks, InfobaseLock, Vec<BoundaryNote>), BoundaryRefusal> {
    let workspace_lock =
        acquire_workspace_lock(config, command.as_str()).map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::WorkspaceLock,
            error: error.into(),
        })?;
    let local_lock = if changes_the_stand(config, command) {
        Some(
            acquire_local_lock(config, command.as_str()).map_err(|error| BoundaryRefusal {
                phase: InfobaseTransferPhase::WorkspaceLock,
                error: error.into(),
            })?,
        )
    } else {
        None
    };
    // Память базы пишет только команда, которая пишет базу; чтение замка не берёт
    // (`INV.CLI.THE-MEMORY-LOCK-FOLLOWS-THE-WORKPATH-LOCK`).
    let memory_lock = if base == BaseAccess::Writes {
        acquire_memory_lock(config, command.as_str()).map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::WorkspaceLock,
            error: error.into(),
        })?
    } else {
        None
    };
    let workspace_lock = CommandLocks {
        _memory: memory_lock,
        _local: local_lock,
        _workspace: workspace_lock,
    };
    let infobase_lock =
        acquire_infobase_lock(config, command.as_str(), base).map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseLock,
            error: error.into(),
        })?;
    if base != BaseAccess::Untouched {
        // `infobase create` снимает простаивающих агентов сам, после своих проверок и перед
        // тем, как тронуть базу (`INV.USE-CASES.RECREATE-REFUSES-A-BUSY-BASE-BEFORE-REMOVING`);
        // здесь только отказ занятой базы.
        // Команда, которая открывает базу клиентом 1С, отключает простаивающих живых агентов
        // этой базы от неё; та, что выбирает исполнителя, решает это при выборе (#644).
        crate::use_cases::infobase_lock::settle_live_agents(
            config,
            command.as_str(),
            live_agents_at_the_boundary(command),
        )
        .map(|_| ())
        .map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseLock,
            error: error.into(),
        })?;
    }
    if command.restarts_a_server() {
        // Остановка кластера или его СУБД оставила бы подключение живых агентов к серверным
        // базам устаревшим: следующая сессия взяла бы его без `common connect-ib` (#644).
        crate::use_cases::infobase_lock::release_server_connections();
    }
    let mut notes: Vec<BoundaryNote> = infobase_lock
        .warning()
        .map(|message| BoundaryNote {
            phase: InfobaseTransferPhase::InfobaseLock,
            warning: Warning::Plain(message.to_owned()),
        })
        .into_iter()
        .collect();
    let owner_notes = check_infobase_owner(config, command.as_str(), base, OwnerCheck::Run)
        .map_err(|error| BoundaryRefusal {
            phase: InfobaseTransferPhase::InfobaseOwner,
            error,
        })?;
    notes.extend(owner_notes.into_iter().map(|warning| BoundaryNote {
        phase: InfobaseTransferPhase::InfobaseOwner,
        warning,
    }));
    Ok((workspace_lock, infobase_lock, notes))
}

/// Что команда на границе делает с простаивающими живыми агентами своей базы (#644):
/// восстановление их снимает, клиент 1С (`test`, `launch`) — отключает, остальные решают это при
/// выборе исполнителя.
fn live_agents_at_the_boundary(
    command: CommandName,
) -> crate::use_cases::infobase_lock::LiveAgents {
    use crate::use_cases::infobase_lock::LiveAgents;
    if matches!(command, CommandName::InfobaseRestore) {
        LiveAgents::Remove
    } else if command.opens_the_infobase_without_an_executor() {
        LiveAgents::Disconnect
    } else {
        LiveAgents::Keep
    }
}

/// Замок базы-источника у команды, которая читает её целиком, — `infobase create --from`.
///
/// Берётся на границе вслед за замками своего `workPath` и своей базы и держится, пока его
/// держит вызывающий, — всё время команды, а снимок источника идёт под ним: команды
/// копии-владельца в это время получают `InfobaseBusy`. Источник команда только читает,
/// поэтому замок, который не взять не из-за другой команды, — замечание, а проверки владельца
/// у источника нет: чтение владельцем не делает (`INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER`).
pub(crate) fn hold_source_base(
    source: &AppConfig,
    command: CommandName,
) -> Result<(InfobaseLock, Vec<BoundaryNote>), BoundaryRefusal> {
    let lock =
        acquire_infobase_lock(source, command.as_str(), BaseAccess::Reads).map_err(|error| {
            BoundaryRefusal {
                phase: InfobaseTransferPhase::InfobaseLock,
                error: error.into(),
            }
        })?;
    let notes = lock
        .warning()
        .map(|message| BoundaryNote {
            phase: InfobaseTransferPhase::InfobaseLock,
            warning: Warning::Plain(message.to_owned()),
        })
        .into_iter()
        .collect();
    Ok((lock, notes))
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
    /// Граница команды: восстановление снимает простаивающих живых агентов базы, `test` и `launch`
    /// отключают их, команды с выбором исполнителя оставляют, а остановка кластера и его СУБД
    /// отключает агентов серверных баз (#644,
    /// `INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`).
    #[test]
    fn the_boundary_decides_what_happens_to_the_connected_live_agents() {
        use crate::use_cases::context::CommandName;
        use crate::use_cases::infobase_lock::LiveAgents;
        assert_eq!(
            super::live_agents_at_the_boundary(CommandName::InfobaseRestore),
            LiveAgents::Remove
        );
        for command in [CommandName::Test, CommandName::Launch] {
            assert_eq!(
                super::live_agents_at_the_boundary(command),
                LiveAgents::Disconnect
            );
        }
        for command in [
            CommandName::Build,
            CommandName::Dump,
            CommandName::Apply,
            CommandName::Reset,
            CommandName::InfobaseDump,
        ] {
            assert_eq!(
                super::live_agents_at_the_boundary(command),
                LiveAgents::Keep
            );
        }
        for command in [
            CommandName::ClusterStop,
            CommandName::ClusterRestart,
            CommandName::DbmsStop,
            CommandName::DbmsRestart,
        ] {
            assert!(command.restarts_a_server(), "{command:?}");
        }
        assert!(!CommandName::ClusterStart.restarts_a_server());
        assert!(crate::platform::connection::is_server_identity(
            "server:host\\ib"
        ));
        assert!(!crate::platform::connection::is_server_identity(
            "file:/srv/ib (/srv/ib)"
        ));
    }

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
            local_path: work_path.join("base").join(".local"),
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
            working_copy: None,
            project_file: crate::config::model::default_project_file(),
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

    /// Два запуска одного проекта с разными `workPath` (`--workdir`) не меняют стенд разом: замок
    /// `localPath` берётся после
    /// замка `workPath`, занятый — отказ `WorkspaceBusy`; занятый `workPath` отказывает раньше и
    /// `localPath` не заводит. Команда, стенда не меняющая, замка `localPath` не берёт.
    #[test]
    fn a_stand_command_takes_the_local_lock_after_the_work_path_lock() {
        let dir = tempdir().expect("tempdir");
        let first = sample_config(&dir.path().join("first"));
        let mut second = sample_config(&dir.path().join("second"));
        second.base_path = first.base_path.clone();
        second.local_path = first.local_path.clone();
        fs::create_dir_all(&first.work_path).expect("first work");
        fs::create_dir_all(&second.work_path).expect("second work");

        // Занятый `workPath` отказывает до замка `localPath`.
        let canonical_work = fs::canonicalize(&first.work_path).expect("canonical work");
        let workspace =
            acquire_advisory_lock(&workspace_lock_path(&canonical_work)).expect("workspace lock");
        let refusal = dispatch_with_workspace_lock(
            &first,
            CommandName::DbmsStart,
            BaseAccess::Untouched,
            |_| Ok(()),
            || (),
        )
        .expect_err("busy workspace");
        assert_eq!(refusal.phase, InfobaseTransferPhase::WorkspaceLock);
        assert!(!first.local_path.exists(), "the local lock comes later");
        drop(workspace);

        let refused = dispatch_with_workspace_lock(
            &first,
            CommandName::ClusterStart,
            BaseAccess::Untouched,
            |_| Ok(()),
            || {
                let refusal = dispatch_with_workspace_lock(
                    &second,
                    CommandName::DbmsStop,
                    BaseAccess::Untouched,
                    |_| Ok(()),
                    || (),
                )
                .expect_err("the stand is being changed");
                assert_eq!(refusal.error.kind(), UseCaseErrorKind::WorkspaceBusy);
                assert!(
                    refusal.error.message().contains("localPath"),
                    "{}",
                    refusal.error.message()
                );
                // `status` и рабочие команды замка `localPath` не ждут.
                dispatch_with_workspace_lock(
                    &second,
                    CommandName::Build,
                    BaseAccess::Untouched,
                    |_| Ok(()),
                    || (),
                )
                .expect("a command that leaves the stand alone");
            },
        );
        refused.expect("the first command holds both locks");
        assert_eq!(
            fs::read_to_string(first.local_path.join(".gitignore")).expect("gitignore"),
            "*
"
        );
    }

    /// Память о базе лежит под `localPath` и общая для запусков с разными `workPath`: команда,
    /// которая пишет базу, берёт замок памяти после замка `workPath`, занятый — `WorkspaceBusy`.
    /// Занятый `workPath` отказывает раньше и памяти не заводит; команда чтения замка памяти не
    /// берёт. Замок — первый писатель памяти: `localPath` появляется со своим `.gitignore`.
    #[test]
    fn a_write_command_takes_the_memory_lock_after_the_work_path_lock() {
        let dir = tempdir().expect("tempdir");
        let mut first = sample_config(&dir.path().join("first"));
        first.infobase = crate::config::model::InfobaseConfig::file("Srvr=srv;Ref=demo");
        first.infobase_name = Some("origin".to_owned());
        let mut second = sample_config(&dir.path().join("second"));
        second.base_path = first.base_path.clone();
        second.local_path = first.local_path.clone();
        second.infobase = first.infobase.clone();
        second.infobase_name = first.infobase_name.clone();
        fs::create_dir_all(&first.work_path).expect("first work");

        let canonical_work = fs::canonicalize(&first.work_path).expect("canonical work");
        let workspace =
            acquire_advisory_lock(&workspace_lock_path(&canonical_work)).expect("workspace lock");
        let refusal = dispatch_with_workspace_lock(
            &first,
            CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || (),
        )
        .expect_err("busy workspace");
        assert_eq!(refusal.phase, InfobaseTransferPhase::WorkspaceLock);
        assert!(!first.local_path.exists(), "the memory lock comes later");
        drop(workspace);

        dispatch_with_workspace_lock(
            &first,
            CommandName::Build,
            BaseAccess::Writes,
            |_| Ok(()),
            || {
                let refusal = dispatch_with_workspace_lock(
                    &second,
                    CommandName::Dump,
                    BaseAccess::Writes,
                    |_| Ok(()),
                    || (),
                )
                .expect_err("the memory is being written");
                assert_eq!(refusal.phase, InfobaseTransferPhase::WorkspaceLock);
                assert_eq!(refusal.error.kind(), UseCaseErrorKind::WorkspaceBusy);
                assert!(
                    refusal.error.message().contains("memory of the infobase"),
                    "{}",
                    refusal.error.message()
                );
                dispatch_with_workspace_lock(
                    &second,
                    CommandName::Status,
                    BaseAccess::Reads,
                    |_| Ok(()),
                    || (),
                )
                .expect("a read takes no memory lock");
            },
        )
        .expect("the first command holds the memory lock");
        assert_eq!(
            fs::read_to_string(first.local_path.join(".gitignore")).expect("gitignore"),
            "*\n"
        );
        assert!(first.local_path.join("infobases").join("origin").is_dir());
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

    /// Чья база — проверка границы: на базе другой рабочей копии команда записи идёт, а
    /// предупреждение шага `infobase owner` граница отдаёт до сценария, а значит раньше
    /// проверок памяти и поколения, которые идут в нём.
    #[test]
    fn a_base_of_another_copy_warns_before_the_scenario() {
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
        let warned = Cell::new(false);

        dispatch_with_workspace_lock(
            &second,
            CommandName::Build,
            BaseAccess::Writes,
            |notes| {
                assert!(!ran.get(), "the warning comes before the scenario");
                assert_eq!(notes.len(), 1, "{notes:?}");
                assert_eq!(notes[0].phase, InfobaseTransferPhase::InfobaseOwner);
                assert!(
                    notes[0].message().contains("of another working copy"),
                    "{notes:?}"
                );
                warned.set(true);
                Ok(())
            },
            || ran.set(true),
        )
        .expect("a write on a base of another copy runs");

        assert!(warned.get() && ran.get());
        ran.set(false);
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
