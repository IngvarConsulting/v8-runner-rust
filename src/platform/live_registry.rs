//! The registry of live Designer agents of this OS account without a shared file (#735): the
//! keepers themselves are the registry. A keeper's record lies in its working copy
//! (`workPath/agent/live`); what the account shares is only the keepers' channels and a lock.
//!
//! - **Scope.** The account's registry is named by where its state directory lies
//!   (`support::machine::account`): `V8_RUNNER_STATE_DIR` gives each test its own registry. On
//!   Windows that directory is only named, never created.
//! - **Channels.** A keeper serves the Windows named pipe
//!   `\\.\pipe\v8-runner-keeper-<scope>-<key>` or the Unix socket `<state>/keepers/<key32>.sock`
//!   (the socket path is limited to ~104 bytes, so it stays in the short state directory; it holds
//!   no data). The live keepers of the account are the channels that exist now: on Windows they
//!   are listed by name, a busy keeper still counts, and a pipe served by another account (pipe
//!   names are shared by the machine) does not.
//! - **Lock.** [`Registry::lock`] is the account-wide lock under which a command counts keepers
//!   and starts one: on Windows a named pipe created as the first and only instance (it exists
//!   while held and goes with its holder's process), on Unix a file lock next to the sockets.
//! - **Former layout.** Keepers of 0.17.0–0.17.3 kept their records under `<state>/agents/<key>`
//!   and served `\\.\pipe\v8-runner-agent-<key>` or `<state>/agents/<key32>.sock`: they are still
//!   found there ([`legacy_agents_path`]) until they idle out.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::support::fs::{acquire_advisory_lock, AdvisoryLockGuard};
use crate::support::machine::account::ensure_private_dir;

/// Name prefix of a keeper's Windows pipe; the scope and the key follow.
#[cfg_attr(not(windows), allow(dead_code))]
const KEEPER_PIPE_PREFIX: &str = "v8-runner-keeper-";
/// Name prefix of the account lock's Windows pipe; the scope follows.
#[cfg_attr(not(windows), allow(dead_code))]
const LOCK_PIPE_PREFIX: &str = "v8-runner-registry-";
/// How long a command waits for the account lock before it refuses: the lock is held only to
/// count keepers, start one or ask the agents of one infobase, never across a command.
const LOCK_PATIENCE: Duration = Duration::from_secs(120);

/// The live-agent registry of this OS account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    state: PathBuf,
    scope: String,
}

/// A keeper's channel that exists now: the key it names (on Unix only its first 32 digits)
/// and the channel string a client connects to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedChannel {
    pub key: String,
    pub channel: String,
}

impl Registry {
    /// The registry of this OS account: named by where its state directory lies. Nothing is
    /// created.
    pub fn of_account() -> std::io::Result<Self> {
        Ok(Self::at(
            crate::support::machine::account::account_state_location()?,
        ))
    }

    /// The registry named by the state directory `state`.
    pub fn at(state: PathBuf) -> Self {
        let scope = scope_of(&state);
        Self { state, scope }
    }

    /// Where the account's state directory lies; it may not exist.
    pub fn state(&self) -> &Path {
        &self.state
    }

    /// The channel of the keeper of `key`.
    pub fn channel(&self, key: &str) -> String {
        if cfg!(windows) {
            format!(r"\\.\pipe\{KEEPER_PIPE_PREFIX}{}-{key}", self.scope)
        } else {
            let short = &key[..key.len().min(32)];
            self.keepers_dir()
                .join(format!("{short}.sock"))
                .display()
                .to_string()
        }
    }

    /// Where the sockets and the lock lie on Unix: `<state>/keepers`, created owner-only.
    /// Windows creates nothing there.
    pub fn prepare(&self) -> std::io::Result<()> {
        if cfg!(windows) {
            return Ok(());
        }
        ensure_private_dir(&self.state)?;
        ensure_private_dir(&self.keepers_dir())
    }

    fn keepers_dir(&self) -> PathBuf {
        self.state.join("keepers")
    }

