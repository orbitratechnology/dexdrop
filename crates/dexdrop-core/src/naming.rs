//! "Keep both" names for destination conflicts (FR-043).

use std::path::{Path, PathBuf};

/// How a receiver treats a name that already exists at the destination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConflictPolicy {
    /// Save the new file under a free name: `photo (1).jpg`. The default.
    #[default]
    KeepBoth,
    /// Replace the existing file. Only after explicit confirmation.
    Replace,
    /// Leave the existing file and skip the incoming one.
    Skip,
}

/// `name` with ` (n)` before its extension: `report (2).pdf`,
/// `archive (1).tar.gz`, `.profile (1)`.
pub fn numbered(name: &str, n: u32) -> String {
    let (stem, ext) = split_extension(name);
    format!("{stem} ({n}){ext}")
}

fn split_extension(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    for double in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"] {
        if lower.len() > double.len() && lower.ends_with(double) {
            let at = name.len() - double.len();
            return (&name[..at], &name[at..]);
        }
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

/// The first path in `dir` named `name`, or `name (1)`, `name (2)`, … that
/// does not exist yet.
pub fn first_free(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !exists(&candidate) {
        return candidate;
    }
    // Bounded: many files of one name cannot cost thousands of lookups each.
    for n in 1..=100 {
        let candidate = dir.join(numbered(name, n));
        if !exists(&candidate) {
            return candidate;
        }
    }
    let mut n = 101 + (crate::unix_now() % 1_000_000) as u32;
    loop {
        let candidate = dir.join(numbered(name, n));
        if !exists(&candidate) {
            return candidate;
        }
        n = n.wrapping_mul(31).wrapping_add(7) % 100_000_000 + 101;
    }
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_names() {
        assert_eq!(numbered("photo.jpg", 1), "photo (1).jpg");
        assert_eq!(numbered("archive.tar.gz", 2), "archive (2).tar.gz");
        assert_eq!(numbered(".profile", 1), ".profile (1)");
        assert_eq!(numbered("README", 3), "README (3)");
    }

    #[test]
    fn finds_free_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(first_free(dir.path(), "a.txt"), dir.path().join("a.txt"));
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("a (1).txt"), b"x").unwrap();
        assert_eq!(
            first_free(dir.path(), "a.txt"),
            dir.path().join("a (2).txt")
        );
    }
}
