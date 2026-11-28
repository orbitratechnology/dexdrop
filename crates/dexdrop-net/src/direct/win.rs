//! Windows' direct link: hosting with the Wi-Fi Direct advertisement
//! publisher in legacy mode, a soft access point with DexDrop's name and
//! passphrase that any device joins as an ordinary WPA2 network (Windows
//! chooses its band and serves its addresses, usually 192.168.137.x), and
//! joining such a network with the WLAN API and a profile made for it.
//!
//! The profile is deleted when the transfer is over, and Windows goes back
//! to the network it was on. Windows 11 may ask that desktop apps have
//! location access before they change Wi-Fi; a refusal says where.
//!
//! The profile's XML and the address checks are written everywhere (and
//! tested on every CI host); the calls exist only on Windows.

use std::net::{IpAddr, Ipv4Addr};

#[cfg(windows)]
pub use device::WindowsLink;

/// `text` for an XML element's content.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// A WLAN profile for joining a WPA2 (AES) network by hand, never on its
/// own: the network a device hosts for one transfer.
fn profile_xml(ssid: &str, passphrase: &str) -> String {
    let (ssid, passphrase) = (escape(ssid), escape(passphrase));
    format!(
        r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>{ssid}</name>
  <SSIDConfig><SSID><name>{ssid}</name></SSID></SSIDConfig>
  <connectionType>ESS</connectionType>
  <connectionMode>manual</connectionMode>
  <MSM>
    <security>
      <authEncryption>
        <authentication>WPA2PSK</authentication>
        <encryption>AES</encryption>
        <useOneX>false</useOneX>
      </authEncryption>
      <sharedKey>
        <keyType>passPhrase</keyType>
        <protected>false</protected>
        <keyMaterial>{passphrase}</keyMaterial>
      </sharedKey>
    </security>
  </MSM>
</WLANProfile>
"#
    )
}

/// Whether `ip` is on the same /24 as `host`: this computer has an
/// address on the joined network (its DHCP answered).
fn same_network(ip: Ipv4Addr, host: IpAddr) -> bool {
    match host {
        IpAddr::V4(host) => ip.octets()[..3] == host.octets()[..3],
        IpAddr::V6(_) => false,
    }
}

/// The IPv4 address that appeared: the soft access point's, where this
/// computer hosts.
fn appeared(before: &[Ipv4Addr], now: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    now.iter()
        .find(|ip| !before.contains(ip) && !ip.is_loopback() && !ip.is_link_local())
        .copied()
}

#[cfg(windows)]
mod device {
    use std::{
        net::{IpAddr, Ipv4Addr},
        sync::{Mutex, mpsc},
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    use windows::{
        Devices::WiFiDirect::{
            WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher,
            WiFiDirectAdvertisementPublisherStatus,
        },
        Security::Credentials::PasswordCredential,
        Win32::{
            Foundation::HANDLE,
            NetworkManagement::WiFi::{
                WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST, WlanCloseHandle, WlanConnect,
                WlanDeleteProfile, WlanDisconnect, WlanEnumInterfaces, WlanFreeMemory,
                WlanOpenHandle, WlanSetProfile, dot11_BSS_type_infrastructure,
                wlan_connection_mode_profile, wlan_interface_state_connected,
            },
        },
        core::{GUID, HSTRING, PCWSTR},
    };

    use super::{appeared, profile_xml, same_network};
    use crate::direct::{Band, DirectHint, DirectLink, Hosted, Joined, new_credentials};

    const ERROR_ACCESS_DENIED: u32 = 5;
    /// How long the soft access point and a join may take to come up.
    const COME_UP: Duration = Duration::from_secs(30);

    fn ipv4s() -> Vec<Ipv4Addr> {
        if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| match i.ip() {
                IpAddr::V4(ip) => Some(ip),
                IpAddr::V6(_) => None,
            })
            .collect()
    }

    fn wlan_error(what: &str, code: u32) -> String {
        if code == ERROR_ACCESS_DENIED {
            "Windows does not let DexDrop change Wi-Fi. Allow location access for desktop \
             apps in Settings › Privacy & security › Location."
                .into()
        } else {
            format!("Windows could not {what} (error {code}).")
        }
    }

    /// A WLAN client handle, closed when dropped.
    struct Wlan(HANDLE);

