//! PhotoKit, from whichever thread calls (never the main one: the calls
//! wait for PhotoKit's answers, which come on PhotoKit's own queues).

use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc,
    },
    time::Duration,
};

use block2::RcBlock;
use dexdrop_core::asset::{
    AssetId, AssetProvider, AssetSource, MediaItem, MediaType, Page, Query, Sort, SourceReader,
    Stat,
};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSPredicate, NSSortDescriptor, NSString,
};
use objc2_photos::{
    PHAccessLevel, PHAsset, PHAssetCollection, PHAssetCollectionSubtype, PHAssetCollectionType,
    PHAssetMediaType, PHAssetResource, PHAssetResourceManager, PHAssetResourceRequestOptions,
    PHAssetResourceType, PHAuthorizationStatus, PHCachingImageManager, PHFetchOptions,
    PHFetchResult, PHImageContentMode, PHImageRequestID, PHImageRequestOptions,
    PHImageRequestOptionsDeliveryMode, PHImageRequestOptionsResizeMode, PHPhotoLibrary,
};

use super::{Access, SCHEME, sanitize, thumb_name};

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    /// `UIImageJPEGRepresentation(UIImage *, CGFloat) -> NSData *`.
    fn UIImageJPEGRepresentation(image: *mut AnyObject, quality: f64) -> *mut NSData;
}

fn status_access(status: PHAuthorizationStatus) -> Access {
    match status {
        PHAuthorizationStatus::Authorized => Access::All,
        PHAuthorizationStatus::Limited => Access::Some,
        PHAuthorizationStatus::NotDetermined => Access::NotAsked,
        _ => Access::Refused,
    }
}

pub fn access() -> Access {
    // SAFETY: a class method with a valid access level.
    status_access(unsafe {
        PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite)
    })
}

pub fn request_access() -> Access {
    if access() != Access::NotAsked {
        return access();
    }
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(Some(tx));
    let handler = RcBlock::new(move |status: PHAuthorizationStatus| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(status);
        }
    });
    // SAFETY: the block lives until PhotoKit calls it (PhotoKit copies it).
    unsafe {
        PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(
            PHAccessLevel::ReadWrite,
            &handler,
        );
    }
    // The user answers the prompt: no deadline.
    rx.recv().map(status_access).unwrap_or(Access::Refused)
}

pub fn choose_more() {
    run_on_main(|| {
        // SAFETY: on the main thread, as UIKit requires; every object is
        // checked for nil before use.
        unsafe {
            let Some(controller) = front_controller() else {
                return;
            };
            let library = PHPhotoLibrary::sharedPhotoLibrary();
            let _: () = msg_send![
                &*library,
                presentLimitedLibraryPickerFromViewController: &*controller
            ];
        }
    });
}

/// Runs `work` on the main thread (GCD's main queue).
fn run_on_main(work: impl FnOnce() + Send + 'static) {
    unsafe extern "C" {
        static _dispatch_main_q: AnyObject;
        fn dispatch_async_f(
            queue: *const AnyObject,
            context: *mut std::ffi::c_void,
            work: extern "C" fn(*mut std::ffi::c_void),
        );
    }
    extern "C" fn trampoline(context: *mut std::ffi::c_void) {
        // SAFETY: the box `run_on_main` leaked, taken back once.
        let work = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
        work();
    }
    let work: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(work));
    // SAFETY: the main queue lives for the process; the context is a leaked
    // box the trampoline takes back exactly once.
    unsafe {
        dispatch_async_f(
            &raw const _dispatch_main_q,
            Box::into_raw(work).cast(),
            trampoline,
        );
    }
}

