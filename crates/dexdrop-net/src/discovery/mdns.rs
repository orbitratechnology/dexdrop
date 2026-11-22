//! Discovery with the pure-Rust `mdns-sd` responder (Windows, Linux).

use std::{collections::HashMap, net::SocketAddr, time::Instant};

use dexdrop_core::{PeerId, SERVICE_TYPE};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use super::{
    Advertisement, Discovered, DiscoveryEvent, discovered, instance_name, local_addrs, parse_txt,
    txt_entries,
};

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
                        let Some(found) = parse(&info) else {
                            continue;
                        };
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
        let instance = instance_name(&ad.peer_id);
        let host = format!("{instance}.local.");
        let entries = txt_entries(ad);
        let properties: Vec<(&str, &str)> = entries.iter().map(|(k, v)| (*k, v.as_str())).collect();
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
    let txt = parse_txt(|key| info.get_property_val_str(key).map(str::to_owned))?;
    let port = info.get_port();
    let addrs = info
        .get_addresses()
        .iter()
        .map(|ip| SocketAddr::new(ip.to_ip_addr(), port))
        .collect();
    discovered(txt, addrs)
}
