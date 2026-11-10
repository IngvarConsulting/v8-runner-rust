//! Кто держит файловую базу — по состоянию файлов, а не по тексту платформы.
//!
//! Замеры: macOS — раздел «`ibcmd infobase dump|restore` файловой базы и признак сеанса»
//! (#435) и «Удержание файловой базы на время команды» (#468); Windows — раздел «Чужие сеансы
//! файловой базы при обновлении конфигурации БД (#211)».
//!
//! - Сеанс любого приложения держит `1Cv8.1CD`: на POSIX блокировкой `fcntl`, на Windows
//!   режимом совместного доступа — открыть файл без совместного доступа нельзя (ошибка 32).
//! - Конфигуратор (интерактивный, пакетный, агент после `connect-ib`) держит ещё `1Cv8.cgr`: на
//!   POSIX `F_WRLCK`, на Windows файл не открывается даже с совместным чтением и записью.
//! - Блокировка начала сеансов — файл `1Cv8.cdn`.
//!
//! Проба на Windows на миг сама открывает `1Cv8.1CD` монопольно; её зовут только после отказа
//! исполнителя, когда раннер уже ничего в базе не делает.

#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::Path;

use crate::domain::sessions::{SessionLock, SessionLockState};

/// Файл базы, который держит любой сеанс.
pub const DATABASE_FILE: &str = "1Cv8.1CD";
/// Файл, который держит Конфигуратор.
pub const DESIGNER_FILE: &str = "1Cv8.cgr";
/// Файл блокировки начала сеансов.
pub const SESSION_LOCK_FILE: &str = "1Cv8.cdn";

/// Держит ли кто-нибудь файл.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Файла нет.
    Absent,
    Free,
    /// Держит другой процесс; номер есть только у блокировки `fcntl`.
    Held {
        pid: Option<i32>,
    },
}

impl Hold {
    pub const fn is_held(self) -> bool {
        matches!(self, Self::Held { .. })
    }
}

/// Держит ли базу хоть один сеанс.
pub fn sessions_hold(base: &Path) -> io::Result<Hold> {
    probe(&base.join(DATABASE_FILE), Access::Exclusive)
}

/// Держит ли базу Конфигуратор.
pub fn designer_hold(base: &Path) -> io::Result<Hold> {
    probe(&base.join(DESIGNER_FILE), Access::Shared)
}

/// Блокировка начала сеансов у базы. Раннер её не ставит, поэтому стоящая — чужая; сообщение —
/// четвёртое поле записи `{1,<начало>,<конец>,"<сообщение>","<код>",…}`. Нечитаемый файл тоже
/// блокировка: испорченный `1Cv8.cdn` не пускает никого (#468).
pub fn session_lock(base: &Path) -> SessionLock {
    match std::fs::read(base.join(SESSION_LOCK_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => SessionLock::none(),
        Err(_) => SessionLock {
            state: SessionLockState::Foreign,
            message: None,
        },
        Ok(bytes) => SessionLock {
            state: SessionLockState::Foreign,
            message: lock_message(&String::from_utf8_lossy(&bytes)),
        },
    }
}

/// Первая строка в кавычках записи `1Cv8.cdn`; `""` внутри — кавычка.
fn lock_message(record: &str) -> Option<String> {
    let start = record.find('"')? + 1;
    let mut message = String::new();
    let mut chars = record[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                message.push('"');
            } else {
                return Some(message);
            }
        } else {
            message.push(c);
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Access {
    /// Признак «держит кто угодно».
    Exclusive,
    /// Признак «держит монопольно».
    Shared,
}

#[cfg(windows)]
fn probe(path: &Path, access: Access) -> io::Result<Hold> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let share = match access {
        Access::Exclusive => 0,
        Access::Shared => FILE_SHARE_READ | FILE_SHARE_WRITE,
    };
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(share)
        .open(path)
    {
        Ok(_) => Ok(Hold::Free),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Hold::Absent),
        Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
            Ok(Hold::Held { pid: None })
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn probe(path: &Path, _access: Access) -> io::Result<Hold> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Hold::Absent),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("expected a regular file at {}", path.display()),
        ));
    }
    lock_holder(&file)
}

