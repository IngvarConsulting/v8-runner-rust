//! Замок файловой базы на время команды.
//!
//! Лежит рядом с каталогом базы, а не в нём: копия каталога базы его не уносит. Берётся
//! после замка `workPath` на той же границе адаптера и без ожидания: занятая база —
//! отказ `InfobaseBusy`, который называет команду-владельца и её рабочую копию. Замок —
//! тот же замок команды, что у `workPath` ([`take_command_lock`]); живёт он, пока жив
//! процесс: после `kill -9` следующая команда заменяет запись умершего владельца.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::support::error::AppError;
use crate::support::path::nearest_existing_canonical_path;
use crate::use_cases::command_lock::{command_lock_holder, take_command_lock, CommandLockGuard};

/// Что команда делает с файловой базой — от этого зависит, берёт ли она замок базы и что
/// делает, если замок не взять не из-за другой команды.
///
/// Команда записи и команда чтения — в смысле
/// `INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseAccess {
    /// Базу не открывает: исходники, публикация, загрузка утилит.
    Untouched,
    /// Открывает базу, но командой записи на ней не является: без замка идёт дальше.
    Reads,
    /// Создаёт, меняет или заменяет базу либо работает в ней: без замка отказывает.
    Writes,
}

/// Замок базы, взятый командой, или предупреждение, что команда чтения идёт без него.
#[derive(Debug, Default)]
pub(crate) struct InfobaseLock {
    _guard: Option<CommandLockGuard>,
    warning: Option<String>,
}

impl InfobaseLock {
    /// Предупреждение команды чтения, которой замок не достался.
    pub(crate) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }
}

/// Берёт замок файловой базы команды, если команда её открывает.
///
/// Серверную базу и команду, которая базу не открывает, замок не касается. Занятая база —
/// `InfobaseBusy`. Замок, который не взять по другой причине (каталог не пишется,
/// файловая система не блокирует), команда записи превращает в отказ с каталогом и
/// причиной, а команда чтения — в предупреждение.
pub(crate) fn acquire_infobase_lock(
    config: &AppConfig,
    command_name: &str,
    access: BaseAccess,
) -> Result<InfobaseLock, AppError> {
    let writes = match access {
        BaseAccess::Untouched => return Ok(InfobaseLock::default()),
        BaseAccess::Reads => false,
        BaseAccess::Writes => true,
    };
    let Some(base_dir) = config.v8_connection().file_infobase_dir(&config.base_path) else {
        return Ok(InfobaseLock::default());
    };
    // Чтение каталогов не создаёт: нет родителя — нет и базы, беречь нечего, и отказ о
    // несуществующей базе остаётся за сценарием и платформой. Команда записи родителя
    // создаёт: `infobase create` заводит базу там, где её ещё нет.
    if !writes && base_dir.parent().is_some_and(|parent| !parent.exists()) {
        return Ok(InfobaseLock::default());
    }
    let canonical_work_path = nearest_existing_canonical_path(&config.work_path)
        .unwrap_or_else(|_| config.work_path.clone());
    let error = match infobase_lock_path(&base_dir) {
        None => std::io::Error::new(
            ErrorKind::InvalidInput,
            "the infobase directory has no parent to hold its lock",
        ),
        Some(lock_path) => {
            match take_command_lock(&lock_path, command_name, &canonical_work_path) {
                Ok(guard) => {
                    return Ok(InfobaseLock {
                        _guard: Some(guard),
                        warning: None,
                    })
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::AlreadyExists
                    ) =>
                {
                    return Err(AppError::InfobaseBusy(render_busy_message(
                        command_name,
                        &base_dir,
                        &lock_path,
                        &error,
                    )));
                }
                Err(error) => error,
            }
        }
    };
    let beside = base_dir.parent().unwrap_or(&base_dir).display().to_string();
    if writes {
        return Err(AppError::Runtime(format!(
            "cannot start {command_name}: the infobase lock cannot be taken next to '{beside}': {error}; a command that writes the infobase '{}' does not run without its lock",
            base_dir.display()
        )));
    }
    Ok(InfobaseLock {
        _guard: None,
        warning: Some(format!(
            "infobase lock was not taken next to '{beside}': {error}; {command_name} reads the infobase '{}' without it, and another command may change the infobase meanwhile",
            base_dir.display()
        )),
    })
}

