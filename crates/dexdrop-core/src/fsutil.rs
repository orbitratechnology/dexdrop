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
