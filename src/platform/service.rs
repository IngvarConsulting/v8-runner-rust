//! Долгоживущая служба стенда: запуск, переживающий выход раннера, и то, что о таком процессе
//! можно узнать у системы.
//!
//! Единственный владелец запуска службы — режим `ProcessRunner::spawn_service`; прямой
//! `CreateProcessW` живёт только здесь (`INV.PLATFORM.A-SERVICE-OUTLIVES-THE-RUNNER`). Команда
//! `std::process::Command` сюда не подходит: у неё нет `STARTUPINFO` для скрытой консоли и списка
//! наследуемых дескрипторов, а без консоли службу нельзя остановить мягко (замер #534). На Unix
//! служба уходит в свой сеанс (`setsid`), и этот вызов тоже живёт только здесь.

use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Что запустить службой.
#[cfg_attr(
    not(any(unix, windows)),
    allow(dead_code, reason = "запуска службы на этой системе нет")
)]
pub struct ServiceRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub workdir: Option<PathBuf>,
    /// Куда идут stdout и stderr службы.
    pub log_path: PathBuf,
    /// Что служба прочитает из stdin: пароль СУБД `ibsrv -W`, чтобы его не было в командной строке.
    pub stdin: Option<String>,
}

/// Служба не переживёт выход раннера.
#[derive(Debug, thiserror::Error)]
pub enum ServiceSpawnError {
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "читает только запуск на Windows")
    )]
    #[error("the runner runs in a Job that kills its processes on close and forbids leaving it: the service would not outlive the command")]
    WouldNotSurvive,
    #[cfg(not(any(unix, windows)))]
    #[error("services run on Windows and Unix only")]
    Unsupported,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Слушает ли кто-нибудь порт на петле.