/// The view controller in front: the key window's root, then what it
/// presents. Main thread only.
unsafe fn front_controller() -> Option<Retained<AnyObject>> {
    let app_class = AnyClass::get(c"UIApplication")?;
    // SAFETY (whole function): documented UIKit messages on the main
    // thread, each result checked for nil.
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![app_class, sharedApplication];
        let app = app?;
        let scenes: Retained<AnyObject> = msg_send![&*app, connectedScenes];
        let scenes: Retained<AnyObject> = msg_send![&*scenes, allObjects];
        let count: usize = msg_send![&*scenes, count];
        let window_scene = AnyClass::get(c"UIWindowScene")?;
        for i in 0..count {
            let scene: Retained<AnyObject> = msg_send![&*scenes, objectAtIndex: i];
            let is_window_scene: bool = msg_send![&*scene, isKindOfClass: window_scene];
            if !is_window_scene {
                continue;
            }
            let windows: Retained<AnyObject> = msg_send![&*scene, windows];
            let n: usize = msg_send![&*windows, count];
            for j in 0..n {
                let window: Retained<AnyObject> = msg_send![&*windows, objectAtIndex: j];
                let key: bool = msg_send![&*window, isKeyWindow];
                if !key {
                    continue;
                }
                let mut controller: Option<Retained<AnyObject>> =
                    msg_send![&*window, rootViewController];
                while let Some(c) = controller.clone() {
                    let presented: Option<Retained<AnyObject>> =
                        msg_send![&*c, presentedViewController];
                    match presented {
                        Some(p) => controller = Some(p),
                        None => break,
                    }
                }
                return controller;
            }
        }
        None
    }
}

// ------------------------------------------------------------ the provider

/// The `ios-photos` provider.
pub struct Photos {
    thumbs: PathBuf,
}

impl Photos {
    pub fn new(thumbs: PathBuf) -> Self {
        Photos { thumbs }
    }
}

fn readable() -> io::Result<()> {
    match access() {
        Access::All | Access::Some => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the photo library is not allowed",
        )),
    }
}

/// `mediaType == 1 OR mediaType == 2` for `kinds` (PhotoKit's images and
/// videos; it holds no other kind DexDrop lists).
fn predicate(kinds: &[MediaType]) -> Option<Retained<NSPredicate>> {
    let mut terms = Vec::new();
    if kinds.contains(&MediaType::Image) {
        terms.push(format!("mediaType == {}", PHAssetMediaType::Image.0));
    }
    if kinds.contains(&MediaType::Video) {
        terms.push(format!("mediaType == {}", PHAssetMediaType::Video.0));
    }
    if kinds.contains(&MediaType::Audio) {
        terms.push(format!("mediaType == {}", PHAssetMediaType::Audio.0));
    }
    if terms.is_empty() {
        terms.push("mediaType == -1".into());
    }
    // SAFETY: a format of literals only, no arguments to substitute.
    Some(unsafe {
        NSPredicate::predicateWithFormat_argumentArray(
            &NSString::from_str(&terms.join(" OR ")),
            None,
        )
    })
}

fn options(kinds: &[MediaType], sort: Sort) -> Retained<PHFetchOptions> {
    // SAFETY: documented PhotoKit setters with valid values.
    unsafe {
        let options = PHFetchOptions::new();
        options.setPredicate(predicate(kinds).as_deref());
        let oldest = sort == Sort::Oldest;
        let key = NSString::from_str("creationDate");
        let by = NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&key), oldest);
        options.setSortDescriptors(Some(&NSArray::from_retained_slice(&[by])));
        options
    }
}

/// The album called `title`: a user album or a smart album (Favorites,
/// Screenshots…).
fn album(title: &str) -> Option<Retained<PHAssetCollection>> {
    for kind in [
        PHAssetCollectionType::Album,
        PHAssetCollectionType::SmartAlbum,
    ] {
        // SAFETY: documented PhotoKit fetches; results used while retained.
        unsafe {
            let found = PHAssetCollection::fetchAssetCollectionsWithType_subtype_options(
                kind,
                PHAssetCollectionSubtype::Any,
                None,
            );
            for i in 0..found.count() {
                let c = found.objectAtIndex(i);
                if c.localizedTitle().is_some_and(|t| t.to_string() == title) {
                    return Some(c);
                }
            }
        }
    }
    None
}

