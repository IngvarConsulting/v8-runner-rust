//! Долгоживущая служба стенда: запуск, переживающий выход раннера, и то, что о таком процессе
//! можно узнать у системы.
//!
//! Единственный владелец запуска службы — режим `ProcessRunner::spawn_service`; прямой
//! `CreateProcessW` живёт только здесь (`INV.PLATFORM.A-SERVICE-OUTLIVES-THE-RUNNER`). Команда
//! `std::process::Command` сюда не подходит: у неё нет `STARTUPINFO` для скрытой консоли и списка
//! наследуемых дескрипторов, а без консоли службу нельзя остановить мягко (замер #534).

use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Что запустить службой.
#[derive(Debug, Clone)]
pub struct ServiceRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub workdir: Option<PathBuf>,
    /// Куда идут stdout и stderr службы.
    pub log_path: PathBuf,
}

/// Служба не переживёт выход раннера.
#[derive(Debug, thiserror::Error)]
pub enum ServiceSpawnError {
    #[error("the runner runs in a Job that kills its processes on close and forbids leaving it: the service would not outlive the command")]
    WouldNotSurvive,
    #[cfg(not(windows))]
    #[error("services run on Windows only")]
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

/// Имя файла образа в виде, в котором его сравнивает запись.
pub fn image_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, OsString};
    use std::fs::{File, OpenOptions};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::AsRawHandle;
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

    fn command_line(request: &ServiceRequest) -> Vec<u16> {
        let mut line = Vec::new();
        quote(request.program.as_os_str(), &mut line);
        for arg in &request.args {
            line.push(u16::from(b' '));
            quote(OsStr::new(arg), &mut line);
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
        let null_input = OpenOptions::new().read(true).open("NUL")?;
        let stdout = inheritable(&log)?;
        let stdin = inheritable(&null_input)?;
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

        let program = wide(request.program.as_os_str());
        let mut line = command_line(request);
        let environment = environment_block();
        let workdir = request.workdir.as_ref().map(|dir| wide(dir.as_os_str()));
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

#[cfg(not(windows))]
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
