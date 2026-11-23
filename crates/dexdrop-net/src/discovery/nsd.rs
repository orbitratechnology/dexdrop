//! Android: the system's DNS-SD service, `NsdManager`.
//!
//! Wi-Fi drivers on Android drop multicast unless an app holds a multicast
//! lock, which needs a manifest permission the packaged app cannot declare
//! (see docs/STATUS.md). `NsdManager` does the multicast work in the system
//! instead and needs no permission — the same arrangement as Bonjour on
//! Apple platforms.
//!
//! `NsdManager` reports through Java callback interfaces, so a small Java
//! bridge (`android/dev/dexdrop/NsdBridge.java`, built into `android/nsd.dex`
//! by `scripts/build-nsd-dex.sh`) is embedded here and loaded from memory
//! (`InMemoryDexClassLoader`). One thread drives it: it sends commands and
//! polls the bridge's event queue, so Java never calls into Rust.
//!
//! Without a Java VM to reach — the `dexdrop` CLI run from a shell, or
//! Android before 12, where `JNI_GetCreatedJavaVMs` is not public — the
//! mdns-sd responder is used instead.

use std::{collections::HashMap, net::SocketAddr};

use dexdrop_core::{PeerId, SERVICE_TYPE};

use super::{Discovered, discovered, parse_txt};

/// Separates an event's fields (see `NsdBridge.java`).
const SEP: char = '\u{1}';
/// Separates TXT entries.
const TXT_SEP: char = '\u{2}';

/// The service type as `NsdManager` takes it: `_dexdrop._tcp`.
fn nsd_type() -> &'static str {
    SERVICE_TYPE.strip_suffix(".local.").unwrap_or(SERVICE_TYPE)
}

/// TXT entries in the bridge's form; separators and other control
/// characters are dropped from values (a device name is free text).
fn encode_txt(entries: &[(&str, String)]) -> String {
    entries
        .iter()
        .map(|(k, v)| {
            let v: String = v.chars().filter(|c| !c.is_control()).collect();
            format!("{k}={v}")
        })
        .collect::<Vec<_>>()
        .join(&TXT_SEP.to_string())
}

/// What the bridge reported.
#[derive(Debug, PartialEq)]
enum BridgeEvent {
    Found { service: String, found: Discovered },
    Lost { service: String },
    Failed(String),
}

/// Reads one event line; `None` for anything malformed or not ours.
fn parse_event(line: &str) -> Option<BridgeEvent> {
    let mut fields = line.split(SEP);
    match fields.next()? {
        "F" => {
            let service = fields.next()?.to_owned();
            // IPv6 link-local addresses arrive without their scope, which
            // makes them unusable: skip them.
            let ips: Vec<std::net::IpAddr> = fields
                .next()?
                .split(',')
                .filter_map(|a| a.parse().ok())
                .filter(|ip: &std::net::IpAddr| match ip {
                    std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
                    std::net::IpAddr::V4(_) => true,
                })
                .collect();
            let port: u16 = fields.next()?.parse().ok()?;
            let txt: HashMap<&str, &str> = fields
                .next()
                .unwrap_or("")
                .split(TXT_SEP)
                .filter_map(|entry| entry.split_once('='))
                .collect();
            let txt = parse_txt(|key| txt.get(key).map(|v| (*v).to_owned()))?;
            let addrs = ips
                .into_iter()
                .map(|ip| SocketAddr::new(ip, port))
                .collect();
            let found = discovered(txt, addrs)?;
            Some(BridgeEvent::Found { service, found })
        }
        "L" => Some(BridgeEvent::Lost {
            service: fields.next()?.to_owned(),
        }),
        "E" => Some(BridgeEvent::Failed(fields.next()?.to_owned())),
        _ => None,
    }
}

/// Which service names belong to which peer, so a loss can be reported.
#[derive(Default)]
struct Services(HashMap<String, PeerId>);

