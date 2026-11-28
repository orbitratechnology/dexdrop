//! Linux's direct link, through NetworkManager on the system bus: hosting a
//! WPA2 access point for one transfer (5 GHz, then 2.4 GHz), with
//! NetworkManager's shared IPv4 (its own DHCP; this computer is
//! 10.42.0.1/24), and joining such a network as an ordinary Wi-Fi client.
//!
//! Both are connections DexDrop adds for the occasion and deletes when it
//! is over, so nothing stays in the user's network settings. A computer
//! with one Wi-Fi radio leaves its own network while it hosts or joins,
//! and NetworkManager goes back to it afterwards. Desktop sessions may
//! change networks without a password (NetworkManager's polkit defaults);
//! elsewhere NetworkManager refuses and the reason is shown.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use super::{Band, DirectHint, DirectLink, Hosted, Joined, new_credentials};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE_TYPE_WIFI: u32 = 2;
/// `NM_WIFI_DEVICE_CAP_AP`, `NM_WIFI_DEVICE_CAP_FREQ_5GHZ`.
const CAP_AP: u32 = 0x40;
const CAP_5GHZ: u32 = 0x400;
/// `NM_ACTIVE_CONNECTION_STATE_ACTIVATED`; above it, going down.
const ACTIVATED: u32 = 2;
/// How long a network may take to come up (an access point's 5 GHz
/// channel check can take seconds).
const COME_UP: Duration = Duration::from_secs(40);

type Settings<'a> = HashMap<&'a str, HashMap<&'a str, Value<'a>>>;

/// The settings of an access point DexDrop hosts: WPA2 (RSN, CCMP) with
/// `passphrase`, on `band` (`a`: 5 GHz, `bg`: 2.4 GHz), IPv4 shared.
fn access_point<'a>(ssid: &'a str, passphrase: &'a str, band: &'a str) -> Settings<'a> {
    let mut s = network(ssid, passphrase, "ap");
    if let Some(wifi) = s.get_mut("802-11-wireless") {
        wifi.insert("band", Value::from(band));
    }
    s.insert("ipv4", HashMap::from([("method", Value::from("shared"))]));
    s.insert("ipv6", HashMap::from([("method", Value::from("ignore"))]));
    s
}

/// The settings for joining a network another device hosts.
fn client<'a>(ssid: &'a str, passphrase: &'a str) -> Settings<'a> {
    let mut s = network(ssid, passphrase, "infrastructure");
    s.insert("ipv4", HashMap::from([("method", Value::from("auto"))]));
    s.insert("ipv6", HashMap::from([("method", Value::from("ignore"))]));
    s
}

fn network<'a>(ssid: &'a str, passphrase: &'a str, mode: &'a str) -> Settings<'a> {
    HashMap::from([
        (
            "connection",
            HashMap::from([
                ("id", Value::from("DexDrop direct link")),
                ("type", Value::from("802-11-wireless")),
                // Only for this transfer: never joined again on its own.
                ("autoconnect", Value::from(false)),
            ]),
        ),
        (
            "802-11-wireless",
            HashMap::from([
                ("ssid", Value::from(ssid.as_bytes().to_vec())),
                ("mode", Value::from(mode)),
            ]),
        ),
        (
            "802-11-wireless-security",
            HashMap::from([
                ("key-mgmt", Value::from("wpa-psk")),
                ("psk", Value::from(passphrase)),
                ("proto", Value::from(vec!["rsn"])),
                ("pairwise", Value::from(vec!["ccmp"])),
                ("group", Value::from(vec!["ccmp"])),
            ]),
        ),
    ])
}

/// What NetworkManager answered, said for the user.
fn explain(err: zbus::Error) -> String {
    let text = err.to_string();
    if text.contains("NotAuthorized") || text.contains("PermissionDenied") {
        "This computer does not let DexDrop change its Wi-Fi.".into()
    } else if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        "NetworkManager is not running.".into()
    } else {
        format!("NetworkManager: {text}")
    }
}

/// A connection DexDrop added and activated.
struct Activated {
    settings: OwnedObjectPath,
    active: OwnedObjectPath,
}

/// NetworkManager's Wi-Fi, for hosting and joining.
#[derive(Debug, Default)]
pub struct NetworkManagerLink {
    /// The Wi-Fi device and its capabilities, found once.
    wifi: OnceLock<Option<(OwnedObjectPath, u32)>>,
    /// The access point while it is up.
    hosting: Mutex<Option<(OwnedObjectPath, OwnedObjectPath)>>,
}

