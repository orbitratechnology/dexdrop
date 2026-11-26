//! Selected items as references, not copies (docs/FILE_SELECTION.md).
//!
//! A photo, video, song or document picked on a phone stays where it is: the
//! selection holds a **reference path** (`dexdrop-asset:<scheme>/<token>/<name>`,
//! [`AssetPath`]) and the provider that made it (an [`AssetProvider`], one per
//! scheme: Android's MediaStore, the Storage Access Framework, iOS's PhotoKit,
//! …) resolves it when something needs the item: its metadata for a
//! manifest, its bytes when a transfer starts ([`crate::source`]), a
//! thumbnail for a list. The token is the provider's own and opaque above
//! it: no content URI or PhotoKit object leaves the provider.
//!
//! Providers are blocking: the engine reads sources on blocking threads, and
//! the app calls them off the UI thread.

use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};

use serde::{Deserialize, Serialize};

/// What a reference path starts with.
pub const PREFIX: &str = "dexdrop-asset:";

/// An item of a provider: `<scheme>/<token>`, opaque to everyone else.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AssetId(String);

impl AssetId {
    /// `token` of the provider `scheme` (any text: it is escaped).
    pub fn new(scheme: &str, token: &str) -> Self {
        AssetId(format!("{scheme}/{}", escape(token)))
    }

    /// The provider's scheme.
    pub fn scheme(&self) -> &str {
        self.0.split_once('/').map_or(&self.0, |(s, _)| s)
    }

    /// The provider's token, as given to [`AssetId::new`].
    pub fn token(&self) -> String {
        unescape(self.0.split_once('/').map_or("", |(_, t)| t))
    }

    /// The id as stored and compared (stable across runs for providers whose
    /// tokens are).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A selected item's reference path: what the selection and the engine
/// carry in place of a file's path. Its file name is the item's name, so
/// lists and labels show it as any file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPath {
    pub id: AssetId,
    pub name: String,
}

impl AssetPath {
    pub fn new(id: AssetId, name: &str) -> Self {
        let name = name.replace(['/', '\\', '\0'], "_");
        let name = if name.is_empty() || name == "." || name == ".." {
            "item".to_owned()
        } else {
            name
        };
        AssetPath { id, name }
    }

    /// `dexdrop-asset:<scheme>/<token>/<name>`.
    pub fn to_path(&self) -> PathBuf {
        PathBuf::from(format!("{PREFIX}{}/{}", self.id.0, self.name))
    }

    /// The reference `path` names, if it is one.
    pub fn parse(path: &Path) -> Option<Self> {
        let text = path.to_str()?.strip_prefix(PREFIX)?;
        let (scheme, rest) = text.split_once('/')?;
        let (token, name) = rest.split_once('/')?;
        if scheme.is_empty() || token.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(AssetPath {
            id: AssetId(format!("{scheme}/{token}")),
            name: name.to_owned(),
        })
    }
}

/// Whether `path` is a reference to an asset rather than a file's path.
pub fn is_reference(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.starts_with(PREFIX))
}

/// '%' and '/' escaped, so a token holds any text.
fn escape(token: &str) -> String {
    token.replace('%', "%25").replace('/', "%2F")
}

fn unescape(token: &str) -> String {
    token.replace("%2F", "/").replace("%25", "%")
}

/// What an item is, as the browser groups it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaType {
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Other,
}

impl MediaType {
    /// From a MIME type (`image/heic`), else from a name's extension.
    pub fn of(mime: &str, name: &str) -> Self {
        let mime = mime.to_ascii_lowercase();
        if let Some(t) = mime.split('/').next() {
            match t {
                "image" => return MediaType::Image,
                "video" => return MediaType::Video,
                "audio" => return MediaType::Audio,
                _ => {}
            }
        }
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "avif" | "bmp" | "dng"
            | "raw" | "tif" | "tiff" => MediaType::Image,
            "mp4" | "mov" | "m4v" | "mkv" | "webm" | "avi" | "3gp" | "wmv" => MediaType::Video,
            "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" | "wma" => MediaType::Audio,
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" => MediaType::Archive,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "txt" | "md" | "rtf" | "csv" | "epub" | "pages" | "numbers" | "key" => {
                MediaType::Document
            }
            _ => MediaType::Other,
        }
    }
}

/// Where an item lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetSource {
    AndroidMediaStore,
    AndroidDocumentProvider,
    IosPhotoLibrary,
    IosDocumentProvider,
    LocalFile,
}