fn fetch(q: &Query) -> io::Result<Retained<PHFetchResult<PHAsset>>> {
    let options = options(&q.kinds, q.sort);
    // SAFETY: documented PhotoKit fetches with valid options.
    unsafe {
        match &q.album {
            Some(title) => {
                let collection = album(title)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such album"))?;
                Ok(PHAsset::fetchAssetsInAssetCollection_options(
                    &collection,
                    Some(&options),
                ))
            }
            None => Ok(PHAsset::fetchAssetsWithOptions(Some(&options))),
        }
    }
}

/// The resource holding what Photos shows: the edited version of an edited
/// photo or video, else the original (a Live Photo's still, not its video).
unsafe fn original(asset: &PHAsset) -> Option<Retained<PHAssetResource>> {
    // SAFETY: documented PhotoKit calls on a retained asset.
    unsafe {
        let resources = PHAssetResource::assetResourcesForAsset(asset);
        let wanted: &[PHAssetResourceType] = match asset.mediaType() {
            PHAssetMediaType::Video => &[
                PHAssetResourceType::FullSizeVideo,
                PHAssetResourceType::Video,
            ],
            PHAssetMediaType::Audio => &[PHAssetResourceType::Audio],
            _ => &[
                PHAssetResourceType::FullSizePhoto,
                PHAssetResourceType::Photo,
            ],
        };
        for kind in wanted {
            for i in 0..resources.count() {
                let resource = resources.objectAtIndex(i);
                if resource.r#type() == *kind {
                    return Some(resource);
                }
            }
        }
        None
    }
}

/// The resource's size in bytes. PhotoKit keeps it in the resource's
/// `fileSize` key (no property declares it); `None` when it does not say.
unsafe fn file_size(resource: &PHAssetResource) -> Option<u64> {
    // SAFETY: key-value coding on a retained NSObject; an unknown key raises
    // only for keys the class does not hold, and PHAssetResource has held
    // `fileSize` since iOS 9.
    unsafe {
        let key = NSString::from_str("fileSize");
        let value: Option<Retained<AnyObject>> = msg_send![resource, valueForKey: &*key];
        let value = value?;
        let n: i64 = msg_send![&*value, longLongValue];
        u64::try_from(n).ok().filter(|n| *n > 0)
    }
}

fn seconds(date: Option<Retained<objc2_foundation::NSDate>>) -> Option<i64> {
    date.map(|d| d.timeIntervalSince1970() as i64)
}

/// An asset as an item; `None` when it has no file to send.
unsafe fn describe(asset: &PHAsset) -> Option<MediaItem> {
    // SAFETY: documented PhotoKit getters on a retained asset.
    unsafe {
        let resource = original(asset)?;
        let media_type = match asset.mediaType() {
            PHAssetMediaType::Video => MediaType::Video,
            PHAssetMediaType::Audio => MediaType::Audio,
            _ => MediaType::Image,
        };
        let name = sanitize(&resource.originalFilename().to_string());
        let duration =
            (media_type != MediaType::Image).then(|| (asset.duration() * 1000.0).max(0.0) as u64);
        Some(MediaItem {
            id: AssetId::new(SCHEME, &asset.localIdentifier().to_string()),
            media_type,
            // The type is the name's (PhotoKit's is a UTI, not a MIME type).
            mime_type: String::new(),
            size: file_size(&resource),
            source: AssetSource::IosPhotoLibrary,
            created_at: seconds(asset.creationDate()),
            modified_at: seconds(asset.modificationDate()).or(seconds(asset.creationDate())),
            duration,
            width: u32::try_from(asset.pixelWidth()).ok().filter(|w| *w > 0),
            height: u32::try_from(asset.pixelHeight()).ok().filter(|h| *h > 0),
            album: None,
            name,
        })
    }
}