    /// The keepers' channels of this registry that exist now. On Windows the pipes are listed
    /// by name; a pipe whose server is a process of another account (pipe names are shared by
    /// the whole machine) is not one of this account's keepers. A busy keeper still counts. On
    /// Unix a socket counts while it accepts a connection.
    pub fn channels(&self) -> Vec<ListedChannel> {
        #[cfg(windows)]
        {
            let prefix = format!("{KEEPER_PIPE_PREFIX}{}-", self.scope);
            pipe_names()
                .into_iter()
                .filter_map(|name| {
                    let key = name.strip_prefix(&prefix)?.to_owned();
                    let channel = format!(r"\\.\pipe\{name}");
                    (is_key(&key) && pipe_owner(&channel) != PipeOwner::Foreign)
                        .then_some(ListedChannel { channel, key })
                })
                .collect()
        }
        #[cfg(unix)]
        {
            sockets_in(&self.keepers_dir())
        }
        #[cfg(not(any(unix, windows)))]
        {
            Vec::new()
        }
    }

    /// Whether the channel of `key` exists now: on Windows by its name in the list of pipes
    /// (it is never opened), on Unix by its socket accepting a connection. The name holds the
    /// key, an HMAC with the working copy's salt: no other account can know it.
    pub fn has_channel(&self, key: &str) -> bool {
        let channel = self.channel(key);
        #[cfg(windows)]
        {
            let name = channel.trim_start_matches(r"\\.\pipe\");
            pipe_names().iter().any(|listed| listed == name)
        }
        #[cfg(unix)]
        {
            socket_answers(Path::new(&channel))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = channel;
            false
        }
    }

    /// Takes the account lock, waiting for another holder up to [`LOCK_PATIENCE`]. The lock is
    /// not reentrant: callers hold it only to count keepers, start one or ask the agents of one
    /// infobase, never across a command, and take a keeper's home lock only after it.
    pub fn lock(&self) -> std::io::Result<RegistryLock> {
        #[cfg(windows)]
        {
            let name = format!(r"\\.\pipe\{LOCK_PIPE_PREFIX}{}", self.scope);
            let deadline = Instant::now() + LOCK_PATIENCE;
            let mut owner_checked = false;
            loop {
                if let Some(handle) = pipe_lock::try_take(&name)? {
                    return Ok(RegistryLock { _held: handle });
                }
                // Имена каналов общие на машину: канал замка, занятый процессом другой учётной
                // записи, не освободится — отказ сразу, а не после срока.
                if !owner_checked {
                    owner_checked = true;
                    if pipe_owner(&name) == PipeOwner::Foreign {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            format!(
                                "{name}, the live agent registry lock of this account, is held \
                                 by a process of another account"
                            ),
                        ));
                    }
                }
                if Instant::now() >= deadline {
                    return Err(lock_timed_out());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        #[cfg(not(windows))]
        {
            self.prepare()?;
            let path = self.keepers_dir().join("registry.lock");
            let deadline = Instant::now() + LOCK_PATIENCE;
            loop {
                match crate::support::fs::try_acquire_advisory_lock(&path) {
                    Ok(guard) => return Ok(RegistryLock { _held: guard }),
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        return Err(lock_timed_out())
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
}

fn lock_timed_out() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!(
            "the live agent registry of this OS account stayed locked for {} s",
            LOCK_PATIENCE.as_secs()
        ),
    )
}

/// The account lock, held until dropped.
pub struct RegistryLock {
    #[cfg(windows)]
    _held: std::os::windows::io::OwnedHandle,
    #[cfg(not(windows))]
    _held: AdvisoryLockGuard,
}

/// The registry's scope: the first 8 bytes of SHA-256 over the state location as one spelling
/// (lower case on Windows, without a trailing separator), so two spellings of one directory
/// share a registry.
fn scope_of(state: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut spelled = state.display().to_string();
    while spelled.len() > 1 && (spelled.ends_with('/') || spelled.ends_with('\\')) {
        spelled.pop();
    }
    if cfg!(windows) {
        spelled = spelled.replace('/', "\\").to_lowercase();
    }
    Sha256::digest(spelled.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A live-agent key as the runner writes it: 16 to 64 hexadecimal digits.
pub fn is_key(key: &str) -> bool {
    (16..=64).contains(&key.len()) && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Names of the Windows named pipes that exist now, without the `\\.\pipe\` prefix.
#[cfg(windows)]
fn pipe_names() -> Vec<String> {
    match std::fs::read_dir(r"\\.\pipe\") {
        Ok(entries) => entries
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "the named pipes of this machine cannot be listed");
            Vec::new()
        }
    }
}

/// Sockets in `dir` whose keeper answers.
#[cfg(unix)]
fn sockets_in(dir: &Path) -> Vec<ListedChannel> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let key = path
                .file_name()?
                .to_str()?
                .strip_suffix(".sock")?
                .to_owned();
            (is_key(&key) && socket_answers(&path)).then(|| ListedChannel {
                key,
                channel: path.display().to_string(),
            })
        })
        .collect()
}

/// Whether a keeper listens on the socket `path`. A socket that refuses a connection was left
/// by a keeper that is gone; the file stays until the next keeper of that key binds over it
/// (removing it here could race that bind). Another refusal (a full backlog) may be a live
/// keeper, and it counts.
#[cfg(unix)]
fn socket_answers(path: &Path) -> bool {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => true,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            false
        }
        Err(_) => true,
    }
}

/// Whose process serves a Windows named pipe.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipeOwner {
    /// A process of this account.
    Own,
    /// A process of another account, or a pipe whose DACL keeps this account out (this
    /// account's pipes always let it in).
    Foreign,
    /// Not known: every instance is busy, the pipe is gone, or its server cannot be asked.
    Unknown,
}

/// Who serves the pipe `name`: the pipe is opened for a moment and closed, nothing is sent.
#[cfg(windows)]
fn pipe_owner(name: &str) -> PipeOwner {
    use std::os::windows::io::AsRawHandle;
    const ERROR_ACCESS_DENIED: i32 = 5;
    let opened = std::fs::OpenOptions::new().write(true).open(name);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
            return PipeOwner::Foreign
        }
        Err(_) => return PipeOwner::Unknown,
    };
    let mut server = 0u32;
    // SAFETY: a live pipe handle owned by `file`; `server` outlives the call.
    let named = unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(
            file.as_raw_handle().cast(),
            &mut server,
        )
    };
    if named == 0 {
        return PipeOwner::Unknown;
    }
    match crate::support::machine::account::runs_as_this_account(server) {
        Ok(true) => PipeOwner::Own,
        Ok(false) => PipeOwner::Foreign,
        Err(_) => PipeOwner::Unknown,
    }
}

/// The account lock on Windows: a named pipe made as the first and only instance. Making it
/// fails while another holder has it; it goes away with its last handle, so a holder that dies
/// releases it.
#[cfg(windows)]
mod pipe_lock {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND,
    };
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    };

    /// The lock, if it is free; `None` — another holder has it.
    pub(super) fn try_take(name: &str) -> std::io::Result<Option<OwnedHandle>> {
        const ERROR_ACCESS_DENIED: i32 = 5;
        const ERROR_PIPE_BUSY: i32 = 231;
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let sddl = super::own_pipe_sddl()?;
        let raw = crate::support::temp::with_security(&sddl, |attributes| {
            // SAFETY: `wide` is a null-terminated UTF-16 name and `attributes` a live
            // SECURITY_ATTRIBUTES for this call; the call keeps neither.
            unsafe {
                CreateNamedPipeW(
                    wide.as_ptr(),
                    PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    0,
                    0,
                    0,
                    std::ptr::from_ref(attributes),
                )
            }
        })?;
        if raw == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(ERROR_ACCESS_DENIED | ERROR_PIPE_BUSY) => Ok(None),
                _ => Err(error),
            };
        }
        // SAFETY: the call succeeded and handed over this handle; the guard closes it once.
        Ok(Some(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }
}

/// DACL of the account's pipes: protected, full access only to the user of the process token
/// and LocalSystem. The object owner (`OW`) does not do: for an elevated administrator it is
/// the Administrators group.
#[cfg(windows)]
pub(crate) fn own_pipe_sddl() -> std::io::Result<String> {
    let user = crate::support::machine::account::account_id()?;
    Ok(format!("D:P(A;;GA;;;{user})(A;;GA;;;SY)"))
}

/// Where the keepers of 0.17.0–0.17.3 kept their records: `<state>/agents`. Read only: nothing
/// is created there any more.
pub fn legacy_agents_path(state: &Path) -> PathBuf {
    state.join("agents")
}

/// The channel of a keeper of 0.17.0–0.17.3 with key `key`.
pub fn legacy_channel(state: &Path, key: &str) -> String {
    if cfg!(windows) {
        format!(r"\\.\pipe\v8-runner-agent-{key}")
    } else {
        let short = &key[..key.len().min(32)];
        legacy_agents_path(state)
            .join(format!("{short}.sock"))
            .display()
            .to_string()
    }
}

/// Takes the lock of 0.17.0–0.17.3, `<state>/agents/registry.lock`, if that directory is still
/// there: the sweep of their leftovers works under it, as their commands did.
pub fn lock_legacy_registry(state: &Path) -> Option<std::io::Result<AdvisoryLockGuard>> {
    let dir = legacy_agents_path(state);
    dir.is_dir()
        .then(|| acquire_advisory_lock(&dir.join("registry.lock")))
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;

    /// The account lock has one holder at a time, and a holder that lets go frees it.
    #[test]
    fn the_registry_lock_is_held_by_one_holder_at_a_time() {
        let state = tempfile::tempdir().unwrap();
        let registry = Registry::at(state.path().join("state"));
        let held = registry.lock().unwrap();
        let (tried, answer) = std::sync::mpsc::channel();
        let (trying, started) = std::sync::mpsc::channel();
        let other = registry.clone();
        std::thread::spawn(move || {
            let _ = trying.send(());
            let taken = other.lock().map(drop);
            let _ = tried.send(taken.map_err(|error| error.to_string()));
        });
        started
            .recv_timeout(Duration::from_secs(30))
            .expect("the second holder started");
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            answer.try_recv().is_err(),
            "a second holder takes the lock while the first holds it"
        );
        drop(held);
        answer
            .recv_timeout(Duration::from_secs(30))
            .expect("the second holder never got the lock")
            .unwrap();
        if cfg!(windows) {
            assert!(
                !state.path().join("state").exists(),
                "the account lock on Windows leaves no file"
            );
        }
    }

    /// Two spellings of one state directory share a registry; another directory has its own.
    #[test]
    fn the_scope_follows_the_state_location() {
        let one = Registry::at(PathBuf::from("/var/state/v8-runner"));
        let slash = Registry::at(PathBuf::from("/var/state/v8-runner/"));
        let other = Registry::at(PathBuf::from("/var/state/other"));
        assert_eq!(
            one.channel(&"ab".repeat(32)),
            slash.channel(&"ab".repeat(32))
        );
        assert_ne!(
            one.channel(&"ab".repeat(32)),
            other.channel(&"ab".repeat(32))
        );
        if cfg!(windows) {
            let upper = Registry::at(PathBuf::from(r"C:\Users\U\AppData\Local\v8-runner"));
            let lower = Registry::at(PathBuf::from(r"c:\users\u\appdata\local\v8-runner"));
            assert_eq!(upper.scope, lower.scope);
        }
    }

    /// A keeper's channel is listed while it exists and gone with it; on Windows it is found by
    /// name without being opened.
    #[test]
    fn a_listed_channel_lives_as_long_as_its_keeper() {
        // Путь сокета короток: временный каталог macOS для него слишком длинен.
        let state = if cfg!(unix) {
            tempfile::tempdir_in("/tmp").unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let registry = Registry::at(state.path().to_path_buf());
        registry.prepare().unwrap();
        let key = uuid::Uuid::new_v4().simple().to_string();
        assert!(!registry.has_channel(&key));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener =
                crate::platform::agent_keeper::bind_for_test(&registry.channel(&key)).unwrap();
            assert!(registry.has_channel(&key));
            assert!(registry
                .channels()
                .iter()
                .any(|listed| listed.key.starts_with(&key[..32.min(key.len())])));
            drop(listener);
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while registry.has_channel(&key) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !registry.has_channel(&key),
            "the channel outlived its keeper"
        );
    }
}
