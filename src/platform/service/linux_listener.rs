use std::fs;
use std::io;
use std::net::Ipv4Addr;
use std::path::Path;

const PROC: &str = "/proc";
const LISTEN: &str = "0A";
const ESTABLISHED: &str = "01";

pub(super) struct TcpEntry<'a> {
    local: &'a str,
    remote: &'a str,
    state: &'a str,
    pub inode: &'a str,
}

impl TcpEntry<'_> {
    pub fn is_listener(&self) -> bool {
        self.state == LISTEN
    }

    pub fn local_port(&self) -> Option<u16> {
        // In both kernel tables, the port occupies the last four hex digits.
        let port = self.local.get(self.local.len().checked_sub(4)?..)?;
        u16::from_str_radix(port, 16).ok()
    }
}

// The kernel prints IPv4 addresses and individual IPv6 words in native byte order.
fn ipv4_hex(address: Ipv4Addr) -> u32 {
    u32::from_ne_bytes(address.octets())
}

fn endpoint(address: Ipv4Addr, port: u16) -> String {
    format!("{:08X}:{port:04X}", ipv4_hex(address))
}

pub(super) fn tcp_entries(table: &str) -> impl Iterator<Item = TcpEntry<'_>> {
    table.lines().skip(1).filter_map(|line| {
        let mut fields = line.split_whitespace();
        Some(TcpEntry {
            local: fields.nth(1)?,
            remote: fields.next()?,
            state: fields.next()?,
            inode: fields.nth(5)?,
        })
    })
}

pub(super) fn process_fd_links(
    proc_root: &Path,
    pid: u32,
) -> io::Result<impl Iterator<Item = std::path::PathBuf>> {
    Ok(fs::read_dir(proc_root.join(pid.to_string()).join("fd"))?
        .flatten()
        .filter_map(|fd| fs::read_link(fd.path()).ok()))
}

fn process_holds_socket(proc_root: &Path, pid: u32, inode: &str) -> io::Result<bool> {
    let socket = format!("socket:[{inode}]");
    Ok(process_fd_links(proc_root, pid)?
        .any(|link| link.as_os_str() == std::ffi::OsStr::new(&socket)))
}

// Other managed services can listen on wildcard addresses in their own network namespace.
pub(crate) fn listens_on(pid: u32, port: u16) -> bool {
    let proc_root = Path::new(PROC);
    let Ok(table) = fs::read_to_string(proc_root.join(pid.to_string()).join("net/tcp")) else {
        return false;
    };
    let loopback = endpoint(Ipv4Addr::LOCALHOST, port);
    let wildcard = endpoint(Ipv4Addr::UNSPECIFIED, port);
    let sockets: std::collections::HashSet<_> = tcp_entries(&table)
        .filter(|row| row.is_listener() && (row.local == loopback || row.local == wildcard))
        .map(|row| format!("socket:[{}]", row.inode))
        .collect();
    !sockets.is_empty()
        && process_fd_links(proc_root, pid).is_ok_and(|mut links| {
            links.any(|link| sockets.contains(link.to_string_lossy().as_ref()))
        })
}

pub(crate) fn debug_loopback_owner(pid: u32, port: u16) -> bool {
    prove_debug_listener(Path::new(PROC), pid, port).unwrap_or(false)
}