impl NetworkManagerLink {
    fn wifi(&self) -> Option<&(OwnedObjectPath, u32)> {
        self.wifi
            .get_or_init(|| match find_wifi() {
                Ok(found) => found,
                Err(err) => {
                    tracing::debug!("direct link: {}", explain(err));
                    None
                }
            })
            .as_ref()
    }
}

fn nm(conn: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(conn, NM, NM_PATH, NM)
}

/// The first Wi-Fi device and its capabilities (`WirelessCapabilities`).
fn find_wifi() -> zbus::Result<Option<(OwnedObjectPath, u32)>> {
    let conn = Connection::system()?;
    let devices: Vec<OwnedObjectPath> = nm(&conn)?.call("GetDevices", &())?;
    for path in devices {
        let device = Proxy::new(
            &conn,
            NM,
            path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )?;
        if device.get_property::<u32>("DeviceType")? != DEVICE_TYPE_WIFI {
            continue;
        }
        let wireless = Proxy::new(
            &conn,
            NM,
            path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
        )?;
        let caps = wireless.get_property::<u32>("WirelessCapabilities")?;
        return Ok(Some((path.clone(), caps)));
    }
    Ok(None)
}

/// Adds `settings` as a connection, activates it on `device` and waits
/// until it is up (or deletes it and says why not).
fn activate(device: &OwnedObjectPath, settings: &Settings<'_>) -> Result<Activated, String> {
    let conn = Connection::system().map_err(explain)?;
    let root = OwnedObjectPath::try_from("/").expect("a valid path");
    let (path, active): (OwnedObjectPath, OwnedObjectPath) = nm(&conn)
        .map_err(explain)?
        .call("AddAndActivateConnection", &(settings, device, &root))
        .map_err(explain)?;
    let made = Activated {
        settings: path,
        active,
    };
    let proxy = Proxy::new(
        &conn,
        NM,
        made.active.clone(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .map_err(explain)?;
    let deadline = Instant::now() + COME_UP;
    loop {
        match proxy.get_property::<u32>("State") {
            Ok(ACTIVATED) => return Ok(made),
            Ok(state) if state > ACTIVATED => break,
            // The active connection went away: it failed.
            Err(_) => break,
            Ok(_) => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        // NetworkManager's activation, polled: this runs on a blocking
        // thread of the engine's, never the UI's.
        std::thread::sleep(Duration::from_millis(200));
    }
    remove(&made.settings, &made.active);
    Err("The Wi-Fi network did not come up.".into())
}

/// This computer's IPv4 address and prefix on an active connection.
fn address(active: &OwnedObjectPath) -> Result<(IpAddr, u8), String> {
    let conn = Connection::system().map_err(explain)?;
    let proxy = Proxy::new(
        &conn,
        NM,
        active.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .map_err(explain)?;
    let config: OwnedObjectPath = proxy.get_property("Ip4Config").map_err(explain)?;
    let ip4 = Proxy::new(
        &conn,
        NM,
        config.as_str(),
        "org.freedesktop.NetworkManager.IP4Config",
    )
    .map_err(explain)?;
    let data: Vec<HashMap<String, OwnedValue>> =
        ip4.get_property("AddressData").map_err(explain)?;
    first_address(&data).ok_or_else(|| "The Wi-Fi network has no address.".into())
}

/// The first `address`/`prefix` pair of NetworkManager's `AddressData`.
fn first_address(data: &[HashMap<String, OwnedValue>]) -> Option<(IpAddr, u8)> {
    data.iter().find_map(|entry| {
        let ip = String::try_from(entry.get("address")?.try_clone().ok()?).ok()?;
        let prefix = u32::try_from(entry.get("prefix")?.try_clone().ok()?).ok()?;
        Some((ip.parse().ok()?, u8::try_from(prefix).ok()?))
    })
}

/// Takes a connection down and deletes it.
fn remove(settings: &OwnedObjectPath, active: &OwnedObjectPath) {
    let Ok(conn) = Connection::system() else {
        return;
    };
    if let Ok(nm) = nm(&conn) {
        let _: zbus::Result<()> = nm.call("DeactivateConnection", &(active,));
    }
    if let Ok(proxy) = Proxy::new(
        &conn,
        NM,
        settings.as_str(),
        "org.freedesktop.NetworkManager.Settings.Connection",
    ) {
        let _: zbus::Result<()> = proxy.call("Delete", &());
    }
}

impl DirectLink for NetworkManagerLink {
    fn can_host(&self) -> bool {
        self.wifi().is_some_and(|(_, caps)| caps & CAP_AP != 0)
    }

    fn can_join(&self) -> bool {
        self.wifi().is_some()
    }

    fn host(&self) -> Result<Hosted, String> {
        let (device, caps) = self.wifi().ok_or("This computer has no Wi-Fi.")?.clone();
        let (ssid, passphrase) = new_credentials();
        let bands: &[(&str, Band)] = if caps & CAP_5GHZ != 0 {
            &[("a", Band::Ghz5), ("bg", Band::Ghz2_4)]
        } else {
            &[("bg", Band::Ghz2_4)]
        };
        let mut why = String::from("The Wi-Fi network did not come up.");
        for (band, named) in bands {
            match activate(&device, &access_point(&ssid, &passphrase, band)) {
                Ok(up) => {
                    let (ip, prefix) = match address(&up.active) {
                        Ok(found) => found,
                        Err(err) => {
                            remove(&up.settings, &up.active);
                            return Err(err);
                        }
                    };
                    *self.hosting.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((up.settings, up.active));
                    return Ok(Hosted {
                        ssid,
                        passphrase,
                        band: *named,
                        ip,
                        prefix,
                    });
                }
                // Not allowed to change the Wi-Fi: another band will not help.
                Err(err) if err.starts_with("This computer does not") => return Err(err),
                Err(err) => why = err,
            }
        }
        Err(why)
    }

    fn stop(&self) {
        if let Some((settings, active)) = self
            .hosting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            remove(&settings, &active);
        }
    }

    fn join(&self, hint: &DirectHint) -> Result<Box<dyn Joined>, String> {
        let (device, _) = self.wifi().ok_or("This computer has no Wi-Fi.")?.clone();
        let up = activate(&device, &client(&hint.ssid, &hint.passphrase))?;
        Ok(Box::new(NetworkManagerJoined(up)))
    }
}

/// A network joined for one transfer. Its subnet routes to the host, so
/// sockets need no binding; dropping it leaves and deletes it.
struct NetworkManagerJoined(Activated);

impl Joined for NetworkManagerJoined {}

impl Drop for NetworkManagerJoined {
    fn drop(&mut self) {
        remove(&self.0.settings, &self.0.active);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The access point is WPA2 with DexDrop's credentials, on the band
    /// asked, sharing IPv4 (NetworkManager's DHCP), and never comes back on
    /// its own.
    #[test]
    fn an_access_point_for_one_transfer() {
        let s = access_point("DIRECT-K7-DexDrop", "QX9MK7P2RT4VQX9M", "a");
        let wifi = &s["802-11-wireless"];
        assert_eq!(wifi["mode"], Value::from("ap"));
        assert_eq!(wifi["band"], Value::from("a"));
        assert_eq!(wifi["ssid"], Value::from(b"DIRECT-K7-DexDrop".to_vec()));
        let security = &s["802-11-wireless-security"];
        assert_eq!(security["key-mgmt"], Value::from("wpa-psk"));
        assert_eq!(security["psk"], Value::from("QX9MK7P2RT4VQX9M"));
        assert_eq!(s["ipv4"]["method"], Value::from("shared"));
        assert_eq!(s["connection"]["autoconnect"], Value::from(false));

        let join = client("DIRECT-K7-DexDrop", "QX9MK7P2RT4VQX9M");
        assert_eq!(
            join["802-11-wireless"]["mode"],
            Value::from("infrastructure")
        );
        assert!(!join["802-11-wireless"].contains_key("band"));
        assert_eq!(join["ipv4"]["method"], Value::from("auto"));
    }

    #[test]
    fn the_hosts_address_is_read_from_address_data() {
        let entry = HashMap::from([
            (
                "address".to_owned(),
                OwnedValue::try_from(Value::from("10.42.0.1")).unwrap(),
            ),
            (
                "prefix".to_owned(),
                OwnedValue::try_from(Value::from(24u32)).unwrap(),
            ),
        ]);
        assert_eq!(
            first_address(&[entry]),
            Some(("10.42.0.1".parse().unwrap(), 24))
        );
        assert_eq!(first_address(&[]), None);
    }

    /// Without NetworkManager (CI's runners), the link can do nothing and
    /// says so, instead of failing the engine.
    #[test]
    fn without_network_manager_nothing_is_offered() {
        let link = NetworkManagerLink::default();
        if link.wifi().is_none() {
            assert!(!link.can_host() && !link.can_join());
        }
    }
}
