//! Inspect a macOS process’s TCP sockets through libproc.

use libc::{c_int, proc_fdinfo};
use std::mem::size_of;
use std::net::Ipv4Addr;

const MAX_FDS: usize = 16_384;
const PROC_PIDFDSOCKETINFO: c_int = 3;
const SOCKINFO_TCP: i32 = 2;
const TSI_S_LISTEN: i32 = 1;
const TSI_S_ESTABLISHED: i32 = 4;

// Darwin LP64 socket_fdinfo, from the SDK's <sys/proc_info.h>. Keep the largest
// union member opaque; check the byte count returned by the kernel before reading.
const SOCKET_INFO_SIZE: usize = 792;
const SOCKET_TYPE: usize = 176;
const SOCKET_PROTOCOL: usize = 180;
const SOCKET_FAMILY: usize = 184;
const SOCKET_OPTIONS: usize = 188;
const SOCKET_KIND: usize = 256;
const TCP_PEER_PORT: usize = 264;
const TCP_LOCAL_PORT: usize = 268;
const TCP_GENERATION: usize = 272;
const TCP_PEER_IP4: usize = 308;
const TCP_LOCAL_IP4: usize = 324;
const TCP_STATE: usize = 344;

#[repr(align(8))]
struct SocketInfo([u8; SOCKET_INFO_SIZE]);

impl SocketInfo {
    fn i32_at(&self, offset: usize) -> i32 {
        i32::from_ne_bytes(self.0[offset..][..4].try_into().expect("SDK i32 field"))
    }

    fn is_ipv4_tcp(&self) -> bool {
        self.i32_at(SOCKET_KIND) == SOCKINFO_TCP
            && self.i32_at(SOCKET_TYPE) == libc::SOCK_STREAM
            && self.i32_at(SOCKET_PROTOCOL) == libc::IPPROTO_TCP
            && self.i32_at(SOCKET_FAMILY) == libc::AF_INET
    }

    fn listens_on(&self, port: u16) -> bool {
        let options = i32::from(u16::from_ne_bytes(
            self.0[SOCKET_OPTIONS..][..2]
                .try_into()
                .expect("SDK options field"),
        ));
        self.i32_at(TCP_STATE) == TSI_S_LISTEN
            && options & libc::SO_REUSEPORT == 0
            && self.0[TCP_LOCAL_IP4..][..4] == Ipv4Addr::LOCALHOST.octets()
            && self.i32_at(TCP_LOCAL_PORT) == i32::from(port.to_be())
    }

    fn accepts_from(&self, server_port: u16, client_port: u16) -> bool {
        self.i32_at(TCP_STATE) == TSI_S_ESTABLISHED
            && self.0[TCP_LOCAL_IP4..][..4] == Ipv4Addr::LOCALHOST.octets()
            && self.0[TCP_PEER_IP4..][..4] == Ipv4Addr::LOCALHOST.octets()
            && self.i32_at(TCP_LOCAL_PORT) == i32::from(server_port.to_be())
            && self.i32_at(TCP_PEER_PORT) == i32::from(client_port.to_be())
    }

    fn generation(&self) -> u64 {
        u64::from_ne_bytes(
            self.0[TCP_GENERATION..][..8]
                .try_into()
                .expect("SDK generation field"),
        )
    }
}

pub(crate) fn listens_on(pid: u32, port: u16) -> bool {
    find_generation(pid, |info| info.listens_on(port)).is_some()
}

/// The server side of one live TCP connection to an IPv4 loopback client.
pub(crate) fn connection_generation(pid: u32, server_port: u16, client_port: u16) -> Option<u64> {
    find_generation(pid, |info| info.accepts_from(server_port, client_port))
}

fn find_generation(pid: u32, matches_socket: impl Fn(&SocketInfo) -> bool) -> Option<u64> {
    let pid = c_int::try_from(pid).ok()?;
    if pid <= 0 {
        return None;
    }
    let entry_size = size_of::<proc_fdinfo>();
    // SAFETY: a null pointer and length zero request the needed byte count.
    let bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let bytes = usize::try_from(bytes).ok()?;
    if bytes == 0 || !bytes.is_multiple_of(entry_size) {
        return None;
    }
    let capacity = (bytes / entry_size).checked_add(16)?;
    if capacity > MAX_FDS {
        return None;
    }
    let mut entries = vec![
        proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0
        };
        capacity
    ];
    let capacity_bytes = c_int::try_from(capacity * entry_size).ok()?;
    // SAFETY: entries provides capacity_bytes of writable, aligned proc_fdinfo storage.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            entries.as_mut_ptr().cast(),
            capacity_bytes,
        )
    };
    let read = usize::try_from(read).ok()?;
    if read == 0 || read >= capacity * entry_size || !read.is_multiple_of(entry_size) {
        return None;
    }

    for fd in &entries[..read / entry_size] {
        if fd.proc_fdtype != libc::PROX_FDTYPE_SOCKET as u32 {
            continue;
        }
        let mut info = SocketInfo([0; SOCKET_INFO_SIZE]);
        // SAFETY: info provides the aligned, writable storage required for socket_fdinfo.
        let read = unsafe {
            libc::proc_pidfdinfo(
                pid,
                fd.proc_fd,
                PROC_PIDFDSOCKETINFO,
                info.0.as_mut_ptr().cast(),
                SOCKET_INFO_SIZE as c_int,
            )
        };
        // An unrelated descriptor may close between enumeration and inspection.
        if read == SOCKET_INFO_SIZE as c_int && info.is_ipv4_tcp() && matches_socket(&info) {
            let generation = info.generation();
            return (generation != 0).then_some(generation);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn macos_proves_only_an_exact_loopback_listener_held_by_the_pid() {
        let pid = std::process::id();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(listens_on(pid, port));
        assert!(!listens_on(1, port));
        let wildcard = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        assert!(!listens_on(pid, wildcard.local_addr().unwrap().port()));
        drop(listener);
        assert!(!listens_on(pid, port));
    }

    #[test]
    fn macos_finds_the_accepted_socket_by_both_endpoints() {
        let pid = std::process::id();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let client_port = client.local_addr().unwrap().port();
        assert_eq!(
            connection_generation(pid, address.port(), client_port),
            None
        );
        let (peer, _) = listener.accept().unwrap();
        let generation = connection_generation(pid, address.port(), client_port).unwrap();
        assert_eq!(connection_generation(1, address.port(), client_port), None);
        assert_eq!(
            connection_generation(pid, address.port(), client_port),
            Some(generation)
        );
        drop(peer);
        assert_eq!(
            connection_generation(pid, address.port(), client_port),
            None
        );
    }
}