pub fn port_answers(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Что система знает о процессе: опознание своего — `INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessFacts {
    /// Время старта в единицах системы; сравнивается только на равенство.
    pub started: u64,
    /// Имя образа без пути, в нижнем регистре.
    pub image: String,
}

pub use imp::{listening_pid, process_facts, spawn_hidden};

/// Процесс из перечня системы: кто его родитель и с какой командной строкой он запущен.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent: u32,
    pub command: String,
}

/// Процессы с этими именами образов. Единственный перечень процессов раннера: по командной строке
/// узнают чужой сервер, по родителю — остатки упавшего агента кластера. `None` — перечня нет (запрос
/// не запустился или не ответил вовремя): это не «процессов нет», и разрушительные шаги по нему не
/// идут.
pub fn processes(images: &[&str]) -> Option<Vec<ProcessRow>> {
    if !cfg!(windows) || images.is_empty() {
        return Some(Vec::new());
    }
    let filter = images
        .iter()
        .map(|image| format!("Name='{image}'"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let query = format!(
        "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); Get-CimInstance Win32_Process -Filter \"{filter}\" | ForEach-Object {{ [string]$_.ProcessId + ' ' + [string]$_.ParentProcessId + ' ' + $_.CommandLine }}"
    );
    let Ok(mut child) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &query])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return None;
    };
    // Ответ не дольше этого: зависший запрос не должен держать замок.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| process_rows(&String::from_utf8_lossy(&output.stdout)))
}

/// Разбор строк `pid родитель командная строка`.
fn process_rows(listing: &str) -> Vec<ProcessRow> {
    listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, ' ');
            Some(ProcessRow {
                pid: parts.next()?.parse().ok()?,
                parent: parts.next()?.parse().ok()?,
                command: parts.next().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

/// Первый процесс, в чьей командной строке есть все эти ключи.
pub fn first_with_keys(rows: &[ProcessRow], keys: &[String]) -> Option<u32> {
    let keys: Vec<String> = keys.iter().map(|key| key.to_lowercase()).collect();
    rows.iter().find_map(|row| {
        let tokens = spelled_keys(command_tokens(&row.command.to_lowercase()));
        keys.iter()
            .all(|key| tokens.contains(key))
            .then_some(row.pid)
    })
}

/// Ключи одного вида независимо от написания: `--database-x` то же, что `--db-x`, а значение
/// отдельным словом то же, что через `=`.
fn spelled_keys(tokens: Vec<String>) -> Vec<String> {
    let tokens: Vec<String> = tokens
        .into_iter()
        .map(|token| token.replacen("--database-", "--db-", 1))
        .collect();
    let joined = tokens
        .windows(2)
        .filter(|pair| {
            pair[0].starts_with("--") && !pair[0].contains('=') && !pair[1].starts_with('-')
        })
        .map(|pair| format!("{}={}", pair[0], pair[1]));
    joined.chain(tokens.iter().cloned()).collect()
}

/// Слова командной строки по правилам Windows: пробел внутри кавычек их не делит, а косые черты
/// перед кавычкой экранируют её и друг друга.
fn command_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut slashes = 0usize;
    for symbol in command.chars() {
        if symbol == BACKSLASH {
            slashes += 1;
            continue;
        }
        if symbol == QUOTE_MARK {
            token.extend(std::iter::repeat_n(BACKSLASH, slashes / 2));
            if slashes % 2 == 1 {
                token.push(QUOTE_MARK);
            } else {
                quoted = !quoted;
            }
            slashes = 0;
            continue;
        }
        token.extend(std::iter::repeat_n(BACKSLASH, slashes));
        slashes = 0;
        if symbol.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(symbol);
        }
    }
    token.extend(std::iter::repeat_n(BACKSLASH, slashes));
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

const QUOTE_MARK: char = '"';
const BACKSLASH: char = '\\';

/// Имя файла образа в виде, в котором его сравнивает запись.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "читает только запуск на Windows")
)]
pub fn image_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    /// Строки перечня в старом виде `pid командная строка`, с родителем 1.
    fn server_with(listing: &str, keys: &[String]) -> Option<u32> {
        let rows: Vec<ProcessRow> = listing
            .lines()
            .filter_map(|line| {
                let (pid, command) = line.trim().split_once(' ')?;
                Some(ProcessRow {
                    pid: pid.parse().ok()?,
                    parent: 1,
                    command: command.to_owned(),
                })
            })
            .collect();
        first_with_keys(&rows, keys)
    }

    #[test]
    fn a_listing_names_the_parent_and_keeps_the_command_whole() {
        let rows = process_rows("10 4 C:\\1cv8\\rmngr.exe -port 24001 -host w\n11 4 \n bad\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].parent, 4);
        assert_eq!(rows[0].command, "C:\\1cv8\\rmngr.exe -port 24001 -host w");
        assert_eq!(rows[1].command, "");
    }

    #[test]
    fn a_trailing_backslash_before_the_quote_is_one_backslash() {
        let listing = r#"10 ibsrv.exe "--db-path=E:\my base\\" --name=x"#;
        assert_eq!(
            server_with(listing, &[r"--db-path=E:\my base\".to_owned()]),
            Some(10)
        );
    }

    #[test]
    fn a_server_is_found_by_any_spelling_of_its_keys() {
        let keys = [
            "--db-server=127.0.0.1 port=24432".to_owned(),
            "--db-name=demo".to_owned(),
        ];
        let long = r#"10 ibsrv.exe --dbms=PostgreSQL "--database-server=127.0.0.1 port=24432" --database-name=demo"#;
        assert_eq!(server_with(long, &keys), Some(10));
        let separate = r#"20 ibsrv.exe --db-server "127.0.0.1 port=24432" --db-name demo -W"#;
        assert_eq!(server_with(separate, &keys), Some(20));
    }

    #[test]
    fn a_server_is_found_by_all_of_its_keys_only() {
        let listing = r#"10 "C:\1cv8\ibsrv.exe" "--data=E:\a\data" "--db-path=E:\a\ib" --name=x
20 C:\1cv8\ibsrv.exe --data=E:\b --db-path=E:\b\ib2 --name=y
30 C:\1cv8\ibsrv.exe --data=E:\c --dbms=PostgreSQL "--db-server=127.0.0.1 port=24432" --db-name=demo -W"#;
        let keys = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            server_with(listing, &keys(&[r"--db-path=E:\A\IB"])),
            Some(10)
        );
        assert_eq!(
            server_with(listing, &keys(&[r"--db-path=E:\b\ib2"])),
            Some(20)
        );
        assert_eq!(server_with(listing, &keys(&[r"--db-path=E:\a\i"])), None);
        assert_eq!(
            server_with(
                listing,
                &keys(&["--db-server=127.0.0.1 port=24432", "--db-name=demo"])
            ),
            Some(30)
        );
        assert_eq!(
            server_with(
                listing,
                &keys(&["--db-server=127.0.0.1 port=24432", "--db-name=other"])
            ),
            None
        );
    }
}

#[cfg(windows)]
mod imp {
    use std::borrow::Cow;
    use std::ffi::{OsStr, OsString};
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::io::Write as _;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CloseHandle, SetHandleInformation, ERROR_ACCESS_DENIED, FALSE, FILETIME, HANDLE,
        HANDLE_FLAG_INHERIT, TRUE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::System::JobObjects::{
        IsProcessInJob, JobObjectExtendedLimitInformation, QueryInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetProcessTimes,
        InitializeProcThreadAttributeList, OpenProcess, QueryFullProcessImageNameW,
        UpdateProcThreadAttribute, WaitForSingleObject, CREATE_BREAKAWAY_FROM_JOB,
        CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESHOWWINDOW,
        STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    use super::{image_name, ProcessFacts, ServiceRequest, ServiceSpawnError};

    const SW_HIDE: u16 = 0;
    const AF_INET: u32 = 2;

    /// Как выйти из Job раннера. Замер #534: Job с kill-on-close снимает службу вместе с раннером,
    /// а отделиться без `BREAKAWAY_OK` нельзя: `CreateProcess` отвечает ошибкой 5.
    fn breakaway_flag() -> Result<u32, ServiceSpawnError> {
        let mut in_job = FALSE;
        // SAFETY: псевдодескриптор текущего процесса и указатель на живую переменную.
        if unsafe { IsProcessInJob(GetCurrentProcess(), null_mut(), &mut in_job) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if in_job == FALSE {
            return Ok(0);
        }
        // SAFETY: нулевая структура — допустимое значение этого C-типа.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: пустой дескриптор значит Job текущего процесса; размер равен буферу.
        let queried = unsafe {
            QueryInformationJobObject(
                null_mut(),
                JobObjectExtendedLimitInformation,
                (&raw mut info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                null_mut(),
            )
        };
        if queried == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        decide_breakaway(info.BasicLimitInformation.LimitFlags)
    }

    fn decide_breakaway(flags: u32) -> Result<u32, ServiceSpawnError> {
        let may_leave =
            flags & (JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK) != 0;
        if may_leave {
            Ok(CREATE_BREAKAWAY_FROM_JOB)
        } else if flags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE != 0 {
            Err(ServiceSpawnError::WouldNotSurvive)
        } else {
            Ok(0)
        }
    }

    /// Кавычки по правилам MSVC: их разбирает и `pg_ctl`, и платформа.
    fn quote(arg: &OsStr, out: &mut Vec<u16>) {
        let wide: Vec<u16> = arg.encode_wide().collect();
        let plain = !wide.is_empty()
            && !wide
                .iter()
                .any(|&c| c == u16::from(b' ') || c == u16::from(b'\t') || c == u16::from(b'"'));
        if plain {
            out.extend(wide);
            return;
        }
        out.push(u16::from(b'"'));
        let mut backslashes = 0usize;
        for c in wide {
            if c == u16::from(b'\\') {
                backslashes += 1;
                continue;
            }
            if c == u16::from(b'"') {
                out.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            } else {
                out.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            }
            backslashes = 0;
            out.push(c);
        }
        out.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
        out.push(u16::from(b'"'));
    }

    /// Командная строка службы из аргументов, которые уже прошли
    /// [`crate::support::path::child_line`].
    fn command_line(program: &Path, args: &[Cow<'_, str>]) -> Vec<u16> {
        let mut line = Vec::new();
        quote(program.as_os_str(), &mut line);
        for arg in args {
            line.push(u16::from(b' '));
            quote(OsStr::new(arg.as_ref()), &mut line);
        }
        line.push(0);
        line
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Окружение ребёнка без закрытого протокола раннера с его хозяином.
    fn environment_block() -> Vec<u16> {
        let mut block = Vec::new();
        for (name, value) in std::env::vars_os() {
            if name.eq_ignore_ascii_case("V8_RUNNER_CLIENT_OWNER") {
                continue;
            }
            block.extend(name.encode_wide());
            block.push(u16::from(b'='));
            block.extend(value.encode_wide());
            block.push(0);
        }
        block.push(0);
        block
    }

    fn inheritable(file: &File) -> std::io::Result<HANDLE> {
        let handle = file.as_raw_handle().cast();
        // SAFETY: дескриптор принадлежит живому `file`; владение не передаётся.
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(handle)
    }

    /// Список атрибутов с перечнем наследуемых дескрипторов. Владеет буфером.
    struct HandleList {
        buffer: Vec<u8>,
        handles: Box<[HANDLE; 2]>,
        initialized: bool,
    }

    impl HandleList {
        fn new(handles: [HANDLE; 2]) -> std::io::Result<Self> {
            let mut size = 0usize;
            // SAFETY: первый вызов только спрашивает размер, буфера нет; он и должен вернуть ошибку.
            unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
            let mut list = Self {
                buffer: vec![0; size],
                handles: Box::new(handles),
                initialized: false,
            };
            // SAFETY: буфер запрошенного размера живёт столько же, сколько список.
            if unsafe { InitializeProcThreadAttributeList(list.raw(), 1, 0, &mut size) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            list.initialized = true;
            // SAFETY: значение атрибута — массив дескрипторов в куче, живущий вместе со списком.
            let updated = unsafe {
                UpdateProcThreadAttribute(
                    list.raw(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    list.handles.as_ptr().cast(),
                    std::mem::size_of::<[HANDLE; 2]>(),
                    null_mut(),
                    null(),
                )
            };
            if updated == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(list)
        }

        fn raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.buffer.as_mut_ptr().cast()
        }
    }

    impl Drop for HandleList {
        fn drop(&mut self) {
            if self.initialized {
                // SAFETY: список инициализирован в `new` и удаляется один раз.
                unsafe { DeleteProcThreadAttributeList(self.raw()) };
            }
        }
    }

    /// Чтение из канала, в котором уже лежит этот текст; запись закрыта, поэтому читатель увидит конец.
    fn input_with(text: &str) -> std::io::Result<File> {
        let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
        // Читателя ещё нет: запись не должна ждать, поэтому буфер вмещает весь текст.
        let size = u32::try_from(text.len() + 1)
            .map_err(|_| std::io::Error::other("the text for stdin is too long"))?;
        // SAFETY: указатели смотрят на живые переменные, атрибуты по умолчанию.
        if unsafe { CreatePipe(&mut read, &mut write, null(), size) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: оба дескриптора только что созданы и переходят во владение `File`.
        let (read, mut write) = unsafe {
            (
                File::from_raw_handle(read.cast()),
                File::from_raw_handle(write.cast()),
            )
        };
        write.write_all(text.as_bytes())?;
        write.write_all(b"\n")?;
        Ok(read)
    }

    /// Запускает службу со скрытой консолью, без унаследованных дескрипторов, вывод в журнал.
    pub fn spawn_hidden(request: &ServiceRequest) -> Result<u32, ServiceSpawnError> {
        create_hidden(request, breakaway_flag()?)
    }

    fn create_hidden(request: &ServiceRequest, breakaway: u32) -> Result<u32, ServiceSpawnError> {
        if let Some(parent) = request.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&request.log_path)?;
        let input = match &request.stdin {
            Some(text) => input_with(text)?,
            None => OpenOptions::new().read(true).open("NUL")?,
        };
        let stdout = inheritable(&log)?;
        let stdin = inheritable(&input)?;
        let mut list = HandleList::new([stdout, stdin])?;

        // SAFETY: нулевая структура — допустимое значение этого C-типа.
        let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESHOWWINDOW | STARTF_USESTDHANDLES;
        startup.StartupInfo.wShowWindow = SW_HIDE;
        startup.StartupInfo.hStdInput = stdin;
        startup.StartupInfo.hStdOutput = stdout;
        startup.StartupInfo.hStdError = stdout;
        startup.lpAttributeList = list.raw();

        // Служба получает пути без приставки `\\?\`
        // (`INV.PLATFORM.A-PROCESS-ARGUMENT-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX`).
        let child = crate::support::path::child_line(&request.args, request.workdir.as_deref())
            .map_err(|refused| io::Error::new(io::ErrorKind::InvalidInput, refused))?;
        let program = wide(request.program.as_os_str());
        let mut line = command_line(&request.program, &child.args);
        let environment = environment_block();
        let workdir = child.workdir.as_deref().map(|dir| wide(dir.as_os_str()));
        // SAFETY: нулевая структура — допустимое значение этого C-типа.
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: все указатели смотрят на живые буферы с завершающим нулём; `line` изменяем, как
        // требует функция; `startup` — расширенная форма, о чём говорит флаг.
        let created = unsafe {
            CreateProcessW(
                program.as_ptr(),
                line.as_mut_ptr(),
                null(),
                null(),
                TRUE,
                CREATE_NEW_CONSOLE
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT
                    | breakaway,
                environment.as_ptr().cast(),
                workdir.as_ref().map_or(null(), |dir| dir.as_ptr()),
                (&raw const startup).cast(),
                &mut process,
            )
        };
        if created == 0 {
            let error = std::io::Error::last_os_error();
            // Внешний Job отделяться не разрешает, хотя ближайший разрешил.
            if breakaway != 0 && error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                return Err(ServiceSpawnError::WouldNotSurvive);
            }
            return Err(error.into());
        }
        // SAFETY: оба дескриптора возвращены успешным вызовом и больше не нужны.
        unsafe {
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
        }
        Ok(process.dwProcessId)
    }

    /// Факты о процессе; `None`, если его нет или он недоступен.
    pub fn process_facts(pid: u32) -> Option<ProcessFacts> {
        // SAFETY: значения передаются по значению, результат проверяется.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                FALSE,
                pid,
            )
        };
        if handle.is_null() {
            return None;
        }
        // Завершённый процесс отвечает, пока кто-то держит его дескриптор.
        // SAFETY: дескриптор открыт выше, нулевое ожидание только опрашивает состояние.
        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
            // SAFETY: дескриптор открыт выше и закрывается один раз.
            unsafe { CloseHandle(handle) };
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: дескриптор открыт выше, указатели смотрят на живые переменные.
        let timed =
            unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
                != 0;
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        // SAFETY: размер равен длине буфера, функция пишет не больше него.
        let named =
            unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) } != 0;
        // SAFETY: дескриптор открыт выше и закрывается один раз.
        unsafe { CloseHandle(handle) };
        if !timed || !named {
            return None;
        }
        let path = OsString::from_wide(&buffer[..size as usize]);
        Some(ProcessFacts {
            started: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            image: image_name(std::path::Path::new(&path)),
        })
    }

    /// Какой процесс слушает порт на IPv4.
    pub fn listening_pid(port: u16) -> Option<u32> {
        let mut size = 0u32;
        // SAFETY: первый вызов спрашивает размер: пустой буфер допустим.
        unsafe {
            GetExtendedTcpTable(
                null_mut(),
                &mut size,
                FALSE,
                AF_INET,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        let mut buffer = Vec::new();
        // Таблица могла вырасти между вызовами: новый размер называет сам отказ.
        let answered = (0..3).any(|_| {
            buffer.resize(size as usize, 0u8);
            // SAFETY: буфер запрошенного размера, функция пишет не больше `size`.
            let code = unsafe {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut size,
                    FALSE,
                    AF_INET,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            code == 0
        });
        if !answered {
            return None;
        }
        let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        // SAFETY: успешный вызов записал таблицу в начало буфера; поля читаются без выравнивания,
        // потому что буфер байтовый; записей ровно `dwNumEntries`.
        unsafe {
            let count = std::ptr::addr_of!((*table).dwNumEntries).read_unaligned() as usize;
            let rows = std::ptr::addr_of!((*table).table)
                .cast::<windows_sys::Win32::NetworkManagement::IpHelper::MIB_TCPROW_OWNER_PID>(
            );
            (0..count).find_map(|index| {
                let row = rows.add(index).read_unaligned();
                // Порт в таблице — младшие 16 бит в сетевом порядке.
                (u16::from_be(row.dwLocalPort as u16) == port).then_some(row.dwOwningPid)
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_job_that_kills_on_close_and_forbids_leaving_refuses_the_service() {
            assert!(matches!(
                decide_breakaway(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE),
                Err(ServiceSpawnError::WouldNotSurvive)
            ));
            assert_eq!(
                decide_breakaway(
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK
                )
                .unwrap(),
                CREATE_BREAKAWAY_FROM_JOB
            );
            assert_eq!(decide_breakaway(0).unwrap(), 0);
        }

        #[test]
        fn a_hidden_service_writes_to_its_log_and_gets_a_real_pid() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("sub").join("service.log");
            let system = std::env::var_os("ComSpec").expect("ComSpec");
            // Без отделения от Job: `cargo test` держит тесты в Job, из которого выйти нельзя.
            let pid = create_hidden(
                &ServiceRequest {
                    program: system.into(),
                    args: vec!["/C".to_owned(), "echo hello".to_owned()],
                    workdir: Some(dir.path().to_path_buf()),
                    log_path: log.clone(),
                    stdin: None,
                },
                0,
            )
            .unwrap();
            assert_ne!(pid, 0);
            for _ in 0..100 {
                if std::fs::read_to_string(&log).is_ok_and(|text| text.contains("hello")) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            panic!("the service did not write to its log");
        }

        /// Служба получает путь без приставки `\\?\`, а путь тома без буквы до запуска не доходит
        /// (`INV.PLATFORM.A-PROCESS-ARGUMENT-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX`).
        #[test]
        fn a_service_receives_paths_without_the_verbatim_prefix() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("service.log");
            let system = std::env::var_os("ComSpec").expect("ComSpec");
            let verbatim = format!(r"\\?\{}", dir.path().display());
            create_hidden(
                &ServiceRequest {
                    program: system.clone().into(),
                    // Одним аргументом, как в соседнем тесте: `cmd /C` с несколькими кавычками
                    // кавычки не снимает.
                    args: vec!["/C".to_owned(), r"echo \\?\C:\dir\base.dt".to_owned()],
                    workdir: Some(verbatim.into()),
                    log_path: log.clone(),
                    stdin: None,
                },
                0,
            )
            .unwrap();
            let mut text = String::new();
            for _ in 0..100 {
                text =
                    String::from_utf8_lossy(&std::fs::read(&log).unwrap_or_default()).into_owned();
                if text.contains("base.dt") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            assert!(
                text.contains("base.dt"),
                "the service did not run: {text:?}"
            );
            assert!(!text.contains(r"\\?\"), "{text}");

            let refused = create_hidden(
                &ServiceRequest {
                    program: system.into(),
                    args: vec![r"\\?\Volume{0b2c1f4e-1111-2222-3333-444455556666}\x".to_owned()],
                    workdir: None,
                    log_path: dir.path().join("refused.log"),
                    stdin: None,
                },
                0,
            )
            .expect_err("a volume path is refused before the start");
            assert!(refused.to_string().contains("Volume{"), "{refused}");
        }

        #[test]
        fn a_service_reads_what_was_given_to_its_stdin_and_the_argument_stays_clean() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("service.log");
            let system = std::env::var_os("ComSpec").expect("ComSpec");
            create_hidden(
                &ServiceRequest {
                    program: system.into(),
                    args: vec!["/C".to_owned(), "findstr .".to_owned()],
                    workdir: Some(dir.path().to_path_buf()),
                    log_path: log.clone(),
                    stdin: Some("secret-value".to_owned()),
                },
                0,
            )
            .unwrap();
            for _ in 0..100 {
                if std::fs::read_to_string(&log).is_ok_and(|text| text.contains("secret-value")) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            panic!("the service did not read its stdin");
        }

        #[test]
        fn a_listener_is_found_by_its_port_and_a_process_by_its_pid() {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            assert_eq!(listening_pid(port), Some(std::process::id()));
            let facts = process_facts(std::process::id()).unwrap();
            assert!(
                facts.started > 0 && facts.image.ends_with(".exe"),
                "{facts:?}"
            );
        }

        #[test]
        fn arguments_are_quoted_by_the_msvc_rules() {
            let render = |arg: &str| {
                let mut out = Vec::new();
                quote(OsStr::new(arg), &mut out);
                String::from_utf16(&out).unwrap()
            };
            assert_eq!(render("plain"), "plain");
            assert_eq!(render("two words"), "\"two words\"");
            assert_eq!(render("a\"b"), "\"a\\\"b\"");
            assert_eq!(render("C:\\dir with\\"), "\"C:\\dir with\\\\\"");
            assert_eq!(render(""), "\"\"");
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::fs::OpenOptions;
    use std::io::Write as _;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};

    use super::{ProcessFacts, ServiceRequest, ServiceSpawnError};

    /// Запускает службу в своём сеансе и своей группе процессов (`setsid`): сигнал терминала или
    /// группы раннера до неё не доходит. Наследует она только журнал на stdout и stderr и stdin;
    /// остальные дескрипторы закрываются при `exec`. Job на Unix нет, поэтому отказа
    /// `WouldNotSurvive` здесь не бывает.
    pub fn spawn_hidden(request: &ServiceRequest) -> Result<u32, ServiceSpawnError> {
        if let Some(parent) = request.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&request.log_path)?;
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        if let Some(workdir) = &request.workdir {
            command.current_dir(workdir);
        }
        // SAFETY: между `fork` и `exec` вызываются только безопасные для сигналов `setsid`,
        // `close_range`, `getdtablesize` и `fcntl`; память не выделяется.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                close_on_exec_beyond_stdio();
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let pid = child.id();
        if let (Some(text), Some(mut input)) = (&request.stdin, child.stdin.take()) {
            // Служба могла выйти, не прочитав: это видно по её журналу, а не по записи сюда.
            let written = input
                .write_all(text.as_bytes())
                .and_then(|()| input.write_all(b"\n"));
            if let Err(error) = written {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    // Без pid в ответе запись о службе не появится, и снять её потом будет
                    // некому: служба снимается здесь же и не остаётся зомби.
                    drop(input);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
        }
        // Пока раннер жив, завершённая служба без ожидания остаётся зомби; поток ждёт её и не
        // держит выход раннера.
        std::thread::spawn(move || child.wait());
        Ok(pid)
    }

    /// Помечает «закрыть при `exec`» все дескрипторы, кроме stdin, stdout и stderr, даже
    /// открытые чужим кодом без `O_CLOEXEC`. Вызывается между `fork` и `exec`.
    fn close_on_exec_beyond_stdio() {
        #[cfg(target_os = "linux")]
        {
            const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
            // SAFETY: системный вызов с числовыми аргументами; ядро старее 5.11 отвечает ошибкой,
            // и тогда дескрипторы помечает цикл ниже.
            let marked = unsafe {
                libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOSE_RANGE_CLOEXEC)
            };
            if marked == 0 {
                return;
            }
        }
        // SAFETY: `getdtablesize` и `fcntl` безопасны для сигналов; незанятый номер отвечает
        // `EBADF` и пропускается.
        unsafe {
            let limit = libc::getdtablesize().clamp(3, 65_536);
            for fd in 3..limit {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }
        }
    }

    /// Факты о процессе по `/proc/<pid>`; `None`, если его нет, он чужой учётной записи или уже
    /// завершён и ждёт родителя. Время старта — поле 22 `stat`, тики часов после загрузки. Это
    /// значение у процесса не меняется, а пересчёт в секунды от эпохи через `btime` сдвигается
    /// вместе с часами системы и дал бы своему процессу другое время. Сравнивается оно только
    /// на равенство.
    #[cfg(target_os = "linux")]
    pub fn process_facts(pid: u32) -> Option<ProcessFacts> {
        use crate::support::machine::proc_stat_field;

        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        if matches!(proc_stat_field(&stat, 3), Some("Z" | "X")) {
            return None;
        }
        let started = proc_stat_field(&stat, 22)?.parse().ok()?;
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        Some(ProcessFacts {
            started,
            image: super::image_name(&without_deleted_mark(exe)),
        })
    }

    /// Путь образа из `/proc/<pid>/exe` без приписки ` (deleted)`, которую ядро ставит, когда
    /// файл образа заменён или удалён после старта: процесс остаётся тем же.
    #[cfg(any(target_os = "linux", test))]
    pub(super) fn without_deleted_mark(exe: std::path::PathBuf) -> std::path::PathBuf {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        const MARK: &[u8] = b" (deleted)";
        match exe.as_os_str().as_bytes().strip_suffix(MARK) {
            Some(kept) => std::ffi::OsString::from_vec(kept.to_vec()).into(),
            None => exe,
        }
    }

    /// Факты о процессе по `proc_pidinfo`; время старта — микросекунды от эпохи.
    #[cfg(target_vendor = "apple")]
    pub fn process_facts(pid: u32) -> Option<ProcessFacts> {
        use std::os::unix::ffi::OsStringExt as _;

        /// `SZOMB` из `sys/proc.h`: процесс завершён и ждёт родителя.
        const SZOMB: u32 = 5;
        let pid = libc::c_int::try_from(pid).ok().filter(|pid| *pid > 0)?;
        // SAFETY: нулевая структура — допустимое значение этого C-типа.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
        // SAFETY: буфер — живая структура указанного размера.
        let written = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size)
        };
        if written != size || info.pbi_status == SZOMB {
            return None;
        }
        let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: размер равен длине буфера, функция пишет не больше него.
        let length =
            unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
        let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
        path.truncate(length);
        let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(path));
        Some(ProcessFacts {
            started: info
                .pbi_start_tvsec
                .checked_mul(1_000_000)?
                .checked_add(info.pbi_start_tvusec)?,
            image: super::image_name(&path),
        })
    }

    // TODO(#577): на других Unix раннер не знает, как спросить время старта процесса. Там свой
    // процесс не опознаётся, и запись о нём считается устаревшей.
    #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
    pub fn process_facts(_pid: u32) -> Option<ProcessFacts> {
        None
    }

    /// Владельца порта на Unix раннер не ищет: управляемые службы стенда работают только на
    /// Windows.
    pub fn listening_pid(_port: u16) -> Option<u32> {
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::Path;
        use std::time::Duration;

        fn wait_for(log: &Path, text: &str) -> bool {
            (0..100).any(|_| {
                if std::fs::read_to_string(log).is_ok_and(|seen| seen.contains(text)) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(100));
                false
            })
        }

        fn shell(script: &str, log: &Path, stdin: Option<&str>) -> ServiceRequest {
            ServiceRequest {
                program: "/bin/sh".into(),
                args: vec!["-c".to_owned(), script.to_owned()],
                workdir: log.parent().map(Path::to_path_buf),
                log_path: log.to_path_buf(),
                stdin: stdin.map(str::to_owned),
            }
        }

        #[test]
        fn a_hidden_service_writes_to_its_log_and_gets_a_real_pid() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("sub").join("service.log");
            let pid = spawn_hidden(&shell("echo hello", &log, None)).unwrap();
            assert_ne!(pid, 0);
            assert!(
                wait_for(&log, "hello"),
                "the service did not write to its log"
            );
        }

        #[test]
        fn a_service_reads_what_was_given_to_its_stdin_and_the_argument_stays_clean() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("service.log");
            let request = shell("cat", &log, Some("secret-value"));
            spawn_hidden(&request).unwrap();
            assert!(
                wait_for(&log, "secret-value"),
                "the service did not read its stdin"
            );
            assert!(!request.args.iter().any(|arg| arg.contains("secret-value")));
        }

        #[test]
        fn a_service_runs_in_its_own_session_and_keeps_no_descriptor_of_the_runner() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("service.log");
            // Дескриптор без `O_CLOEXEC`: служба его наследовать не должна.
            // SAFETY: `dup` возвращает новый дескриптор или -1; он закрывается ниже.
            let leaked = unsafe { libc::dup(2) };
            assert!(leaked > 2);
            let script = format!(
                "if [ -e /dev/fd/{leaked} ]; then echo leaked; else echo clean; fi; exec sleep 30"
            );
            let pid = spawn_hidden(&shell(&script, &log, None));
            // SAFETY: дескриптор открыт выше и закрывается один раз.
            unsafe { libc::close(leaked) };
            let pid = libc::pid_t::try_from(pid.unwrap()).unwrap();
            let answered = wait_for(&log, "clean") || wait_for(&log, "leaked");
            // SAFETY: числовые аргументы; процесс — служба этого теста.
            let (session, group) = unsafe { (libc::getsid(pid), libc::getpgid(pid)) };
            // SAFETY: снимаем службу этого теста.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            assert!(answered, "the service did not answer");
            assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "clean");
            assert_eq!(session, pid);
            assert_eq!(group, pid);
        }

        #[test]
        fn a_missing_program_is_refused_at_the_start() {
            let dir = tempfile::tempdir().unwrap();
            let mut request = shell("true", &dir.path().join("service.log"), None);
            request.program = dir.path().join("no-such-program");
            assert!(matches!(
                spawn_hidden(&request),
                Err(ServiceSpawnError::Io(_))
            ));
        }

        #[test]
        fn a_replaced_image_keeps_its_name() {
            use std::path::PathBuf;
            assert_eq!(
                without_deleted_mark(PathBuf::from("/opt/v8-runner/keeper (deleted)")),
                PathBuf::from("/opt/v8-runner/keeper")
            );
            assert_eq!(
                without_deleted_mark(PathBuf::from("/usr/bin/sleep")),
                PathBuf::from("/usr/bin/sleep")
            );
            assert_eq!(
                without_deleted_mark(PathBuf::from("/tmp/(deleted)")),
                PathBuf::from("/tmp/(deleted)")
            );
        }

        #[cfg(any(target_os = "linux", target_vendor = "apple"))]
        #[test]
        fn a_process_is_known_by_its_start_and_image_and_a_gone_one_is_not() {
            let facts = process_facts(std::process::id()).unwrap();
            let image = std::env::current_exe().unwrap();
            assert_eq!(facts.image, super::super::image_name(&image));
            assert!(facts.started > 0, "{facts:?}");
            assert_eq!(process_facts(std::process::id()), Some(facts));

            // `sleep` бывает ссылкой (на busybox, coreutils): образ — файл, куда она ведёт.
            let sleep = std::env::split_paths(&std::env::var_os("PATH").unwrap())
                .map(|dir| dir.join("sleep"))
                .find(|path| path.is_file())
                .expect("sleep on PATH");
            let mut child = Command::new(&sleep).arg("30").spawn().unwrap();
            let child_facts = process_facts(child.id()).unwrap();
            assert_eq!(
                child_facts.image,
                super::super::image_name(&std::fs::canonicalize(&sleep).unwrap())
            );
            child.kill().unwrap();
            child.wait().unwrap();
            assert_eq!(process_facts(child.id()), None);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::{ProcessFacts, ServiceRequest, ServiceSpawnError};

    pub fn spawn_hidden(_request: &ServiceRequest) -> Result<u32, ServiceSpawnError> {
        Err(ServiceSpawnError::Unsupported)
    }

    pub fn process_facts(_pid: u32) -> Option<ProcessFacts> {
        None
    }

    pub fn listening_pid(_port: u16) -> Option<u32> {
        None
    }
}
