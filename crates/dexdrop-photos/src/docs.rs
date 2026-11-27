//! Documents picked in place on iOS (`ios-doc`): the system's document
//! picker in open mode (`UIDocumentPickerViewController`, `asCopy: false`)
//! hands over the provider's own files, not copies (NSH's `pick_paths`
//! copies). Each one is kept as a bookmark of its URL, the item's token,
//! resolved when the item is described or read; its security scope is open
//! only while that happens (a transfer's read included), so nothing stays
//! accessible longer than it is used and nothing is copied.
//!
//! Not handled yet: an iCloud Drive file not downloaded to the device
//! (a placeholder) fails to open; a coordinated read (`NSFileCoordinator`)
//! would download it first.

use std::{
    cell::RefCell,
    fs::File,
    io,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

use dexdrop_core::asset::{
    AssetId, AssetProvider, AssetSource, MediaItem, MediaType, SourceReader, Stat, register,
};
use objc2::{
    AllocAnyThread, DefinedClass, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject, Bool, NSObject},
};
use objc2_foundation::{
    NSArray, NSData, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
};

use super::{hex, sanitize, unhex};
use crate::ios::{front_controller, run_on_main};

use super::DOC_SCHEME;

#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

/// Where the picker's answer goes.
struct Ivars {
    answer: Mutex<Option<mpsc::Sender<Vec<Retained<NSURL>>>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the delegate
    // methods below take the arguments UIKit passes them.
    #[unsafe(super(NSObject))]
    #[ivars = Ivars]
    struct PickerDelegate;

    impl PickerDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(&self, _picker: &AnyObject, urls: &NSArray<NSURL>) {
            self.answer(urls.to_vec());
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &AnyObject) {
            self.answer(Vec::new());
        }
    }
);

impl PickerDelegate {
    fn new(answer: mpsc::Sender<Vec<Retained<NSURL>>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            answer: Mutex::new(Some(answer)),
        });
        // SAFETY: NSObject's designated initializer, on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }

    fn answer(&self, urls: Vec<Retained<NSURL>>) {
        if let Some(tx) = self.ivars().answer.lock().ok().and_then(|mut a| a.take()) {
            let _ = tx.send(urls);
        }
    }
}

thread_local! {
    /// The delegate of the picker showing (a picker holds its delegate
    /// weakly): kept on the main thread until the next pick replaces it.
    static DELEGATE: RefCell<Option<Retained<PickerDelegate>>> = const { RefCell::new(None) };
}

/// Shows the picker (main thread only); `tx` gets the URLs picked, none
/// when cancelled or when the picker cannot show.
unsafe fn present(tx: mpsc::Sender<Vec<Retained<NSURL>>>) {
    // SAFETY (whole function): documented UIKit and UTType messages on the
    // main thread, each result checked for nil before use.
    unsafe {
        let shown = (|| {
            let controller = front_controller()?;
            let utt = AnyClass::get(c"UTType")?;
            let any: Option<Retained<AnyObject>> =
                msg_send![utt, typeWithIdentifier: &*NSString::from_str("public.item")];
            let types = NSArray::from_retained_slice(&[any?]);
            let class = AnyClass::get(c"UIDocumentPickerViewController")?;
            let alloc: Allocated<AnyObject> = msg_send![class, alloc];
            let picker: Option<Retained<AnyObject>> =
                msg_send![alloc, initForOpeningContentTypes: &*types, asCopy: false];
            let picker = picker?;
            let _: () = msg_send![&*picker, setAllowsMultipleSelection: true];
            let delegate = PickerDelegate::new(tx.clone());
            let _: () = msg_send![&*picker, setDelegate: &*delegate];
            DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
            let _: () = msg_send![
                &*controller,
                presentViewController: &*picker,
                animated: true,
                completion: std::ptr::null::<block2::Block<dyn Fn()>>()
            ];
            Some(())
        })();
        if shown.is_none() {
            let _ = tx.send(Vec::new());
        }
    }
}

/// Shows the system's document picker (several, any type) and waits for the
/// user: the documents picked, as items read in place; none when
/// cancelled. Blocks: call it off the main thread.
pub fn pick() -> io::Result<Vec<MediaItem>> {
    let (tx, rx) = mpsc::channel();
    // SAFETY: run on the main thread.
    run_on_main(move || unsafe { present(tx) });
    let urls = rx
        .recv()
        .map_err(|_| io::Error::other("the document picker did not answer"))?;
    let mut items = Vec::new();
    for url in urls {
        let scope = Scope::open(url);
        // A file URL the picker handed over, read in its scope.
        let bookmark = scope
            .url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::empty(),
                None,
                None,
            );
        let Ok(bookmark) = bookmark else { continue };
        let id = AssetId::new(DOC_SCHEME, &hex(&bookmark.to_vec()));
        if let Ok(item) = describe(&id, &scope) {
            items.push(item);
        }
    }
    Ok(items)
}

/// A document's URL with its security scope open, closed when dropped.
struct Scope {
    url: Retained<NSURL>,
    opened: bool,
}

// SAFETY: NSURL is immutable and thread-safe; starting and stopping its
// security scope may happen on any thread.
unsafe impl Send for Scope {}

impl Scope {
    fn open(url: Retained<NSURL>) -> Scope {
        // SAFETY: a file URL; balanced by the stop in drop.
        let opened = unsafe { url.startAccessingSecurityScopedResource() };
        Scope { url, opened }
    }

    /// The document `id` names, its scope open.
    fn of(id: &AssetId) -> io::Result<Scope> {
        let bytes = unhex(&id.token())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a document"))?;
        let data = NSData::with_bytes(&bytes);
        let mut stale = Bool::NO;
        // SAFETY: bookmark data this module made; `stale` is a valid out
        // pointer.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::empty(),
                None,
                &mut stale,
            )
        }
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                e.localizedDescription().to_string(),
            )
        })?;
        Ok(Scope::open(url))
    }

    fn path(&self) -> io::Result<PathBuf> {
        self.url
            .path()
            .map(|p| PathBuf::from(p.to_string()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not a file"))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.opened {
            // SAFETY: balances the start in `open`.
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
    }
}

fn describe(id: &AssetId, scope: &Scope) -> io::Result<MediaItem> {
    let path = scope.path()?;
    let meta = std::fs::metadata(&path)?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
    }
    let name = sanitize(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    Ok(MediaItem {
        id: id.clone(),
        media_type: MediaType::of("", &name),
        name,
        mime_type: String::new(),
        size: Some(meta.len()),
        source: AssetSource::IosDocumentProvider,
        created_at: None,
        modified_at: modified,
        duration: None,
        width: None,
        height: None,
        album: None,
    })
}

/// A document read in place, its scope open while it is read.
struct Opened {
    file: File,
    _scope: Scope,
}

impl io::Read for Opened {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl SourceReader for Opened {
    fn stat(&self) -> io::Result<Stat> {
        SourceReader::stat(&self.file)
    }
}

/// The `ios-doc` provider.
struct Docs;

impl AssetProvider for Docs {
    fn scheme(&self) -> &str {
        DOC_SCHEME
    }

    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem> {
        describe(id, &Scope::of(id)?)
    }

    fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>> {
        let scope = Scope::of(id)?;
        let file = File::open(scope.path()?)?;
        Ok(Box::new(Opened {
            file,
            _scope: scope,
        }))
    }
}

pub fn register_docs() {
    register(Arc::new(Docs));
}