/// Чужая блокировка `fcntl` на весь файл: `F_GETLK` спрашивает, не беря свою, — своя не
/// пустила бы и платформу.
#[cfg(unix)]
pub fn lock_holder(file: &File) -> io::Result<Hold> {
    use std::os::fd::AsRawFd;
    // SAFETY: flock contains only integer fields. Zero means SEEK_SET, offset 0,
    // length 0 (the whole file), and supplies valid storage for F_GETLK's answer.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    // SAFETY: file owns a live descriptor, and lock is writable for this call.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if lock.l_type == libc::F_UNLCK as libc::c_short {
        return Ok(Hold::Free);
    }
    // An open file description lock has no owning process: the kernel reports -1.
    Ok(Hold::Held {
        pid: (lock.l_pid > 0).then_some(lock.l_pid),
    })
}

/// Отказ ОС, потому что файл держит другой процесс: на Windows — `ERROR_SHARING_VIOLATION`
/// у открытия и у переноса (замер #597). На POSIX такого отказа нет.
pub fn is_sharing_violation(error: &io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    cfg!(windows) && error.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

/// Записи платформы верхнего уровня каталога базы ([`is_platform_name`]). Ссылки не
/// разворачиваются, вложенные каталоги не обходятся: запись берётся целиком. Каталога нет —
/// записей нет.
pub fn platform_entries(dir: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
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

/// Имя записи, которую заводит платформа в каталоге файловой базы: начинается на `1Cv8`
/// (замер #597). На Windows регистр не различается — имена файлов там к нему нечувствительны;
/// на POSIX сравнение точное, и чужой `1CV8notes` записью платформы не считается.
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

#[cfg(not(any(unix, windows)))]
fn probe(_path: &Path, _access: Access) -> io::Result<Hold> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no file base probe on this platform",
    ))
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
    fn a_base_without_files_is_absent_and_unlocked() {
        let dir = tempfile::tempdir().expect("dir");
        assert_eq!(sessions_hold(dir.path()).expect("probe"), Hold::Absent);
        assert_eq!(designer_hold(dir.path()).expect("probe"), Hold::Absent);
        assert_eq!(session_lock(dir.path()), SessionLock::none());
    }

    #[test]
    fn a_base_nobody_holds_is_free() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join(DATABASE_FILE), b"db").expect("db");
        std::fs::write(dir.path().join(DESIGNER_FILE), b"cgr").expect("cgr");
        assert_eq!(sessions_hold(dir.path()).expect("probe"), Hold::Free);
        assert_eq!(designer_hold(dir.path()).expect("probe"), Hold::Free);
    }

    /// Открытый без совместного доступа файл — то, что видит проба при сеансе на Windows.
    #[cfg(windows)]
    #[test]
    fn a_file_another_handle_holds_is_held() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join(DATABASE_FILE);
        std::fs::write(&path, b"db").expect("db");
        let _session = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .expect("session handle");
        assert!(sessions_hold(dir.path()).expect("probe").is_held());
    }

    #[test]
    fn a_foreign_lock_names_its_message() {
        let dir = tempfile::tempdir().expect("dir");
        // Запись из замера #468: UTF-8 с BOM.
        std::fs::write(
            dir.path().join(SESSION_LOCK_FILE),
            "\u{feff}{1,20261008190813,00010101000000,\"probe \"\"468\"\", wait\",\"K468\",\"\",0}",
        )
        .expect("cdn");
        let lock = session_lock(dir.path());
        assert_eq!(lock.state, SessionLockState::Foreign);
        assert_eq!(lock.message.as_deref(), Some("probe \"468\", wait"));

        std::fs::write(dir.path().join(SESSION_LOCK_FILE), "garbage").expect("cdn");
        let lock = session_lock(dir.path());
        assert_eq!(lock.state, SessionLockState::Foreign);
        assert_eq!(lock.message, None);
    }
}
