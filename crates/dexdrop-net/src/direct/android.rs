//! Android's direct link: a Wi-Fi Direct group this phone hosts (5 GHz,
//! then 2.4 GHz when 5 GHz is refused), and a network another device
//! hosts joined with `WifiNetworkSpecifier` (Android 10+), through
//! `android/dev/dexdrop/DirectLinkBridge.java`.
//!
//! A joined network has no internet and is not the process's default, so
//! the engine's sockets to the host are bound to it with the NDK's
//! `android_setsocknetwork` and the network's handle.

//!
//! The bridge's answers are read everywhere (and tested on every CI host);
//! the calls exist only on Android.

use std::net::IpAddr;

use super::{Band, Hosted};
#[cfg(target_os = "android")]
pub use device::AndroidLink;

/// What the bridge answered: `ok` and fields, or `error` and why.
fn answer(reply: Result<String, String>) -> Result<Vec<String>, String> {
    let reply = reply?;
    let mut lines = reply.split('\n').map(str::to_owned);
    match lines.next().as_deref() {
        Some("ok") => Ok(lines.collect()),
        Some("error") => Err(match lines.next().as_deref() {
            Some("permission") => "DexDrop may not use nearby Wi-Fi devices. Allow it in \
                                   Settings › Apps › DexDrop › Permissions."
                .to_owned(),
            Some(why) => why.to_owned(),
            None => "Android refused.".to_owned(),
        }),
        _ => Err(format!("Unexpected answer from Android: {reply}")),
    }
}

/// The bridge's `ok` fields for a hosted group: name, passphrase, band,
/// this device's address (a Wi-Fi Direct group is a /24).
fn hosted(fields: &[String]) -> Result<Hosted, String> {
    let [ssid, passphrase, band, ip] = fields else {
        return Err("Android described the direct link oddly.".into());
    };
    Ok(Hosted {
        ssid: ssid.clone(),
        passphrase: passphrase.clone(),
        band: match band.as_str() {
            "5" => Band::Ghz5,
            "2.4" => Band::Ghz2_4,
            _ => Band::Auto,
        },
        ip: ip
            .parse::<IpAddr>()
            .map_err(|_| format!("Android gave an odd address: {ip}"))?,
        prefix: 24,
    })
}

#[cfg(target_os = "android")]
mod device {
    use std::{os::fd::AsRawFd, sync::OnceLock};

    use super::{answer, hosted};
    use crate::direct::{DirectHint, DirectLink, Hosted, Joined, new_credentials};
    use crate::jvm::Bridge;

    static BRIDGE: Bridge = Bridge::new("dev.dexdrop.DirectLinkBridge");

    /// Android's Wi-Fi Direct and Wi-Fi network requests.
    #[derive(Debug, Default)]
    pub struct AndroidLink {
        /// `(can_host, can_join)`, asked once.
        capabilities: OnceLock<(bool, bool)>,
    }

    impl AndroidLink {
        fn capabilities(&self) -> (bool, bool) {
            *self
                .capabilities
                .get_or_init(|| match BRIDGE.call("capabilities", &[]) {
                    Ok(text) => {
                        let mut parts = text.split(',');
                        (parts.next() == Some("host"), parts.next() == Some("join"))
                    }
                    Err(err) => {
                        tracing::warn!("direct link: {err}");
                        (false, false)
                    }
                })
        }
    }

    impl DirectLink for AndroidLink {
        fn can_host(&self) -> bool {
            self.capabilities().0
        }

        fn can_join(&self) -> bool {
            self.capabilities().1
        }

        fn host(&self) -> Result<Hosted, String> {
            let (ssid, passphrase) = new_credentials();
            // 5 GHz first: faster, and a phone's 2.4 GHz is often crowded; a
            // device or country that does not allow it gets 2.4 GHz.
            let mut why = String::new();
            for band in ["5", "2.4"] {
                match answer(BRIDGE.call("host", &[&ssid, &passphrase, band])) {
                    Ok(fields) => return hosted(&fields),
                    Err(err) if err.starts_with("DexDrop may not") => return Err(err),
                    Err(err) => why = err,
                }
            }
            Err(why)
        }

        fn stop(&self) {
            if let Err(err) = answer(BRIDGE.call("stop", &[])) {
                tracing::warn!("direct link: stopping: {err}");
            }
        }

        fn join(&self, hint: &DirectHint) -> Result<Box<dyn Joined>, String> {
            let fields = answer(BRIDGE.call("join", &[&hint.ssid, &hint.passphrase]))?;
            let handle = fields
                .first()
                .and_then(|h| h.parse::<u64>().ok())
                .ok_or("Android named no network.")?;
            Ok(Box::new(AndroidJoined { handle }))
        }
    }

    /// A network joined for one transfer; dropping it leaves the network.
    struct AndroidJoined {
        /// `Network.getNetworkHandle()`.
        handle: u64,
    }

    #[link(name = "android")]
    unsafe extern "C" {
        /// `<android/multinetwork.h>` (API 23+): binds a socket to a network.
        fn android_setsocknetwork(network: u64, fd: std::ffi::c_int) -> std::ffi::c_int;
    }

    impl Joined for AndroidJoined {
        fn prepare(&self, socket: &socket2::Socket) -> std::io::Result<()> {
            // SAFETY: a valid socket and a handle Android gave for a network
            // this process requested.
            if unsafe { android_setsocknetwork(self.handle, socket.as_raw_fd()) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
    }

    impl Drop for AndroidJoined {
        fn drop(&mut self) {
            let _ = BRIDGE.call("leave", &[&self.handle.to_string()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read() {
        let fields = answer(Ok("ok\nDIRECT-k7-DexDrop\nQX9MK7P2\n5\n192.168.49.1".into())).unwrap();
        let group = hosted(&fields).unwrap();
        assert_eq!(group.band, Band::Ghz5);
        assert!(group.contains("192.168.49.7".parse().unwrap()));
        assert!(
            answer(Ok("error\npermission".into()))
                .unwrap_err()
                .contains("Settings")
        );
        assert_eq!(
            answer(Ok("error\nTurn on Wi-Fi first.".into())).unwrap_err(),
            "Turn on Wi-Fi first."
        );
    }
}
