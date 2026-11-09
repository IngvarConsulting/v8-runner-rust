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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "живой агент Конфигуратора (#577) берёт это на следующем этапе"
    )
)]
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
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{LocalFree, HANDLE};
        use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut raw: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of this process needs no closing; `raw` outlives the call.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the call succeeded and handed over this handle; the guard closes it once.
        let token = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut size = 0u32;
        // SAFETY: a null buffer with zero length only asks for the needed length.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut size,
            )
        };
        if size == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // `u64` elements keep the buffer aligned for `TOKEN_USER` and the SID after it.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `size` bytes and outlives the call.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the call above wrote a `TOKEN_USER` at the start of the aligned buffer.
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: the SID lives in `buffer`; on success `text` holds a string to `LocalFree`.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
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

    #[cfg(not(any(unix, windows)))]
    pub fn account_id() -> std::io::Result<String> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this system names no account",
        ))
    }

    /// Where the account's state directory lies, from `V8_RUNNER_STATE_DIR` or else from the
    /// system: `%LOCALAPPDATA%\v8-runner` on Windows, `$XDG_STATE_HOME/v8-runner` or
    /// `~/.local/state/v8-runner` on Unix. Relative values are ignored, as the XDG
    /// specification asks.
    pub(super) fn state_dir_location(
        env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Option<PathBuf> {
        if let Some(dir) = absolute_env(env, STATE_DIR_ENV) {
            return Some(dir);
        }
        if cfg!(windows) {
            return absolute_env(env, "LOCALAPPDATA").map(|dir| dir.join("v8-runner"));
        }
        absolute_env(env, "XDG_STATE_HOME")
            .or_else(|| absolute_env(env, "HOME").map(|home| home.join(".local").join("state")))
            .map(|dir| dir.join("v8-runner"))
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
    /// outlive a command and do not belong to one working copy. On Unix an existing one must
    /// belong to the account and is narrowed to 0700.
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "тесты не трогают каталог состояния своей учётной записи"
        )
    )]
    pub fn account_state_dir() -> std::io::Result<PathBuf> {
        let dir = state_dir_location(&|name| std::env::var_os(name)).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the system names no state directory for this account",
            )
        })?;
        ensure_private_dir(&dir)?;
        Ok(dir)
    }

    /// Creates `dir` owner-only, with its parents as usual; leaves an existing one in place.
    pub(super) fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match crate::support::temp::create_private_dir(dir) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = std::fs::symlink_metadata(dir)?;
            // SAFETY: `getuid` takes nothing and cannot fail.
            if !metadata.is_dir() || metadata.uid() != unsafe { libc::getuid() } {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("'{}' is not a directory of this account", dir.display()),
                ));
            }
            if metadata.mode() & 0o777 != 0o700 {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        #[cfg(not(unix))]
        if !dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("'{}' is not a directory", dir.display()),
            ));
        }
        Ok(())
    }

    /// The directory of live Designer agents under the account's state directory.
    pub fn agents_dir(state: &Path) -> std::io::Result<PathBuf> {
        let dir = state.join("agents");
        ensure_private_dir(&dir)?;
        Ok(dir)
    }

    /// Takes the account's agent registry lock, `<state>/agents/registry.lock`, waiting for
    /// another holder. Callers hold it only to read and write records, never across a start.
    pub fn lock_agent_registry(
        state: &Path,
    ) -> std::io::Result<crate::support::fs::AdvisoryLockGuard> {
        crate::support::fs::acquire_advisory_lock(&agents_dir(state)?.join("registry.lock"))
    }

    /// Length of the account's salt in bytes.
    const SALT_LEN: usize = 32;

    /// The account's salt in `<state>/account-salt`, made on first use under the registry
    /// lock. The file is readable by the account only: the state directory is owner-only, and
    /// on Unix the file itself is 0600.
    pub fn account_salt(state: &Path) -> std::io::Result<[u8; SALT_LEN]> {
        let path = state.join("account-salt");
        if let Some(salt) = read_salt(&path)? {
            return Ok(salt);
        }
        let _registry = lock_agent_registry(state)?;
        if let Some(salt) = read_salt(&path)? {
            return Ok(salt);
        }
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(std::io::Error::other)?;
        crate::support::fs::write_file_atomically(&path, |file| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            std::io::Write::write_all(file, &salt)
        })?;
        Ok(salt)
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
                        format!("'{}' holds no salt of the runner", path.display()),
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
        let salt = account_salt(state)?;
        let account = account_id()?;
        let mut message = Vec::new();
        for part in std::iter::once(account.as_str()).chain(parts.iter().copied()) {
            message.extend_from_slice(&(part.len() as u64).to_be_bytes());
            message.extend_from_slice(part.as_bytes());
        }
        Ok(hmac_sha256(&salt, &message)
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
        account_id, account_key, account_salt, ensure_private_dir, hmac_sha256,
        lock_agent_registry, state_dir_location, STATE_DIR_ENV,
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
            ])),
            Some(own)
        );
        // A relative value is not a location.
        assert_eq!(
            state_dir_location(&given(vec![(STATE_DIR_ENV, PathBuf::from("relative"))])),
            None
        );
        if cfg!(windows) {
            assert_eq!(
                state_dir_location(&given(vec![("LOCALAPPDATA", root.join("local"))])),
                Some(root.join("local").join("v8-runner"))
            );
        } else {
            assert_eq!(
                state_dir_location(&given(vec![
                    ("XDG_STATE_HOME", root.join("xdg")),
                    ("HOME", root.join("home")),
                ])),
                Some(root.join("xdg").join("v8-runner"))
            );
            assert_eq!(
                state_dir_location(&given(vec![
                    ("XDG_STATE_HOME", PathBuf::from("relative")),
                    ("HOME", root.join("home")),
                ])),
                Some(
                    root.join("home")
                        .join(".local")
                        .join("state")
                        .join("v8-runner")
                )
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
            assert_eq!(mode(state.path().join("agents")), 0o700);
        }
        std::fs::write(state.path().join("account-salt"), b"short").unwrap();
        assert_eq!(
            account_salt(state.path()).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn the_registry_lock_is_held_by_one_holder_at_a_time() {
        let state = tempfile::tempdir().unwrap();
        let held = lock_agent_registry(state.path()).unwrap();
        let path = state.path().join("agents").join("registry.lock");
        assert_eq!(
            crate::support::fs::try_acquire_advisory_lock(&path)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(held);
        drop(crate::support::fs::try_acquire_advisory_lock(&path).unwrap());
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
