use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::support::error::AppError;
use crate::support::path::nearest_existing_canonical_path;
use crate::use_cases::command_lock::{command_lock_holder, take_command_lock, CommandLockGuard};

const WORKSPACE_LOCK_FILE_NAME: &str = ".v8-runner.workspace.lock";
#[cfg(test)]
const WORKSPACE_LOCK_SIDECAR_FILE_NAME: &str = ".v8-runner.workspace.lock.json";

pub(crate) fn acquire_workspace_lock(
    config: &AppConfig,
    command_name: &str,
) -> Result<CommandLockGuard, AppError> {
    // Замок — первый писатель в `workPath`: каталог появляется вместе с `.gitignore`
    // (`INV.USE-CASES.THE-WORK-PATH-IGNORES-ITSELF`).
    crate::support::local_dir::prepare_work_dir(&config.work_path).map_err(|error| {
        AppError::Runtime(format!(
            "failed to prepare workPath '{}': {error}",
            config.work_path.display()
        ))
    })?;
    let canonical_work_path =
        nearest_existing_canonical_path(&config.work_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to canonicalize workPath '{}': {error}",
                config.work_path.display()
            ))
        })?;
    let lock_path = workspace_lock_path(&canonical_work_path);

    take_command_lock(&lock_path, command_name, &canonical_work_path).map_err(|error| {
        match error.kind() {
            ErrorKind::WouldBlock | ErrorKind::AlreadyExists => AppError::WorkspaceBusy(format!(
                "{}; {error}",
                render_busy_message(command_name, &canonical_work_path, &lock_path)
            )),
            _ => AppError::Runtime(format!(
                "failed to acquire {command_name} workspace lock '{}': {error}",
                lock_path.display()
            )),
        }
    })
}

/// Замок `<localPath>/.lock` команды, меняющей стенд. Берётся после замка `workPath`: запуски
/// одного проекта с разными `workPath` (`--workdir`) не меняют один стенд разом
/// (`INV.CLI.THE-LOCAL-LOCK-FOLLOWS-THE-WORKPATH-LOCK`). Замок — первый писатель в
/// `localPath`: каталог заводит [`LocalDir::prepare`](crate::support::local_dir::LocalDir::prepare).
pub(crate) fn acquire_local_lock(
    config: &AppConfig,
    command_name: &str,
) -> Result<CommandLockGuard, AppError> {
    let local = config.local_dir();
    local.prepare().map_err(|error| {
        AppError::Runtime(format!(
            "failed to prepare localPath '{}': {error}",
            local.path().display()
        ))
    })?;
    let lock_path = local.lock_path();
    let canonical_work_path = nearest_existing_canonical_path(&config.work_path)
        .unwrap_or_else(|_| config.work_path.clone());
    take_command_lock(&lock_path, command_name, &canonical_work_path).map_err(|error| {
        match error.kind() {
            ErrorKind::WouldBlock | ErrorKind::AlreadyExists => {
                let holder = match command_lock_holder(&lock_path) {
                    Some(holder) => format!(
                        "is already locked by '{}' of workspace '{}' (pid {}, started at {})",
                        holder.command,
                        holder.canonical_work_path.display(),
                        holder.pid,
                        holder.started_at.to_rfc3339(),
                    ),
                    None => "is already in use by another command".to_owned(),
                };
                AppError::WorkspaceBusy(format!(
                    "cannot start {command_name}: localPath '{}' {holder}; {error}",
                    local.path().display()
                ))
            }
            _ => AppError::Runtime(format!(
                "failed to acquire {command_name} localPath lock '{}': {error}",
                lock_path.display()
            )),
        }
    })
}

