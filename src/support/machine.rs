//! What this machine can tell about itself: its identifier, its host name, whether one
//! of its processes still runs, and the OS account the runner runs as with that account's
//! state directory.

/// Whether a process with `pid` exists on this machine. A process that exists but may not
/// be inspected counts as alive; on Unix an unreaped zombie counts as alive too, so a
/// caller that waits for its own child to be reaped sees the difference. Whether the
/// process still runs is [`is_process_running`].
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        // 0 and negative values address process groups, not one process.
        return false;
    }
    // SAFETY: signal 0 sends nothing; `kill` only checks that the process exists.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether a process with `pid` runs on this machine. A process that exists but may not
/// be opened counts as alive.
#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    // SAFETY: plain call with value arguments; a null result reports failure.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return std::io::Error::last_os_error().raw_os_error()
            == i32::try_from(ERROR_ACCESS_DENIED).ok();
    }
    // SAFETY: `OpenProcess` succeeded and handed over this handle; the guard closes it once.
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: the handle stays open for the call; a zero timeout only polls the state.
    // Only a signalled handle means the process has ended: a failed wait (`WAIT_FAILED`)
    // tells nothing, and refusing is the safe side for every caller.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) != WAIT_OBJECT_0 }
}

/// Without a way to ask, every process is taken as alive: callers use the answer to
/// decide whether something may be taken over, and refusing is the safe side.
#[cfg(not(any(unix, windows)))]
pub fn is_process_alive(_pid: u32) -> bool {
    true
}

/// Whether a process with `pid` still runs on this machine: like [`is_process_alive`],
/// except that on Linux a process that has ended but is not yet reaped by its parent
/// (state `Z` or `X` in `/proc/<pid>/stat`) does not run. A runner killed with `kill -9`
/// stays such a zombie until its parent waits for it. Other Unix systems give no portable
/// way to tell, so there a zombie still counts as running.
pub fn is_process_running(pid: u32) -> bool {
    is_process_alive(pid) && !has_ended_unreaped(pid)
}

/// Whether `/proc` shows the process as ended and waiting to be reaped. An unreadable
/// entry (the process is gone, or `/proc` hides other users' processes) answers `false`
/// and leaves the answer to [`is_process_alive`].
#[cfg(target_os = "linux")]
fn has_ended_unreaped(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| matches!(proc_stat_field(&stat, 3), Some("Z" | "X")))
}

#[cfg(not(target_os = "linux"))]
fn has_ended_unreaped(_pid: u32) -> bool {
    false
}

/// Field `field` of a `/proc/<pid>/stat` line, numbered as in `proc(5)`: 3 is the state,
/// 22 the start time in clock ticks after boot. Fields from 3 on follow the process name,
/// which is enclosed in parentheses and may itself hold parentheses and spaces, so the
/// name ends at the last `)`. The only reader of that line in the runner.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn proc_stat_field(stat: &str, field: usize) -> Option<&str> {
    let (_pid_and_name, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().nth(field.checked_sub(3)?)
}

/// Name of this machine, if the system gives one.
#[cfg(unix)]
pub fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the pointer and length describe `buffer`, which outlives the call.
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result != 0 {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0)?;
    let name = String::from_utf8_lossy(&buffer[..end]).into_owned();
    (!name.is_empty()).then_some(name)
}

