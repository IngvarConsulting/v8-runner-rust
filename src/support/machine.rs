//! What this machine can tell about itself: its identifier, its host name and whether one
//! of its processes still runs.

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
        .is_ok_and(|stat| matches!(process_state(&stat), Some("Z" | "X")))
}

#[cfg(not(target_os = "linux"))]
fn has_ended_unreaped(_pid: u32) -> bool {
    false
}

/// The state field of a `/proc/<pid>/stat` line. It follows the process name, which is
/// enclosed in parentheses and may itself hold parentheses and spaces, so the name ends
/// at the last `)`.
#[cfg(any(target_os = "linux", test))]
fn process_state(stat: &str) -> Option<&str> {
    let (_pid_and_name, after_name) = stat.rsplit_once(')')?;
    after_name.split_whitespace().next()
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
/// gives none.
pub fn machine_id() -> Option<String> {
    system_machine_id()
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
}

#[cfg(target_os = "linux")]
fn system_machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
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
    Some(uuid.iter().map(|byte| format!("{byte:02x}")).collect())
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
    Some(String::from_utf16_lossy(&text[..end]))
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
fn system_machine_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::{host_name, is_process_alive, is_process_running, machine_id, process_state};

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
        assert_eq!(process_state("42 (a) b (c)) Z 1 42"), Some("Z"));
        assert_eq!(process_state("42 (sleep) S 1 42"), Some("S"));
        assert_eq!(process_state("42 sleep"), None);
    }

    #[test]
    fn a_pid_beyond_any_process_is_not_alive() {
        assert!(!is_process_alive(i32::MAX as u32));
    }

    #[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
    #[test]
    fn this_machine_keeps_one_identifier() {
        if std::path::Path::new("/etc/machine-id").exists() || !cfg!(target_os = "linux") {
            let id = machine_id().expect("machine id");
            assert_eq!(machine_id().as_deref(), Some(id.as_str()));
            assert_eq!(id, id.trim().to_ascii_lowercase());
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn this_machine_has_a_name() {
        assert!(host_name().is_some());
    }
}