/// Файл замка рядом с каталогом базы: `.<имя каталога>.v8-runner.infobase.lock`.
fn infobase_lock_path(base_dir: &Path) -> Option<PathBuf> {
    let parent = base_dir.parent()?;
    let name = base_dir.file_name()?;
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(name);
    lock_name.push(".v8-runner.infobase.lock");
    Some(parent.join(lock_name))
}

/// Отказ занятой базы называет владельца, если запись о нём относится к нынешней
/// блокировке, и всегда — ответ общего механизма: для записи переиспользованного pid,
/// записи с другой машины или от прежней версии только он говорит, что файл удаляют вручную.
fn render_busy_message(
    command_name: &str,
    base_dir: &Path,
    lock_path: &Path,
    error: &std::io::Error,
) -> String {
    match command_lock_holder(lock_path) {
        Some(holder) => format!(
            "cannot start {command_name}: infobase '{}' is in use by '{}' of the working copy with workPath '{}' (pid {}, started at {}); retry when it finishes; {error}",
            base_dir.display(),
            holder.command,
            holder.canonical_work_path.display(),
            holder.pid,
            holder.started_at.to_rfc3339(),
        ),
        None => format!(
            "cannot start {command_name}: infobase '{}' is already in use by another command; {error}",
            base_dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{acquire_infobase_lock, infobase_lock_path, BaseAccess};
    use crate::config::model::{
        AppConfig, InfobaseConfig, InfobaseDbmsConfig, SourceFormat, TestsConfig, ToolsConfig,
    };
    use crate::support::error::AppError;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn config(work_path: &Path, infobase: InfobaseConfig) -> AppConfig {
        AppConfig {
            base_path: work_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase,
            infobases: Default::default(),
            infobase_name: None,
            source_sets: Vec::new(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn file_base(work_path: &Path, base: &Path) -> AppConfig {
        config(
            work_path,
            InfobaseConfig::file(format!("File={}", base.display())),
        )
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_held_base_refuses_a_second_command_and_names_the_first() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("shared").join("ib");
        fs::create_dir_all(&base).expect("base");
        let first_work = dir.path().join("first");
        let second_work = dir.path().join("second");
        fs::create_dir_all(&first_work).expect("first work");
        fs::create_dir_all(&second_work).expect("second work");

        let held =
            acquire_infobase_lock(&file_base(&first_work, &base), "push", BaseAccess::Writes)
                .expect("first command takes the base");
        let busy = acquire_infobase_lock(
            &file_base(&second_work, &base),
            "infobase.dump",
            BaseAccess::Reads,
        )
        .expect_err("second command is refused");

        let AppError::InfobaseBusy(message) = &busy else {
            panic!("busy base is InfobaseBusy: {busy}");
        };
        let canonical_first = fs::canonicalize(&first_work).expect("canonical first");
        assert!(message.contains("'push'"), "{message}");
        assert!(
            message.contains(&canonical_first.display().to_string()),
            "{message}"
        );
        assert!(message.contains("cannot start infobase.dump"), "{message}");
        drop(held);

        let again =
            acquire_infobase_lock(&file_base(&second_work, &base), "push", BaseAccess::Writes)
                .expect("the lock lives only with its command");
        drop(again);
        assert_eq!(entries(&dir.path().join("shared")), ["ib"]);
    }

    /// Чтение не создаёт каталог-родитель базы: базы там нет, и замок не нужен; запись его
    /// создаёт, как создаст и базу.
    #[test]
    fn a_read_of_a_base_without_a_parent_creates_nothing() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("missing").join("ib");

        let read = acquire_infobase_lock(
            &file_base(dir.path(), &base),
            "infobase.dump",
            BaseAccess::Reads,
        )
        .expect("a read of a missing base is left to the platform");

        assert!(read.warning().is_none());
        assert!(!dir.path().join("missing").exists());
        let _write = acquire_infobase_lock(
            &file_base(dir.path(), &base),
            "infobase create",
            BaseAccess::Writes,
        )
        .expect("a write creates the parent for its lock");
        assert!(dir.path().join("missing").is_dir());
    }

    /// Запись, которую общий механизм не заменяет (здесь — без отметки о замке ОС, как у
    /// прежних версий), держит базу; отказ сохраняет его подсказку удалить файл вручную.
    #[test]
    fn a_record_left_for_manual_removal_is_named_in_the_refusal() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        fs::create_dir_all(&base).expect("base");
        let lock_path =
            infobase_lock_path(&fs::canonicalize(&base).expect("canonical")).expect("lock path");
        fs::write(
            &lock_path,
            format!(
                "{{\"tool\":\"v8-runner\",\"pid\":{},\"owner_id\":\"legacy\",\"created_at\":\"2026-09-02T00:00:00Z\"}}",
                std::process::id()
            ),
        )
        .expect("legacy record");

        let busy = acquire_infobase_lock(&file_base(dir.path(), &base), "push", BaseAccess::Writes)
            .expect_err("the record holds the base");

        let AppError::InfobaseBusy(message) = &busy else {
            panic!("a held base is InfobaseBusy: {busy}");
        };
        assert!(message.contains("remove this file manually"), "{message}");
        assert!(
            message.contains(&lock_path.display().to_string()),
            "{message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_base_reached_through_a_symlink_is_the_same_base() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        fs::create_dir_all(&base).expect("base");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&base, &link).expect("symlink");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work");

        let _held = acquire_infobase_lock(&file_base(&work, &base), "push", BaseAccess::Writes)
            .expect("lock");
        let busy = acquire_infobase_lock(&file_base(&work, &link), "test", BaseAccess::Writes)
            .expect_err("same base through a symlink");

        assert!(matches!(busy, AppError::InfobaseBusy(_)), "{busy}");
    }

    #[test]
    fn a_server_base_and_an_untouched_base_take_no_lock() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        let server = config(
            dir.path(),
            InfobaseConfig::server(
                "Srvr=srv;Ref=db",
                InfobaseDbmsConfig::new("PostgreSQL", "db", "ib"),
            ),
        );
        let lock = acquire_infobase_lock(&server, "push", BaseAccess::Writes).expect("server");
        assert!(lock.warning().is_none());

        let _held = acquire_infobase_lock(
            &file_base(dir.path(), &base),
            "convert",
            BaseAccess::Untouched,
        )
        .expect("untouched");
        let _first =
            acquire_infobase_lock(&file_base(dir.path(), &base), "push", BaseAccess::Writes)
                .expect("an untouched command held nothing");
    }

    /// Замок, который не взять не из-за другой команды: имя его системного файла занято
    /// каталогом. Команда записи отказывает с каталогом и причиной, команда чтения идёт
    /// дальше с предупреждением.
    #[test]
    fn a_base_lock_that_cannot_be_taken_refuses_a_write_and_warns_a_read() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        fs::create_dir_all(&base).expect("base");
        let lock_path =
            infobase_lock_path(&fs::canonicalize(&base).expect("canonical")).expect("lock path");
        let mut system = lock_path.into_os_string();
        system.push(".system");
        fs::create_dir(&system).expect("blocker");
        let config = file_base(dir.path(), &base);
        let beside = fs::canonicalize(dir.path()).expect("canonical dir");

        let refused = acquire_infobase_lock(&config, "push", BaseAccess::Writes)
            .expect_err("a write needs the lock");
        assert!(matches!(refused, AppError::Runtime(_)), "{refused}");
        assert!(
            refused.to_string().contains(&beside.display().to_string()),
            "{refused}"
        );

        let read = acquire_infobase_lock(&config, "infobase.dump", BaseAccess::Reads)
            .expect("a read goes on");
        let warning = read
            .warning()
            .expect("a read says it goes without the lock");
        assert!(warning.contains("infobase lock"), "{warning}");
        assert!(warning.contains(&beside.display().to_string()), "{warning}");
    }
}
