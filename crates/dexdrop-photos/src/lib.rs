//! The iOS photo library (PhotoKit) for Send's Recent: the newest photos
//! and videos with small thumbnails, and each one's original file, exported
//! when the user picks it.
//!
//! NSH's `nsh-extra` `photos` hands over photos re-encoded as JPEGs (as a
//! picked image) and no videos; a file transfer sends what the library
//! holds, so DexDrop reads PhotoKit itself: `PHAsset` for the list,
//! `PHImageManager` for thumbnails, `PHAssetResourceManager` for the
//! original (the edited version when the photo was edited, as Photos shows
//! it; downloaded from iCloud when only there).
//!
//! Every call blocks until PhotoKit answers: call them off the UI thread.
//! Elsewhere than iOS there is no library: [`access`] is
//! [`Access::Unavailable`] and the rest is empty.

use std::path::{Path, PathBuf};

#[cfg(target_os = "ios")]
mod ios;

/// What the user lets DexDrop read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The whole library.
    Full,
    /// The photos the user selected ("Limited Access").
    Limited,
    /// Not asked yet: [`request_access`] shows the system's prompt.
    NotDetermined,
    /// Refused, or restricted by the device's owner: only the system's
    /// settings change it.
    Denied,
    /// No photo library here (not iOS).
    Unavailable,
}

/// One photo or video of the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    /// PhotoKit's `localIdentifier`.
    pub id: String,
    /// The original file's name (`IMG_0042.HEIC`).
    pub name: String,
    pub video: bool,
    /// When it was taken (or added), seconds since 1970.
    pub created: u64,
    /// A video's length in milliseconds (0 for photos).
    pub duration_ms: u64,
    /// A small JPEG of it, when one could be made.
    pub thumb: Option<PathBuf>,
}

/// What the user lets DexDrop read now, without asking.
pub fn access() -> Access {
    #[cfg(target_os = "ios")]
    return ios::access();
    #[cfg(not(target_os = "ios"))]
    Access::Unavailable
}

/// Asks the user with the system's prompt the first time (the app's
/// `NSPhotoLibraryUsageDescription` says why) and returns the answer; later
/// calls return [`access`] at once.
pub fn request_access() -> Access {
    #[cfg(target_os = "ios")]
    return ios::request_access();
    #[cfg(not(target_os = "ios"))]
    Access::Unavailable
}

/// With Limited Access: shows the system's sheet where the user selects
/// more photos. Returns at once; [`recent`] reads the new selection later.
pub fn choose_more() {
    #[cfg(target_os = "ios")]
    ios::choose_more();
}

/// The newest `limit` photos and videos, newest first, without the hidden
/// ones, each with a thumbnail of about 256 px in `thumb_dir` (made once).
pub fn recent(limit: usize, thumb_dir: &Path) -> Vec<Asset> {
    #[cfg(target_os = "ios")]
    return ios::recent(limit, thumb_dir);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (limit, thumb_dir);
        Vec::new()
    }
}

/// Writes the original file of the asset `id` into `dir` (which must
/// exist), under its own name, and returns its path. Downloads it from
/// iCloud when the device has only a preview.
pub fn export(id: &str, dir: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "ios")]
    return ios::export(id, dir);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = (id, dir);
        Err("this device has no photo library".into())
    }
}

/// The name a thumbnail of `id` is kept under: `id` with only characters
/// file names take (`localIdentifier`s carry slashes).
pub fn thumb_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.jpg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_are_named_safely() {
        assert_eq!(
            thumb_name("9F983DBA-EC35-42B8-8773-B597CF782EDD/L0/001"),
            "9F983DBA-EC35-42B8-8773-B597CF782EDD_L0_001.jpg"
        );
    }

    #[test]
    fn there_is_no_library_off_ios() {
        if cfg!(target_os = "ios") {
            return;
        }
        assert_eq!(access(), Access::Unavailable);
        assert!(recent(10, Path::new(".")).is_empty());
        assert!(export("x", Path::new(".")).is_err());
    }
}
