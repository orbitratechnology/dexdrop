//! What was added to a computer lately, for Send to offer in one tap: the
//! newest files in the usual folders, sent from their own files (nothing is
//! copied). Phones offer none: their photos and videos take a broad photo
//! permission, which the stores restrict (docs/ARCHITECTURE.md ADR-008).

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
    /// A small picture of it: the photo itself; `None` for other files.
    pub thumb: Option<PathBuf>,
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
            });
        }
    }
    found.sort_by(|a, b| b.added.cmp(&a.added).then_with(|| a.path.cmp(&b.path)));
    found.truncate(limit);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
