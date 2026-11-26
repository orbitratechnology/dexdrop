//! The device's media and documents as asset providers
//! (`dexdrop_core::asset`, docs/FILE_SELECTION.md): listed page by page from
//! the system's index, described without reading them, opened in place when
//! a transfer starts. Nothing is copied into the app.
//!
//! - Android: `android-media` (MediaStore: photos, videos, audio; paged
//!   `ContentResolver` queries, the system's thumbnails, the original opened
//!   as a file descriptor) and `android-doc` (documents the user picks in the
//!   system's document picker, `ACTION_OPEN_DOCUMENT`, the read grant kept),
//!   through `android/dev/dexdrop/AssetsBridge.java`.
//! - iOS: `ios-photos` (PhotoKit) and `ios-doc` (the document picker, opened
//!   in place): `dexdrop-photos`, registered by the app.
//!
//! [`register_platform`] registers this platform's providers; the app calls
//! it once at start.

use std::path::{Path, PathBuf};

use dexdrop_core::asset::{AssetId, AssetSource, MediaItem, MediaType};

/// MediaStore's photos, videos and audio.
pub const ANDROID_MEDIA: &str = "android-media";
/// Documents picked in Android's document picker.
pub const ANDROID_DOC: &str = "android-doc";

/// What the app may read of the device's media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// All of it.
    All,
    /// What the user chose (Android 14's and iOS's limited access).
    Some,
    /// Not yet allowed.
    None,
    /// Nothing to allow (computers read their folders).
    NotNeeded,
}

const READ_MEDIA_IMAGES: &str = "android.permission.READ_MEDIA_IMAGES";
const READ_MEDIA_VIDEO: &str = "android.permission.READ_MEDIA_VIDEO";
const READ_MEDIA_AUDIO: &str = "android.permission.READ_MEDIA_AUDIO";
const READ_MEDIA_VISUAL_USER_SELECTED: &str = "android.permission.READ_MEDIA_VISUAL_USER_SELECTED";
const READ_EXTERNAL_STORAGE: &str = "android.permission.READ_EXTERNAL_STORAGE";

/// Android's permissions to read photos, videos and audio on API level
/// `api`: the granular `READ_MEDIA_*` from Android 13 (33), with
/// `READ_MEDIA_VISUAL_USER_SELECTED` from 14 (34: the user may allow only
/// some photos and videos), `READ_EXTERNAL_STORAGE` before. Asked with NSH's
/// `nsh::permissions`; dexdrop-net's `Cargo.toml` declares them.
pub fn android_permissions(api: u32) -> &'static [&'static str] {
    if api >= 34 {
        &[
            READ_MEDIA_IMAGES,
            READ_MEDIA_VIDEO,
            READ_MEDIA_AUDIO,
            READ_MEDIA_VISUAL_USER_SELECTED,
        ]
    } else if api >= 33 {
        &[READ_MEDIA_IMAGES, READ_MEDIA_VIDEO, READ_MEDIA_AUDIO]
    } else {
        &[READ_EXTERNAL_STORAGE]
    }
}

/// What `permissions` allow, from whether each is granted (same order).
pub fn android_access(permissions: &[&str], granted: &[bool]) -> Access {
    let mut some = false;
    for (name, granted) in permissions.iter().zip(granted) {
        if *granted {
            if *name == READ_MEDIA_VISUAL_USER_SELECTED || *name == READ_MEDIA_AUDIO {
                some = true;
            } else {
                return Access::All;
            }
        }
    }
    if some { Access::Some } else { Access::None }
}

/// The device's Android API level (`ro.build.version.sdk`); 0 when unknown
/// or elsewhere.
pub fn android_api_level() -> u32 {
    #[cfg(target_os = "android")]
    {
        let mut value = [0 as libc::c_char; 92]; // PROP_VALUE_MAX
        // SAFETY: a NUL-terminated name and a buffer of PROP_VALUE_MAX
        // bytes, as `__system_property_get` requires.
        let len = unsafe {
            libc::__system_property_get(c"ro.build.version.sdk".as_ptr(), value.as_mut_ptr())
        };
        if len <= 0 {
            return 0;
        }
        // SAFETY: the call wrote a NUL-terminated value into the buffer.
        let value = unsafe { std::ffi::CStr::from_ptr(value.as_ptr()) };
        value.to_string_lossy().trim().parse().unwrap_or(0)
    }
    #[cfg(not(target_os = "android"))]
    0
}

