//! What this machine can tell about itself: its host name and whether one of its
//! processes still runs.

/// Whether a process with `pid` runs on this machine. A process that exists but may not
/// be inspected counts as alive; on Unix an unreaped zombie counts as alive too.
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
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, WAIT_TIMEOUT};
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
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
}

/// Without a way to ask, every process is taken as alive: callers use the answer to
/// decide whether something may be taken over, and refusing is the safe side.
#[cfg(not(any(unix, windows)))]
pub fn is_process_alive(_pid: u32) -> bool {
    true
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

/// Name of this machine, if the system gives one. Windows sets `COMPUTERNAME` for
/// every process.
#[cfg(windows)]
pub fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(not(any(unix, windows)))]
pub fn host_name() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::{host_name, is_process_alive};

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
    fn a_pid_beyond_any_process_is_not_alive() {
        assert!(!is_process_alive(i32::MAX as u32));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn this_machine_has_a_name() {
        assert!(host_name().is_some());
    }
}
