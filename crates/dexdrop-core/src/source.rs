//! What the engine reads from: a file's path, or a reference to an asset a
//! provider resolves ([`crate::asset`]). Every read of a source to send goes
//! through [`stat`] and [`open`], so a reference streams from where the item
//! is, as a file does.

use std::{io, path::Path};

use crate::asset::{AssetPath, SourceReader, Stat, provider_of};

/// What `path` is now: a file's or folder's metadata (links not followed),
/// or the referenced item's size and modification time.
pub fn stat(path: &Path) -> io::Result<Stat> {
    if let Some(asset) = AssetPath::parse(path) {
        let item = provider_of(&asset.id)?.metadata(&asset.id)?;
        let len = item.size.ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "the item's size is not known")
        })?;
        return Ok(Stat {
            is_dir: false,
            len,
            mtime: item.modified_at.and_then(|t| u64::try_from(t).ok()),
        });
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() && !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file or folder",
        ));
    }
    Ok(Stat {
        is_dir: meta.is_dir(),
        len: meta.len(),
        mtime: crate::manifest::mtime_of(&meta),
    })
}

/// `path`'s content, to read forward from its start: the file, or the
/// referenced item's original as its provider streams it.
pub fn open(path: &Path) -> io::Result<Box<dyn SourceReader>> {
    if let Some(asset) = AssetPath::parse(path) {
        return provider_of(&asset.id)?.open(&asset.id);
    }
    Ok(Box::new(std::fs::File::open(path)?))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        collections::HashMap,
        io::Read,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::asset::{AssetId, AssetProvider, AssetSource, MediaItem, MediaType, register};

    /// Items in memory, read through a reader that is not a file (as
    /// PhotoKit's stream is not): what tests stand in for a platform with.
    pub struct MemoryProvider {
        pub scheme: &'static str,
        pub items: Mutex<HashMap<String, (Vec<u8>, i64)>>,
    }

    struct MemoryReader {
        data: std::io::Cursor<Vec<u8>>,
        mtime: i64,
    }

    impl Read for MemoryReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.data.read(buf)
        }
    }

    impl SourceReader for MemoryReader {
        fn stat(&self) -> io::Result<Stat> {
            Ok(Stat {
                is_dir: false,
                len: self.data.get_ref().len() as u64,
                mtime: Some(self.mtime as u64),
            })
        }
    }

    impl AssetProvider for MemoryProvider {
        fn scheme(&self) -> &str {
            self.scheme
        }
        fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
            let items = self.items.lock().unwrap();
            let (data, mtime) = items
                .get(&id.token())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            Ok(MediaItem {
                id: id.clone(),
                name: id.token(),
                mime_type: String::new(),
                size: Some(data.len() as u64),
                media_type: MediaType::of("", &id.token()),
                source: AssetSource::LocalFile,
                created_at: None,
                modified_at: Some(*mtime),
                duration: None,
                width: None,
                height: None,
                album: None,
            })
        }
        fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
            let items = self.items.lock().unwrap();
            let (data, mtime) = items
                .get(&id.token())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            Ok(Box::new(MemoryReader {
                data: std::io::Cursor::new(data.clone()),
                mtime: *mtime,
            }))
        }
    }

    #[test]
    fn references_and_files_read_alike() {
        let provider = Arc::new(MemoryProvider {
            scheme: "test-memory",
            items: Mutex::new(HashMap::from([(
                "clip.mp4".to_owned(),
                (vec![7u8; 5000], 1_700_000_000),
            )])),
        });
        register(provider);
        let path = AssetPath::new(AssetId::new("test-memory", "clip.mp4"), "clip.mp4").to_path();
        let st = stat(&path).unwrap();
        assert_eq!(
            (st.is_dir, st.len, st.mtime),
            (false, 5000, Some(1_700_000_000))
        );
        let mut read = Vec::new();
        open(&path).unwrap().read_to_end(&mut read).unwrap();
        assert_eq!(read, vec![7u8; 5000]);

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hello").unwrap();
        assert_eq!(stat(&file).unwrap().len, 5);
        let mut text = String::new();
        open(&file).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello");
        // A reference no provider claims is an error, not a panic.
        let lost = AssetPath::new(AssetId::new("nobody", "x"), "x").to_path();
        assert!(stat(&lost).is_err());
    }
}
