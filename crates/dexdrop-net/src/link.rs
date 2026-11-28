//! Which link a connection between nearby devices takes: the network both
//! are on, or Wi-Fi from one device straight to the other (Apple's
//! peer-to-peer Wi-Fi, AWDL; docs/CONNECTIVITY.md, 4.2).
//!
//! Over AWDL both ends use link-local IPv6 addresses on the `awdl0`
//! interface, so the address's scope names the link.

use std::net::SocketAddr;

use dexdrop_core::api::Route;

/// How a connection on a local link goes, from the other end's address.
pub(crate) fn local_route(peer: SocketAddr) -> Route {
    if over_peer_to_peer(peer) {
        Route::PeerToPeer
    } else {
        Route::Nearby
    }
}

fn over_peer_to_peer(addr: SocketAddr) -> bool {
    match addr {
        SocketAddr::V6(v6) if v6.scope_id() != 0 => {
            interface_name(v6.scope_id()).is_some_and(|name| is_peer_to_peer_interface(&name))
        }
        _ => false,
    }
}

/// Apple's peer-to-peer Wi-Fi interface (`awdl0`).
fn is_peer_to_peer_interface(name: &str) -> bool {
    name.starts_with("awdl")
}

#[cfg(unix)]
fn interface_name(index: u32) -> Option<String> {
    let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
    // SAFETY: `buf` holds IF_NAMESIZE bytes, as the call requires.
    let name = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if name.is_null() {
        return None;
    }
    // SAFETY: on success the name is NUL-terminated within `buf`.
    Some(
        unsafe { std::ffi::CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(not(unix))]
fn interface_name(_index: u32) -> Option<String> {
    None
}

/// `SO_RECV_ANYIF` (XNU, `sys/socket.h`): the socket also takes
/// connections that arrive on restricted interfaces, which Apple's
/// peer-to-peer Wi-Fi is. `dns_sd.h` asks for it on a listening socket
/// advertised with `kDNSServiceFlagsIncludeP2P`.
#[cfg(target_vendor = "apple")]
const SO_RECV_ANYIF: libc::c_int = 0x1104;

/// Lets a listening socket accept connections over Apple's peer-to-peer
/// Wi-Fi. Elsewhere there is nothing to allow.
#[cfg(target_vendor = "apple")]
pub(crate) fn allow_peer_to_peer(socket: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let on: libc::c_int = 1;
    // SAFETY: a valid socket and a c_int option value of the given size.
    let err = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_RECV_ANYIF,
            &on as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if err == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_vendor = "apple"))]
pub(crate) fn allow_peer_to_peer<T>(_socket: &T) -> std::io::Result<()> {
    Ok(())
}

/// Whether a socket accepts over peer-to-peer Wi-Fi (for tests).
#[cfg(all(test, target_vendor = "apple"))]
pub(crate) fn allows_peer_to_peer(socket: &impl std::os::fd::AsRawFd) -> bool {
    let mut on: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: a valid socket and a c_int-sized out value.
    let err = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_RECV_ANYIF,
            &mut on as *mut libc::c_int as *mut libc::c_void,
            &mut len,
        )
    };
    err == 0 && on != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn only_the_peer_to_peer_interface_is_peer_to_peer() {
        assert!(is_peer_to_peer_interface("awdl0"));
        for name in ["en0", "lo0", "wlan0", "eth0", "utun3", "bridge100"] {
            assert!(!is_peer_to_peer_interface(name), "{name}");
        }
    }

    /// Addresses on the shared network, loopback's link-local ones
    /// included, are said to be on this network.
    #[test]
    fn other_links_are_this_network() {
        let v4 = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 20), 4242));
        assert_eq!(local_route(v4), Route::Nearby);
        let global = SocketAddr::from((Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 4242));
        assert_eq!(local_route(global), Route::Nearby);
        #[cfg(unix)]
        {
            let lo = ["lo0", "lo"]
                .iter()
                .find_map(|n| {
                    let name = std::ffi::CString::new(*n).unwrap();
                    // SAFETY: a NUL-terminated interface name.
                    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
                    (index != 0).then_some(index)
                })
                .expect("a loopback interface");
            assert!(interface_name(lo).is_some_and(|n| n.starts_with("lo")));
            let scoped = SocketAddr::V6(std::net::SocketAddrV6::new(
                Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                4242,
                0,
                lo,
            ));
            assert_eq!(local_route(scoped), Route::Nearby);
        }
    }

    /// On a Mac with peer-to-peer Wi-Fi (`awdl0`), a link-local address
    /// scoped to it is a device-to-device link. Runners without Wi-Fi have
    /// no such interface, and the test says so.
    #[cfg(target_os = "macos")]
    #[test]
    fn awdl_addresses_are_peer_to_peer() {
        let name = std::ffi::CString::new("awdl0").unwrap();
        // SAFETY: a NUL-terminated interface name.
        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        if index == 0 {
            eprintln!("no awdl0 on this Mac (no Wi-Fi): skipped");
            return;
        }
        let peer = SocketAddr::V6(std::net::SocketAddrV6::new(
            Ipv6Addr::new(0xfe80, 0, 0, 0, 0x1c2b, 0x3aff, 0xfe4d, 0x5e6f),
            4242,
            0,
            index,
        ));
        assert_eq!(local_route(peer), Route::PeerToPeer);
    }
}