/// An item as a list shows it: everything known without reading it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: AssetId,
    pub name: String,
    pub mime_type: String,
    /// Bytes, when the provider knows them before reading.
    pub size: Option<u64>,
    pub media_type: MediaType,
    pub source: AssetSource,
    /// Unix seconds.
    pub created_at: Option<i64>,
    /// Unix seconds.
    pub modified_at: Option<i64>,
    /// Milliseconds (videos, audio).
    pub duration: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The album, folder or collection it is in, when the provider says.
    pub album: Option<String>,
}

impl MediaItem {
    /// The reference path the selection holds for it.
    pub fn path(&self) -> PathBuf {
        AssetPath::new(self.id.clone(), &self.name).to_path()
    }
}

/// What a browser asks a provider for: one page of a kind, newest first
/// unless sorted otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub kinds: Vec<MediaType>,
    pub offset: usize,
    pub limit: usize,
    /// Case-insensitive part of the name.
    pub search: Option<String>,
    pub sort: Sort,
    /// Only items at least this large (the "Large files" view).
    pub min_size: Option<u64>,
    /// One album, by the name the provider gave in [`MediaItem::album`].
    pub album: Option<String>,
}

impl Query {
    pub fn of(kinds: &[MediaType]) -> Self {
        Query {
            kinds: kinds.to_vec(),
            offset: 0,
            limit: 60,
            search: None,
            sort: Sort::Newest,
            min_size: None,
            album: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Largest,
    Name,
}

/// One page of a query.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub items: Vec<MediaItem>,
    /// More items follow (ask again from `offset + items.len()`).
    pub more: bool,
}

/// What a source is now: checked against the manifest before and after
/// reading, so an item edited meanwhile is not sent half old, half new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub is_dir: bool,
    pub len: u64,
    /// Unix seconds.
    pub mtime: Option<u64>,
}

/// An open source: read forward, in bounded chunks, by the engine.
pub trait SourceReader: io::Read + Send {
    /// Its size and modification time as they are now.
    fn stat(&self) -> io::Result<Stat>;
}

impl SourceReader for std::fs::File {
    fn stat(&self) -> io::Result<Stat> {
        let meta = self.metadata()?;
        Ok(Stat {
            is_dir: meta.is_dir(),
            len: meta.len(),
            mtime: crate::manifest::mtime_of(&meta),
        })
    }
}

/// A place items come from: lists them (pages), describes one without
/// reading it, opens its original for a transfer, makes thumbnails.
pub trait AssetProvider: Send + Sync {
    /// The scheme of its [`AssetId`]s (`android-media`, `ios-photos`).
    fn scheme(&self) -> &str;

    /// An item's metadata, without reading its content.
    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem>;

    /// The original, to read forward from its start. Nothing is copied: the
    /// reader reads the item where it is (a file descriptor, a stream).
    fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>>;

    /// One page of items (browsers); unsupported by pick-only providers.
    fn list(&self, query: &Query) -> io::Result<Page> {
        let _ = query;
        Err(io::Error::new(io::ErrorKind::Unsupported, "not listable"))
    }

    /// A thumbnail of about `px` pixels on its longer side, as an image file
    /// in the provider's cache (made once); `None` when there is none.
    fn thumbnail(&self, id: &AssetId, px: u32) -> io::Result<Option<PathBuf>> {
        let _ = (id, px);
        Ok(None)
    }

    /// The selection let go of `id` (a security scope can close).
    fn release(&self, id: &AssetId) {
        let _ = id;
    }

    /// The albums (folders, collections) holding items of `kinds`, with how
    /// many each holds, largest first; none when the provider has none.
    fn albums(&self, kinds: &[MediaType]) -> io::Result<Vec<(String, usize)>> {
        let _ = kinds;
        Ok(Vec::new())
    }
}

/// A file sent under another name, read where it is: an app's APK as
/// `"Maps 11.2.apk"` without copying it. Built in: scheme [`LocalFiles::SCHEME`],
/// the token the file's absolute path.
pub struct LocalFiles;

impl LocalFiles {
    pub const SCHEME: &'static str = "local";

    /// The reference path that sends `file` as `name`.
    pub fn reference(file: &Path, name: &str) -> PathBuf {
        AssetPath::new(AssetId::new(Self::SCHEME, &file.to_string_lossy()), name).to_path()
    }
}

