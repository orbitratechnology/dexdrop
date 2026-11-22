//! Nearby-device discovery over DNS-SD / mDNS (FR-010 – FR-013).
//!
//! Every instance advertises one fixed service type,
//! [`dexdrop_core::SERVICE_TYPE`], with a TXT record holding only what a
//! session needs: protocol version, peer ID, display name, platform and
//! whether a pairing code is open. File names and account names are never
//! advertised.
//!
//! This adapter uses the pure-Rust `mdns-sd` responder, which works on
//! Windows, Linux and macOS. iOS must use the Network framework instead
//! (Bonjour without the multicast entitlement) and Android `NsdManager`;
//! those adapters implement the same [`DiscoveryEvent`] contract.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use dexdrop_core::{PeerId, SERVICE_TYPE, protocol::PROTOCOL_VERSION};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

/// A device found nearby.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    pub addrs: Vec<SocketAddr>,
    /// The device shows a pairing code right now.
    pub code_open: bool,
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
}

/// A running mDNS responder and browser.
pub struct Discovery {
    daemon: ServiceDaemon,
    registered: Option<String>,
    own_id: PeerId,
}

impl Discovery {
    /// Starts browsing; events go to `events`.
    pub fn start(
        own_id: PeerId,
        events: mpsc::UnboundedSender<DiscoveryEvent>,
    ) -> Result<Self, String> {
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let receiver = daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            // fullname -> peer, so a removal can be mapped back.
            let mut names: HashMap<String, PeerId> = HashMap::new();
            let mut last_seen: HashMap<PeerId, Instant> = HashMap::new();
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let Some(found) = parse(&info) else { continue };
                        if found.peer_id == own_id {
                            continue;
                        }
                        names.insert(info.get_fullname().to_owned(), found.peer_id);
                        last_seen.insert(found.peer_id, Instant::now());
                        if events.send(DiscoveryEvent::Found(found)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        if let Some(id) = names.remove(&fullname) {
                            last_seen.remove(&id);
                            if events.send(DiscoveryEvent::Lost(id)).is_err() {
                                break;
                            }
                        }
                    }
                    ServiceEvent::SearchStopped(_) => {
                        let _ =
                            events.send(DiscoveryEvent::Unavailable("Discovery stopped.".into()));
                        break;
                    }
                    _ => {}
                }
            }
        });
        Ok(Self {
            daemon,
            registered: None,
            own_id,
        })
    }

    /// Advertises this device, replacing an earlier advertisement.
    pub fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
        self.withdraw();
        let id = ad.peer_id.to_string();
        let instance = format!("dexdrop-{}", &id[..16]);
        let host = format!("{instance}.local.");
        let version = PROTOCOL_VERSION.to_string();
        let code = if ad.code_open { "1" } else { "0" };
        let properties: [(&str, &str); 5] = [
            ("v", &version),
            ("id", &id),
            ("n", &ad.name),
            ("p", &ad.platform),
            ("c", code),
        ];
        let addrs = local_addrs();
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host,
            &addrs[..],
            ad.port,
            &properties[..],
        )
        .map_err(|e| e.to_string())?;
        let info = if addrs.is_empty() {
            info.enable_addr_auto()
        } else {
            info
        };
        let fullname = info.get_fullname().to_owned();
        self.daemon.register(info).map_err(|e| e.to_string())?;
        self.registered = Some(fullname);
        Ok(())
    }

    /// Stops advertising (browsing continues).
    pub fn withdraw(&mut self) {
        if let Some(fullname) = self.registered.take() {
            let _ = self.daemon.unregister(&fullname);
        }
    }

    /// This device's peer ID.
    pub fn own_id(&self) -> PeerId {
        self.own_id
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.withdraw();
        let _ = self.daemon.shutdown();
    }
}

fn parse(info: &mdns_sd::ResolvedService) -> Option<Discovered> {
    let version: u16 = info.get_property_val_str("v")?.parse().ok()?;
    if version == 0 {
        return None;
    }
    let peer_id: PeerId = info.get_property_val_str("id")?.parse().ok()?;
    let name = info
        .get_property_val_str("n")
        .unwrap_or("Unknown device")
        .chars()
        .take(64)
        .collect();
    let platform = info
        .get_property_val_str("p")
        .unwrap_or("")
        .chars()
        .take(16)
        .collect();
    let code_open = info.get_property_val_str("c") == Some("1");
    let port = info.get_port();
    let mut addrs: Vec<SocketAddr> = info
        .get_addresses()
        .iter()
        .map(|ip| ip.to_ip_addr())
        .filter(|ip| !ip.is_unspecified())
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    // IPv4 first: link-local IPv6 needs scope IDs that are easy to get wrong.
    addrs.sort_by_key(|a| (a.is_ipv6(), a.ip().is_loopback()));
    addrs.dedup();
    if addrs.is_empty() {
        return None;
    }
    Some(Discovered {
        peer_id,
        name,
        platform,
        addrs,
        code_open,
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
