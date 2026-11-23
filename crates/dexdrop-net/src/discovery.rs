//! Nearby-device discovery over DNS-SD / mDNS (FR-010 – FR-013).
//!
//! Every instance advertises one fixed service type,
//! [`dexdrop_core::SERVICE_TYPE`], with a TXT record holding only what a
//! session needs: protocol version, peer ID, display name, platform and
//! whether a pairing code is open. File names and account names are never
//! advertised.
//!
//! Two adapters implement the same [`DiscoveryEvent`] contract:
//!
//! - Apple platforms (macOS, iOS devices) use the system's DNS-SD service,
//!   mDNSResponder, through the `dns_sd` C API (`bonjour`). On a Mac a
//!   second responder cannot share the mDNS port with the system's, and iOS
//!   apps get local-network discovery only through the system's service
//!   (`NSBonjourServices` and the user's local-network permission).
//! - Android uses the system's `NsdManager` through a small Java bridge
//!   (`nsd`): apps cannot hold the multicast lock mdns-sd would need there.
//! - Everywhere else, and in the iOS Simulator, the pure-Rust `mdns-sd`
//!   responder (`mdns`), which is also Android's fallback.

use std::net::{IpAddr, SocketAddr};

use dexdrop_core::{PeerId, protocol::PROTOCOL_VERSION};

// Apple devices and Macs use Bonjour. The iOS Simulator uses mdns-sd: the
// Mac's mDNSResponder applies its local-network trust check to simulator
// apps and, on CI, cannot read their bundle record, so it ignores them.
#[cfg(any(
    all(
        target_vendor = "apple",
        not(all(target_os = "ios", target_abi = "sim"))
    ),
    dexdrop_check_bonjour
))]
pub mod bonjour;
#[cfg(any(
    not(target_vendor = "apple"),
    all(target_os = "ios", target_abi = "sim")
))]
pub mod mdns;

#[cfg(any(target_os = "android", test))]
pub mod nsd;

#[cfg(all(
    target_vendor = "apple",
    not(all(target_os = "ios", target_abi = "sim"))
))]
pub use bonjour::Discovery;
#[cfg(any(
    all(not(target_vendor = "apple"), not(target_os = "android")),
    all(target_os = "ios", target_abi = "sim")
))]
pub use mdns::Discovery;
#[cfg(target_os = "android")]
pub use nsd::Discovery;

/// A device found nearby.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    pub addrs: Vec<SocketAddr>,
    /// The device shows a pairing code right now.
    pub code_open: bool,
    pub avatar: Option<String>,
}

/// What discovery reports.
#[derive(Clone, Debug)]
pub enum DiscoveryEvent {
    Found(Discovered),
    Lost(PeerId),
    /// Discovery could not start or stopped working.
    Unavailable(String),
}

/// What this device advertises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    pub port: u16,
    pub code_open: bool,
    pub avatar: Option<String>,
}

/// The TXT record entries this device advertises.
pub(crate) fn txt_entries(ad: &Advertisement) -> Vec<(&'static str, String)> {
    let mut entries = vec![
        ("v", PROTOCOL_VERSION.to_string()),
        ("id", ad.peer_id.to_string()),
        ("n", ad.name.clone()),
        ("p", ad.platform.clone()),
        ("c", if ad.code_open { "1" } else { "0" }.to_owned()),
    ];
    if let Some(avatar) = &ad.avatar {
        entries.push(("a", avatar.clone()));
    }
    entries
}

/// The instance name this device registers under.
pub(crate) fn instance_name(peer_id: &PeerId) -> String {
    format!("dexdrop-{}", &peer_id.to_string()[..16])
}

/// What a TXT record says about a device, before its addresses are known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerTxt {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    pub code_open: bool,
    pub avatar: Option<String>,
}

/// Reads a TXT record through `get`; `None` for anything not ours.
pub(crate) fn parse_txt(get: impl Fn(&str) -> Option<String>) -> Option<PeerTxt> {
    let version: u16 = get("v")?.parse().ok()?;
    if version == 0 {
        return None;
    }
    let peer_id: PeerId = get("id")?.parse().ok()?;
    let name = get("n")
        .unwrap_or_else(|| "Unknown device".into())
        .chars()
        .take(64)
        .collect();
    let platform = get("p").unwrap_or_default().chars().take(16).collect();
    let code_open = get("c").as_deref() == Some("1");
    let avatar = dexdrop_core::avatar::Avatar::clean(get("a").as_deref());
    Some(PeerTxt {
        peer_id,
        name,
        platform,
        code_open,
        avatar,
    })
}

/// A found device from its TXT record and addresses: IPv4 first, since
/// link-local IPv6 needs scope IDs that are easy to get wrong.
pub(crate) fn discovered(txt: PeerTxt, mut addrs: Vec<SocketAddr>) -> Option<Discovered> {
    addrs.retain(|a| !a.ip().is_unspecified());
    addrs.sort_by_key(|a| (a.is_ipv6(), a.ip().is_loopback()));
    addrs.dedup();
    if addrs.is_empty() {
        return None;
    }
    Some(Discovered {
        peer_id: txt.peer_id,
        name: txt.name,
        platform: txt.platform,
        addrs,
        code_open: txt.code_open,
        avatar: txt.avatar,
    })
}

/// This device's LAN addresses: up, not loopback, IPv4 and global IPv6.
pub fn local_addrs() -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| match ip {
            IpAddr::V4(v4) => !v4.is_link_local() && !v4.is_unspecified(),
            IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80 && !v6.is_unspecified(),
        })
        .collect();
    out.sort_by_key(|ip| ip.is_ipv6());
    out.dedup();
    out
}