impl AssetProvider for LocalFiles {
    fn scheme(&self) -> &str {
        Self::SCHEME
    }

    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
        let path = PathBuf::from(id.token());
        let meta = std::fs::metadata(&path)?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mtime = crate::manifest::mtime_of(&meta).and_then(|t| i64::try_from(t).ok());
        Ok(MediaItem {
            id: id.clone(),
            media_type: MediaType::of("", &name),
            name,
            mime_type: String::new(),
            size: Some(meta.len()),
            source: AssetSource::LocalFile,
            created_at: None,
            modified_at: mtime,
            duration: None,
            width: None,
            height: None,
            album: None,
        })
    }

    fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
        Ok(Box::new(std::fs::File::open(id.token())?))
    }
}

fn providers() -> &'static RwLock<Vec<Arc<dyn AssetProvider>>> {
    static PROVIDERS: OnceLock<RwLock<Vec<Arc<dyn AssetProvider>>>> = OnceLock::new();
    PROVIDERS.get_or_init(|| RwLock::new(vec![Arc::new(LocalFiles)]))
}

/// Makes `provider` resolve the ids of its scheme (replacing one registered
/// before for the same scheme).
pub fn register(provider: Arc<dyn AssetProvider>) {
    let mut list = providers().write().unwrap_or_else(|e| e.into_inner());
    list.retain(|p| p.scheme() != provider.scheme());
    list.push(provider);
}

/// The provider of `scheme`, if one is registered.
pub fn provider(scheme: &str) -> Option<Arc<dyn AssetProvider>> {
    providers()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|p| p.scheme() == scheme)
        .cloned()
}

/// The provider of `id`, or why there is none.
pub fn provider_of(id: &AssetId) -> io::Result<Arc<dyn AssetProvider>> {
    provider(id.scheme()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no provider for {}", id.scheme()),
        )
    })
}

/// The metadata of the item `path` references.
pub fn metadata(path: &Path) -> io::Result<MediaItem> {
    let asset = AssetPath::parse(path)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not an asset reference"))?;
    provider_of(&asset.id)?.metadata(&asset.id)
}

/// Tells the provider of `path` the selection let go of it.
pub fn release(path: &Path) {
    if let Some(asset) = AssetPath::parse(path)
        && let Ok(p) = provider_of(&asset.id)
    {
        p.release(&asset.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_paths_round_trip_any_token() {
        let id = AssetId::new(
            "android-doc",
            "content://com.android.providers/document/raw%3A/a b",
        );
        assert_eq!(id.scheme(), "android-doc");
        assert_eq!(
            id.token(),
            "content://com.android.providers/document/raw%3A/a b"
        );
        let path = AssetPath::new(id.clone(), "Holiday 2026.mov").to_path();
        assert!(is_reference(&path));
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "Holiday 2026.mov"
        );
        let back = AssetPath::parse(&path).unwrap();
        assert_eq!(back.id, id);
        assert_eq!(back.name, "Holiday 2026.mov");
        assert!(AssetPath::parse(Path::new("/home/me/file.txt")).is_none());
        assert!(!is_reference(Path::new("/home/me/file.txt")));
    }

    #[test]
    fn names_cannot_escape_their_reference() {
        let path = AssetPath::new(AssetId::new("x", "1"), "../../etc/passwd").to_path();
        let back = AssetPath::parse(&path).unwrap();
        assert!(!back.name.contains('/'));
    }

    #[test]
    fn a_local_file_goes_under_another_name() {
        let dir = tempfile::tempdir().unwrap();
        let apk = dir.path().join("base.apk");
        std::fs::write(&apk, b"PK\x03\x04 app").unwrap();
        let path = LocalFiles::reference(&apk, "Maps 11.2.apk");
        assert_eq!(path.file_name().unwrap(), "Maps 11.2.apk");
        let st = crate::source::stat(&path).unwrap();
        assert_eq!(st.len, 8);
        let mut back = Vec::new();
        io::Read::read_to_end(&mut crate::source::open(&path).unwrap(), &mut back).unwrap();
        assert_eq!(back, b"PK\x03\x04 app");
    }

    #[test]
    fn media_types_come_from_mime_or_name() {
        assert_eq!(MediaType::of("image/heic", "x"), MediaType::Image);
        assert_eq!(MediaType::of("", "clip.MOV"), MediaType::Video);
        assert_eq!(MediaType::of("audio/mpeg", "a"), MediaType::Audio);
        assert_eq!(MediaType::of("", "report.pdf"), MediaType::Document);
        assert_eq!(MediaType::of("", "x.zip"), MediaType::Archive);
        assert_eq!(
            MediaType::of("application/octet-stream", "x.bin"),
            MediaType::Other
        );
    }
}
