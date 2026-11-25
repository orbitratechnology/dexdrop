//! Android: what the app asks of the system through NSH's Android backend
//! (`nsh-platform-android`, whose library is `nsh`): the runtime
//! permissions to read photos and videos, asked for and answered from Rust
//! with the activity in front (no Java of DexDrop's own).

use dexdrop_net::media::{self, Access};
use nsh::permissions::{self, Permission};

/// The permissions this device's Android version has for photos and videos.
fn names() -> &'static [&'static str] {
    media::android_permissions(media::android_api_level())
}

fn granted(states: &[Permission]) -> Vec<bool> {
    states.iter().map(|s| *s == Permission::Granted).collect()
}

/// What the app may read of the photos and videos now, without asking.
pub fn photo_access() -> Access {
    match permissions::states(names()) {
        Ok(states) => media::android_access(names(), &granted(&states)),
        Err(err) => {
            log::warn!("photos: {err}");
            Access::None
        }
    }
}

/// Whether asking shows the system's prompt: some of the permissions are
/// neither granted nor refused for good (after refusals only the system's
/// settings change them).
pub fn can_ask_photos() -> bool {
    permissions::states(names()).is_ok_and(|states| states.contains(&Permission::Default))
}

/// Asks the user for the photos and videos with the system's prompt and
/// answers with what they allow then.
pub async fn ask_photos() -> Result<Access, String> {
    permissions::request(names())
        .await
        .map(|states| media::android_access(names(), &granted(&states)))
        .map_err(|err| err.to_string())
}
