//! Файлы файловой базы: признак открытых сеансов по состоянию файла `1Cv8.1CD` и записи
//! платформы верхнего уровня каталога базы.
//!
//! Признак сеанса — единственный владелец вопроса «держит ли базу кто-то, кроме нас». На POSIX
//! это чужая advisory-блокировка на `1Cv8.1CD` (`F_GETLK`, замер #435); на Windows — отказ
//! открыть `1Cv8.1CD` без разделения доступа (`ERROR_SHARING_VIOLATION`, замер #597): всякий
//! держатель базы открывает файл с разделением чтения и записи, но не исключительно.

use std::path::{Path, PathBuf};

/// Имя файла данных файловой базы.
pub const DATA_FILE: &str = "1Cv8.1CD";

/// Что известно о сеансах файловой базы по её файлу данных.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSessions {
    /// Файла данных нет: базы нет.
    Absent,
    /// Файл есть, и никто, кроме проверяющего, его не держит.
    Free,
    /// Файл держит другой процесс. `pid` — его номер, если ОС его назвала.
    Held { path: PathBuf, pid: Option<i32> },
}

/// Ошибка проверки: файл не прочитать или это не обычный файл.
#[derive(Debug, thiserror::Error)]
#[error("cannot probe file infobase '{}': {source}", path.display())]
pub struct ProbeError {
    pub path: PathBuf,
    pub source: std::io::Error,
}

/// Держит ли кто-нибудь файловую базу в каталоге `dir`. Сам проверяющий базу не берёт: на
/// POSIX он только спрашивает ядро, на Windows закрывает свой дескриптор сразу.
pub fn probe_sessions(dir: &Path) -> Result<FileSessions, ProbeError> {
    probe_data_file(&dir.join(DATA_FILE))
}

#[cfg(unix)]
fn probe_data_file(path: &Path) -> Result<FileSessions, ProbeError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let failed = |source: std::io::Error| ProbeError {
        path: path.to_path_buf(),
        source,
    };
    // A malformed target such as a FIFO must fail the metadata check without blocking.
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSessions::Absent)
        }
        Err(source) => return Err(failed(source)),
    };
    let metadata = file.metadata().map_err(failed)?;
    if !metadata.is_file() {
        return Err(failed(not_a_regular_file()));
    }
    // SAFETY: flock contains only integer fields. Zero means SEEK_SET, offset 0,
    // length 0 (the whole file), and supplies valid storage for F_GETLK's answer.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    // SAFETY: file owns a live descriptor, and lock is writable for this call.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) } == -1 {
        return Err(failed(std::io::Error::last_os_error()));
    }
    if lock.l_type != libc::F_UNLCK as libc::c_short {
        // An open file description lock has no owning process: the kernel reports -1.
        return Ok(FileSessions::Held {
            path: path.to_path_buf(),
            pid: (lock.l_pid > 0).then_some(lock.l_pid),
        });
    }
    Ok(FileSessions::Free)
}

#[cfg(windows)]
fn probe_data_file(path: &Path) -> Result<FileSessions, ProbeError> {
    use std::os::windows::fs::OpenOptionsExt;
    let failed = |source: std::io::Error| ProbeError {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(failed(not_a_regular_file())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSessions::Absent)
        }
        Err(source) => return Err(failed(source)),
    }
    // Без разделения доступа: открытие удаётся, только пока файл не открыт никем.
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
    {
        Ok(file) => {
            drop(file);
            Ok(FileSessions::Free)
        }
        Err(error) if is_held(&error) => Ok(FileSessions::Held {
            path: path.to_path_buf(),
            pid: None,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileSessions::Absent),
        Err(source) => Err(failed(source)),
    }
}

/// Отказ ОС, потому что файл держит другой процесс: на Windows — `ERROR_SHARING_VIOLATION`
/// (замер #597). На POSIX такого отказа у открытия и переноса нет.
pub fn is_held(error: &std::io::Error) -> bool {
    /// `ERROR_SHARING_VIOLATION`.
    const SHARING_VIOLATION: i32 = 32;
    cfg!(windows) && error.raw_os_error() == Some(SHARING_VIOLATION)
}

fn not_a_regular_file() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "expected a regular 1Cv8.1CD file",
    )
}

/// Записи платформы верхнего уровня каталога базы: имена на `1Cv8` (без учёта регистра).
/// Ссылки не разворачиваются, вложенные каталоги не обходятся: запись берётся целиком.
/// Каталога нет — записей нет.
pub fn platform_entries(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if is_platform_name(&entry.file_name().to_string_lossy()) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

/// Имя записи, которую заводит платформа: начинается на `1Cv8`. На Windows регистр не
/// различается — имена файлов там к нему нечувствительны; на POSIX сравнение точное, и чужой
/// `1CV8notes` записью платформы не считается.
pub fn is_platform_name(name: &str) -> bool {
    const PREFIX: &str = "1Cv8";
    name.get(..PREFIX.len()).is_some_and(|head| {
        if cfg!(windows) {
            head.eq_ignore_ascii_case(PREFIX)
        } else {
            head == PREFIX
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_top_level_1cv8_names_are_platform_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in [
            "1Cv8.1CD",
            "1Cv8.1CL",
            "1Cv8Log",
            "notes.txt",
            "x1Cv8.1CD",
            "1Cv",
        ] {
            std::fs::write(dir.path().join(name), "").expect("write");
        }
        std::fs::create_dir(dir.path().join("sub")).expect("sub");
        std::fs::write(dir.path().join("sub").join("1Cv8.1CD"), "").expect("nested");
        let names: Vec<String> = platform_entries(dir.path())
            .expect("entries")
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["1Cv8.1CD", "1Cv8.1CL", "1Cv8Log"]);
        assert!(is_platform_name("1Cv8FTxt"));
        assert_eq!(is_platform_name("1CV8notes"), cfg!(windows));
        assert!(platform_entries(&dir.path().join("missing"))
            .expect("missing")
            .is_empty());
    }

    #[test]
    fn a_base_without_its_data_file_is_absent_and_a_closed_one_is_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            probe_sessions(dir.path()).expect("probe"),
            FileSessions::Absent
        );
        std::fs::write(dir.path().join(DATA_FILE), "data").expect("write");
        assert_eq!(
            probe_sessions(dir.path()).expect("probe"),
            FileSessions::Free
        );
    }

    /// На Windows открытый другим дескриптором файл — занятая база.
    #[cfg(windows)]
    #[test]
    fn an_open_data_file_is_held_on_windows() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DATA_FILE);
        std::fs::write(&path, "data").expect("write");
        // Как держит платформа: с разделением чтения и записи, без разделения удаления.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .expect("open");
        assert!(matches!(
            probe_sessions(dir.path()).expect("probe"),
            FileSessions::Held { .. }
        ));
        drop(held);
        assert_eq!(
            probe_sessions(dir.path()).expect("probe"),
            FileSessions::Free
        );
    }
}