    impl Wlan {
        fn open() -> Result<Self, String> {
            let (mut version, mut handle) = (0u32, HANDLE::default());
            // SAFETY: out-pointers to locals.
            let code = unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) };
            if code != 0 {
                return Err(wlan_error("reach its Wi-Fi service", code));
            }
            Ok(Wlan(handle))
        }

        /// The Wi-Fi interfaces and whether each is connected.
        fn interfaces(&self) -> Vec<(GUID, bool)> {
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
            // SAFETY: a valid handle; the list is freed below.
            if unsafe { WlanEnumInterfaces(self.0, None, &mut list) } != 0 || list.is_null() {
                return Vec::new();
            }
            // SAFETY: the list holds dwNumberOfItems entries from
            // InterfaceInfo on.
            let found = unsafe {
                let count = (*list).dwNumberOfItems as usize;
                let first = (*list).InterfaceInfo.as_ptr();
                (0..count)
                    .map(|i| {
                        let info = &*first.add(i);
                        (
                            info.InterfaceGuid,
                            info.isState == wlan_interface_state_connected,
                        )
                    })
                    .collect()
            };
            // SAFETY: memory WlanEnumInterfaces gave.
            unsafe { WlanFreeMemory(list as *const _) };
            found
        }
    }

    impl Drop for Wlan {
        fn drop(&mut self) {
            // SAFETY: the handle WlanOpenHandle gave, closed once.
            unsafe { WlanCloseHandle(self.0, None) };
        }
    }

    /// Starts the publisher with DexDrop's name and passphrase and waits
    /// until Windows says it started.
    fn start_publisher(
        ssid: &str,
        passphrase: &str,
    ) -> Result<WiFiDirectAdvertisementPublisher, String> {
        let up = || -> windows::core::Result<WiFiDirectAdvertisementPublisher> {
            let publisher = WiFiDirectAdvertisementPublisher::new()?;
            let advertisement = publisher.Advertisement()?;
            let legacy = advertisement.LegacySettings()?;
            legacy.SetIsEnabled(true)?;
            legacy.SetSsid(&HSTRING::from(ssid))?;
            let credential = PasswordCredential::new()?;
            credential.SetPassword(&HSTRING::from(passphrase))?;
            legacy.SetPassphrase(&credential)?;
            advertisement.SetListenStateDiscoverability(
                WiFiDirectAdvertisementListenStateDiscoverability::Normal,
            )?;
            publisher.Start()?;
            Ok(publisher)
        };
        let publisher = up().map_err(|e| format!("Windows could not start Wi-Fi Direct: {e}"))?;
        let deadline = Instant::now() + COME_UP;
        loop {
            match publisher.Status() {
                Ok(WiFiDirectAdvertisementPublisherStatus::Started) => return Ok(publisher),
                Ok(WiFiDirectAdvertisementPublisherStatus::Aborted)
                | Ok(WiFiDirectAdvertisementPublisherStatus::Stopped) => {
                    return Err("Windows stopped the direct link: Wi-Fi is off, or this \
                                computer has no Wi-Fi Direct."
                        .into());
                }
                Err(e) => return Err(e.to_string()),
                Ok(_) if Instant::now() >= deadline => {
                    let _ = publisher.Stop();
                    return Err("The direct link did not come up.".into());
                }
                // Polled on the publisher's own thread, never the UI's.
                Ok(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// Windows' Wi-Fi Direct publisher and WLAN API.
    #[derive(Debug, Default)]
    pub struct WindowsLink {
        /// While hosting: what stops the publisher's thread, and the thread.
        hosting: Mutex<Option<(mpsc::Sender<()>, JoinHandle<()>)>>,
    }

    impl DirectLink for WindowsLink {
        fn can_host(&self) -> bool {
            self.can_join()
        }

        fn can_join(&self) -> bool {
            Wlan::open().is_ok_and(|w| !w.interfaces().is_empty())
        }

        fn host(&self) -> Result<Hosted, String> {
            self.stop();
            let (ssid, passphrase) = new_credentials();
            let before = ipv4s();
            let (started_tx, started) = mpsc::channel();
            let (stop, stopped) = mpsc::channel::<()>();
            let (name, secret) = (ssid.clone(), passphrase.clone());
            // The publisher lives on a thread of its own until stopped.
            let thread = std::thread::Builder::new()
                .name("dexdrop-wifi-direct".into())
                .spawn(move || match start_publisher(&name, &secret) {
                    Ok(publisher) => {
                        let _ = started_tx.send(Ok(()));
                        let _ = stopped.recv();
                        let _ = publisher.Stop();
                    }
                    Err(why) => {
                        let _ = started_tx.send(Err(why));
                    }
                })
                .map_err(|e| e.to_string())?;
            let result = started
                .recv_timeout(COME_UP + Duration::from_secs(5))
                .unwrap_or_else(|_| Err("Windows did not answer.".into()));
            if let Err(why) = result {
                let _ = stop.send(());
                let _ = thread.join();
                return Err(why);
            }
            *self.hosting.lock().unwrap_or_else(|e| e.into_inner()) = Some((stop, thread));
            // The soft access point's address, once Windows gives it one.
            let deadline = Instant::now() + COME_UP;
            loop {
                if let Some(ip) = appeared(&before, &ipv4s()) {
                    return Ok(Hosted {
                        ssid,
                        passphrase,
                        band: Band::Auto,
                        ip: IpAddr::V4(ip),
                        prefix: 24,
                    });
                }
                if Instant::now() >= deadline {
                    self.stop();
                    return Err("The direct link has no address.".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        fn stop(&self) {
            if let Some((stop, thread)) = self
                .hosting
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                let _ = stop.send(());
                let _ = thread.join();
            }
        }

        fn join(&self, hint: &DirectHint) -> Result<Box<dyn Joined>, String> {
            let wlan = Wlan::open()?;
            let (interface, _) = *wlan
                .interfaces()
                .first()
                .ok_or("This computer has no Wi-Fi.")?;
            let xml = HSTRING::from(profile_xml(&hint.ssid, &hint.passphrase));
            let name = HSTRING::from(hint.ssid.as_str());
            let mut reason = 0u32;
            // SAFETY: a valid handle and interface; NUL-terminated strings
            // that outlive the call.
            let code = unsafe {
                WlanSetProfile(
                    wlan.0,
                    &interface,
                    0,
                    PCWSTR(xml.as_ptr()),
                    PCWSTR::null(),
                    true,
                    None,
                    &mut reason,
                )
            };
            if code != 0 {
                return Err(wlan_error("save the direct link's network", code));
            }
            let joined = WindowsJoined {
                interface,
                profile: hint.ssid.clone(),
            };
            let params = WLAN_CONNECTION_PARAMETERS {
                wlanConnectionMode: wlan_connection_mode_profile,
                strProfile: PCWSTR(name.as_ptr()),
                pDot11Ssid: std::ptr::null_mut(),
                pDesiredBssidList: std::ptr::null_mut(),
                dot11BssType: dot11_BSS_type_infrastructure,
                dwFlags: 0,
            };
            // SAFETY: a valid handle, interface and parameters whose
            // profile name outlives the call.
            let code = unsafe { WlanConnect(wlan.0, &interface, &params, None) };
            if code != 0 {
                return Err(wlan_error("join the direct link", code));
            }
            // Connected, then an address on the host's network (its DHCP).
            let deadline = Instant::now() + COME_UP;
            loop {
                let connected = wlan
                    .interfaces()
                    .iter()
                    .any(|(id, up)| *id == interface && *up);
                if connected && ipv4s().iter().any(|ip| same_network(*ip, hint.host.ip())) {
                    return Ok(Box::new(joined));
                }
                if Instant::now() >= deadline {
                    return Err("The direct link was not joined (out of range?).".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }

    /// A network joined for one transfer; dropping it disconnects and
    /// deletes its profile, and Windows goes back to its own network.
    struct WindowsJoined {
        interface: GUID,
        profile: String,
    }

    impl Joined for WindowsJoined {}

    impl Drop for WindowsJoined {
        fn drop(&mut self) {
            let Ok(wlan) = Wlan::open() else {
                return;
            };
            let name = HSTRING::from(self.profile.as_str());
            // SAFETY: a valid handle and interface; the name outlives the
            // calls.
            unsafe {
                WlanDisconnect(wlan.0, &self.interface, None);
                WlanDeleteProfile(wlan.0, &self.interface, PCWSTR(name.as_ptr()), None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The profile joins a WPA2 (AES) network by hand, with the name and
    /// passphrase escaped for XML.
    #[test]
    fn a_profile_for_the_hosted_network() {
        let xml = profile_xml("DIRECT-K7-DexDrop", "QX9MK7P2RT4VQX9M");
        assert!(xml.contains("<name>DIRECT-K7-DexDrop</name>"));
        assert!(xml.contains("<authentication>WPA2PSK</authentication>"));
        assert!(xml.contains("<encryption>AES</encryption>"));
        assert!(xml.contains("<keyMaterial>QX9MK7P2RT4VQX9M</keyMaterial>"));
        assert!(xml.contains("<connectionMode>manual</connectionMode>"));
        let odd = profile_xml("Tom & <Jerry>", "pass\"word'1");
        assert!(odd.contains("Tom &amp; &lt;Jerry&gt;"));
        assert!(odd.contains("pass&quot;word&apos;1"));
    }

    #[test]
    fn addresses_on_the_hosted_network() {
        let host: IpAddr = "192.168.137.1".parse().unwrap();
        assert!(same_network("192.168.137.42".parse().unwrap(), host));
        assert!(!same_network("192.168.1.42".parse().unwrap(), host));
        let before = ["192.168.1.20".parse().unwrap()];
        let now = [
            "192.168.1.20".parse().unwrap(),
            "169.254.3.4".parse().unwrap(),
            "192.168.137.1".parse().unwrap(),
        ];
        assert_eq!(
            appeared(&before, &now),
            Some("192.168.137.1".parse().unwrap())
        );
        assert_eq!(appeared(&before, &before), None);
    }
}
