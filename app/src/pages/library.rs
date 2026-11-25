//! iOS: the photo library behind Send's Recent (`dexdrop-photos`, PhotoKit).
//! Its items are not files the app can read: each shows under a path of its
//! own ([`item_path`]: `photo-library:<id>/<name>`), and picking one exports
//! the original into DexDrop's staging folder, which the selection then
//! holds (the engine deletes it once nothing selects or sends it, ADR-004).
//!
//! Headless runs have no library: the folders of `DEXDROP_RECENT_DIRS`
//! stand in for it, their files "exported" as copies. With
//! `DEXDROP_RECENT_GRANT` (a file's path) access is not given until that
//! file exists, and asking waits for it, as the system's prompt waits for
//! the user.

use std::path::{Path, PathBuf};

use dexdrop_net::media::{Access, Kind, Recent};

const PREFIX: &str = "photo-library:";

/// The path an item of the library shows under (never a real file: it is
/// relative, and the selection holds absolute paths).
pub fn item_path(id: &str, name: &str) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{}", id.replace('/', "|"))).join(name)
}

/// The library's id of an [`item_path`]; `None` for a real file.
pub fn item_id(path: &Path) -> Option<String> {
    let first = path.components().next()?.as_os_str().to_str()?;
    first.strip_prefix(PREFIX).map(|id| id.replace('|', "/"))
}

fn stand_in() -> Option<Vec<PathBuf>> {
    std::env::var_os("DEXDROP_RECENT_DIRS").map(|dirs| std::env::split_paths(&dirs).collect())
}

/// The stand-in's grant: `None` without `DEXDROP_RECENT_GRANT`.
fn stand_in_granted() -> Option<bool> {
    std::env::var_os("DEXDROP_RECENT_GRANT").map(|file| Path::new(&file).exists())
}

/// What the user lets DexDrop read, as Recent shows it.
pub fn access() -> Access {
    if stand_in().is_some() {
        return match stand_in_granted() {
            Some(false) => Access::None,
            _ => Access::All,
        };
    }
    map(dexdrop_photos::access())
}

fn map(access: dexdrop_photos::Access) -> Access {
    match access {
        dexdrop_photos::Access::Full => Access::All,
        dexdrop_photos::Access::Limited => Access::Some,
        dexdrop_photos::Access::NotDetermined | dexdrop_photos::Access::Denied => Access::None,
        dexdrop_photos::Access::Unavailable => Access::NotNeeded,
    }
}

/// Whether asking shows the system's prompt (not refused before).
pub fn can_ask() -> bool {
    if stand_in().is_some() {
        return stand_in_granted() == Some(false);
    }
    dexdrop_photos::access() == dexdrop_photos::Access::NotDetermined
}

/// Asks with the system's prompt; blocks until the user answers.
pub fn ask() -> Access {
    if stand_in().is_some() {
        // The "user" answers when the test creates the file.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while stand_in_granted() == Some(false) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        return access();
    }
    map(dexdrop_photos::request_access())
}

/// With Limited Access, the system's sheet to select more photos.
pub fn choose_more() {
    dexdrop_photos::choose_more();
}

/// The newest `limit` photos and videos, under their [`item_path`]s.
pub fn recent(limit: usize, thumbs: &Path) -> Vec<Recent> {
    if let Some(dirs) = stand_in() {
        return dexdrop_net::media::newest_files(&dirs, limit)
            .into_iter()
            .filter(|r| r.kind != Kind::File)
            .map(|r| {
                let name = r
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Recent {
                    path: item_path(&r.path.to_string_lossy(), &name),
                    ..r
                }
            })
            .collect();
    }
    dexdrop_photos::recent(limit, thumbs)
        .into_iter()
        .map(|a| Recent {
            path: item_path(&a.id, &a.name),
            kind: if a.video { Kind::Video } else { Kind::Photo },
            // Not known until exported.
            size: 0,
            added: a.created,
            duration_ms: a.duration_ms,
            thumb: a.thumb,
        })
        .collect()
}

/// Writes the original of the item `id` into `dir` and returns its path.
pub fn export(id: &str, dir: &Path) -> Result<PathBuf, String> {
    if stand_in().is_some() {
        let from = PathBuf::from(id);
        let to = dir.join(from.file_name().ok_or("no file name")?);
        std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        return Ok(to);
    }
    dexdrop_photos::export(id, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_keep_their_id_and_name() {
        let id = "9F983DBA-EC35-42B8-8773-B597CF782EDD/L0/001";
        let path = item_path(id, "IMG_0042.HEIC");
        assert_eq!(item_id(&path).as_deref(), Some(id));
        assert_eq!(path.file_name().unwrap(), "IMG_0042.HEIC");
        assert!(path.is_relative());
        assert_eq!(item_id(Path::new("/tmp/IMG_0042.HEIC")), None);
    }
}
