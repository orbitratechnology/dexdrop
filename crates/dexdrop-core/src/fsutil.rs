//! Small filesystem helpers: atomic writes of small state files.

use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use serde::{Serialize, de::DeserializeOwned};

/// Writes `bytes` to `path` atomically: a temporary sibling is written,
/// flushed to disk and renamed over the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let tmp = path.with_extension(format!("tmp-{}", crate::hex(&crate::random_bytes::<4>())));
    #[cfg(target_arch = "wasm32")]
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        restrict_permissions(&file)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Serializes `value` as JSON and writes it atomically.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

/// Reads JSON from `path`; `Ok(None)` when the file does not exist.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// App-private state files are readable by their owner only (Unix).
#[cfg(unix)]
fn restrict_permissions(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

/// Free space on the filesystem holding `path`, when the platform says.
#[cfg(target_arch = "wasm32")]
pub fn available_space(_path: &Path) -> Option<u64> {
    None
}

/// Free space on the filesystem holding `path`, when the platform says.
#[cfg(not(target_arch = "wasm32"))]
pub fn available_space(path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    loop {
        if probe.exists() {
            return fs4::available_space(&probe).ok();
        }
        if !probe.pop() {
            return None;
        }
    }
}

/// The local path of a `file:` URI from a drag and drop (FR-036):
/// `file:///home/a/My%20file.txt`, `file:///C:/Users/a/x.txt`,
/// `file://server/share/x.txt` (a Windows network share). `None` for other
/// schemes.
pub fn file_uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("file://"))
        .map(|_| &uri[7..])?;
    let (host, path) = match rest.find('/') {
        Some(0) => ("", rest),
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let bytes = percent_decode(path);
    let host = if host.eq_ignore_ascii_case("localhost") {
        ""
    } else {
        host
    };
    if !host.is_empty() {
        let tail = String::from_utf8_lossy(&bytes).replace('/', "\\");
        return Some(std::path::PathBuf::from(format!("\\\\{host}{tail}")));
    }
    // `/C:/x` is the drive path `C:/x`.
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return Some(std::path::PathBuf::from(
            String::from_utf8_lossy(&bytes[1..]).into_owned(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
            &bytes,
        )))
    }
    #[cfg(not(unix))]
    {
        Some(std::path::PathBuf::from(
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn decodes_file_uris_from_every_platform() {
        assert_eq!(
            file_uri_to_path("file:///home/a/My%20file.txt"),
            Some(PathBuf::from("/home/a/My file.txt"))
        );
        assert_eq!(
            file_uri_to_path("FILE:///tmp/x"),
            Some(PathBuf::from("/tmp/x"))
        );
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/x"),
            Some(PathBuf::from("/tmp/x"))
        );
        assert_eq!(
            file_uri_to_path("file:///Users/a/%E6%97%A5%E6%9C%AC.txt"),
            Some(PathBuf::from("/Users/a/日本.txt"))
        );
        assert_eq!(
            file_uri_to_path("file:///C:/Users/a%20b/x.txt"),
            Some(PathBuf::from("C:/Users/a b/x.txt"))
        );
        assert_eq!(
            file_uri_to_path("file://server/share/dir/x.txt"),
            Some(PathBuf::from("\\\\server\\share\\dir\\x.txt"))
        );
        // A stray % stays as it is.
        assert_eq!(
            file_uri_to_path("file:///tmp/100%"),
            Some(PathBuf::from("/tmp/100%"))
        );
        assert_eq!(
            file_uri_to_path("file:///tmp/%zz"),
            Some(PathBuf::from("/tmp/%zz"))
        );
        assert_eq!(file_uri_to_path("https://example.com/x"), None);
        assert_eq!(file_uri_to_path("file:"), None);
    }
}