fn asset_of(id: &AssetId) -> io::Result<Retained<PHAsset>> {
    let token = NSString::from_str(&id.token());
    // SAFETY: documented PhotoKit fetch with a valid identifier array.
    unsafe {
        let found = PHAsset::fetchAssetsWithLocalIdentifiers_options(
            &NSArray::from_retained_slice(&[token]),
            None,
        );
        if found.count() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the photo is no longer in the library",
            ));
        }
        Ok(found.objectAtIndex(0))
    }
}

/// The image manager thumbnails come from: PhotoKit's caching manager, kept
/// for the process (it keeps what it made for the sizes asked).
fn images() -> &'static Retained<PHCachingImageManager> {
    static MANAGER: OnceLock<Retained<PHCachingImageManager>> = OnceLock::new();
    // SAFETY: a plain allocation of PhotoKit's class.
    MANAGER.get_or_init(|| unsafe { PHCachingImageManager::new() })
}

/// How long a thumbnail may take: past it the tile shows its kind.
const THUMB_WAIT: Duration = Duration::from_secs(5);

/// Writes a JPEG of about `px` of `asset` to `path`; `false` when none
/// could be made (only in iCloud: thumbnails are not downloaded) or it took
/// longer than [`THUMB_WAIT`]. Asked asynchronously, with a deadline: a
/// synchronous request blocks the thread until PhotoKit answers.
unsafe fn write_thumb(asset: &PHAsset, px: u32, path: &Path) -> bool {
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    let tx = Mutex::new(Some(tx));
    let handler = RcBlock::new(move |image: *mut AnyObject, _info: *mut NSDictionary| {
        let jpeg = if image.is_null() {
            None
        } else {
            // SAFETY: a UIImage PhotoKit hands over for the call; the
            // NSData returned is autoreleased and copied out at once.
            unsafe {
                let data = UIImageJPEGRepresentation(image, 0.8);
                data.as_ref().map(|d| d.to_vec())
            }
        };
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(jpeg);
        }
    });
    let side = f64::from(px);
    // SAFETY: documented PhotoKit calls; PhotoKit copies the handler and
    // calls it once (high quality only) on the main queue, which this
    // thread never blocks.
    let request = unsafe {
        let options = PHImageRequestOptions::new();
        options.setSynchronous(false);
        options.setDeliveryMode(PHImageRequestOptionsDeliveryMode::HighQualityFormat);
        options.setResizeMode(PHImageRequestOptionsResizeMode::Fast);
        options.setNetworkAccessAllowed(false);
        // Bound for macOS only by objc2-photos (UIImage on iOS): sent as is.
        let request: PHImageRequestID = msg_send![
            &**images(),
            requestImageForAsset: asset,
            targetSize: CGSize::new(side, side),
            contentMode: PHImageContentMode::AspectFill,
            options: &*options,
            resultHandler: &*handler
        ];
        request
    };
    match rx.recv_timeout(THUMB_WAIT) {
        Ok(Some(jpeg)) => {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
            std::fs::write(path, jpeg).is_ok()
        }
        Ok(None) => false,
        Err(_) => {
            // SAFETY: the ID PhotoKit returned for this request.
            unsafe { images().cancelImageRequest(request) };
            false
        }
    }
}

