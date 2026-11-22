//! Building the list of files and folders to send (FR-030, FR-031, FR-057).
//!
//! Folders are walked without following symbolic links; every folder,
//! including empty ones, becomes an entry so the receiver recreates the tree.
//! Only relative paths go on the wire: absolute source paths stay on the
//! sending device.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

use crate::{
    naming,
    protocol::{EntryKind, MAX_ENTRIES, WireEntry},
};

/// One entry of an outgoing transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    pub wire: WireEntry,
    /// The file to read; `None` for folders.
    pub source: Option<PathBuf>,
}

/// An outgoing transfer's manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: Vec<SourceEntry>,
    pub files: u64,
    pub folders: u64,
    pub total_bytes: u64,
    /// Symbolic links and special files left out.
    pub skipped: u64,
    pub label: String,
}

/// Why a manifest could not be built.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("nothing to send")]
    Empty,
    #[error("too many files and folders (the limit is {MAX_ENTRIES})")]
    TooManyEntries,
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl Manifest {
    /// Builds the manifest of `paths` (files and folders).
    pub fn build(paths: &[PathBuf]) -> Result<Self, ManifestError> {
        let mut manifest = Manifest::default();
        let mut top_names = HashSet::new();
        for path in paths {
            let meta = fs::symlink_metadata(path).map_err(|source| ManifestError::Io {
                path: display_name(path),
                source,
            })?;
            let name = unique_top_name(&mut top_names, &display_name(path));
            if meta.is_dir() {
                manifest.add_folder(path, &name)?;
            } else if meta.is_file() {
                manifest.push_file(path.clone(), name, &meta)?;
            } else {
                manifest.skipped += 1;
            }
        }
        if manifest.entries.is_empty() {
            return Err(ManifestError::Empty);
        }
        manifest.label = label_for(paths, &manifest);
        Ok(manifest)
    }

    fn add_folder(&mut self, root: &Path, top: &str) -> Result<(), ManifestError> {
        self.push_folder(top.to_owned())?;
        let walker = walkdir::WalkDir::new(root)
            .follow_links(false)
            .min_depth(1)
            .sort_by_file_name();
        for item in walker {
            let item = item.map_err(|e| ManifestError::Io {
                path: top.to_owned(),
                source: e.into(),
            })?;
            let rel = item
                .path()
                .strip_prefix(root)
                .expect("walkdir yields paths under its root");
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let wire_path = format!("{top}/{rel}");
            let file_type = item.file_type();
            if file_type.is_dir() {
                self.push_folder(wire_path)?;
            } else if file_type.is_file() {
                let meta = item.metadata().map_err(|e| ManifestError::Io {
                    path: wire_path.clone(),
                    source: e.into(),
                })?;
                self.push_file(item.path().to_path_buf(), wire_path, &meta)?;
            } else {
                self.skipped += 1;
            }
        }
        Ok(())
    }

    fn push_folder(&mut self, path: String) -> Result<(), ManifestError> {
        self.check_room()?;
        self.folders += 1;
        self.entries.push(SourceEntry {
            wire: WireEntry {
                path,
                kind: EntryKind::Folder,
                size: 0,
                mtime: None,
            },
            source: None,
        });
        Ok(())
    }

    fn push_file(
        &mut self,
        source: PathBuf,
        path: String,
        meta: &fs::Metadata,
    ) -> Result<(), ManifestError> {
        self.check_room()?;
        self.files += 1;
        self.total_bytes += meta.len();
        self.entries.push(SourceEntry {
            wire: WireEntry {
                path,
                kind: EntryKind::File,
                size: meta.len(),
                mtime: mtime_of(meta),
            },
            source: Some(source),
        });
        Ok(())
    }

    fn check_room(&self) -> Result<(), ManifestError> {
        if self.entries.len() as u64 >= MAX_ENTRIES {
            Err(ManifestError::TooManyEntries)
        } else {
            Ok(())
        }
    }

    /// The wire entries, in order.
    pub fn wire_entries(&self) -> Vec<WireEntry> {
        self.entries.iter().map(|e| e.wire.clone()).collect()
    }
}

/// Modification time in Unix seconds.
pub fn mtime_of(meta: &fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Untitled".to_owned())
}

fn unique_top_name(taken: &mut HashSet<String>, name: &str) -> String {
    let mut candidate = name.to_owned();
    let mut n = 1;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = naming::numbered(name, n);
        n += 1;
    }
    candidate
}

fn label_for(paths: &[PathBuf], manifest: &Manifest) -> String {
    match paths {
        [one] => display_name(one),
        [first, rest @ ..] => format!("{} and {} more", display_name(first), rest.len()),
        [] => crate::format::item_count(manifest.files, manifest.folders),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_files_and_nested_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Project");
        fs::create_dir_all(root.join("src/empty")).unwrap();
        fs::write(root.join("src/main.rs"), b"fn main() {}").unwrap();
        fs::write(root.join("README.md"), b"hello").unwrap();
        let single = dir.path().join("note.txt");
        fs::write(&single, b"abc").unwrap();

        let manifest = Manifest::build(&[root.clone(), single.clone()]).unwrap();
        let paths: Vec<_> = manifest
            .entries
            .iter()
            .map(|e| e.wire.path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![
                "Project",
                "Project/README.md",
                "Project/src",
                "Project/src/empty",
                "Project/src/main.rs",
                "note.txt"
            ]
        );
        assert_eq!(manifest.files, 3);
        assert_eq!(manifest.folders, 3);
        assert_eq!(manifest.total_bytes, 12 + 5 + 3);
        assert_eq!(manifest.label, "Project and 1 more");
        for entry in &manifest.entries {
            assert!(crate::pathsafe::sanitize(&entry.wire.path).is_ok());
        }
    }

    #[test]
    fn renames_duplicate_top_level_names() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("x.txt"), b"1").unwrap();
        fs::write(b.path().join("x.txt"), b"2").unwrap();
        let manifest = Manifest::build(&[a.path().join("x.txt"), b.path().join("x.txt")]).unwrap();
        assert_eq!(manifest.entries[0].wire.path, "x.txt");
        assert_eq!(manifest.entries[1].wire.path, "x (1).txt");
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("d")).unwrap();
        fs::write(dir.path().join("d/real"), b"x").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("d/link")).unwrap();
        let manifest = Manifest::build(&[dir.path().join("d")]).unwrap();
        assert_eq!(manifest.files, 1);
        assert_eq!(manifest.skipped, 1);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(matches!(Manifest::build(&[]), Err(ManifestError::Empty)));
    }
}
