//! Android: what the app asks of the system through NSH's Android backend
//! (`nsh-platform-android`, whose library is `nsh`): the runtime
//! permissions to read photos, videos and audio for Send's media browser
//! (`pages::library`), asked for and answered from Rust with the activity in
//! front.

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
