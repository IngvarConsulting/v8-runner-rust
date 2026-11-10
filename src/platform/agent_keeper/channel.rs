//! Канал сторожа, открытый только своей учётной записи ОС
//! (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`): именованный канал Windows с DACL «только
//! пользователь процесса и LocalSystem» или сокет Unix с правами 0600 в каталоге 0700.
//!
//! Клиент сверяет и другой конец: на Windows канал должен обслуживать процесс сторожа из записи
//! (`GetNamedPipeServerProcessId`), на Unix сокет — процесс своей учётной записи (`SO_PEERCRED`,
//! `getpeereid`). Так чужой процесс, занявший имя канала раньше сторожа, команд не получит.

use tokio::io::{AsyncRead, AsyncWrite};

/// Подключение к сторожу с любой стороны.
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

pub type Stream = Box<dyn Duplex>;

#[cfg(windows)]
mod imp {
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::Stream;

    /// Сколько клиент ждёт свободного экземпляра канала, прежде чем отказать.
    const BUSY_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

    /// Слушатель сторожа: следующий экземпляр канала создаётся до того, как отдан принятый.
    pub struct Listener {
        name: String,
        next: NamedPipeServer,
    }

    /// DACL канала: защищённый, полный доступ только пользователю из маркера процесса и
    /// LocalSystem. Владелец объекта (`OW`) не годится: у повышенного администратора им
    /// становится группа «Администраторы».
    fn pipe_sddl() -> std::io::Result<String> {
        let user = crate::support::machine::account::account_id()?;
        Ok(format!("D:P(A;;GA;;;{user})(A;;GA;;;SY)"))
    }

    fn instance(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        crate::support::temp::with_security(&pipe_sddl()?, |attributes| {
            // SAFETY: `attributes` points to a live SECURITY_ATTRIBUTES whose descriptor stays
            // valid for this call; the call does not keep the pointer.
            unsafe {
                options.create_with_security_attributes_raw(
                    name,
                    std::ptr::from_ref(attributes).cast_mut().cast(),
                )
            }
        })?
    }

    impl Listener {
        /// Первый экземпляр канала. Канал с этим именем уже есть — работает другой сторож:
        /// отказ `AddrInUse`.
        pub fn bind(name: &str) -> std::io::Result<Self> {
            let next = instance(name, true).map_err(|error| {
                if error.raw_os_error() == Some(5) {
                    std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!("another keeper serves {name}"),
                    )
                } else {
                    error
                }
            })?;
            Ok(Self {
                name: name.to_owned(),
                next,
            })
        }

        pub async fn accept(&mut self) -> std::io::Result<Stream> {
            self.next.connect().await?;
            let accepted = std::mem::replace(&mut self.next, instance(&self.name, false)?);
            Ok(Box::new(accepted))
        }

        /// Канал уходит вместе с последним дескриптором: убирать на диске нечего.
        pub fn remove(&self) {}

        /// Дескриптор ожидающего экземпляра: тест читает по нему DACL канала.
        #[cfg(test)]
        pub fn handle(&self) -> std::os::windows::io::RawHandle {
            std::os::windows::io::AsRawHandle::as_raw_handle(&self.next)
        }
    }

    /// Подключение к каналу, который обслуживает процесс `keeper`.
    pub async fn connect(name: &str, keeper: u32) -> std::io::Result<Stream> {
        const ERROR_PIPE_BUSY: i32 = 231;
        let started = std::time::Instant::now();
        loop {
            match ClientOptions::new().open(name) {
                Ok(client) => {
                    let server = server_pid(&client)?;
                    if server != keeper {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!(
                                "{name} is served by process {server}, not by the keeper {keeper}"
                            ),
                        ));
                    }
                    return Ok(Box::new(client));
                }
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY)
                        && started.elapsed() < BUSY_PATIENCE =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Процесс, который обслуживает канал на том конце.
    fn server_pid(
        client: &tokio::net::windows::named_pipe::NamedPipeClient,
    ) -> std::io::Result<u32> {
        use std::os::windows::io::AsRawHandle;
        let mut pid = 0u32;
        // SAFETY: a live pipe handle owned by `client`; `pid` outlives the call.
        let ok = unsafe {
            windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(
                client.as_raw_handle().cast(),
                &mut pid,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(pid)
    }
}

#[cfg(unix)]
mod imp {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use tokio::net::{UnixListener, UnixStream};

    use super::Stream;

    /// Предел пути сокета: `sun_path` — 108 байт на Linux и 104 на macOS, с нулём в конце.
    const SOCKET_PATH_LIMIT: usize = 103;

    pub struct Listener {
        path: PathBuf,
        listener: UnixListener,
    }

    impl Listener {
        /// Сокет по пути `name`. Отвечающий сокет — работает другой сторож: отказ `AddrInUse`;
        /// оставшийся от снятого сторожа файл убирается.
        pub fn bind(name: &str) -> std::io::Result<Self> {
            if name.len() > SOCKET_PATH_LIMIT {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "the keeper socket path '{name}' is longer than {SOCKET_PATH_LIMIT} bytes; \
                         set {} to a shorter directory",
                        crate::support::machine::account::STATE_DIR_ENV
                    ),
                ));
            }
            let path = PathBuf::from(name);
            if path.exists() {
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!("another keeper serves {name}"),
                    ));
                }
                std::fs::remove_file(&path)?;
            }
            let listener = UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { path, listener })
        }

        pub async fn accept(&mut self) -> std::io::Result<Stream> {
            let (stream, _) = self.listener.accept().await?;
            Ok(Box::new(stream))
        }

        pub fn remove(&self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Подключение к сокету, который слушает процесс своей учётной записи. На Linux сверяется
    /// и номер процесса сторожа (`SO_PEERCRED`); macOS его не называет, а сокет и так лежит в
    /// каталоге 0700.
    pub async fn connect(name: &str, keeper: u32) -> std::io::Result<Stream> {
        let stream = UnixStream::connect(name).await?;
        let credentials = stream.peer_cred()?;
        #[cfg(target_os = "linux")]
        if let Some(server) = credentials.pid() {
            if u32::try_from(server).ok() != Some(keeper) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("{name} is served by process {server}, not by the keeper {keeper}"),
                ));
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = keeper;
        let peer = credentials.uid();
        // SAFETY: `getuid` takes nothing and cannot fail.
        let own = unsafe { libc::getuid() };
        if peer != own {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{name} is served by uid {peer}, not by this account ({own})"),
            ));
        }
        Ok(Box::new(stream))
    }
}