/// Замок памяти выбранной базы `<localPath>/infobases/<ключ>/.lock` у команды, которая пишет
/// базу. Память лежит под `localPath` и общая для запусков с разными `workPath` (`--workdir`,
/// проекция Unica), поэтому её охраняет свой замок, а не замок `workPath`
/// (`INV.CLI.THE-MEMORY-LOCK-FOLLOWS-THE-WORKPATH-LOCK`). Берётся после замков `workPath` и
/// `localPath`; занятый — отказ `WorkspaceBusy`. Замок — первый писатель памяти: `localPath`
/// заводится вместе со своим `.gitignore`. База, адрес которой не распознан, памяти не имеет, и
/// замка нет.
pub(crate) fn acquire_memory_lock(
    config: &AppConfig,
    command_name: &str,
) -> Result<Option<CommandLockGuard>, AppError> {
    let Some(memory) =
        crate::change_detection::source_sets::SourceSetsService::new(config).base_memory_dir()
    else {
        return Ok(None);
    };
    let lock_path = memory_lock_path(&memory);
    config
        .local_dir()
        .prepare_for(&lock_path)
        .and_then(|()| std::fs::create_dir_all(&memory))
        .map_err(|error| {
            AppError::Runtime(format!(
                "failed to prepare the memory of the infobase '{}': {error}",
                memory.display()
            ))
        })?;
    let canonical_work_path = nearest_existing_canonical_path(&config.work_path)
        .unwrap_or_else(|_| config.work_path.clone());
    take_command_lock(&lock_path, command_name, &canonical_work_path)
        .map(Some)
        .map_err(|error| match error.kind() {
            ErrorKind::WouldBlock | ErrorKind::AlreadyExists => {
                let holder = match command_lock_holder(&lock_path) {
                    Some(holder) => format!(
                        "is already locked by '{}' of workspace '{}' (pid {}, started at {})",
                        holder.command,
                        holder.canonical_work_path.display(),
                        holder.pid,
                        holder.started_at.to_rfc3339(),
                    ),
                    None => "is already in use by another command".to_owned(),
                };
                AppError::WorkspaceBusy(format!(
                    "cannot start {command_name}: the memory of the infobase '{}' {holder}; retry when it finishes; {error}",
                    memory.display()
                ))
            }
            _ => AppError::Runtime(format!(
                "failed to acquire {command_name} memory lock '{}': {error}",
                lock_path.display()
            )),
        })
}

/// Файл замка памяти базы: тот же `.lock`, что у `localPath`, но в каталоге памяти.
pub(crate) fn memory_lock_path(memory: &Path) -> PathBuf {
    memory.join(crate::support::local_dir::LOCAL_LOCK_FILE_NAME)
}

pub(crate) fn workspace_lock_path(work_path: &Path) -> PathBuf {
    work_path.join(WORKSPACE_LOCK_FILE_NAME)
}

/// Файл, который заводит сам замок `workPath`: файл замка и всё, что названо от него, —
/// системный файл блокировки, sidecar с описанием владельца и их временные копии.
pub(crate) fn is_workspace_lock_file(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(WORKSPACE_LOCK_FILE_NAME))
}

