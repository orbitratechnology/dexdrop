//! PhotoKit, from whichever thread calls (never the main one: the calls
//! wait for PhotoKit's answers).

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    time::Duration,
};

use block2::RcBlock;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSSortDescriptor, NSString, NSURL};
use objc2_photos::{
    PHAccessLevel, PHAsset, PHAssetMediaType, PHAssetResource, PHAssetResourceManager,
    PHAssetResourceRequestOptions, PHAssetResourceType, PHAuthorizationStatus, PHFetchOptions,
    PHImageContentMode, PHImageManager, PHImageRequestOptions, PHImageRequestOptionsDeliveryMode,
    PHImageRequestOptionsResizeMode, PHPhotoLibrary,
};

use super::{Access, Asset, thumb_name};

/// Thumbnails' longer side, in pixels.
const THUMB_PX: f64 = 256.0;

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    /// `UIImageJPEGRepresentation(UIImage *, CGFloat) -> NSData *`.
    fn UIImageJPEGRepresentation(image: *mut AnyObject, quality: f64) -> *mut NSData;
}

fn status_access(status: PHAuthorizationStatus) -> Access {
    match status {
        PHAuthorizationStatus::Authorized => Access::Full,
        PHAuthorizationStatus::Limited => Access::Limited,
        PHAuthorizationStatus::NotDetermined => Access::NotDetermined,
        _ => Access::Denied,
    }
}

pub fn access() -> Access {
    // SAFETY: a class method with a valid access level.
    status_access(unsafe {
        PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite)
    })
}

pub fn request_access() -> Access {
    if access() != Access::NotDetermined {
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
    rx.recv().map(status_access).unwrap_or(Access::Denied)
}

pub fn choose_more() {
    dispatch2::DispatchQueue::main().exec_async(|| {
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

pub fn recent(limit: usize, thumb_dir: &Path) -> Vec<Asset> {
    if !matches!(access(), Access::Full | Access::Limited) {
        return Vec::new();
    }
    objc2::rc::autoreleasepool(|_| recent_in_pool(limit, thumb_dir))
}

fn recent_in_pool(limit: usize, thumb_dir: &Path) -> Vec<Asset> {
    let _ = std::fs::create_dir_all(thumb_dir);
    // SAFETY: documented PhotoKit calls with valid arguments; every result
    // is used while retained.
    unsafe {
        let options = PHFetchOptions::new();
        let key = NSString::from_str("creationDate");
        let newest = NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&key), false);
        options.setSortDescriptors(Some(&NSArray::from_retained_slice(&[newest])));
        // Audio assets are left out below; ask for a few more.
        options.setFetchLimit(limit + 8);
        let found = PHAsset::fetchAssetsWithOptions(Some(&options));
        let mut out = Vec::new();
        for i in 0..found.count() {
            if out.len() == limit {
                break;
            }
            let asset = found.objectAtIndex(i);
            let video = match asset.mediaType() {
                PHAssetMediaType::Image => false,
                PHAssetMediaType::Video => true,
                _ => continue,
            };
            let Some(resource) = original(&asset) else {
                continue;
            };
            let id = asset.localIdentifier().to_string();
            let created = asset
                .creationDate()
                .map(|d| d.timeIntervalSince1970().max(0.0) as u64)
                .unwrap_or(0);
            let thumb_path = thumb_dir.join(thumb_name(&id));
            // A pool per picture: this thread has none of its own, and the
            // image and its JPEG are autoreleased.
            let made = thumb_path.exists()
                || objc2::rc::autoreleasepool(|_| write_thumb(&asset, &thumb_path));
            let thumb = if made { Some(thumb_path) } else { None };
            out.push(Asset {
                id,
                name: resource.originalFilename().to_string(),
                video,
                created,
                duration_ms: if video {
                    (asset.duration() * 1000.0).max(0.0) as u64
                } else {
                    0
                },
                thumb,
            });
        }
        out
    }
}

/// The resource holding what Photos shows: the edited version of an edited
/// photo or video, else the original (a Live Photo's still, not its video).
unsafe fn original(asset: &PHAsset) -> Option<Retained<PHAssetResource>> {
    // SAFETY: documented PhotoKit calls on a retained asset.
    unsafe {
        let resources = PHAssetResource::assetResourcesForAsset(asset);
        let wanted: &[PHAssetResourceType] = if asset.mediaType() == PHAssetMediaType::Video {
            &[
                PHAssetResourceType::FullSizeVideo,
                PHAssetResourceType::Video,
            ]
        } else {
            &[
                PHAssetResourceType::FullSizePhoto,
                PHAssetResourceType::Photo,
            ]
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

/// Writes a JPEG of about [`THUMB_PX`] of `asset` to `path`; `false` when
/// none could be made (only in iCloud: thumbnails are not downloaded).
unsafe fn write_thumb(asset: &PHAsset, path: &Path) -> bool {
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
    // SAFETY: documented PhotoKit calls; synchronous, so the handler runs
    // on this thread before the call returns.
    unsafe {
        let options = PHImageRequestOptions::new();
        options.setSynchronous(true);
        options.setDeliveryMode(PHImageRequestOptionsDeliveryMode::HighQualityFormat);
        options.setResizeMode(PHImageRequestOptionsResizeMode::Fast);
        options.setNetworkAccessAllowed(false);
        let manager = PHImageManager::defaultManager();
        // Bound for macOS only by objc2-photos (UIImage on iOS): sent as is.
        let _: i32 = msg_send![
            &*manager,
            requestImageForAsset: asset,
            targetSize: CGSize::new(THUMB_PX, THUMB_PX),
            contentMode: PHImageContentMode::AspectFill,
            options: &*options,
            resultHandler: &*handler
        ];
    }
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Some(jpeg)) => std::fs::write(path, jpeg).is_ok(),
        _ => false,
    }
}

pub fn export(id: &str, dir: &Path) -> Result<PathBuf, String> {
    objc2::rc::autoreleasepool(|_| export_in_pool(id, dir))
}

fn export_in_pool(id: &str, dir: &Path) -> Result<PathBuf, String> {
    // SAFETY: documented PhotoKit calls with valid arguments.
    unsafe {
        let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
        let found = PHAsset::fetchAssetsWithLocalIdentifiers_options(&ids, None);
        if found.count() == 0 {
            return Err("the photo is no longer in the library".into());
        }
        let asset = found.objectAtIndex(0);
        let resource = original(&asset).ok_or("the photo has no file to send")?;
        let name = sanitize(&resource.originalFilename().to_string());
        let path = dir.join(&name);
        let _ = std::fs::remove_file(&path);
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let options = PHAssetResourceRequestOptions::new();
        options.setNetworkAccessAllowed(true);
        let (tx, rx) = mpsc::channel::<Option<String>>();
        let tx = Mutex::new(Some(tx));
        let handler = RcBlock::new(move |error: *mut NSError| {
            let failed = error.as_ref().map(|e| e.localizedDescription().to_string());
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(failed);
            }
        });
        PHAssetResourceManager::defaultManager()
            .writeDataForAssetResource_toFile_options_completionHandler(
                &resource,
                &url,
                Some(&options),
                &handler,
            );
        // A long video may first come down from iCloud.
        match rx.recv_timeout(Duration::from_secs(30 * 60)) {
            Ok(None) => Ok(path),
            Ok(Some(why)) => Err(why),
            Err(_) => Err("the photo library did not answer".into()),
        }
    }
}

/// `name` as a single file name.
fn sanitize(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        "photo".into()
    } else {
        name
    }
}