/// Separates a line's fields (see `AssetsBridge.java`).
const SEP: char = '\u{1}';

/// One of the bridge's lines (`token kind name mime size added modified
/// duration width height album`) as an item of `scheme`; `None` when
/// malformed.
pub fn parse_line(scheme: &str, line: &str) -> Option<MediaItem> {
    let f: Vec<&str> = line.split(SEP).collect();
    if f.len() < 11 || f[0].is_empty() {
        return None;
    }
    let name = if f[2].is_empty() {
        f[0].rsplit(['/', ':']).next().unwrap_or("item")
    } else {
        f[2]
    };
    let media_type = match f[1] {
        "image" => MediaType::Image,
        "video" => MediaType::Video,
        "audio" => MediaType::Audio,
        _ => MediaType::of(f[3], name),
    };
    let num = |s: &str| s.trim().parse::<i64>().ok();
    let positive = |s: &str| num(s).filter(|n| *n > 0);
    Some(MediaItem {
        id: AssetId::new(scheme, f[0]),
        name: name.to_owned(),
        mime_type: f[3].to_owned(),
        size: num(f[4]).and_then(|n| u64::try_from(n).ok()),
        media_type,
        source: if f[1] == "doc" {
            AssetSource::AndroidDocumentProvider
        } else {
            AssetSource::AndroidMediaStore
        },
        created_at: positive(f[5]),
        modified_at: positive(f[6]).or_else(|| positive(f[5])),
        duration: positive(f[7]).map(|n| n as u64),
        width: positive(f[8]).and_then(|n| u32::try_from(n).ok()),
        height: positive(f[9]).and_then(|n| u32::try_from(n).ok()),
        album: (!f[10].is_empty()).then(|| f[10].to_owned()),
    })
}

/// The bridge's lines as items (malformed ones left out).
pub fn parse(scheme: &str, text: &str) -> Vec<MediaItem> {
    text.lines().filter_map(|l| parse_line(scheme, l)).collect()
}

/// The bridge's album lines (`name count`), largest first.
pub fn parse_albums(text: &str) -> Vec<(String, usize)> {
    let mut albums: Vec<(String, usize)> = text
        .lines()
        .filter_map(|l| {
            let (name, n) = l.split_once(SEP)?;
            Some((name.to_owned(), n.trim().parse().ok()?))
        })
        .collect();
    albums.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    albums
}

/// Where a thumbnail of `id` at `px` goes in `dir`: a name from the id, so
/// it is made once.
pub fn thumbnail_path(dir: &Path, id: &AssetId, px: u32) -> PathBuf {
    let hash = blake3::hash(id.as_str().as_bytes()).to_hex();
    dir.join(format!("{}-{px}.jpg", &hash[..24]))
}

/// Registers this platform's providers (none here: files are files).
/// `cache` holds thumbnails.
#[cfg(not(target_os = "android"))]
pub fn register_platform(cache: &Path) {
    let _ = cache;
}

#[cfg(target_os = "android")]
pub use android::{pick_documents, register_platform};

#[cfg(target_os = "android")]
mod android {
    use std::{
        fs::File,
        io,
        os::fd::FromRawFd,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use dexdrop_core::asset::{
        AssetId, AssetProvider, MediaItem, MediaType, Page, Query, Sort, SourceReader, Stat,
        register,
    };

    use super::{ANDROID_DOC, ANDROID_MEDIA, parse, parse_albums, thumbnail_path};
    use crate::jvm::Bridge;

    static BRIDGE: Bridge = Bridge::new("dev.dexdrop.AssetsBridge");

    fn call(method: &str, args: &[&str]) -> io::Result<String> {
        BRIDGE.call(method, args).map_err(io::Error::other)
    }

    /// An original opened in place: a file descriptor the provider handed
    /// over (a media file, a document, or a provider's pipe), read forward.
    struct Opened {
        file: File,
        id: AssetId,
    }

    impl io::Read for Opened {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.file.read(buf)
        }
    }

