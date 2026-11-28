//! The direct link (docs/CONNECTIVITY.md 4.3): one device hosts a Wi-Fi
//! network for one transfer (an Android Wi-Fi Direct group, a Windows or
//! Linux access point), and a device with no network in common joins it
//! with the credentials its invitation carries. Then TCP and TLS run as on
//! any network: the device that joins still proves who it is.
//!
//! Each platform implements [`DirectLink`]; the engine owns it as it owns
//! discovery (`EngineConfig::direct_link`). Its calls block, so the engine
//! makes them on blocking threads. [`Loopback`] stands in for a radio in
//! tests.

use std::{
    io,
    net::{IpAddr, Ipv4Addr},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub use dexdrop_core::invitation::{Band, DirectHint};

#[cfg(any(target_os = "android", test))]
mod android;
#[cfg(target_os = "android")]
pub use android::AndroidLink;

/// This platform's direct link, if it has one yet (docs/CONNECTIVITY.md
/// 4.3, phases): Android's from the app's Java VM on.
pub fn platform() -> Option<std::sync::Arc<dyn DirectLink>> {
    #[cfg(target_os = "android")]
    if crate::jvm::has_context() {
        return Some(std::sync::Arc::new(AndroidLink::default()));
    }
    None
}

/// A platform's way of hosting and joining a network for one transfer.
pub trait DirectLink: Send + Sync + std::fmt::Debug + 'static {
    /// This device can host a network others join.
    fn can_host(&self) -> bool;
    /// This device can join a network another device hosts.
    fn can_join(&self) -> bool;
    /// Hosts a network with credentials chosen for it; returns once it is
    /// up, or why it could not start (in words for the user).
    fn host(&self) -> Result<Hosted, String>;
    /// Stops hosting.
    fn stop(&self);
    /// Joins the network `hint` names; returns once joined. Dropping what
    /// it returns leaves the network.
    fn join(&self, hint: &DirectHint) -> Result<Box<dyn Joined>, String>;
}

/// A network this device hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hosted {
    pub ssid: String,
    pub passphrase: String,
    pub band: Band,
    /// This device's address on it.
    pub ip: IpAddr,
    /// Its prefix length: the addresses of the devices that join.
    pub prefix: u8,
}

impl Hosted {
    /// Whether `ip` is on this network: a device that joined it.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.ip, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let bits = u32::from(self.prefix.min(32));
                let mask = u32::MAX.checked_shl(32 - bits).unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let bits = u32::from(self.prefix.min(128));
                let mask = u128::MAX.checked_shl(128 - bits).unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// A network this device joined. Sockets to the host go through
/// [`Joined::prepare`]: where the system keeps a joined network apart
/// (Android binds sockets to it), that is where they are bound.
pub trait Joined: Send + Sync {
    fn prepare(&self, socket: &socket2::Socket) -> io::Result<()> {
        let _ = socket;
        Ok(())
    }
}

/// Credentials for a new network: a Wi-Fi Direct name (Android requires the
/// `DIRECT-xy-` start) and a 16-character passphrase, both random.
pub fn new_credentials() -> (String, String) {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let pick = |bytes: &[u8]| -> String {
        bytes
            .iter()
            .map(|b| ALPHABET[(b & 31) as usize] as char)
            .collect()
    };
    let tag = pick(&dexdrop_core::random_bytes::<2>());
    let passphrase = pick(&dexdrop_core::random_bytes::<16>());
    (format!("DIRECT-{tag}-DexDrop"), passphrase)
}

/// A direct link over loopback, for tests (and the app's headless tests,
/// `DEXDROP_DIRECT_LINK=loopback`): hosting "starts" a network at
/// 127.0.0.1/8, and joining any valid hint succeeds. It counts what it was
/// asked to do.
#[derive(Debug, Default)]
pub struct Loopback {
    pub hosts: AtomicUsize,
    pub stops: AtomicUsize,
    pub joins: AtomicUsize,
    /// When set, hosting fails with this reason.
    pub refuse_host: Mutex<Option<String>>,
}

impl DirectLink for Loopback {
    fn can_host(&self) -> bool {
        true
    }

    fn can_join(&self) -> bool {
        true
    }

    fn host(&self) -> Result<Hosted, String> {
        if let Some(why) = self
            .refuse_host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Err(why);
        }
        self.hosts.fetch_add(1, Ordering::SeqCst);
        let (ssid, passphrase) = new_credentials();
        Ok(Hosted {
            ssid,
            passphrase,
            band: Band::Ghz5,
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            prefix: 8,
        })
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }

    fn join(&self, hint: &DirectHint) -> Result<Box<dyn Joined>, String> {
        if !hint.is_valid() {
            return Err("The direct link's credentials are not valid.".into());
        }
        self.joins.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(LoopbackJoined))
    }
}

struct LoopbackJoined;

impl Joined for LoopbackJoined {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn credentials_make_a_joinable_network() {
        let (ssid, passphrase) = new_credentials();
        assert!(ssid.starts_with("DIRECT-") && ssid.ends_with("-DexDrop"));
        let hint = DirectHint {
            ssid,
            passphrase,
            band: Band::Ghz5,
            host: "192.168.49.1:1".parse().unwrap(),
        };
        assert!(hint.is_valid());
        assert_ne!(new_credentials(), new_credentials());
    }

    /// A Wi-Fi Direct group's devices (192.168.49.0/24) are on it; others,
    /// and the other address family, are not.
    #[test]
    fn a_hosted_network_holds_its_subnet() {
        let group = Hosted {
            ssid: "DIRECT-k7-DexDrop".into(),
            passphrase: "QX9MK7P2RT4VQX9M".into(),
            band: Band::Ghz5,
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 49, 1)),
            prefix: 24,
        };
        assert!(group.contains(IpAddr::V4(Ipv4Addr::new(192, 168, 49, 23))));
        // A dual-stack listener sees IPv4 peers as ::ffff:a.b.c.d.
        assert!(group.contains(IpAddr::V6(Ipv4Addr::new(192, 168, 49, 23).to_ipv6_mapped())));
        assert!(!group.contains(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23))));
        assert!(!group.contains(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }
}