impl AssetProvider for Photos {
    fn scheme(&self) -> &str {
        SCHEME
    }

    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
        readable()?;
        objc2::rc::autoreleasepool(|_| {
            let asset = asset_of(id)?;
            // SAFETY: a retained asset.
            unsafe { describe(&asset) }.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "the photo has no file to send")
            })
        })
    }

    fn list(&self, q: &Query) -> io::Result<Page> {
        readable()?;
        objc2::rc::autoreleasepool(|_| {
            let found = fetch(q)?;
            // SAFETY: a fetch result PhotoKit made; indexes below its count.
            let total = unsafe { found.count() };
            let search = q.search.as_ref().map(|s| s.to_lowercase());
            let filtered = search.is_some() || q.min_size.is_some();
            let mut items = Vec::new();
            // Without a filter, the page is the fetch's rows offset..: the
            // rest are never read. A name or a size is the resource's,
            // which PhotoKit cannot filter on: rows are read in order until
            // the page is full.
            let mut skipped = 0;
            let start = if filtered { 0 } else { q.offset };
            let mut at = start;
            while at < total && items.len() < q.limit + 1 {
                // SAFETY: `at` is below the count; a pool per row, as
                // PhotoKit's objects are autoreleased.
                let item =
                    objc2::rc::autoreleasepool(|_| unsafe { describe(&found.objectAtIndex(at)) });
                at += 1;
                let Some(item) = item else { continue };
                if search
                    .as_ref()
                    .is_some_and(|s| !item.name.to_lowercase().contains(s))
                {
                    continue;
                }
                if q.min_size.is_some_and(|min| item.size.unwrap_or(0) < min) {
                    continue;
                }
                if filtered && skipped < q.offset {
                    skipped += 1;
                    continue;
                }
                items.push(item);
            }
            if q.sort == Sort::Largest {
                items.sort_by_key(|i| std::cmp::Reverse(i.size));
            } else if q.sort == Sort::Name {
                items.sort_by_key(|i| i.name.to_lowercase());
            }
            let more = items.len() > q.limit;
            items.truncate(q.limit);
            Ok(Page { items, more })
        })
    }

    fn albums(&self, kinds: &[MediaType]) -> io::Result<Vec<(String, usize)>> {
        readable()?;
        objc2::rc::autoreleasepool(|_| {
            let options = options(kinds, Sort::Newest);
            let mut out = Vec::new();
            for kind in [
                PHAssetCollectionType::SmartAlbum,
                PHAssetCollectionType::Album,
            ] {
                // SAFETY: documented PhotoKit fetches; results used while
                // retained.
                unsafe {
                    let found = PHAssetCollection::fetchAssetCollectionsWithType_subtype_options(
                        kind,
                        PHAssetCollectionSubtype::Any,
                        None,
                    );
                    for i in 0..found.count() {
                        let c = found.objectAtIndex(i);
                        // "Recents" is everything: the "All" chip.
                        if c.assetCollectionSubtype()
                            == PHAssetCollectionSubtype::SmartAlbumUserLibrary
                        {
                            continue;
                        }
                        let Some(title) = c.localizedTitle() else {
                            continue;
                        };
                        let n = PHAsset::fetchAssetsInAssetCollection_options(&c, Some(&options))
                            .count();
                        if n > 0 {
                            out.push((title.to_string(), n));
                        }
                    }
                }
            }
            out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            Ok(out)
        })
    }

    fn thumbnail(&self, id: &AssetId, px: u32) -> io::Result<Option<PathBuf>> {
        let path = self.thumbs.join(thumb_name(&id.token(), px));
        if path.metadata().is_ok_and(|m| m.len() > 0) {
            return Ok(Some(path));
        }
        readable()?;
        objc2::rc::autoreleasepool(|_| {
            let asset = asset_of(id)?;
            // SAFETY: a retained asset.
            let made = unsafe { write_thumb(&asset, px, &path) };
            Ok(made.then_some(path))
        })
    }

    fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
        readable()?;
        let item = self.metadata(id)?;
        let stat = Stat {
            is_dir: false,
            len: item.size.ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "the photo's size is not known")
            })?,
            mtime: item.modified_at.and_then(|t| u64::try_from(t).ok()),
        };
        objc2::rc::autoreleasepool(|_| {
            let asset = asset_of(id)?;
            // SAFETY: a retained asset.
            let resource = unsafe { original(&asset) }.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "the photo has no file to send")
            })?;
            Ok(Box::new(Stream::start(&resource, stat)) as Box<dyn SourceReader>)
        })
    }
}