fn render_busy_message(command_name: &str, canonical_work_path: &Path, lock_path: &Path) -> String {
    match command_lock_holder(lock_path) {
        Some(holder) => format!(
            "cannot start {command_name}: workspace '{}' is already locked by '{}' (pid {}, started at {})",
            canonical_work_path.display(),
            holder.command,
            holder.pid,
            holder.started_at.to_rfc3339(),
        ),
        None => format!(
            "cannot start {command_name}: workspace '{}' is already in use by another command",
            canonical_work_path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        acquire_workspace_lock, workspace_lock_path, CommandLockGuard,
        WORKSPACE_LOCK_SIDECAR_FILE_NAME,
    };
    use crate::config::model::{
        AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::support::fs::acquire_advisory_lock;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    /// Замок — первый писатель в `workPath`: каталог и родитель `.build`, которого не было,
    /// появляются вместе с `.gitignore` со строкой `*` (`INV.USE-CASES.THE-WORK-PATH-IGNORES-ITSELF`).
    #[test]
    fn the_workspace_lock_creates_the_work_path_with_its_gitignore() {
        let dir = tempdir().expect("tempdir");
        let config = sample_config(&dir.path().join(".build").join("v8-runner"));
        let guard = acquire_workspace_lock(&config, "build").expect("workspace lock");
        for ignored in [config.work_path.clone(), dir.path().join(".build")] {
            assert_eq!(
                fs::read_to_string(ignored.join(".gitignore")).expect("gitignore"),
                "*\n"
            );
        }
        drop(guard);
    }

    fn sample_config(work_path: &Path) -> AppConfig {
        AppConfig {
            local_path: work_path.join("base").join(".local"),
            base_path: work_path.join("base"),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            working_copy: None,
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

    fn hold_lock(config: &AppConfig, command_name: &str) -> CommandLockGuard {
        acquire_workspace_lock(config, command_name).expect("workspace lock")
    }

    #[cfg(unix)]
    #[test]
    fn conflicts_use_canonical_workspace_path_and_sidecar_metadata() {
        let dir = tempdir().expect("tempdir");
        let real_work = dir.path().join("real-work");
        fs::create_dir_all(&real_work).expect("work dir");
        let link_work = dir.path().join("work-link");
        std::os::unix::fs::symlink(&real_work, &link_work).expect("symlink");

        let first = sample_config(&real_work);
        let second = sample_config(&link_work);
        let _guard = hold_lock(&first, "build");

        let error = acquire_workspace_lock(&second, "test").expect_err("busy workspace");
        let message = error.to_string();

        assert!(message.contains(
            &std::fs::canonicalize(&real_work)
                .expect("canonical")
                .display()
                .to_string()
        ));
        assert!(message.contains("'build'"));
        assert!(message.contains("pid"));
        assert!(message.contains("started at"));
    }

    #[test]
    fn stale_sidecar_metadata_falls_back_to_generic_busy_message() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = std::fs::canonicalize(&work).expect("canonical");
        let lock_path = workspace_lock_path(&canonical_work);
        let _guard = acquire_advisory_lock(&lock_path).expect("workspace lock");
        let sidecar_path = canonical_work.join(WORKSPACE_LOCK_SIDECAR_FILE_NAME);
        fs::write(
            &sidecar_path,
            r#"{"pid":999999,"command":"push","started_at":"2026-01-01T00:00:00Z","canonical_work_path":"/tmp/stale"}"#,
        )
        .expect("sidecar");

        let error = acquire_workspace_lock(&config, "test").expect_err("busy workspace");
        let message = error.to_string();

        assert!(message.contains("already in use by another command"));
        assert!(!message.contains("999999"));
        assert!(!message.contains("'build'"));
    }

    #[test]
    fn next_lock_acquisition_cleans_stale_sidecar_temp_files() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = std::fs::canonicalize(&work).expect("canonical");
        let stale_temp =
            canonical_work.join(format!("{WORKSPACE_LOCK_SIDECAR_FILE_NAME}.tmp.stale"));
        fs::write(&stale_temp, b"stale").expect("stale temp");

        let _guard = hold_lock(&config, "build");

        assert!(!stale_temp.exists());
    }

    /// Замок — файл ОС; sidecar — диагностика. Невозможность записать sidecar не
    /// отменяет владение каталогом: второй запуск всё равно получает «занято».
    #[test]
    fn an_unwritable_sidecar_does_not_release_the_lock() {
        let work = tempdir().expect("work");
        let config = sample_config(work.path());
        let canonical =
            crate::support::path::nearest_existing_canonical_path(work.path()).expect("canonical");
        // Каталог на месте sidecar: переименовать поверх него файл нельзя.
        std::fs::create_dir_all(canonical.join(WORKSPACE_LOCK_SIDECAR_FILE_NAME)).expect("blocker");

        let guard = acquire_workspace_lock(&config, "build")
            .expect("the OS lock is taken even when the sidecar cannot be written");
        let busy = acquire_workspace_lock(&config, "dump").expect_err("second owner is refused");
        assert!(
            matches!(busy, crate::support::error::AppError::WorkspaceBusy(_)),
            "{busy}"
        );
        drop(guard);

        acquire_workspace_lock(&config, "dump").expect("released after the first owner is gone");
    }

    #[test]
    fn drop_removes_sidecar_file() {
        let dir = tempdir().expect("tempdir");
        let work = dir.path().join("work");
        fs::create_dir_all(&work).expect("work dir");
        let config = sample_config(&work);
        let canonical_work = std::fs::canonicalize(&work).expect("canonical");
        let sidecar = canonical_work.join(".v8-runner.workspace.lock.json");

        let guard = hold_lock(&config, "build");
        assert!(workspace_lock_path(&canonical_work).exists());
        assert!(sidecar.exists());
        drop(guard);

        assert!(!sidecar.exists());
    }
}
