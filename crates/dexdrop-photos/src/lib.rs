//! The iOS photo library as DexDrop's `ios-photos` asset provider
//! (`dexdrop_core::asset`, docs/FILE_SELECTION.md):
//!
//! - **listed** a page at a time from a `PHFetchResult` (lazy: PhotoKit
//!   reads rows as they are indexed), newest or oldest first, by album;
//! - **described** without reading: the original resource's name and size
//!   (`PHAssetResource`), the asset's dates, length and pixel size;
//! - **thumbnails** from a shared `PHCachingImageManager`, written once as
//!   small JPEGs in the app's cache, local only (no iCloud download for a
//!   thumbnail);
//! - **opened** as a stream: `PHAssetResourceManager` hands the original's
//!   bytes over in pieces (downloaded from iCloud when it is there) into a
//!   bounded channel the engine reads from; dropping the reader cancels the
//!   request. Nothing is exported to a file.
//!
//! Elsewhere there is no library: [`register`] does nothing and [`access`]
//! says [`Access::NotNeeded`].

use std::path::Path;

#[cfg(target_os = "ios")]
mod ios;

/// The scheme of this provider's ids.
pub const SCHEME: &str = "ios-photos";

/// What the app may read of the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// All of it.
    All,
    /// The photos the user chose (iOS's limited access).
    Some,
    /// Not asked yet: asking shows the system's prompt.
    NotAsked,
    /// Refused (or restricted): only Settings change it.
    Refused,
    /// No photo library here.
    NotNeeded,
}

/// What the app may read now, without asking.
pub fn access() -> Access {
    #[cfg(target_os = "ios")]
    return ios::access();
    #[cfg(not(target_os = "ios"))]
    Access::NotNeeded
}

/// Asks with the system's prompt when not asked yet and waits for the
/// answer (call it off the main thread); otherwise answers at once.
pub fn request_access() -> Access {
    #[cfg(target_os = "ios")]
    return ios::request_access();
    #[cfg(not(target_os = "ios"))]
    Access::NotNeeded
}

/// Limited access: shows the system's sheet to choose more photos.
pub fn choose_more() {
    #[cfg(target_os = "ios")]
    ios::choose_more();
}

/// Registers the `ios-photos` provider; thumbnails go to `cache`.
pub fn register(cache: &Path) {
    #[cfg(target_os = "ios")]
    dexdrop_core::asset::register(std::sync::Arc::new(ios::Photos::new(
        cache.join("thumbnails"),
    )));
    #[cfg(not(target_os = "ios"))]
    let _ = cache;
}

/// A file name for a thumbnail of the asset `id` (local identifiers hold
/// `/`).
pub fn thumb_name(id: &str, px: u32) -> String {
    let clean: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{clean}-{px}.jpg")
}

/// `name` as a single file name.
pub fn sanitize(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        "photo".into()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_names_are_file_names() {
        let name = thumb_name("9F983DBA-EC35-42B8-8773-B597CF782EDD/L0/001", 256);
        assert_eq!(name, "9F983DBA_EC35_42B8_8773_B597CF782EDD_L0_001-256.jpg");
        assert_eq!(sanitize("a/b.jpg"), "a_b.jpg");
        assert_eq!(sanitize(".."), "photo");
    }

    #[test]
    fn elsewhere_there_is_no_library() {
        assert_eq!(access(), Access::NotNeeded);
        register(Path::new("/nowhere"));
        assert!(dexdrop_core::asset::provider(SCHEME).is_none());
    }
}