// ------------------------------------------------------------ streaming

/// Pieces of an original in flight: what PhotoKit handed over and the
/// engine has not read yet. Bounded: PhotoKit's handler waits while it is
/// full, so a slow transfer holds a few pieces, not the file.
const PIECES: usize = 8;

enum Piece {
    Data(Vec<u8>),
    Done(Result<(), String>),
}

/// An original read as PhotoKit streams it (`requestDataForAssetResource`,
/// downloaded from iCloud when it is there). Dropping it cancels the
/// request.
struct Stream {
    rx: mpsc::Receiver<Piece>,
    current: Vec<u8>,
    at: usize,
    finished: bool,
    request: Arc<AtomicI32>,
    dropped: Arc<AtomicBool>,
    stat: Stat,
}

impl Stream {
    fn start(resource: &PHAssetResource, stat: Stat) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Piece>(PIECES);
        let request = Arc::new(AtomicI32::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let data_tx = tx.clone();
        let gone = dropped.clone();
        let on_data = RcBlock::new(move |data: std::ptr::NonNull<NSData>| {
            if gone.load(Ordering::Relaxed) {
                return;
            }
            // SAFETY: an NSData PhotoKit hands over for the call, copied out
            // at once.
            let bytes = unsafe { data.as_ref() }.to_vec();
            // Waits while the engine has PIECES unread: PhotoKit's queue
            // slows down with the transfer.
            if data_tx.send(Piece::Data(bytes)).is_err() {
                gone.store(true, Ordering::Relaxed);
            }
        });
        let done_tx = Mutex::new(Some(tx));
        let on_done = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: an NSError PhotoKit hands over for the call, or nil.
            let result = match unsafe { error.as_ref() } {
                Some(e) => Err(e.localizedDescription().to_string()),
                None => Ok(()),
            };
            if let Some(tx) = done_tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(Piece::Done(result));
            }
        });
        // SAFETY: documented PhotoKit calls; PhotoKit copies both blocks
        // and calls them on its own queue.
        let id = unsafe {
            let options = PHAssetResourceRequestOptions::new();
            options.setNetworkAccessAllowed(true);
            PHAssetResourceManager::defaultManager()
                .requestDataForAssetResource_options_dataReceivedHandler_completionHandler(
                    resource,
                    Some(&options),
                    &on_data,
                    &on_done,
                )
        };
        request.store(id, Ordering::Relaxed);
        Stream {
            rx,
            current: Vec::new(),
            at: 0,
            finished: false,
            request,
            dropped,
            stat,
        }
    }
}

/// How long the engine waits for the next piece (a long video may first
/// come down from iCloud, a piece at a time).
const PIECE_WAIT: Duration = Duration::from_secs(10 * 60);

impl io::Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at == self.current.len() {
            if self.finished {
                return Ok(0);
            }
            match self.rx.recv_timeout(PIECE_WAIT) {
                Ok(Piece::Data(d)) => {
                    self.current = d;
                    self.at = 0;
                }
                Ok(Piece::Done(Ok(()))) => self.finished = true,
                Ok(Piece::Done(Err(why))) => {
                    self.finished = true;
                    return Err(io::Error::other(why));
                }
                Err(_) => {
                    self.finished = true;
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the photo library did not answer",
                    ));
                }
            }
        }
        let n = buf.len().min(self.current.len() - self.at);
        buf[..n].copy_from_slice(&self.current[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl SourceReader for Stream {
    fn stat(&self) -> io::Result<Stat> {
        Ok(self.stat)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Relaxed);
        if !self.finished {
            // SAFETY: the ID PhotoKit returned for this request.
            unsafe {
                PHAssetResourceManager::defaultManager()
                    .cancelDataRequest(self.request.load(Ordering::Relaxed));
            }
        }
        // Unblock a handler waiting on a full channel.
        while self.rx.try_recv().is_ok() {}
    }
}
