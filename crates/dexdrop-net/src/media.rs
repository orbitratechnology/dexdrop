//! What was added to the device lately, for Send to offer in one tap: the
//! newest photos and videos on Android (the system's `MediaStore`, through
//! the Java bridge `android/dev/dexdrop/MediaBridge.java`), the newest files
//! in the usual folders on computers. Items are sent from their own files;
//! nothing is copied. Nothing is offered on iOS: NSH 0.7.0's photo library
//! hands over re-encoded JPEGs, not the originals (ADR-005).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// What an item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Photo,
    Video,
    File,
}

/// One recent item.
#[derive(Clone, Debug, PartialEq)]
pub struct Recent {
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    /// When it was added (or last changed), seconds since 1970.
    pub added: u64,
    /// A video's length in milliseconds (0 otherwise).
    pub duration_ms: u64,
    /// A small picture of it: a thumbnail on Android, the photo itself on
    /// computers; `None` for other files.
    pub thumb: Option<PathBuf>,
}

/// Whether the app may read the device's photos and videos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// All of them.
    All,
    /// Those the user chose (Android 14's limited access).
    Some,
    /// Not yet allowed.
    None,
    /// No such permission here (computers read their folders).
    NotNeeded,
}

const READ_MEDIA_IMAGES: &str = "android.permission.READ_MEDIA_IMAGES";
const READ_MEDIA_VIDEO: &str = "android.permission.READ_MEDIA_VIDEO";
const READ_MEDIA_VISUAL_USER_SELECTED: &str = "android.permission.READ_MEDIA_VISUAL_USER_SELECTED";
const READ_EXTERNAL_STORAGE: &str = "android.permission.READ_EXTERNAL_STORAGE";

/// Android's permissions to read photos and videos on API level `api`:
/// `READ_MEDIA_IMAGES` and `READ_MEDIA_VIDEO` from Android 13 (33), with
/// `READ_MEDIA_VISUAL_USER_SELECTED` from 14 (34: the user may allow only
/// some), `READ_EXTERNAL_STORAGE` before. The app asks with NSH's
/// `nsh::permissions`; dexdrop-net's `Cargo.toml` declares them all.
pub fn android_permissions(api: u32) -> &'static [&'static str] {
    if api >= 34 {
        &[
            READ_MEDIA_IMAGES,
            READ_MEDIA_VIDEO,
            READ_MEDIA_VISUAL_USER_SELECTED,
        ]
    } else if api >= 33 {
        &[READ_MEDIA_IMAGES, READ_MEDIA_VIDEO]
    } else {
        &[READ_EXTERNAL_STORAGE]
    }
}

/// What `permissions` allow, from whether each is granted (same order).
pub fn android_access(permissions: &[&str], granted: &[bool]) -> Access {
    let mut some = false;
    for (name, granted) in permissions.iter().zip(granted) {
        if *granted {
            if *name == READ_MEDIA_VISUAL_USER_SELECTED {
                some = true;
            } else {
                return Access::All;
            }
        }
    }
    if some { Access::Some } else { Access::None }
}

/// The device's Android API level (`ro.build.version.sdk`); 0 when unknown.
#[cfg(target_os = "android")]
pub fn android_api_level() -> u32 {
    let mut value = [0 as libc::c_char; 92]; // PROP_VALUE_MAX
    // SAFETY: a NUL-terminated name and a buffer of PROP_VALUE_MAX bytes,
    // as `__system_property_get` requires.
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

/// Separates an item's fields (see `MediaBridge.java`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const SEP: char = '\u{1}';

/// The bridge's lines, as items (malformed lines left out).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn parse(text: &str) -> Vec<Recent> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(SEP).collect();
            if f.len() < 6 || f[0].is_empty() {
                return None;
            }
            Some(Recent {
                path: PathBuf::from(f[0]),
                kind: if f[1] == "video" {
                    Kind::Video
                } else {
                    Kind::Photo
                },
                size: f[2].parse().ok()?,
                added: f[3].parse().unwrap_or(0),
                duration_ms: f[4].parse().unwrap_or(0),
                thumb: (!f[5].is_empty()).then(|| PathBuf::from(f[5])),
            })
        })
        .collect()
}

/// The kind of a file by its extension.
pub fn kind_of(path: &Path) -> Kind {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "avif" | "bmp" => Kind::Photo,
        "mp4" | "mov" | "m4v" | "mkv" | "webm" | "avi" | "3gp" => Kind::Video,
        _ => Kind::File,
    }
}

/// The newest files directly in `dirs` (not in their subfolders), newest
/// first, at most `limit`: hidden files, links and DexDrop's partial
/// downloads left out.
pub fn newest_files(dirs: &[PathBuf], limit: usize) -> Vec<Recent> {
    let mut found: Vec<Recent> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with("dexdrop-part") || name == "desktop.ini" {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let added = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let path = e.path();
            let kind = kind_of(&path);
            found.push(Recent {
                thumb: (kind == Kind::Photo).then(|| path.clone()),
                path,
                kind,
                size: meta.len(),
                added,
                duration_ms: 0,
            });
        }
    }
    found.sort_by(|a, b| b.added.cmp(&a.added).then_with(|| a.path.cmp(&b.path)));
    found.truncate(limit);
    found
}