    impl SourceReader for Opened {
        /// The item as its provider describes it now: a document's
        /// descriptor may be a pipe, whose own metadata says nothing.
        fn stat(&self) -> io::Result<Stat> {
            let item = metadata(&self.id)?;
            Ok(Stat {
                is_dir: false,
                len: item.size.ok_or_else(|| io::Error::other("size unknown"))?,
                mtime: item.modified_at.and_then(|t| u64::try_from(t).ok()),
            })
        }
    }

    fn metadata(id: &AssetId) -> io::Result<MediaItem> {
        let (scheme, method) = if id.scheme() == ANDROID_DOC {
            (ANDROID_DOC, "docMeta")
        } else {
            (ANDROID_MEDIA, "meta")
        };
        parse(scheme, &call(method, &[&id.token()])?)
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn open(id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
        let fd: i32 = call("open", &[&id.token()])?.trim().parse().unwrap_or(-1);
        if fd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the item cannot be opened",
            ));
        }
        // SAFETY: `detachFd` handed this descriptor over: it is open and
        // ours alone, closed when the `File` drops.
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(Box::new(Opened {
            file,
            id: id.clone(),
        }))
    }

    fn thumbnail(dir: &Path, id: &AssetId, px: u32) -> io::Result<Option<PathBuf>> {
        let to = thumbnail_path(dir, id, px);
        if to.metadata().is_ok_and(|m| m.len() > 0) {
            return Ok(Some(to));
        }
        let made = call(
            "thumbnail",
            &[&id.token(), &px.to_string(), &to.to_string_lossy()],
        )?;
        Ok((!made.is_empty()).then_some(to))
    }

    fn kinds(kinds: &[MediaType]) -> String {
        kinds
            .iter()
            .filter_map(|k| match k {
                MediaType::Image => Some("image"),
                MediaType::Video => Some("video"),
                MediaType::Audio => Some("audio"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    struct Media {
        thumbs: PathBuf,
    }

    impl AssetProvider for Media {
        fn scheme(&self) -> &str {
            ANDROID_MEDIA
        }
        fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
            metadata(id)
        }
        fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
            open(id)
        }
        fn list(&self, q: &Query) -> io::Result<Page> {
            let sort = match q.sort {
                Sort::Newest => "newest",
                Sort::Oldest => "oldest",
                Sort::Largest => "largest",
                Sort::Name => "name",
            };
            // One more than asked: whether more follow.
            let text = call(
                "list",
                &[
                    &kinds(&q.kinds),
                    &q.offset.to_string(),
                    &(q.limit + 1).to_string(),
                    q.search.as_deref().unwrap_or(""),
                    sort,
                    &q.min_size.map(|n| n.to_string()).unwrap_or_default(),
                    q.album.as_deref().unwrap_or(""),
                ],
            )?;
            let mut items = parse(ANDROID_MEDIA, &text);
            let more = items.len() > q.limit;
            items.truncate(q.limit);
            Ok(Page { items, more })
        }
        fn thumbnail(&self, id: &AssetId, px: u32) -> io::Result<Option<PathBuf>> {
            thumbnail(&self.thumbs, id, px)
        }
        fn albums(&self, k: &[MediaType]) -> io::Result<Vec<(String, usize)>> {
            Ok(parse_albums(&call("albums", &[&kinds(k)])?))
        }
    }

    struct Docs {
        thumbs: PathBuf,
    }

    impl AssetProvider for Docs {
        fn scheme(&self) -> &str {
            ANDROID_DOC
        }
        fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
            metadata(id)
        }
        fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
            open(id)
        }
        fn thumbnail(&self, id: &AssetId, px: u32) -> io::Result<Option<PathBuf>> {
            thumbnail(&self.thumbs, id, px)
        }
        fn release(&self, id: &AssetId) {
            let _ = call("releaseDoc", &[&id.token()]);
        }
    }

    /// Registers `android-media` and `android-doc`; thumbnails go to
    /// `cache`.
    pub fn register_platform(cache: &Path) {
        let thumbs = cache.join("thumbnails");
        register(Arc::new(Media {
            thumbs: thumbs.clone(),
        }));
        register(Arc::new(Docs { thumbs }));
    }

    /// Shows the system's document picker (several, of `mime_types`; empty:
    /// any) and waits for the user: the documents picked, as items to send
    /// in place (their read grant kept); none when cancelled. Blocks: call
    /// it off the UI thread.
    pub fn pick_documents(mime_types: &[&str]) -> io::Result<Vec<MediaItem>> {
        Ok(parse(
            ANDROID_DOC,
            &call("pickDocuments", &[&mime_types.join(",")])?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: &[&str]) -> String {
        fields.join("\u{1}")
    }

    #[test]
    fn android_access_follows_the_grants() {
        let names = android_permissions(34);
        assert_eq!(names.len(), 4);
        assert_eq!(
            android_access(names, &[true, false, false, false]),
            Access::All
        );
        // Android 14's "allow limited access".
        assert_eq!(
            android_access(names, &[false, false, false, true]),
            Access::Some
        );
        assert_eq!(android_access(names, &[false; 4]), Access::None);
        assert_eq!(android_permissions(33).len(), 3);
        let old = android_permissions(30);
        assert_eq!(old, &[READ_EXTERNAL_STORAGE]);
        assert_eq!(android_access(old, &[true]), Access::All);
    }

    #[test]
    fn reads_the_bridges_lines() {
        let text = [
            line(&[
                "image:41",
                "image",
                "IMG_1.jpg",
                "image/jpeg",
                "2048",
                "1700000000",
                "1700000005",
                "",
                "4032",
                "3024",
                "Camera",
            ]),
            line(&[
                "video:7",
                "video",
                "clip.mp4",
                "video/mp4",
                "3221225472",
                "1700000100",
                "",
                "61000",
                "1920",
                "1080",
                "",
            ]),
            "broken".to_owned(),
            line(&[
                "content://com.android.providers.downloads.documents/document/msf%3A12",
                "doc",
                "report.pdf",
                "application/pdf",
                "",
                "",
                "1700000200",
                "",
                "",
                "",
                "",
            ]),
        ]
        .join("\n");
        let items = parse(ANDROID_MEDIA, &text);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].media_type, MediaType::Image);
        assert_eq!(items[0].id.token(), "image:41");
        assert_eq!(items[0].album.as_deref(), Some("Camera"));
        assert_eq!(items[0].modified_at, Some(1_700_000_005));
        assert_eq!((items[0].width, items[0].height), (Some(4032), Some(3024)));
        // A 3 GiB video: described, not read.
        assert_eq!(items[1].size, Some(3 << 30));
        assert_eq!(items[1].duration, Some(61_000));
        assert_eq!(items[1].modified_at, Some(1_700_000_100));
        assert_eq!(items[1].album, None);
        assert_eq!(items[2].media_type, MediaType::Document);
        assert_eq!(items[2].source, AssetSource::AndroidDocumentProvider);
        // A provider that does not know the size says nothing.
        assert_eq!(items[2].size, None);
        assert!(items[2].path().to_string_lossy().ends_with("/report.pdf"));
    }

    #[test]
    fn albums_largest_first() {
        let albums = parse_albums("Camera\u{1}120\nScreenshots\u{1}300\nbad\nWhatsApp\u{1}120\n");
        assert_eq!(
            albums,
            vec![
                ("Screenshots".to_owned(), 300),
                ("Camera".to_owned(), 120),
                ("WhatsApp".to_owned(), 120)
            ]
        );
    }

    #[test]
    fn thumbnails_have_one_place_per_item_and_size() {
        let dir = Path::new("/cache");
        let a = AssetId::new(ANDROID_MEDIA, "image:1");
        let b = AssetId::new(ANDROID_MEDIA, "image:2");
        assert_eq!(thumbnail_path(dir, &a, 256), thumbnail_path(dir, &a, 256));
        assert_ne!(thumbnail_path(dir, &a, 256), thumbnail_path(dir, &b, 256));
        assert_ne!(thumbnail_path(dir, &a, 256), thumbnail_path(dir, &a, 512));
    }
}
