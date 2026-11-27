# File selection and media browsing: audit and design

Branch `claude/file-selection-architecture` (experimental). Goal: browse and
select photos, videos, audio, documents and apps inside DexDrop, as
references, and stream the originals into the transfer engine only when a
transfer starts: no copy for selection, no whole file in memory.

Decision record: `docs/ARCHITECTURE.md` ADR-009. Store-policy note: an
in-app media browser on Android needs `READ_MEDIA_IMAGES`/`VIDEO`/`AUDIO`,
which Google Play restricts to apps whose core use needs broad media access
(ADR-008 removed them from the main line for that reason). This branch adds
them back behind the browser's own prompt; whether it ships depends on a
Play Console declaration being accepted (a file-transfer app's core use).

## 1. Audit (NSH 0.7.0, DexDrop at the start of the branch)

### NSH APIs

| API | Returns | Copies? | Memory | Streams? | Cancel | Notes |
|---|---|---|---|---|---|---|
| `pick_paths` (Rust, NSH #2138) | paths | **iOS/Android: yes**, app-owned copies (Android: app cache; iOS: `UIDocumentPicker` `asCopy: true`); desktops: no | constant | the copy is a file | the system picker's Cancel only | the only large-file path NSH has; `nsh-core` docs: "returns app-owned copies without loading the files into memory" |
| `pick_files` (Rust) / `Picker.pickFiles` (JS) | bytes | read into memory | **whole file**, ≤ 512 MiB (`MAX_PICKED_FILE_BYTES`) | no | system Cancel | larger files come back `TooLarge` with name and size only |
| `pick_images` / `Picker.pickImages` | bytes (re-encoded unless `PickFormat::Current`) | in memory | whole image | no | system Cancel | photo picker; no videos with paths |
| `next_open_files` (share sheet, Open With) | paths | **Android: copies** into `nsh-shared-files` (16 files, 256 MiB each) | constant | — | — | `ShareReceiver.java` |
| `FileSystem` (JS host fs) | bytes | — | bounded whole reads | no | — | confined to the app's private root; no streams |
| `share`, `open_path` | — | — | — | — | — | local paths only |
| `nsh::permissions` (Android) | states, request | — | — | — | — | runtime permissions from Rust |
| `nsh_extra::photos` | JPEG bytes | re-encoded | whole image | no | — | not originals, no videos |
| JS API | — | — | — | — | — | **no path-based picker in JS** (upstream issue drafted: `docs/upstream/nsh-path-picker-js.md`); DexDrop is Rust-only, so it does not need it |

Native extension mechanisms NSH gives a Rust app: `ndk-context` (Java VM
and application `Context`, set by NSH) for JNI; crates' `[package.metadata.nsh.android]`
(`libraries` for a dex, `permissions`, `queries`, `application` entries such
as services and activities); `[package.metadata.nsh.ios.infoPlist]`;
objc2 for Apple frameworks from Rust (no Swift needed).

### DexDrop before the branch

- The engine sends **paths**: `Manifest::build` stats them
  (`symlink_metadata`, `walkdir`), the sender opens each with
  `std::fs::File`, checks size and mtime against the manifest, reads chunks
  on a blocking thread into a bounded channel (`READ_AHEAD`), hashes
  (BLAKE3) and resumes from verified offsets by seeking. Memory is bounded;
  nothing is read whole except files under the small-file threshold.
- Phones pick through `pick_paths` (a copy, moved into `staging`) or
  `pick_images`/`pick_files` (bytes, written to `staging`). `staging` is
  cleaned when nothing selects or sends a copy (ADR-004), but a 3 GB video
  costs 3 GB of extra storage while selected, and the copy takes time
  before the selection shows it.
- Android already reaches Java through `dexdrop-net`'s dex (NsdManager,
  apps, the foreground service); iOS reaches Apple frameworks with objc2
  (Bonjour, the removed PhotoKit crate in history).

### Reuse / extend / implement

| | What |
|---|---|
| Reuse | the engine's chunked, hashed, resumable sender; `ndk-context` + dex bridges; objc2; `nsh::permissions`; NSH lists (`virtual_list`) and images; `pick_paths` on the desktops |
| Extend | the engine's source access: a reference path resolves through a provider (`dexdrop_core::asset`) instead of `std::fs`; `Manifest::build` and the sender's open/check/read accept any `Read + Seek` source with a size and mtime |
| Implement in the app | Android: MediaStore browsing (paged), thumbnails (`loadThumbnail`), originals by file descriptor, a Storage Access Framework picker returning content URIs with persistable grants; iOS: PhotoKit browsing, `PHCachingImageManager` thumbnails, a streaming `PHAssetResourceManager` reader, a `UIDocumentPicker` opening in place with security scope; the browser UI |
| Upstream | a path-based picker in JS; an option for `pick_paths` and share-in to return references (no copy) on mobile |

## 2. Design

### Reference paths

The app's selection and the engine's `SendFiles` carry `PathBuf`s. A
selected asset is a **reference path**: `dexdrop-asset:<provider>/<token>/<name>`
(`dexdrop_core::asset::AssetPath`). It is opaque above the provider: the
token is the provider's, URL-safe (a MediaStore id, an escaped content URI,
a PhotoKit local identifier); raw content URIs and PhotoKit objects never
leave the provider. Everything that only needs names (labels, rows) keeps
working; everything that touches the bytes goes through `dexdrop_core::source`.

