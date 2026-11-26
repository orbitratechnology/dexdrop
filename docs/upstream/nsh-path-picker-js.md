# Draft issue for longbridge/nsh: expose path-based file picking to the JS API

> Not filed: posting to the NSH repository needs the project owner's go-ahead.

**Title:** Expose path-based file picking to the JS API (and a no-copy option on mobile)

**Problem.** NSH 0.7.0's Rust `pick_paths` (#2138) picks files without reading
them into memory, but JS has only `Picker.pickFiles`, which returns each file's
bytes (at most 512 MiB, `MAX_PICKED_FILE_BYTES`); a larger file comes back
`TooLarge`. File-transfer and media apps cannot let users pick a 2 GB video
from TSX. On iOS and Android `pick_paths` also hands back app-owned copies
(`asCopy: true`, the Android cache), so picking a large file costs its size
in storage and the time to copy it before the app can show it.

**Proposal.**

1. `Picker.pickPaths(options)` in JS, mirroring `pick_paths`: returns
   `{ path, name, size, mime }[]`, cancellable (resolves to `null`), with the
   same filters, and the documented ownership of the returned files (who
   deletes copies, when).
2. An option on `pick_paths`/`pickPaths` (`copy: false`) that returns
   references instead of copies on mobile: Android content URIs with
   persistable read grants, iOS security-scoped URLs (open in place), plus a
   streaming reader (`openPicked(ref) -> Read + Seek` in Rust, a
   `ReadableStream` in JS) and a `release(ref)`.
3. The same for share-in (`next_open_files`): Android's `ShareReceiver`
   copies each shared stream into the cache (16 files, 256 MiB each).

**Why.** Large media and documents are the common case for sharing apps;
copies double storage use and delay selection.
