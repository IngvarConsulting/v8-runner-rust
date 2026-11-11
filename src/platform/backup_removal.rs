//! Удаление прежнего дерева полной выгрузки в фоне (#616,
//! `INV.USE-CASES.A-REPLACED-TREE-IS-REMOVED-IN-THE-BACKGROUND`).
//!
//! После успешной подмены каталога прежнее дерево — резервная копия с отметкой `superseded`.
//! Его удаляет скрытая подкоманда `v8-runner remove-backup --path <каталог>`, запущенная
//! отделённым процессом так же, как служба стенда ([`spawn_hidden`]): она переживает ответ
//! команды. Не успевший или упавший процесс ничего не ломает: оставшееся дерево узнаёт по имени и
//! отметке уборка остатков следующей команды.
//!
//! Запускать себя умеет только исполняемый файл раннера: [`allow_from_this_executable`] зовёт
//! `main`. В модульных тестах исполняемый файл — сборка тестов, и [`spawn`] отказывает: дерево
//! удаляется на месте, как до #616.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::platform::service::{spawn_hidden, ServiceRequest, ServiceSpawnError};

/// Имя скрытой подкоманды фонового удаления.
pub const COMMAND: &str = "remove-backup";

/// Скрытая переменная окружения: задержка перед удалением в миллисекундах, не больше
/// [`MAX_DELAY`]. Она для тестов: так видно, что ответ команды пришёл раньше удаления.
pub const DELAY_ENV: &str = "V8_RUNNER_BACKUP_REMOVAL_DELAY_MS";

const MAX_DELAY: Duration = Duration::from_secs(60);

static RUNNER_EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

/// Разрешает запускать фоновое удаление из исполняемого файла этого процесса. Зовётся из `main`
/// раннера и больше нигде.
pub fn allow_from_this_executable() {
    if let Ok(executable) = std::env::current_exe() {
        let _ = RUNNER_EXECUTABLE.set(executable);
    }
}

/// Почему фоновое удаление не запустилось: тогда дерево удаляется на месте.
#[derive(Debug, thiserror::Error)]
pub enum SpawnRefusal {
    #[error("this process is not the runner executable")]
    NotTheRunner,
    #[error("the path is not valid Unicode: {0}")]
    NotUnicode(PathBuf),
    #[error(transparent)]
    Spawn(#[from] ServiceSpawnError),
}

/// Запускает отделённый процесс раннера, который удалит прежнее дерево `backup`.
pub fn spawn(backup: &Path) -> Result<u32, SpawnRefusal> {
    let program = RUNNER_EXECUTABLE.get().ok_or(SpawnRefusal::NotTheRunner)?;
    let path = backup
        .to_str()
        .ok_or_else(|| SpawnRefusal::NotUnicode(backup.to_path_buf()))?;
    let request = ServiceRequest {
        program: program.clone(),
        args: vec![COMMAND.to_owned(), "--path".to_owned(), path.to_owned()],
        // Текущий каталог процесса держится открытым: не каталог проекта.
        workdir: Some(std::env::temp_dir()),
        // Отказ виден по оставшемуся дереву, его доберёт следующая команда: журнал не нужен.
        log_path: PathBuf::from(if cfg!(windows) { "NUL" } else { "/dev/null" }),
        stdin: None,
    };
    Ok(spawn_hidden(&request)?)
}

/// Тело скрытой подкоманды: удаляет прежнее дерево `backup`, если это резервная копия раннера
/// с отметкой `superseded`. Процесс идёт с пониженным приоритетом: команда, ответившая раньше,
/// и следующая команда важнее.
pub fn run(backup: &Path) -> i32 {
    lower_own_priority();
    std::thread::sleep(delay());
    match crate::support::fs::remove_superseded_backup(backup) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("remove-backup: {error}");
            1
        }
    }
}

fn delay() -> Duration {
    std::env::var(DELAY_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(Duration::ZERO, |ms| {
            Duration::from_millis(ms).min(MAX_DELAY)
        })
}

#[cfg(windows)]
fn lower_own_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, PROCESS_MODE_BACKGROUND_BEGIN,
    };
    // SAFETY: псевдодескриптор своего процесса закрывать не нужно; вызов меняет только приоритет
    // процессора, ввода-вывода и памяти этого процесса.
    unsafe {
        SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(unix)]
fn lower_own_priority() {
    // SAFETY: `nice` меняет только приоритет этого процесса; отказ оставляет прежний.
    unsafe {
        libc::nice(10);
    }
}

#[cfg(not(any(unix, windows)))]
fn lower_own_priority() {}

#[cfg(test)]
mod tests {
    use super::{spawn, SpawnRefusal};

    /// Сборка тестов — не раннер: фоновое удаление из неё не запускается, дерево удаляется на
    /// месте.
    #[test]
    fn a_process_that_is_not_the_runner_does_not_spawn_the_removal() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(spawn(dir.path()), Err(SpawnRefusal::NotTheRunner)));
    }
}