/// Name of this machine, if the system gives one: its DNS host name, asked from the
/// system rather than from the environment, which a process may change.
#[cfg(windows)]
pub fn host_name() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNamePhysicalDnsHostname, GetComputerNameExW,
    };

    let mut length = 0u32;
    // SAFETY: a null buffer with zero length only asks for the needed length, which the
    // call stores in `length`, a local that outlives the call.
    unsafe {
        GetComputerNameExW(
            ComputerNamePhysicalDnsHostname,
            std::ptr::null_mut(),
            &mut length,
        );
    }
    if length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; usize::try_from(length).ok()?];
    // SAFETY: the pointer and `length` describe `buffer`, which outlives the call; on
    // success `length` holds the number of characters written, without the final null.
    let written = unsafe {
        GetComputerNameExW(
            ComputerNamePhysicalDnsHostname,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if written == 0 {
        return None;
    }
    buffer.truncate(usize::try_from(length).ok()?);
    let name = String::from_utf16_lossy(&buffer);
    (!name.is_empty()).then_some(name)
}

#[cfg(not(any(unix, windows)))]
pub fn host_name() -> Option<String> {
    None
}

/// Identifier of this machine that the system keeps across a host rename: `machine-id` on
/// Linux, the hardware UUID on macOS, `MachineGuid` on Windows. `None` when the system
/// gives none. Callers keep it hashed: `machine-id(5)` asks not to show the raw value.
pub fn machine_id() -> Option<String> {
    system_machine_id()
}

/// Hash of a machine identifier: what the runner keeps on disk instead of the value itself.
pub fn machine_hash(identity: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("v8-runner/owner/{identity}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A `machine-id` file's value, if it holds one: 32 hexadecimal digits. An empty file and
/// `uninitialized` — a system still in its first boot — hold none.
#[cfg(any(target_os = "linux", test))]
fn linux_machine_id(text: &str) -> Option<String> {
    let id = text.trim();
    (id.len() == 32 && id.chars().all(|ch| ch.is_ascii_hexdigit())).then(|| id.to_ascii_lowercase())
}

#[cfg(target_os = "linux")]
fn system_machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .into_iter()
        .find_map(|path| linux_machine_id(&std::fs::read_to_string(path).ok()?))
}

#[cfg(target_vendor = "apple")]
fn system_machine_id() -> Option<String> {
    let mut uuid = [0u8; 16];
    let timeout = libc::timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    // SAFETY: `uuid` is the 16-byte buffer the call fills and `timeout` a valid timespec;
    // both outlive the call.
    if unsafe { libc::gethostuuid(uuid.as_mut_ptr(), &timeout) } != 0 {
        return None;
    }
    // A zeroed UUID is the answer of a machine that has none.
    (uuid != [0u8; 16]).then(|| uuid.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
fn system_machine_id() -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
    };

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide("SOFTWARE\\Microsoft\\Cryptography");
    let value = wide("MachineGuid");
    let mut buffer = [0u16; 64];
    let mut size = u32::try_from(std::mem::size_of_val(&buffer)).ok()?;
    // SAFETY: the key and value names are null-terminated UTF-16 strings, and `buffer`
    // with `size` in bytes describes writable memory; all outlive the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let written = usize::try_from(size).ok()? / std::mem::size_of::<u16>();
    let text = &buffer[..written.min(buffer.len())];
    let end = text
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len());
    let guid = String::from_utf16_lossy(&text[..end])
        .trim()
        .to_ascii_lowercase();
    (!guid.is_empty()).then_some(guid)
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
fn system_machine_id() -> Option<String> {
    None
}

/// The OS account the runner runs as and its state directory: where records live that
/// outlive a command and belong to no single working copy, such as live Designer agents.
/// What those records are and how they are locked belongs to their owners in `platform`.
pub mod account {
    use std::path::{Path, PathBuf};

    /// Overrides the account's state directory: tests give each run its own.
    pub const STATE_DIR_ENV: &str = "V8_RUNNER_STATE_DIR";

    /// Identifier of the OS account this process runs as: the SID string on Windows, the
    /// numeric uid on Unix. It never changes while the account exists, unlike its name.
    #[cfg(unix)]
    pub fn account_id() -> std::io::Result<String> {
        // SAFETY: `getuid` takes nothing and cannot fail.
        Ok(unsafe { libc::getuid() }.to_string())
    }

    /// Identifier of the OS account this process runs as: the SID string on Windows, the
    /// numeric uid on Unix. It never changes while the account exists, unlike its name.
    #[cfg(windows)]
    pub fn account_id() -> std::io::Result<String> {
        windows_token::sid_of(windows_token::Which::User)
    }

    /// Whether process `pid` runs as this OS account. A process whose token the account may
    /// not query cannot be checked: that is an error, not another account. `NotFound` — there
    /// is no such process.
    #[cfg(windows)]
    pub fn runs_as_this_account(pid: u32) -> std::io::Result<bool> {
        Ok(windows_token::user_sid_of_process(pid)? == account_id()?)
    }

    /// Whether process `pid` runs as this OS account: by the owner of `/proc/<pid>` on Linux,
    /// elsewhere by whether this account may signal it. Root may signal any process, so there
    /// the answer outside Linux is always yes. `NotFound` — there is no such process.
    #[cfg(unix)]
    pub fn runs_as_this_account(pid: u32) -> std::io::Result<bool> {
        // SAFETY: `getuid` takes nothing and cannot fail.
        let own = unsafe { libc::getuid() };
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt as _;
            let owner = std::fs::metadata(format!("/proc/{pid}"))?.uid();
            Ok(owner == own)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = own;
            let pid = libc::pid_t::try_from(pid).map_err(std::io::Error::other)?;
            // SAFETY: signal 0 only checks that the process exists and may be signalled.
            if unsafe { libc::kill(pid, 0) } == 0 {
                return Ok(true);
            }
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EPERM) => Ok(false),
                Some(libc::ESRCH) => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no process {pid}"),
                )),
                _ => Err(error),
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    pub fn runs_as_this_account(_pid: u32) -> std::io::Result<bool> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this system names no account",
        ))
    }

    /// The SIDs this process's token names: its user and the default owner of what it creates.
    /// An elevated administrator's token makes the Administrators group that owner.
    #[cfg(windows)]
    mod windows_token {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{LocalFree, HANDLE};
        use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenOwner, TokenUser, PSID, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER,
        };
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        #[derive(Clone, Copy)]
        pub(super) enum Which {
            User,
            Owner,
        }

        /// The SID string of the token's user or default owner.
        pub(super) fn sid_of(which: Which) -> std::io::Result<String> {
            // SAFETY: the pseudo handle of this process needs no closing.
            sid_of_process(unsafe { GetCurrentProcess() }, which)
        }

        /// The SID string of the user of process `pid`.
        pub(super) fn user_sid_of_process(pid: u32) -> std::io::Result<String> {
            // SAFETY: plain values; the returned handle is checked and owned below.
            let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if raw.is_null() {
                let error = std::io::Error::last_os_error();
                // ERROR_INVALID_PARAMETER: there is no process with this id.
                return Err(if error.raw_os_error() == Some(87) {
                    std::io::Error::new(std::io::ErrorKind::NotFound, format!("no process {pid}"))
                } else {
                    error
                });
            }
            // SAFETY: the call succeeded and handed over this handle; the guard closes it once.
            let process = unsafe { OwnedHandle::from_raw_handle(raw) };
            sid_of_process(process.as_raw_handle(), Which::User)
        }

        /// The SID string of the user or default owner of the token of `process`, a handle
        /// that stays open for the call.
        fn sid_of_process(process: HANDLE, which: Which) -> std::io::Result<String> {
            let class = match which {
                Which::User => TokenUser,
                Which::Owner => TokenOwner,
            };
            let mut raw: HANDLE = std::ptr::null_mut();
            // SAFETY: the caller keeps `process` open; `raw` outlives the call.
            if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: the call succeeded and handed over this handle; the guard closes it once.
            let token = unsafe { OwnedHandle::from_raw_handle(raw) };
            let mut size = 0u32;
            // SAFETY: a null buffer with zero length only asks for the needed length.
            unsafe {
                GetTokenInformation(
                    token.as_raw_handle(),
                    class,
                    std::ptr::null_mut(),
                    0,
                    &mut size,
                )
            };
            if size == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let bytes = usize::try_from(size).map_err(std::io::Error::other)?;
            // `u64` elements keep the buffer aligned for the structure and the SID after it.
            let mut buffer = vec![0u64; bytes.div_ceil(8)];
            // SAFETY: the buffer holds at least `size` bytes and outlives the call.
            if unsafe {
                GetTokenInformation(
                    token.as_raw_handle(),
                    class,
                    buffer.as_mut_ptr().cast(),
                    size,
                    &mut size,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let sid = match which {
                // SAFETY: the call above wrote a `TOKEN_USER` at the start of the aligned
                // buffer; its SID lives in the same buffer.
                Which::User => unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid },
                // SAFETY: as above, for `TOKEN_OWNER`.
                Which::Owner => unsafe { (*buffer.as_ptr().cast::<TOKEN_OWNER>()).Owner },
            };
            sid_string(sid)
        }

        /// The string form (`S-1-…`) of a SID that stays valid for the call.
        pub(super) fn sid_string(sid: PSID) -> std::io::Result<String> {
            let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
            // SAFETY: the caller keeps `sid` alive; on success `text` holds a string to
            // `LocalFree`.
            if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: `text` is a null-terminated UTF-16 string allocated by the call above.
            let sid = unsafe {
                let length = (0..).take_while(|at| *text.add(*at) != 0).count();
                let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
                LocalFree(text.cast());
                sid
            };
            Ok(sid)
        }
    }

    #[cfg(not(any(unix, windows)))]
    pub fn account_id() -> std::io::Result<String> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this system names no account",
        ))
    }

    /// Where the account's state directory lies, from `V8_RUNNER_STATE_DIR` or else from the
    /// system: `%LOCALAPPDATA%\v8-runner` on Windows, `$XDG_STATE_HOME/v8-runner` or
    /// `~/.local/state/v8-runner` on Unix. A relative override is an error: the caller named
    /// a directory and the runner would otherwise keep its state somewhere else. Relative
    /// system values are ignored, as the XDG specification asks.
    pub(super) fn state_dir_location(
        env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    ) -> std::io::Result<PathBuf> {
        if let Some(dir) = state_dir_override(env)? {
            return Ok(dir);
        }
        let system = if cfg!(windows) {
            absolute_env(env, "LOCALAPPDATA").map(|dir| dir.join("v8-runner"))
        } else {
            absolute_env(env, "XDG_STATE_HOME")
                .or_else(|| absolute_env(env, "HOME").map(|home| home.join(".local").join("state")))
                .map(|dir| dir.join("v8-runner"))
        };
        system.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "the system names no state directory for this account; set {STATE_DIR_ENV}"
                ),
            )
        })
    }

    /// The directory `V8_RUNNER_STATE_DIR` names, if set; a relative one is refused.
    fn state_dir_override(
        env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    ) -> std::io::Result<Option<PathBuf>> {
        let Some(dir) = env(STATE_DIR_ENV).map(PathBuf::from) else {
            return Ok(None);
        };
        if dir.is_absolute() {
            return Ok(Some(dir));
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{STATE_DIR_ENV}='{}' is not an absolute path; give an absolute one or unset it",
                dir.display()
            ),
        ))
    }

    /// An absolute path from the environment variable `name`; a relative one names no
    /// location and is dropped, not resolved against the current directory.
    fn absolute_env(
        env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
        name: &str,
    ) -> Option<PathBuf> {
        env(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }

    /// The state directory of this OS account, created owner-only when missing: records that
    /// outlive a command and do not belong to one working copy. An existing one must belong
    /// to the account; on Unix it is narrowed to 0700.
    pub fn account_state_dir() -> std::io::Result<PathBuf> {
        let dir = state_dir_location(&|name| std::env::var_os(name))?;
        ensure_private_dir(&dir)?;
        Ok(dir)
    }

    /// The state directory of this OS account if it already exists: a reader that only looks
    /// for records does not create it.
    pub fn existing_account_state_dir() -> Option<PathBuf> {
        state_dir_location(&|name| std::env::var_os(name))
            .ok()
            .filter(|dir| dir.is_dir())
    }

    /// Creates `dir` owner-only, with its parents as usual. An existing one is kept only when
    /// it is a directory of this account, not a link: on Unix its uid is the account's and its
    /// mode is narrowed to 0700 through the opened directory itself; on Windows its owner is
    /// the token's user or the default owner it gives what it creates.
    pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match crate::support::temp::create_private_dir(dir) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let foreign = || {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("'{}' is not a directory of this account", dir.display()),
            )
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
            // One descriptor answers both questions and takes the change, so a link swapped
            // in between cannot redirect `chmod` elsewhere.
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(dir)
                .map_err(|error| match error.raw_os_error() {
                    Some(libc::ELOOP | libc::ENOTDIR) => foreign(),
                    _ => error,
                })?;
            let metadata = opened.metadata()?;
            // SAFETY: `getuid` takes nothing and cannot fail.
            if metadata.uid() != unsafe { libc::getuid() } {
                return Err(foreign());
            }
            if metadata.mode() & 0o777 != 0o700 {
                opened.set_permissions(std::fs::Permissions::from_mode(0o700))?;
            }
        }
        #[cfg(windows)]
        {
            let metadata = std::fs::symlink_metadata(dir)?;
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err(foreign());
            }
            let owner = windows_owner_of(dir)?;
            if owner != windows_token::sid_of(windows_token::Which::User)?
                && owner != windows_token::sid_of(windows_token::Which::Owner)?
            {
                return Err(foreign());
            }
        }
        #[cfg(not(any(unix, windows)))]
        if !dir.is_dir() {
            return Err(foreign());
        }
        Ok(())
    }

    /// The SID string of the owner of the file or directory at `path`.
    #[cfg(windows)]
    fn windows_owner_of(path: &Path) -> std::io::Result<String> {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        };

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut owner: PSID = std::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `wide` is a null-terminated UTF-16 path; the out pointers are locals that
        // outlive the call. On success `owner` points into `descriptor`, freed below.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(
                i32::try_from(status).unwrap_or(i32::MAX),
            ));
        }
        let sid = windows_token::sid_string(owner);
        // SAFETY: the descriptor was allocated by the call above and is freed once; `owner`
        // is not used after this.
        unsafe { LocalFree(descriptor) };
        sid
    }

    /// Length of the account's salt in bytes.
    const SALT_LEN: usize = 32;

    /// Name of the salt file in the state directory.
    const SALT_FILE: &str = "account-salt";

    /// The account's salt in `<state>/account-salt`, made on first use. No lock is taken, so
    /// a caller may hold any: each racer writes its own candidate, one publishes it without
    /// replacing another's, and the rest read what was published. The file is readable by
    /// the account only: the state directory is owner-only, and on Unix the file is 0600.
    pub fn account_salt(state: &Path) -> std::io::Result<[u8; SALT_LEN]> {
        let path = state.join(SALT_FILE);
        if let Some(salt) = read_salt(&path)? {
            return Ok(salt);
        }
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(std::io::Error::other)?;
        if crate::support::fs::create_file_once(&path, |file| {
            std::io::Write::write_all(file, &salt)
        })? {
            return Ok(salt);
        }
        read_salt(&path)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("'{}' vanished right after it was made", path.display()),
            )
        })
    }

    /// The salt in `path`; `None` when there is none yet. A file of another length is not a
    /// salt the runner wrote, and the runner refuses rather than replacing it.
    fn read_salt(path: &Path) -> std::io::Result<Option<[u8; SALT_LEN]>> {
        match std::fs::read(path) {
            Ok(bytes) => <[u8; SALT_LEN]>::try_from(bytes.as_slice())
                .map(Some)
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "'{}' holds {} bytes, not the {SALT_LEN}-byte salt the runner \
                             writes; delete the file and the runner makes a new one (keys of \
                             records kept so far change with it)",
                            path.display(),
                            bytes.len()
                        ),
                    )
                }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Key of a record kept for this account: HMAC-SHA256 with the account's salt over the
    /// account id and `parts`, as lowercase hex. Each part goes with its length, so no two
    /// different tuples give the same message. The key names a record without holding its
    /// parts: a connection string may carry a password.
    pub fn account_key(state: &Path, parts: &[&str]) -> std::io::Result<String> {
        key_with(&account_salt(state)?, parts)
    }

    /// [`account_key`] only when the account already has its salt: `None` without one, and
    /// nothing is written. A key that needs no salt yet names no record either.
    pub fn existing_account_key(state: &Path, parts: &[&str]) -> std::io::Result<Option<String>> {
        match read_salt(&state.join(SALT_FILE))? {
            Some(salt) => key_with(&salt, parts).map(Some),
            None => Ok(None),
        }
    }

    fn key_with(salt: &[u8; SALT_LEN], parts: &[&str]) -> std::io::Result<String> {
        let account = account_id()?;
        let mut message = Vec::new();
        for part in std::iter::once(account.as_str()).chain(parts.iter().copied()) {
            let length = u64::try_from(part.len()).map_err(std::io::Error::other)?;
            message.extend_from_slice(&length.to_be_bytes());
            message.extend_from_slice(part.as_bytes());
        }
        Ok(hmac_sha256(salt, &message)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    /// HMAC-SHA256 (RFC 2104) over `sha2`, which the runner already carries.
    pub(super) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        const BLOCK: usize = 64;
        let mut block = [0u8; BLOCK];
        if key.len() > BLOCK {
            block[..32].copy_from_slice(&Sha256::digest(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let pad = |byte: u8| block.map(|at| at ^ byte);
        let inner = Sha256::new()
            .chain_update(pad(0x36))
            .chain_update(message)
            .finalize();
        Sha256::new()
            .chain_update(pad(0x5c))
            .chain_update(inner)
            .finalize()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::account::{
        account_id, account_key, account_salt, ensure_private_dir, hmac_sha256, state_dir_location,
        STATE_DIR_ENV,
    };
    use super::{
        host_name, is_process_alive, is_process_running, linux_machine_id, machine_id,
        proc_stat_field,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn this_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    /// Свой процесс — этой учётной записи; процесс другой учётной записи своим не назван
    /// никогда: либо «чужой», либо «не проверить» — держателем живого агента он не станет
    /// (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
    #[cfg(any(unix, windows))]
    #[test]
    fn a_process_of_another_account_is_told_apart() {
        use super::account::runs_as_this_account;
        assert!(runs_as_this_account(std::process::id()).unwrap());
        let Some(foreign) = a_process_of_another_account() else {
            eprintln!("skipped: no process of another OS account is visible here");
            return;
        };
        let answer = runs_as_this_account(foreign);
        assert!(
            !matches!(answer, Ok(true)),
            "process {foreign} of another account is called own: {answer:?}"
        );
    }

    /// Процесс другой учётной записи, который видно наверняка: на Linux — по владельцу
    /// `/proc/<pid>`, на прочих Unix — `launchd`/`init` у не-root, на Windows — процесс System у
    /// всех, кроме LocalSystem.
    #[cfg(target_os = "linux")]
    fn a_process_of_another_account() -> Option<u32> {
        use std::os::unix::fs::MetadataExt as _;
        // SAFETY: `getuid` takes nothing and cannot fail.
        let own = unsafe { libc::getuid() };
        std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .find_map(|entry| {
                let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
                (entry.metadata().ok()?.uid() != own).then_some(pid)
            })
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn a_process_of_another_account() -> Option<u32> {
        // SAFETY: `getuid` takes nothing and cannot fail.
        (unsafe { libc::getuid() } != 0).then_some(1)
    }

    #[cfg(windows)]
    fn a_process_of_another_account() -> Option<u32> {
        (account_id().ok()? != "S-1-5-18").then_some(4)
    }

    #[cfg(unix)]
    #[test]
    fn a_reaped_child_is_not_alive() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn child");
        let pid = child.id();
        child.wait().expect("reap child");

        assert!(!is_process_alive(pid));
    }

    #[test]
    fn this_process_is_running() {
        assert!(is_process_running(std::process::id()));
    }

    /// A runner killed with `kill -9` stays a zombie until its parent waits for it, and
    /// the record it left must already count as left by a stopped owner.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_killed_unreaped_child_is_not_running() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn child");
        let pid = child.id();
        child.kill().expect("kill child");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while is_process_running(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let exists = is_process_alive(pid);
        let running = is_process_running(pid);
        child.wait().expect("reap child");

        assert!(exists, "the child is not reaped yet");
        assert!(!running, "a killed child waiting to be reaped does not run");
    }

    #[test]
    fn the_state_follows_the_last_parenthesis_of_the_name() {
        assert_eq!(proc_stat_field("42 (a) b (c)) Z 1 42", 3), Some("Z"));
        assert_eq!(proc_stat_field("42 (sleep) S 1 42", 3), Some("S"));
        assert_eq!(proc_stat_field("42 (sleep) S 1 42", 5), Some("42"));
        assert_eq!(proc_stat_field("42 (sleep) S 1 42", 6), None);
        assert_eq!(proc_stat_field("42 (sleep) S 1 42", 2), None);
        assert_eq!(proc_stat_field("42 sleep", 3), None);
    }

    #[test]
    fn a_pid_beyond_any_process_is_not_alive() {
        assert!(!is_process_alive(i32::MAX as u32));
    }

    #[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
    #[test]
    fn this_machine_keeps_one_identifier() {
        // A host without a valid `machine-id` (an empty file in a container) has none, and
        // that is an answer too; whatever the answer, it does not change between calls.
        assert_eq!(machine_id(), machine_id());
    }

    #[test]
    fn a_machine_id_file_holds_32_hex_digits_or_nothing() {
        assert_eq!(
            linux_machine_id("4C2F8E0A9B7D41D6A1F3C5E7D9B2A4C6\n").as_deref(),
            Some("4c2f8e0a9b7d41d6a1f3c5e7d9b2a4c6")
        );
        assert_eq!(linux_machine_id(""), None);
        assert_eq!(linux_machine_id("\n"), None);
        assert_eq!(linux_machine_id("uninitialized\n"), None);
        assert_eq!(linux_machine_id("4c2f8e0a9b7d41d6a1f3c5e7d9b2a4"), None);
        assert_eq!(linux_machine_id("zz2f8e0a9b7d41d6a1f3c5e7d9b2a4c6"), None);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn this_machine_has_a_name() {
        assert!(host_name().is_some());
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        let hex =
            |bytes: [u8; 32]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
        assert_eq!(
            hex(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // A key longer than the block is hashed first (test case 6).
        assert_eq!(
            hex(hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn the_state_directory_comes_from_the_override_then_from_the_system() {
        let root = std::env::temp_dir();
        let given = |pairs: Vec<(&'static str, PathBuf)>| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        let own = root.join("own");
        assert_eq!(
            state_dir_location(&given(vec![
                (STATE_DIR_ENV, own.clone()),
                ("LOCALAPPDATA", root.join("local")),
                ("XDG_STATE_HOME", root.join("xdg")),
            ]))
            .unwrap(),
            own
        );
        // A relative override is refused, not replaced by the system's directory.
        let refused = state_dir_location(&given(vec![
            (STATE_DIR_ENV, PathBuf::from("relative")),
            ("LOCALAPPDATA", root.join("local")),
            ("XDG_STATE_HOME", root.join("xdg")),
        ]))
        .unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
        assert!(refused.to_string().contains(STATE_DIR_ENV), "{refused}");
        assert_eq!(
            state_dir_location(&given(vec![])).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        if cfg!(windows) {
            assert_eq!(
                state_dir_location(&given(vec![("LOCALAPPDATA", root.join("local"))])).unwrap(),
                root.join("local").join("v8-runner")
            );
        } else {
            assert_eq!(
                state_dir_location(&given(vec![
                    ("XDG_STATE_HOME", root.join("xdg")),
                    ("HOME", root.join("home")),
                ]))
                .unwrap(),
                root.join("xdg").join("v8-runner")
            );
            assert_eq!(
                state_dir_location(&given(vec![
                    ("XDG_STATE_HOME", PathBuf::from("relative")),
                    ("HOME", root.join("home")),
                ]))
                .unwrap(),
                root.join("home")
                    .join(".local")
                    .join("state")
                    .join("v8-runner")
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn this_process_runs_as_one_account() {
        let id = account_id().unwrap();
        assert_eq!(account_id().unwrap(), id);
        if cfg!(windows) {
            assert!(id.starts_with("S-1-"), "{id}");
        } else {
            assert!(id.parse::<u32>().is_ok(), "{id}");
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_key_is_the_same_for_the_same_parts_and_tells_every_other_tuple_apart() {
        let state = tempfile::tempdir().unwrap();
        let key = account_key(state.path(), &["ib", "8.3.27", "user"]).unwrap();
        assert_eq!(key.len(), 64);
        assert_eq!(
            account_key(state.path(), &["ib", "8.3.27", "user"]).unwrap(),
            key
        );
        assert_ne!(
            account_key(state.path(), &["ib", "8.3.27", "other"]).unwrap(),
            key
        );
        // The same characters split otherwise are another tuple.
        assert_ne!(
            account_key(state.path(), &["ib8", ".3.27", "user"]).unwrap(),
            key
        );
        // Another account's salt gives another key for the same parts.
        let other = tempfile::tempdir().unwrap();
        assert_ne!(
            account_key(other.path(), &["ib", "8.3.27", "user"]).unwrap(),
            key
        );
        assert!(!key.contains("ib"));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn the_salt_is_made_once_and_only_the_account_reads_it() {
        let state = tempfile::tempdir().unwrap();
        let salt = account_salt(state.path()).unwrap();
        assert_eq!(account_salt(state.path()).unwrap(), salt);
        assert_ne!(salt, [0u8; 32]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                |path: PathBuf| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(state.path().join("account-salt")), 0o600);
        }
        std::fs::write(state.path().join("account-salt"), b"short").unwrap();
        let refused = account_salt(state.path()).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
        let text = refused.to_string();
        assert!(
            text.contains("account-salt") && text.contains("delete"),
            "{text}"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn racing_makers_of_the_salt_agree_on_one() {
        let state = tempfile::tempdir().unwrap();
        let salts: Vec<[u8; 32]> = std::thread::scope(|scope| {
            let makers: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| account_salt(state.path()).unwrap()))
                .collect();
            makers
                .into_iter()
                .map(|maker| maker.join().unwrap())
                .collect()
        });
        assert!(salts.iter().all(|salt| *salt == salts[0]), "{salts:?}");
        let left: Vec<_> = std::fs::read_dir(state.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("account-salt")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_or_a_file_is_not_taken_for_the_state_directory() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        ensure_private_dir(&real).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(
            ensure_private_dir(&link).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let file = root.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            ensure_private_dir(&file).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_directory_this_account_made_is_its_own_and_a_file_is_not() {
        let root = tempfile::tempdir().unwrap();
        // A directory made by plain `create_dir` gets the token's default owner too.
        let plain = root.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        ensure_private_dir(&plain).unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            ensure_private_dir(&file).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_directory_owned_by_another_account_is_refused() {
        // `C:\Windows` belongs to TrustedInstaller, never to the account running tests.
        let windows = std::env::var_os("SystemRoot").expect("SystemRoot");
        assert_eq!(
            ensure_private_dir(std::path::Path::new(&windows))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn the_state_directory_is_created_for_the_account_only() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("nested").join("v8-runner");
        ensure_private_dir(&dir).unwrap();
        ensure_private_dir(&dir).unwrap();
        assert!(dir.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            ensure_private_dir(&dir).unwrap();
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }
}
