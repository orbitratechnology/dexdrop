//! Validation of peer-supplied relative paths (FR-045).
//!
//! A receiver never trusts a path from the network. [`sanitize`] rejects
//! absolute paths, traversal and malformed input, and makes the remaining
//! components portable (reserved characters and Windows device names are
//! replaced rather than rejected, so a file named `a:b` from Linux still
//! arrives on Windows). [`resolve`] joins the result under the destination
//! and refuses to pass through symbolic links.

use std::path::{Path, PathBuf};

use crate::protocol::MAX_PATH_BYTES;

/// The deepest folder nesting accepted.
pub const MAX_DEPTH: usize = 64;
/// The longest single component kept, in bytes.
pub const MAX_COMPONENT_BYTES: usize = 240;

/// Why a path was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("empty path")]
    Empty,
    #[error("path too long")]
    TooLong,
    #[error("path too deep")]
    TooDeep,
    #[error("absolute path")]
    Absolute,
    #[error("path traversal")]
    Traversal,
    #[error("invalid character")]
    InvalidCharacter,
    #[error("path passes through a symbolic link")]
    Symlink,
    #[error("path escapes the destination")]
    Escapes,
}

/// Validates a `/`-separated relative path and returns its portable
/// components.
pub fn sanitize(path: &str) -> Result<Vec<String>, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH_BYTES {
        return Err(PathError::TooLong);
    }
    if path.contains('\0') {
        return Err(PathError::InvalidCharacter);
    }
    if path.starts_with('/') || path.starts_with('\\') || has_drive_prefix(path) {
        return Err(PathError::Absolute);
    }
    let mut components = Vec::new();
    for raw in path.split('/') {
        match raw {
            "" => return Err(PathError::Empty),
            "." | ".." => return Err(PathError::Traversal),
            _ => {}
        }
        // A backslash is an ordinary character on the sender's side, but
        // must never act as a separator here.
        if raw.split('\\').any(|part| part == "..") {
            return Err(PathError::Traversal);
        }
        components.push(portable_component(raw));
        if components.len() > MAX_DEPTH {
            return Err(PathError::TooDeep);
        }
    }
    Ok(components)
}

fn has_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    // `C:`, `C:/…`, `C:\…`. A colon elsewhere (`a:b`, `c:x`) is replaced by
    // `_` like any reserved character.
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'/' || bytes[2] == b'\\')
}

/// One component made safe on every supported filesystem.
fn portable_component(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' => '_',
            c if (c as u32) < 0x20 || c == '\u{7f}' => '_',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces.
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        out.push('_');
    }
    if is_windows_reserved(&out) {
        out.insert(0, '_');
    }
    truncate_component(&out)
}

fn is_windows_reserved(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    // COM1–COM9 and LPT1–LPT9, and the superscript digits Windows also
    // reserves (COM¹, LPT³).
    let mut chars = stem.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let rest: Vec<char> = chars.collect();
    (prefix == "COM" || prefix == "LPT")
        && rest.len() == 1
        && matches!(rest[0], '1'..='9' | '¹' | '²' | '³')
}

fn truncate_component(name: &str) -> String {
    if name.len() <= MAX_COMPONENT_BYTES {
        return name.to_owned();
    }
    // Keep a short extension when there is one.
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= 16 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let mut end = MAX_COMPONENT_BYTES - ext.len();
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &stem[..end], ext)
}

/// Joins sanitized `components` under `root`, refusing to pass through any
/// existing symbolic link, and returns the path.
pub fn resolve(root: &Path, components: &[String]) -> Result<PathBuf, PathError> {
    let mut path = root.to_path_buf();
    for component in components {
        path.push(component);
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            return Err(PathError::Symlink);
        }
    }
    if !path.starts_with(root) {
        return Err(PathError::Escapes);
    }
    Ok(path)
}

/// Fails if any existing component of `path` below `root` is a symbolic
/// link. Checked again right before writing, not only when planning.
pub fn ensure_no_symlinks(root: &Path, path: &Path) -> Result<(), PathError> {
    let rel = path.strip_prefix(root).map_err(|_| PathError::Escapes)?;
    let mut at = root.to_path_buf();
    for component in rel.components() {
        at.push(component);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(PathError::Symlink),
            Ok(_) => {}
            Err(_) => break, // Not created yet: nothing below can be a link.
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_paths() {
        assert_eq!(sanitize("a/b/c.txt").unwrap(), vec!["a", "b", "c.txt"]);
        assert_eq!(
            sanitize("Fotos 2026/IMG 1.HEIC").unwrap(),
            vec!["Fotos 2026", "IMG 1.HEIC"]
        );
        assert_eq!(sanitize("日本語/ファイル.txt").unwrap().len(), 2);
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        for bad in [
            "../x",
            "a/../../x",
            "./x",
            "a/./b",
            "/etc/passwd",
            "\\\\server\\share",
            "C:/Windows",
            "C:",
            "D:\\x",
            "a//b",
            "",
            "a/",
            "a\\..\\..\\b",
            "x\0y",
        ] {
            assert!(sanitize(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(sanitize("a/".repeat(MAX_DEPTH + 1).trim_end_matches('/')).is_err());
        assert_eq!(
            sanitize(&"x".repeat(MAX_PATH_BYTES + 1)),
            Err(PathError::TooLong)
        );
    }

    #[test]
    fn makes_components_portable() {
        assert_eq!(sanitize("a:b?.txt").unwrap(), vec!["a_b_.txt"]);
        assert_eq!(sanitize("c:x").unwrap(), vec!["c_x"]);
        assert_eq!(sanitize("CON").unwrap(), vec!["_CON"]);
        assert_eq!(sanitize("nul.txt").unwrap(), vec!["_nul.txt"]);
        assert_eq!(sanitize("COM1.log").unwrap(), vec!["_COM1.log"]);
        assert_eq!(sanitize("COM0.log").unwrap(), vec!["COM0.log"]);
        assert_eq!(sanitize("CONIN$").unwrap(), vec!["_CONIN$"]);
        assert_eq!(sanitize("conout$.txt").unwrap(), vec!["_conout$.txt"]);
        assert_eq!(sanitize("COM¹.txt").unwrap(), vec!["_COM¹.txt"]);
        assert_eq!(sanitize("LPT³").unwrap(), vec!["_LPT³"]);
        assert_eq!(sanitize("COMPUTER.txt").unwrap(), vec!["COMPUTER.txt"]);
        assert_eq!(sanitize("name. ").unwrap(), vec!["name"]);
        assert_eq!(sanitize("...").unwrap(), vec!["_"]);
        assert_eq!(sanitize("a\\b").unwrap(), vec!["a_b"]);
        let long = format!("{}.txt", "é".repeat(200));
        let out = &sanitize(&long).unwrap()[0];
        assert!(out.len() <= MAX_COMPONENT_BYTES && out.ends_with(".txt"));
    }

    #[test]
    fn resolve_stays_under_root() {
        let dir = tempfile::tempdir().unwrap();
        let comps = sanitize("a/b.txt").unwrap();
        let path = resolve(dir.path(), &comps).unwrap();
        assert!(path.starts_with(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_refuses_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        let comps = sanitize("link/evil.txt").unwrap();
        assert_eq!(resolve(dir.path(), &comps), Err(PathError::Symlink));
    }
}