/// Removes the thumbnails in `dir` that none of `items` shows (they are
/// made once per item; this keeps the folder to the items listed).
pub fn prune_thumbs(dir: &Path, items: &[Recent]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !items
            .iter()
            .any(|i| i.thumb.as_deref() == Some(p.as_path()))
        {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(target_os = "android")]
mod system {
    use std::sync::{Mutex, OnceLock};

    use jni::{
        JavaVM,
        objects::{GlobalRef, JString, JValue},
    };

    use crate::jvm::{describe, java_vm, new_bridge};

    static BRIDGE: OnceLock<Mutex<Option<(JavaVM, GlobalRef)>>> = OnceLock::new();

    /// Calls `method(String…)` on the bridge, made once, and returns its
    /// text.
    pub fn call(method: &str, args: &[&str]) -> Result<String, String> {
        let mut slot = BRIDGE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            let vm = java_vm().ok_or("The system's media library is not reachable.")?;
            let bridge = {
                let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
                let made = new_bridge(&mut env, "dev.dexdrop.MediaBridge")
                    .and_then(|b| env.new_global_ref(b));
                match made {
                    Ok(b) => b,
                    Err(err) => return Err(describe(&mut env, err)),
                }
            };
            *slot = Some((vm, bridge));
        }
        let (vm, bridge) = slot.as_ref().expect("made above");
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let signature = format!(
            "({})Ljava/lang/String;",
            "Ljava/lang/String;".repeat(args.len())
        );
        let result = env.with_local_frame(16, |env| -> jni::errors::Result<String> {
            let strings = args
                .iter()
                .map(|a| env.new_string(a))
                .collect::<Result<Vec<_>, _>>()?;
            let values: Vec<JValue> = strings.iter().map(|s| JValue::Object(s)).collect();
            let out = env
                .call_method(bridge.as_obj(), method, &signature, &values)?
                .l()?;
            if out.is_null() {
                return Ok(String::new());
            }
            Ok(env.get_string(&JString::from(out))?.into())
        });
        result.map_err(|err| describe(&mut env, err))
    }
}

/// The newest photos and videos on this phone (Android), at most `limit`,
/// with thumbnails in `thumb_dir`; empty without access.
#[cfg(target_os = "android")]
pub fn recent_media(limit: usize, thumb_dir: &Path) -> Vec<Recent> {
    let text = system::call(
        "recent",
        &[&limit.to_string(), &thumb_dir.to_string_lossy()],
    )
    .unwrap_or_default();
    let items = parse(&text);
    prune_thumbs(thumb_dir, &items);
    items
}

#[cfg(not(target_os = "android"))]
pub fn recent_media(_limit: usize, _thumb_dir: &Path) -> Vec<Recent> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_access_follows_the_grants() {
        let names = android_permissions(34);
        assert_eq!(names.len(), 3);
        assert_eq!(android_access(names, &[true, false, false]), Access::All);
        assert_eq!(android_access(names, &[false, true, false]), Access::All);
        // Android 14's "allow limited access".
        assert_eq!(android_access(names, &[false, false, true]), Access::Some);
        assert_eq!(android_access(names, &[false; 3]), Access::None);
        assert_eq!(android_permissions(33).len(), 2);
        // Before Android 13: storage.
        let old = android_permissions(30);
        assert_eq!(old, &[READ_EXTERNAL_STORAGE]);
        assert_eq!(android_access(old, &[true]), Access::All);
    }

    #[test]
    fn reads_the_bridges_lines() {
        let text = "/sdcard/DCIM/a.jpg\u{1}photo\u{1}2048\u{1}1700000000\u{1}0\u{1}/c/t/1.jpg\n\
                    /sdcard/DCIM/b.mp4\u{1}video\u{1}9000\u{1}1700000100\u{1}61000\u{1}\n\
                    broken line\n";
        let items = parse(text);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, Kind::Photo);
        assert_eq!(items[0].thumb.as_deref(), Some(Path::new("/c/t/1.jpg")));
        assert_eq!(items[1].kind, Kind::Video);
        assert_eq!(items[1].duration_ms, 61_000);
        assert_eq!(items[1].thumb, None);
    }

    #[test]
    fn lists_the_newest_files_of_folders() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for (dir, name) in [
            (a.path(), "old.pdf"),
            (a.path(), "new.jpg"),
            (b.path(), "clip.mp4"),
            (b.path(), ".hidden"),
            (b.path(), ".x.bin.1a2b.dexdrop-part"),
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::create_dir(a.path().join("folder")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(a.path().join("old.pdf"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let items = newest_files(&[a.path().to_path_buf(), b.path().to_path_buf()], 10);
        let names: Vec<String> = items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert_eq!(names.last().map(String::as_str), Some("old.pdf"));
        let photo = items.iter().find(|i| i.kind == Kind::Photo).unwrap();
        assert_eq!(photo.thumb.as_deref(), Some(photo.path.as_path()));
        assert_eq!(newest_files(&[a.path().to_path_buf()], 1).len(), 1);
    }

    #[test]
    fn keeps_only_the_listed_thumbnails() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("1.jpg");
        let stale = dir.path().join("2.jpg");
        std::fs::write(&kept, b"x").unwrap();
        std::fs::write(&stale, b"x").unwrap();
        let item = Recent {
            path: "/sdcard/a.jpg".into(),
            kind: Kind::Photo,
            size: 1,
            added: 0,
            duration_ms: 0,
            thumb: Some(kept.clone()),
        };
        prune_thumbs(dir.path(), &[item]);
        assert!(kept.exists());
        assert!(!stale.exists());
    }
}