fn prove_debug_listener(proc_root: &Path, pid: u32, port: u16) -> io::Result<bool> {
    // Use the caller's network namespace: this is where the probe connects.
    let tcp = fs::read_to_string(proc_root.join("self/net/tcp"))?;
    let loopback = endpoint(Ipv4Addr::LOCALHOST, port);
    let wildcard = endpoint(Ipv4Addr::UNSPECIFIED, port);
    let mut candidates = tcp_entries(&tcp)
        .filter(|row| row.is_listener() && (row.local == loopback || row.local == wildcard));
    let Some(listener) = candidates.next() else {
        return Ok(false);
    };
    if listener.local != loopback || candidates.next().is_some() {
        return Ok(false);
    }

    // procfs does not say whether an IPv6 wildcard listener also accepts IPv4.
    let tcp6 = fs::read_to_string(proc_root.join("self/net/tcp6"))?;
    let wildcard_v6 = format!("00000000000000000000000000000000:{port:04X}");
    let mapped_v6 = format!(
        "0000000000000000{:08X}{:08X}:{port:04X}",
        u32::from_ne_bytes([0, 0, 255, 255]),
        ipv4_hex(Ipv4Addr::LOCALHOST),
    );
    if tcp_entries(&tcp6)
        .any(|row| row.is_listener() && (row.local == wildcard_v6 || row.local == mapped_v6))
    {
        return Ok(false);
    }
    process_holds_socket(proc_root, pid, listener.inode)
}

pub(crate) fn debug_connection_generation(
    pid: u32,
    server_port: u16,
    client_port: u16,
) -> io::Result<Option<u64>> {
    let proc_root = Path::new(PROC);
    let table = fs::read_to_string(proc_root.join("self/net/tcp"))?;
    let local = endpoint(Ipv4Addr::LOCALHOST, server_port);
    let remote = endpoint(Ipv4Addr::LOCALHOST, client_port);
    let Some(socket) = tcp_entries(&table)
        .find(|row| row.state == ESTABLISHED && row.local == local && row.remote == remote)
    else {
        return Ok(None);
    };
    let Ok(generation @ 1..) = socket.inode.parse::<u64>() else {
        return Ok(None);
    };
    Ok(process_holds_socket(proc_root, pid, socket.inode)?.then_some(generation))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn linux_debug_listener_requires_an_unambiguous_loopback_socket_held_by_the_pid() {
        let proc = tempfile::tempdir().unwrap();
        let root = proc.path();
        fs::create_dir_all(root.join("self/net")).unwrap();
        fs::create_dir_all(root.join("12/fd")).unwrap();
        fs::create_dir_all(root.join("34/fd")).unwrap();
        symlink("socket:[1234]", root.join("12/fd/3")).unwrap();
        symlink("socket:[5678]", root.join("34/fd/4")).unwrap();
        let tcp = root.join("self/net/tcp");
        let tcp6 = root.join("self/net/tcp6");
        let port = 12000;
        let loopback = endpoint(Ipv4Addr::LOCALHOST, port);
        let listener = format!("0: {loopback} 00000000:0000 0A 0:0 0:0 0 0 0 1234\n");
        fs::write(&tcp, format!("header\n{listener}")).unwrap();
        fs::write(&tcp6, "header\n").unwrap();
        assert!(prove_debug_listener(root, 12, port).unwrap());
        assert!(!prove_debug_listener(root, 34, port).unwrap());
        assert!(prove_debug_listener(root, 56, port).is_err());

        let wildcard = endpoint(Ipv4Addr::UNSPECIFIED, port);
        let second = format!("1: {wildcard} 00000000:0000 0A 0:0 0:0 0 0 0 5678\n");
        fs::write(&tcp, format!("header\n{second}")).unwrap();
        assert!(!prove_debug_listener(root, 34, port).unwrap());
        fs::write(&tcp, format!("header\n{listener}{second}")).unwrap();
        assert!(!prove_debug_listener(root, 12, port).unwrap());
        fs::write(
            &tcp,
            format!("header\n{listener}{}", second.replace(&wildcard, &loopback)),
        )
        .unwrap();
        assert!(!prove_debug_listener(root, 12, port).unwrap());

        fs::write(&tcp, format!("header\n{listener}")).unwrap();
        fs::write(
            &tcp6,
            format!("header\n0: 00000000000000000000000000000000:{port:04X} 0000:0000 0A 0:0 0:0 0 0 0 5678\n"),
        ).unwrap();
        assert!(!prove_debug_listener(root, 12, port).unwrap());
        fs::remove_file(tcp6).unwrap();
        assert!(prove_debug_listener(root, 12, port).is_err());
    }
}