### Providers

```rust
pub trait AssetProvider: Send + Sync {
    fn scheme(&self) -> &str;
    fn metadata(&self, id: &AssetId) -> io::Result<MediaItem>;          // no read of the content
    fn open(&self, id: &AssetId) -> io::Result<Box<dyn SourceReader>>; // Read + Send + stat()
    fn list(&self, query: &Query) -> io::Result<Page>;                 // paged (default: unsupported)
    fn thumbnail(&self, id: &AssetId, px: u32) -> io::Result<Option<PathBuf>>; // cached file
    fn albums(&self, kinds: &[MediaType]) -> io::Result<Vec<(String, usize)>>;
    fn release(&self, id: &AssetId);                                   // e.g. a kept grant
}
```

Registered once at start (`asset::register`). Blocking by design: the engine
reads on blocking threads already, and the UI calls providers through
`blocking(…)`. `source::stat(path)` and `source::open(path)` dispatch
reference paths to their provider and everything else to `std::fs`.

| Provider | List | Metadata | Original | Thumbnail |
|---|---|---|---|---|
| `android-media` (MediaStore) | paged `ContentResolver.query` (images, videos, audio; newest first) | the row (size, date modified, duration, dimensions) | `openFileDescriptor(uri, "r")` → `detachFd` → `std::fs::File` (seekable, `fstat` size and mtime) | `loadThumbnail(uri, size)` (API 29+), else the legacy thumbnails, cached by id and size |
| `android-doc` (SAF) | the system picker (`ACTION_OPEN_DOCUMENT`, several, any type) | `OpenableColumns` + `COLUMN_LAST_MODIFIED` | as above; cloud providers that hand a pipe are read forward-only (resume restarts) | `DocumentsContract.getDocumentThumbnail` where offered |
| `ios-photos` (PhotoKit) | paged `PHAsset` fetch (images, videos), newest first | asset dates, duration, pixel size; resource name and size | `PHAssetResourceManager.requestData(for:…)` into a bounded channel (backpressure blocks PhotoKit's delivery); forward seeks skip, backward seeks restart; iCloud allowed with progress | `PHCachingImageManager` at grid size, cached |
| iOS documents | `UIDocumentPickerViewController(forOpeningContentTypes:asCopy: false)` | the file's | the provider's file in place, under security-scoped access kept while selected (coordinated read first for iCloud placeholders) | — |
| `local` (built in) | — | `std::fs` | `std::fs::File` | — (a file sent under another name: an app's APK as `Maps 11.2.apk`, read where it is) |

### What changes for a 3 GB video

Listing reads rows, not files. Selecting adds a reference path (no I/O).
The manifest asks the provider for size and mtime. The transfer opens the
original when the receiver accepts, reads chunks into a bounded channel,
hashes and sends them; pausing drops the reader (PhotoKit's request is
cancelled), resuming opens it again and reads forward, hashing the part the
receiver already has. Nothing is copied; memory stays at `READ_AHEAD`
chunks (plus PhotoKit's 8 pieces in flight on iOS).

## 3. Implementation (branch `claude/file-selection-architecture`)

| Milestone | State | Where |
|---|---|---|
| 1 Audit | done | §1 |
| 2 Upstream issue (path picker in JS, no-copy option) | drafted, not filed | `docs/upstream/nsh-path-picker-js.md` |
| 3 Core types and provider trait | done | `dexdrop-core/src/asset.rs`, `source.rs` |
| 4 Android MediaStore | done: paged queries over images, videos and audio (`MediaStore.Files`, `QUERY_ARG_OFFSET`/`LIMIT` on 11+), search, sort, size floor, albums (buckets), `loadThumbnail`, originals by detached file descriptor | `dexdrop-net/android/dev/dexdrop/AssetsBridge.java`, `dexdrop-net/src/assets.rs` |
| 4 Android documents (SAF) | done: `ACTION_OPEN_DOCUMENT` from an invisible activity, several of any type, `takePersistableUriPermission`, read by descriptor; Files on Android uses it | `PickerActivity.java`, `assets::pick_documents` |
| 5 iOS PhotoKit | done; run in the iOS 26 simulator by CI (run 105): after PhotoKit's prompt (Allow Full Access) the browser lists the library (7 items, thumbnails, a HEIC) and adds a photo as a reference with its size from PhotoKit (433 B, nothing exported); not yet on a device: paged fetch (lazy `PHFetchResult`), albums, `PHCachingImageManager` thumbnails, streamed originals (`requestDataForAssetResource`, iCloud allowed, bounded, cancelled on drop), limited access with "choose more" | `crates/dexdrop-photos` |
| 5 iOS documents in place | **not done**: `UIDocumentPicker` with `asCopy: false` and security-scoped access needs a delegate class (objc2 `define_class!`); Files on iOS still uses NSH's picker, which copies | — |
| 6 Unified selection model | done: selections hold reference paths; Send's sizes come from the provider (`files::stat_of`) | `app/src/files.rs`, `pages/send.rs` |
| 7 Streaming integration | done: the engine reads every source through `source::open`; tested with a non-file provider, uneven reads, pause and resume | `dexdrop-net/tests/assets.rs` |
| 8 Media browser UI | done: Photos, Videos, Audio, Large files; virtualized grid (NSH `virtual_list`, columns) and lists, paged on scroll, lazy thumbnails (4 at a time, newest first), search, sort, albums, multi-select, select all, count and size, Add in thumb reach, permission card, limited-access row | `app/src/pages/library.rs`, `app/tests/library.rs` |
| 9 Downloads / Documents / file browser | partial: documents through the system's picker (SAF) on Android; no in-app folder browser (Android grants no folder listing without `MANAGE_EXTERNAL_STORAGE`, which this app does not ask for) | — |
| 10 Android apps | kept; a single-APK app is now sent in place under a readable name (`LocalFiles`), split APKs are still staged | `dexdrop-net/src/apps.rs` |
| 11 Performance | headless: a library of 5000 lists only its first pages and keeps under 200 tiles; engine: 6 MiB and 24 MiB referenced items stream byte-identical | `app/tests/library.rs`, `dexdrop-net/tests/assets.rs` |
| 12 Docs, CI | done on the branch: ADR-009, TESTING, the APK, emulator and simulator checks expect the browser | `docs/ARCHITECTURE.md`, `.github/workflows/ci.yml`, `scripts/*-check.sh` |

Run on CI's devices: the Android emulator lists a photo pushed into MediaStore with its system thumbnail and adds it with no copy in the app's storage (run 103); the iOS simulator as above (run 105).

Not measured yet: times to first thumbnail and to the first byte sent from
a 3 GB video on real devices (the emulator and simulator checks run the
browser, not timings).