pub use imp::{connect, Listener};

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn channel_name(dir: &std::path::Path) -> String {
        if cfg!(windows) {
            format!(
                r"\\.\pipe\v8-runner-agent-test-{}",
                uuid::Uuid::new_v4().simple()
            )
        } else {
            dir.join("k.sock").display().to_string()
        }
    }

    /// Канал открыт только своей учётной записи: на Windows — защищённый DACL без чужих
    /// записей, на Unix — сокет 0600 (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`).
    #[test]
    fn the_channel_is_open_to_its_own_account_only() {
        let dir = tempfile::tempdir().unwrap();
        let name = channel_name(dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut listener = Listener::bind(&name).unwrap();
            #[cfg(windows)]
            {
                let sddl = pipe_dacl(listener.handle());
                assert!(sddl.starts_with("D:P"), "protected DACL: {sddl}");
                for foreign in [";WD)", ";AU)", ";BU)", ";AN)", ";IU)", ";OW)", ";BA)"] {
                    assert!(!sddl.contains(foreign), "{foreign} in {sddl}");
                }
                let user = crate::support::machine::account::account_id().unwrap();
                assert!(sddl.contains(&format!(";;;{user})")), "{user} in {sddl}");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&name).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600);
            }
            let server = tokio::spawn(async move {
                let mut stream = listener.accept().await.unwrap();
                let mut buffer = [0u8; 4];
                stream.read_exact(&mut buffer).await.unwrap();
                stream.write_all(&buffer).await.unwrap();
                listener
            });
            let mut client = connect(&name, std::process::id()).await.unwrap();
            client.write_all(b"ping").await.unwrap();
            let mut answer = [0u8; 4];
            client.read_exact(&mut answer).await.unwrap();
            assert_eq!(&answer, b"ping");
            let listener = server.await.unwrap();
            #[cfg(windows)]
            assert!(
                matches!(
                    connect(&name, std::process::id() + 1)
                        .await
                        .err()
                        .map(|error| error.kind()),
                    Some(std::io::ErrorKind::PermissionDenied)
                ),
                "a channel served by another process is refused"
            );
            // После обмена: на Unix второй сторож пробует сокет подключением, и сервер,
            // принявший бы эту пробу первой, не дождался бы данных.
            assert_eq!(
                Listener::bind(&name).err().map(|error| error.kind()),
                Some(std::io::ErrorKind::AddrInUse),
                "a second keeper on the same channel is refused"
            );
            listener.remove();
        });
    }

    /// Ушедший клиент кончает чтение сервера: по этому сторож видит, что команда ушла.
    #[test]
    fn a_client_that_leaves_ends_the_server_read() {
        let dir = tempfile::tempdir().unwrap();
        let name = channel_name(dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut listener = Listener::bind(&name).unwrap();
            let server = tokio::spawn(async move {
                let stream = listener.accept().await.unwrap();
                let (read, _write) = tokio::io::split(stream);
                let mut reader = tokio::io::BufReader::new(read);
                let mut line = String::new();
                tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line)
                    .await
                    .unwrap();
                let mut probe = [0u8; 8];
                let read = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    reader.read(&mut probe),
                )
                .await;
                (line, read.map(|read| read.map_err(|error| error.kind())))
            });
            // Клиент — в своём потоке со своим рантаймом, как команда в своём процессе.
            let client_name = name.clone();
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    let stream = connect(&client_name, std::process::id()).await.unwrap();
                    let (_read, mut write) = tokio::io::split(stream);
                    write.write_all(b"hello\n").await.unwrap();
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                });
            })
            .join()
            .unwrap();
            let (line, read) = server.await.unwrap();
            assert_eq!(line, "hello\n");
            assert!(
                matches!(read, Ok(Ok(0) | Err(_))),
                "the read must end: {read:?}"
            );
        });
    }

    /// DACL канала строкой SDDL, как его видит владелец.
    #[cfg(windows)]
    fn pipe_dacl(handle: std::os::windows::io::RawHandle) -> String {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_KERNEL_OBJECT,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a live pipe handle; out pointers are locals; the descriptor is freed below.
        let status = unsafe {
            GetSecurityInfo(
                handle.cast(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, 0, "GetSecurityInfo");
        let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: `descriptor` is valid; `text` receives a LocalAlloc'd string.
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            converted, 0,
            "ConvertSecurityDescriptorToStringSecurityDescriptorW"
        );
        // SAFETY: `text` is a null-terminated UTF-16 string from the call above; both are freed once.
        unsafe {
            let length = (0..).take_while(|at| *text.add(*at) != 0).count();
            let sddl = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
            LocalFree(text.cast());
            LocalFree(descriptor);
            sddl
        }
    }
}