impl Services {
    /// What to report for a bridge event, skipping this device's own.
    fn event(&mut self, own_id: PeerId, event: BridgeEvent) -> Option<super::DiscoveryEvent> {
        use super::DiscoveryEvent;
        match event {
            BridgeEvent::Found { service, found } => {
                if found.peer_id == own_id {
                    return None;
                }
                self.0.insert(service, found.peer_id);
                Some(DiscoveryEvent::Found(found))
            }
            BridgeEvent::Lost { service } => {
                let id = self.0.remove(&service)?;
                // Another name may still point at the same device (renamed
                // after a conflict): it is lost only when none does.
                if self.0.values().any(|p| *p == id) {
                    return None;
                }
                Some(DiscoveryEvent::Lost(id))
            }
            BridgeEvent::Failed(message) => Some(DiscoveryEvent::Unavailable(message)),
        }
    }
}

#[cfg(target_os = "android")]
pub use system::Discovery;

#[cfg(target_os = "android")]
mod system {
    use std::{
        sync::mpsc::{self, TryRecvError},
        time::Duration,
    };

    use dexdrop_core::PeerId;
    use jni::{
        JavaVM,
        objects::{JString, JValue},
    };
    use tokio::sync::mpsc::UnboundedSender;

    use crate::jvm::{describe, java_vm, new_bridge};

    use super::{
        super::{Advertisement, DiscoveryEvent, instance_name, mdns, txt_entries},
        Services, encode_txt, nsd_type, parse_event,
    };

    /// Nearby-device discovery on Android.
    pub enum Discovery {
        /// The system's `NsdManager`.
        System {
            own_id: PeerId,
            commands: mpsc::Sender<Command>,
        },
        /// mdns-sd, where the system's service cannot be reached.
        Fallback(mdns::Discovery),
    }

    pub enum Command {
        Advertise {
            name: String,
            port: u16,
            txt: String,
        },
        Withdraw,
    }

    impl Discovery {
        /// Starts browsing; events go to `events`.
        pub fn start(
            own_id: PeerId,
            events: UnboundedSender<DiscoveryEvent>,
        ) -> Result<Self, String> {
            let Some(vm) = java_vm() else {
                tracing::info!("discovery: no Java VM in this process; using mdns-sd");
                return mdns::Discovery::start(own_id, events).map(Self::Fallback);
            };
            let (commands, rx) = mpsc::channel();
            let (ready, started) = mpsc::channel();
            let thread_events = events.clone();
            std::thread::Builder::new()
                .name("dexdrop-nsd".into())
                .spawn(move || run(vm, own_id, thread_events, rx, ready))
                .map_err(|e| e.to_string())?;
            let err = match started.recv_timeout(Duration::from_secs(10)) {
                Ok(Ok(())) => return Ok(Self::System { own_id, commands }),
                Ok(Err(err)) => err,
                Err(_) => "the bridge did not start in 10 s".into(),
            };
            tracing::warn!("discovery: NsdManager unavailable ({err}); using mdns-sd");
            mdns::Discovery::start(own_id, events).map(Self::Fallback)
        }

        /// Advertises this device, replacing an earlier advertisement.
        pub fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
            match self {
                Self::System { commands, .. } => commands
                    .send(Command::Advertise {
                        name: instance_name(&ad.peer_id),
                        port: ad.port,
                        txt: encode_txt(&txt_entries(ad)),
                    })
                    .map_err(|_| "Nearby discovery has stopped.".to_owned()),
                Self::Fallback(d) => d.advertise(ad),
            }
        }

        /// Stops advertising (browsing continues).
        pub fn withdraw(&mut self) {
            match self {
                Self::System { commands, .. } => {
                    let _ = commands.send(Command::Withdraw);
                }
                Self::Fallback(d) => d.withdraw(),
            }
        }

