//! Android: what the app asks of the system through NSH's Android backend
//! (`nsh-platform-android`, whose library is `nsh`): the runtime
//! permissions to read photos, videos and audio for Send's media browser
//! (`pages::library`), and to host a direct link (`pages::receive`), asked
//! for and answered from Rust with the activity in front.

use dexdrop_net::assets::{self, Access};
use nsh::permissions::{self, Permission};

/// The permissions this device's Android version has for media.
fn names() -> &'static [&'static str] {
    assets::android_permissions(assets::android_api_level())
}

fn granted(states: &[Permission]) -> Vec<bool> {
    states.iter().map(|s| *s == Permission::Granted).collect()
}

/// What the app may read of the media now, without asking.
pub fn media_access() -> Access {
    match permissions::states(names()) {
        Ok(states) => assets::android_access(names(), &granted(&states)),
        Err(err) => {
            log::warn!("media: {err}");
            Access::None
        }
    }
}

/// Asks with the system's prompt (Android 14: all, some, or none) and
/// answers with what the user allows then.
pub async fn ask_media() -> Result<Access, String> {
    permissions::request(names())
        .await
        .map(|states| assets::android_access(names(), &granted(&states)))
        .map_err(|err| err.to_string())
}

/// What hosting a direct link (a Wi-Fi Direct group) needs: nearby Wi-Fi
/// devices from Android 13 (declared never for location), fine location
/// before (the manifest limits it to Android 12).
fn direct_link_permissions() -> &'static [&'static str] {
    if assets::android_api_level() >= 33 {
        &["android.permission.NEARBY_WIFI_DEVICES"]
    } else {
        &["android.permission.ACCESS_FINE_LOCATION"]
    }
}

/// Asks for what hosting a direct link needs, unless it is allowed
/// already; whether it is allowed now.
pub async fn ask_direct_link() -> bool {
    let names = direct_link_permissions();
    if permissions::states(names).is_ok_and(|s| s.iter().all(|s| *s == Permission::Granted)) {
        return true;
    }
    match permissions::request(names).await {
        Ok(states) => states.iter().all(|s| *s == Permission::Granted),
        Err(err) => {
            log::warn!("direct link: {err}");
            false
        }
    }
}