        /// This device's peer ID.
        pub fn own_id(&self) -> PeerId {
            match self {
                Self::System { own_id, .. } => *own_id,
                Self::Fallback(d) => d.own_id(),
            }
        }
    }

    fn run(
        vm: JavaVM,
        own_id: PeerId,
        events: UnboundedSender<DiscoveryEvent>,
        commands: mpsc::Receiver<Command>,
        ready: mpsc::Sender<Result<(), String>>,
    ) {
        let mut env = match vm.attach_current_thread_permanently() {
            Ok(env) => env,
            Err(err) => {
                let _ = ready.send(Err(err.to_string()));
                return;
            }
        };
        let started = new_bridge(&mut env, "dev.dexdrop.NsdBridge").and_then(|bridge| {
            let ty = env.new_string(nsd_type())?;
            env.call_method(
                &bridge,
                "browse",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&ty)],
            )?;
            Ok(bridge)
        });
        let bridge = match started {
            Ok(bridge) => bridge,
            Err(err) => {
                let message = describe(&mut env, err);
                let _ = ready.send(Err(message));
                return;
            }
        };
        let _ = ready.send(Ok(()));
        tracing::info!("discovery: using the system's NsdManager");

        let mut services = Services::default();
        loop {
            // Commands first, then up to 250 ms of events.
            loop {
                let command = match commands.try_recv() {
                    Ok(command) => command,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        let _ = env.call_method(&bridge, "stop", "()V", &[]);
                        return;
                    }
                };
                let result = env.with_local_frame(8, |env| -> jni::errors::Result<()> {
                    match command {
                        Command::Advertise { name, port, txt } => {
                            let name = env.new_string(name)?;
                            let ty = env.new_string(nsd_type())?;
                            let txt = env.new_string(txt)?;
                            env.call_method(
                                &bridge,
                                "advertise",
                                "(Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;)V",
                                &[
                                    JValue::Object(&name),
                                    JValue::Object(&ty),
                                    JValue::Int(port.into()),
                                    JValue::Object(&txt),
                                ],
                            )?;
                        }
                        Command::Withdraw => {
                            env.call_method(&bridge, "withdraw", "()V", &[])?;
                        }
                    }
                    Ok(())
                });
                if let Err(err) = result {
                    let message = describe(&mut env, err);
                    tracing::warn!("discovery: {message}");
                }
            }
            let line = env.with_local_frame(4, |env| -> jni::errors::Result<Option<String>> {
                let line = env
                    .call_method(
                        &bridge,
                        "poll",
                        "(I)Ljava/lang/String;",
                        &[JValue::Int(250)],
                    )?
                    .l()?;
                if line.is_null() {
                    return Ok(None);
                }
                Ok(Some(env.get_string(&JString::from(line))?.into()))
            });
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    let message = describe(&mut env, err);
                    let _ = events.send(DiscoveryEvent::Unavailable(message));
                    return;
                }
            };
            let Some(event) = line.as_deref().and_then(parse_event) else {
                if events.is_closed() {
                    let _ = env.call_method(&bridge, "stop", "()V", &[]);
                    return;
                }
                continue;
            };
            if let Some(event) = services.event(own_id, event)
                && events.send(event).is_err()
            {
                let _ = env.call_method(&bridge, "stop", "()V", &[]);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{Advertisement, DiscoveryEvent, txt_entries};

    fn ad(name: &str, code_open: bool) -> (PeerId, String) {
        let id = dexdrop_core::Identity::generate().unwrap().peer_id();
        let ad = Advertisement {
            peer_id: id,
            name: name.into(),
            platform: "android".into(),
            port: 4242,
            code_open,
            avatar: None,
        };
        (id, encode_txt(&txt_entries(&ad)))
    }

    #[test]
    fn the_service_type_is_in_nsd_form() {
        assert_eq!(nsd_type(), "_dexdrop._tcp");
    }

    #[test]
    fn a_resolved_service_is_read() {
        let (id, txt) = ad("Pixel 8", true);
        let line = format!("F{SEP}dexdrop-abc{SEP}192.168.1.20{SEP}4242{SEP}{txt}");
        let Some(BridgeEvent::Found { service, found }) = parse_event(&line) else {
            panic!("not read: {line:?}");
        };
        assert_eq!(service, "dexdrop-abc");
        assert_eq!(found.peer_id, id);
        assert_eq!(found.name, "Pixel 8");
        assert_eq!(found.platform, "android");
        assert!(found.code_open);
        assert_eq!(found.addrs, vec!["192.168.1.20:4242".parse().unwrap()]);
    }

    #[test]
    fn every_usable_address_is_kept_ipv4_first() {
        let (_, txt) = ad("Pixel 8", false);
        let line = format!(
            "F{SEP}s{SEP}fe80::1,fec0::5054:ff:fe12:3456,10.0.2.16,bogus{SEP}4242{SEP}{txt}"
        );
        let Some(BridgeEvent::Found { found, .. }) = parse_event(&line) else {
            panic!("not read");
        };
        assert_eq!(
            found.addrs,
            vec![
                "10.0.2.16:4242".parse().unwrap(),
                "[fec0::5054:ff:fe12:3456]:4242".parse().unwrap()
            ]
        );
        // Only a link-local address: nothing to connect to.
        let line = format!("F{SEP}s{SEP}fe80::1{SEP}4242{SEP}{txt}");
        assert_eq!(parse_event(&line), None);
    }

    #[test]
    fn control_characters_in_a_name_cannot_break_the_record() {
        let (_, txt) = ad("Bad\u{1}Name\u{2}c=1\nX", false);
        let line = format!("F{SEP}s{SEP}10.0.0.2{SEP}1{SEP}{txt}");
        let Some(BridgeEvent::Found { found, .. }) = parse_event(&line) else {
            panic!("not read");
        };
        assert_eq!(found.name, "BadNamec=1X");
        assert!(!found.code_open);
    }

    #[test]
    fn malformed_events_are_ignored() {
        for line in [
            "",
            "X",
            "F",
            &format!("F{SEP}s{SEP}not-an-ip{SEP}1{SEP}"),
            &format!("F{SEP}s{SEP}10.0.0.2{SEP}99999{SEP}"),
            // Not a DexDrop record.
            &format!("F{SEP}s{SEP}10.0.0.2{SEP}1{SEP}foo=bar"),
        ] {
            assert_eq!(parse_event(line), None, "{line:?}");
        }
        assert_eq!(
            parse_event(&format!("L{SEP}s")),
            Some(BridgeEvent::Lost {
                service: "s".into()
            })
        );
        assert_eq!(
            parse_event(&format!("E{SEP}no")),
            Some(BridgeEvent::Failed("no".into()))
        );
    }

    #[test]
    fn found_and_lost_map_to_peers_and_skip_this_device() {
        let (own, own_txt) = ad("Me", false);
        let (other, other_txt) = ad("Other", false);
        let found = |service: &str, txt: &str| {
            parse_event(&format!("F{SEP}{service}{SEP}10.0.0.2{SEP}1{SEP}{txt}")).unwrap()
        };
        let lost = |service: &str| BridgeEvent::Lost {
            service: service.into(),
        };
        let mut services = Services::default();
        assert!(services.event(own, found("me", &own_txt)).is_none());
        assert!(matches!(
            services.event(own, found("a", &other_txt)),
            Some(DiscoveryEvent::Found(d)) if d.peer_id == other
        ));
        // The same device under a second name (renamed after a conflict).
        assert!(services.event(own, found("a (2)", &other_txt)).is_some());
        assert!(services.event(own, lost("a")).is_none());
        assert!(matches!(
            services.event(own, lost("a (2)")),
            Some(DiscoveryEvent::Lost(id)) if id == other
        ));
        assert!(services.event(own, lost("unknown")).is_none());
    }
}
