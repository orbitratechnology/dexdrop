# Status against the PRD

Legend: **Done** (implemented and tested), **Partial**, **Open**.
"Verified" means exercised in this repository's tests, by running the app on
Linux, or by the end-to-end checks that drive the packaged apps on GitHub's
Windows, macOS and Linux runners and the iOS Simulator on every CI run.

## Platforms (PRD 8.1, Phase 0 exit criteria)

| Target | Build | Run / validate | Notes |
|---|---|---|---|
| Linux x86_64 | Done (`nsh package --target linux`, 15 MB tarball) | Verified: two app instances discover each other and transfer; CLI ⇄ app; 300 MB byte-identical. In CI (under Xvfb), the packaged app is started on every CI run by `scripts/desktop-check.py`: the CLI on the same machine discovers it, sends a file and a folder (4 files, 5.3 MB, nested, with an empty file) that the app accepts and receives byte-identical, and the app records the transfer as completed (screenshots in `docs/screenshots/desktop/`) | Needs GTK 4.14+ (Ubuntu 24.04+) |
| Android | Done, here and in CI (APK, arm64-v8a + armeabi-v7a + x86_64, debug-signed, targetSdk 35) | Checked end to end in an Android 14 emulator on every CI run by agent-device (`scripts/android-emu-check.sh`, screenshots in `docs/screenshots/android/`): launch and tabs; discovery both ways through the system's NsdManager against the CLI running inside the emulator; 3 MB sent with the pairing code (read through its screen-reader label) and 200 KB with the invitation read from the sheet's QR code, both accepted in the app and verified. Not yet run on a phone | Discovery uses `NsdManager` through a Java bridge embedded as a dex (`crates/dexdrop-net/android/`), because NSH v0.6.1's packaging cannot declare `CHANGE_WIFI_MULTICAST_STATE` for a multicast lock. Needs Android 12+ (older versions fall back to mdns-sd). NSH `main` already adds `android.permissions` and sets `ndk-context` (longbridge/nsh#2500, #2686); adopt on its next release (`docs/upstream/nsh-android-permissions.md`). The emulator is behind its own NAT, so phone ⇄ computer discovery needs a real phone. |
| Web | Done (static site, 7 MB) | Verified in Chromium | Browser preview only: states that browsers cannot discover or connect directly (FR-084) |
| Windows | Done in CI (`nsh package --target windows`) | Verified in CI: engine tests and throughput bench pass; the packaged app is started on every CI run by `scripts/desktop-check.py`: the CLI on the same machine discovers it, sends a file and a folder (4 files, 5.3 MB, nested, with an empty file) that the app accepts and receives byte-identical, and the app records the transfer as completed (screenshots in `docs/screenshots/desktop/`) | Not yet tried by hand on a Windows PC |
| macOS | CI on `macos-15` (free for this public repository) | Verified in CI: the packaged app is started on every CI run by `scripts/desktop-check.py`: the CLI on the same machine discovers it, sends a file and a folder (4 files, 5.3 MB, nested, with an empty file) that the app accepts and receives byte-identical, and the app records the transfer as completed (screenshots in `docs/screenshots/desktop/`); the app also lists the CLI as a trusted nearby device | Unsigned, local use only. Uses the system's Bonjour service; macOS 15 asks before an app uses the local network, and the prompt shows `NSLocalNetworkUsageDescription` from `nsh.json` (the CI check answers Allow) |
| iOS | CI on `macos-15` (`ios-simulator`) | Verified in the iOS Simulator by agent-device on every CI run (`scripts/ios-sim-check.sh`, screenshots in `docs/screenshots/ios/`): launch and tabs; the app and the CLI on the Mac discover each other; a 3 MB file sent with the pairing code (read through its screen-reader label) and a 200 KB file sent with the invitation link, both accepted in the app and received byte-identical. Not yet run on a device | Devices discover through the system's Bonjour service (`dns_sd`), declared in `NSBonjourServices` with `NSLocalNetworkUsageDescription`; the simulator build uses the mdns-sd responder, because the Mac's mDNSResponder refuses simulator apps on CI (local-network trust check) |

## App design

The app is built for people who are not technical: few words, an icon on
every action, and the platform's own controls where NSH has them.

- **Navigation:** three tabs, *Share*, *History* and *Settings*, each a
  native navigation stack (`nav_stack`, back by swipe or the system's back
  button). The tab bar is the system's (`UITabBar`, Liquid Glass on iOS 26;
  Material 3 on Android); desktops and the web get a sidebar with the same
  tabs. Switches are the system's on phones (`UISwitch`, Material), and the
  file-conflict choice and the History filter use iOS's segmented control.
- **Share (home):** two large *Send* and *Receive* buttons, nearby devices as
  avatars (tap one to send to it), what is moving now, and recent
  transfers.
- **Theme:** one indigo accent on cool neutrals (`app/theme.json`), with a
  near-black dark mode; every page paints the theme's background, so light
  and dark look the same everywhere. iOS keeps the system's large,
  transparent header; Android's header follows the theme and never covers
  the page.
- **Send:** Photos, Videos or Files on phones (Files or Folder on
  desktops, plus drag and drop onto a drop zone), picked photos as a grid
  of previews and other files with their kind and size, then *Choose
  device*: a radar with the nearby devices, or *Use a code*. While the
  system hands over picked files, a card says so ("Getting your photos…",
  "Preparing 12 files…").
- **Receive:** this device on a radar, ready for nearby devices, with its
  code and QR code for devices that cannot see it.
- **Incoming request:** a bottom sheet with the sender, what it sends, the
  verification number to compare and *Accept* / *Decline*; trust and
  what to do with existing files are under *Options*.
- **Transfer:** a progress ring with speed and time left, *Pause* /
  *Resume* (*Retry* after a failure), *Cancel* (confirmed with the system's
  alert), *Reconnecting* while a dropped connection is retried, and a check
  mark when done, with *Share or save* on phones and *Show in folder* on
  desktops; its files are listed with previews.
- **History details:** each past transfer opens a screen with its photos,
  outcome, the other device, when, how long, average speed, route,
  encryption, issues, the folder it went to and every file (up to 100,
  previews for the first 24), each opening on a tap.
- **Apps (Android):** a fourth Send tile lists the installed apps with
  their icon, name, version, size and last update (system apps behind a
  switch) and APK files from the phone's files; chosen apps are copied as
  `"Name 1.2.apk"` (apps installed as split APKs go as a folder of APKs).
  APKs anywhere in the app show the app they install, and opening a
  received APK hands it to the system's installer. Seeing other apps needs
  a `<queries>` entry and installing needs `REQUEST_INSTALL_PACKAGES`,
  which NSH 0.6.1's `nsh.json` cannot declare: CI adds both to the packaged
  APK with apktool and signs it again (`scripts/patch-android-manifest.sh`).
- **Lists and images, as NSH recommends:** every list that can grow
  (History, this session's transfers, a transfer's files, the Send
  selection's photo grid and files, Apps, trusted devices) is a
  `virtual_list` in the page's scroll view (`ListViewport::Parent`),
  keyed by id, with separators and estimated row sizes, so only rows near
  the screen exist; rows read their data through the row's position, as
  recycled cells must. Bounded content (settings groups, the avatar
  picker, nearby devices) stays plain. Icons are SVG assets
  (`asset!("icons/…")`, shipped with `registry.assets(assets!())`,
  checked at build time) tinted to the theme; photos decode at the size
  they are shown, with the file's icon underneath until (or unless) they
  draw. `cargo run` lays the assets out for the desktop runner
  (`dev_assets`).
- **Performance, against NSH's guide:** measured headless by
  `app/tests/perf.rs`, which counts the mount items each step sends. Idle
  screens send nothing (animations are native keyframes; no signal is
  written per frame; no scroll listeners). A history of 2,000 costs about
  what one of 20 does to open (virtualized). Tabs stay mounted, invisible
  while another shows, so coming back to one sends 3 mount items, where
  it rebuilt the page (≈1,300) before; a video pauses while its tab is
  away. History has a signal of its own, so transfer progress does not
  re-filter it; the Send selection reads file sizes off the engine thread
  and removing a file updates the list in place; app rows rebind on
  recycle instead of being rebuilt. To watch a device: `NSH_FPS=1` (frame
  rate, frame and commit cost) on any build, `--release` for numbers.
- **Player:** videos play, and photos show, inside the app (NSH's
  `video`, with the platform's own controls: AVPlayer, MediaPlayer,
  GStreamer through GTK, Media Foundation), from the Send selection, a
  transfer, History or its details, with length, resolution and size,
  and Share or save (phones) / Show in folder and Open in another app
  (computers). On Linux GTK ends the app when GStreamer's `playbin` is
  missing, so the player checks for the plugin first and says what to
  install. Windows builds enable NSH's `media` feature.
- **Icons:** devices carry their system's mark (Apple, Windows, Android,
  Linux, the browser's globe); files an icon per kind with their format on
  a colored tag (PDF, XLSX, CSV, HTML, SVG, GIF, APK, DMG, EXE, DEB, …).
- **Avatars:** each device chooses an emoji on one of eight colors in
  Settings; others see it (mDNS TXT `a`, and the handshake's hello, whose
  new last field older builds ignore and read as none), on requests,
  transfers, trusted devices and history.
- **Icons:** 36 original 24 × 24 line icons (`app/assets/icons/`), embedded
  as SVG; file types and settings rows have their own colors; devices get
  an avatar with a platform badge.

## Functional requirements (PRD 6), audited 2026-11-23

Every requirement was checked against the code, the tests and the running
app: CI's device checks (Android emulator, iOS Simulator, macOS, Windows and
Linux desktops) and a live iOS 26 Simulator session (native-sim). Evidence
names the test or check; "Gap" is what is still missing.

| ID | P | Status | Evidence | Gap |
|---|---|---|---|---|
| FR-001 No account | P0 | Done | Starts on the home screen; no sign-in exists | |
| FR-002 Cryptographic identity | P0 | Done | Ed25519, OS credential store on Windows, Linux, iOS (core unit tests, CI Windows keystore) | macOS Keychain needs signed builds; Android Keystore needs NSH's `ndk-context` |
| FR-003 Device name | P0 | Done | Generic default (`iPhone DSO6`), rename in Settings (UI test) | |
| FR-004 Local-network explanation | P0 | Done | iOS/macOS: one screen before discovery starts, then the system prompt (UI test `explains_the_local_network_once`, engine test, seen in the iOS Simulator); plist strings set | Android 17's permission (target API 37) when the target moves |
| FR-005 Empty states | P0 | Done | Home: looking / blocked network → use a code / after 8 s what to check; History, Trusted devices, browser preview | |
| FR-010 Discover on LAN | P0 | Done | Bonjour (`dns_sd`) on Apple devices, NsdManager on Android, mdns-sd elsewhere; CI: both directions on Android, iOS, macOS; desktop CLI→app | Real cross-device pairs (phone ⇄ computer) not run |
| FR-011 Device info | P0 | Done | Avatar, name, platform badge; no IP or port in the UI | |
| FR-012 Deduplicate | P0 | Done | Keyed by peer ID (engine) | |
| FR-013 Network changes | P0 | Done | Re-advertises on address change (5 s poll); after returning from the background or waking from sleep (NSH `lifecycle`, `wake_count`) it looks again and retries at once the transfers that gave up | |
| FR-014 Denied permission | P0 | Done | Discovery failure says the network blocks it and points to the code; no prompt loop (the app never asks itself) | No deep link to iOS Settings (NSH has none) |
| FR-015 Trusted devices | P1 | Done | Trust on accept, list, forget (engine tests) | |
| FR-016 Connection quality | P1 | Done | Measured MB/s only, after a transfer | |
| FR-020 QR invitation | P0 | Done | Receive page: QR + code, single-use tokens (CI iOS/Android: invitation read from the QR) | |
| FR-021 Join by invitation/code | P0 | Done | Code entry, paste, QR from a picture; SPAKE2 for codes (engine tests, CI on iOS, Android, macOS) | |
| FR-022 Expiry | P0 | Done | 10 min, 5 attempts, replay rejected (engine tests `invitations_are_single_use`, `code_guesses_are_limited…`) | |
| FR-023 Confirm peer | P0 | Done | Name, platform and the 6-digit verification number on both screens | |
| FR-024 Camera QR | P1 | Open | — | NSH 0.6.1 has no camera API; iOS VisionKit needs a device to test; Android needs a manifest permission NSH cannot declare |
| FR-025 QR from image | P1 | Done | App and CLI (tests) | |
| FR-030 Select files | P0 | Done | Desktop dialogs (CI macOS Open panel); iOS Photos/Files pickers (iOS Simulator); Android picker (CI) | |
| FR-031 Folders | P0 | Partial | Desktop: folder dialog and drop, nested, empty folders (CI desktop check) | Phones cannot pick a folder (NSH pickers are files only) |
| FR-032 Streaming | P0 | Done | Bounded pipeline; 300 MB byte-identical; memory flat | 20 GB test not run |
| FR-033 Pre-send summary | P0 | Done | Selection with count and size; "Send to <device>" | |
| Several devices at once | — | Done | Tap each device (the list stays, each row shows its transfer), "Send to all nearby", group progress with Pause all; one reading of the selection for all (ADR-003); CLI `send "A,B"`; `tests/multi.rs`, `sends_to_several_devices_at_once`, `bench --to N` | Swarm relaying between recipients not done (ADR-003) |
| FR-034 Any file type | P0 | Done | Opaque bytes, no compression | |
| FR-035 Share-sheet entry | P1 | Open | — | Needs an iOS share extension / Android intent filter NSH's packaging cannot add |
| FR-036 Drag and drop | P1 | Done | Desktop drop overlay; path parsing tests on every OS | |
| FR-037 Clipboard text | P2 | Open | — | Later |
| FR-040 Consent | P0 | Done | Request sheet; nothing saved before Accept (engine tests) | |
| FR-041 Details before accepting | P0 | Done | Sender, files, size, verification number, where it is saved | |
| FR-042 Destination | P0 | Done | Desktop: "Save to" folder; phones: app storage + "Share or save" | |
| FR-043 No silent overwrite | P0 | Done | Keep both / Skip / Replace (confirmed) / Decline (engine tests) | |
| FR-044 Free space | P0 | Done | Checked before and during (RAM-disk test) | |
| FR-045 Path safety | P0 | Done | Traversal, absolute, reserved names, symlinks (adversarial tests) | One local race noted under Security |
| FR-046 Verified commit | P0 | Done | Temp file → BLAKE3 → atomic rename | |
| FR-047 Per-device policy | P1 | Done | "Accept without asking" per trusted device | |
| FR-050 Queue states | P0 | Done (adapted) | Preparing, connecting, waiting for an answer, transferring, paused, reconnecting, done, declined, cancelled, failed (History "This session", home "Now") | No separate "Verifying": chunks and files are verified as they arrive |
| FR-051 Progress | P0 | Done | Bytes, MB/s, time left after a stable sample; ≤ 4 updates/s | |
| FR-052 Pause/resume | P0 | Done | From the app in CI (Android, macOS): receiver holds still, file arrives intact | |
| FR-053 Recover after a drop | P0 | Done | Journals every 2 s; reconnect and request what is missing (engine test through a dropping link) | |
| FR-054 Cancel | P0 | Done | Confirmed with the system alert; temp files removed | |
| FR-055 Retry | P0 | Done | Retry with a plain reason; permanent failures not retried | |
| FR-056 Integrity | P0 | Done | Per-chunk and whole-file BLAKE3 (corruption tests) | |
| FR-057 Many files | P0 | Done | 250k cap, paged manifest; 20,000 files at 40 MB peak | |
| FR-058 Concurrency | P1 | Done (adapted) | 64 pipelined requests on one stream | |
| FR-059 Duplicates | P1 | Done | Name and size, then hash, receiver side | |
| FR-060 History | P1 | Done | Local, capped, Clear, Keep-history switch | |
| FR-070 Device name | P0 | Done | | |
| FR-071 Visibility | P0 | Done | Switch; hidden devices still receive by code | |
| FR-072 Network data policy | P0 | Partial | Local transfers never use a relay; internet transfers have their own switch (on by default) | No mobile-data check yet: an internet transfer may use mobile data while the switch is on |
| FR-073 Privacy and storage | P0 | Done | Settings says what stays on the device and where files go | |
| FR-074 Trusted devices | P1 | Done | | |
| FR-075 Diagnostics export | P1 | Done | No names, paths, contents or keys | |
| FR-080 Remote connection | P1 | Done | Iroh 1.5: direct path with hole punching, the route shown (direct / through a relay) and followed live; engine test over a local relay | Real NATs and carrier networks not measured |
| FR-081 Relay fallback | P1 | Partial | Relay carries only encrypted data (two layers: Iroh and DexDrop's TLS); configurable (`DEXDROP_RELAY_URL`, `--relay`) | Uses n0's public relays until the project runs its own `iroh-relay` (PRD 9.5) |
| FR-082 Remote consent | P1 | Done | Same consent, trust and verification number as local; invitations carry the key and relay; known devices from any network | Codes stay local (no rendezvous service) |
| FR-083 – FR-084 Browser | P2 | Partial | Web preview says browsers cannot discover or connect | Phase 7 |

## UX and platform requirements (PRD 7, 8, 10)

| Requirement | Status | Evidence / gap |
|---|---|---|
| 7.1 Structure | Done | Share (home), History, Settings; native stacks; desktop sidebar |
| 7.2 Mobile home | Done | Device name and visibility, nearby devices, Send / Receive, activity badge on History; code fallback under Send › Choose device |
| 7.3 Desktop home | Done | Sidebar, drop target, application menu (File, Edit, View; macOS app and Window menus), Cmd/Ctrl shortcuts, notifications |
| 7.4 Transfer card | Done | Peer and platform, name, size, state, MB/s, time left, Pause/Resume/Retry/Cancel, plain errors |
| 7.5 States | Done | Pressed, hover, keyboard focus ring (every control), disabled, loading, success, error |
| 7.6 Visual design | Done | Theme tokens, light and dark, system type; richer visuals than the PRD's "minimal" at the product owner's request |
| 7.7 Accessibility | Done | Labels and roles (UI tests), focus rings, text follows Dynamic Type / font size (NSH); with large text Home's actions and the transfer controls stack (`tests/device.rs`), 44 pt iOS / 48 dp Android targets (UI test), reduced motion, state announcements; times on the user's 12- or 24-hour clock |
| 8.3 iOS Bonjour without multicast entitlement | Done | Fixed service type, plist keys |
| 8.4 Android local network | Partial | NSD; API 37 permission/picker not needed at target 35 |
| 8.5 / 10.2 Android background transfers | Open | Needs a foreground service or user-initiated job: NSH's packaging cannot declare the `<service>`. Meanwhile, leaving the app mid-transfer posts a notification to return; haptics mark requests and results |
| 8.5 / 10.1 iOS background | Done | Foreground transfers; journals every 2 s; resume on return |
| 10.1 iOS files | Done | Photos and Files pickers, app storage, share sheet export |
| 10.3 macOS | Done | Open panels, Finder drop, menus, Show in folder; menu-bar extra not done (optional) |
| 10.4 Windows | Partial | Dialogs, Explorer drop, notifications, per-user installer, taskbar progress (NSH `WindowProgress`; the Dock on macOS), icon badge for waiting requests |
| 10.5 Linux | Partial | GTK 4.14+ documented, dialogs; tested under X11 (Xvfb) only, not GNOME/KDE on Wayland |

## Against the recommended architecture (Discovery → Connection → Transfer → Verification)

Audited 2026-11-23 against the PRD's technology stack.

| Layer | Recommended | DexDrop | Status |
|---|---|---|---|
| UI | NSH.dev | NSH 0.6.1, Rust-only; native tab bar, headers, pickers, video | Done |
| Core | Rust + Tokio, UI-independent | `dexdrop-core` (protocol, identity, manifest, journal; no I/O) and `dexdrop-net` (engine on Tokio); the app only sends commands and renders events | Done |
| Separate modules | discovery / connection / orchestration / streaming | `discovery/`, `tls.rs` + `remote.rs` (connection), `engine.rs` (orchestration), `session.rs` (streaming); sessions run over any `AsyncRead + AsyncWrite` (`BoxLink`), so a transport is swappable | Done |
| Discovery | mDNS + DNS-SD, Bonjour, Android NSD | Bonjour on Apple, NSD on Android, mdns-sd on Windows and Linux (not Avahi's API; it shares the port) | Done |
| QR / short codes | QR + codes | QR invitations (shown, copied, imported from an image) and 8-character codes proven with SPAKE2 | Done; camera scanning open (FR-024: NSH has no camera API) |
| BLE | optional | — | Not planned for v1 |
| Identity | public-key | Ed25519 device key in the OS keystore, pinned certificates, verification number, trust list | Done |
| Encryption | TLS 1.3 | rustls, TLS 1.3 only, mutual | Done |
| Transport (same network) | Iroh / QUIC preferred, TCP + TLS fallback | **TCP + TLS 1.3** (ADR-001): 3.4× QUIC's throughput at a quarter of its CPU on loopback | Deviation, by benchmark; the lossy-Wi-Fi comparison on real devices is still owed |
| Transport (other networks) | Iroh, NAT traversal, relay fallback | Iroh 1.5 with the same key; LAN first, then direct (hole punching), then relay; the route is shown | Done; production relay of our own still to deploy |
| Manifest | before transfer | paginated, bounded, digest-pinned | Done |
| Chunked streaming, bounded memory | yes | 4 MiB chunks streamed from disk; frames ≤ 256 KiB; 17–40 MB peak for 2,000–20,000 files | Done |
| Backpressure | yes | ≤ 64 requests outstanding (semaphore), TCP/QUIC flow control, ≤ 8 commits in flight | Done |
| Concurrent / multiplexed | multiple streams in one QUIC connection | one connection per transfer with pipelined requests; disk, BLAKE3 and TLS run on separate threads (≈2×: 0.93–1.19 GB/s, 5.37 GB at 51 MB peak); several transfers run at once on separate connections | Partial: no parallel connections per file yet (see \"Large files\" in ARCHITECTURE.md: to be measured on real Wi-Fi first) |
| Adaptive concurrency | yes | fixed limits | Open |
| Zero-copy / reduced-copy | where supported | reads into reused buffers; no `sendfile` (TLS is in user space) | Partial |
| Resume | chunk-level | verified, chunk-aligned prefix per file, journaled every 2 s; the sender sends only what is missing | Done (files are received in order, so a prefix is the chunk set) |
| Integrity | BLAKE3 | per chunk and per file; a failed file is requested again once | Done |
| Phones' files | Files / SAF | Videos, files and APKs through `pick_paths` (the document picker): copied to app storage without memory and with no size limit, moved into `staging`, which the engine cleans (large copies as soon as their transfer ends, others after a week). Photos through the photo library's picker (bytes, ≤ 512 MB each, larger ones named and refused) | Done for 5 GB+, except iPhone videos kept only in the Photos library (the document picker does not reach it; NSH has no video-library picker with paths yet) |
| iOS background | not guaranteed | foreground transfers, journaled, resumed on return | Done as the PRD expects |
| Android background | — | needs a foreground service NSH's packaging cannot declare | Open |
| Browser | WebRTC, optional | web preview of the UI only, no engine | Not planned for v1 |
| Benchmarks | real devices | loopback benches in CI (throughput, files/s, memory, TCP vs QUIC); no real-device numbers yet | Open |

## Quality, acceptance and release gates (PRD 11, 13, 16)

| Area | Status | Gap |
|---|---|---|
| Release blockers (16) | None open | Corruption, unauthenticated transfers, overwrite, traversal, unbounded memory, no resume, relay leaks: each covered by tests |
| Discovery ≤ 3 s, setup ≤ 2 s (11.1) | Not measured | Needs real devices on a LAN |
| Goodput vs reference (11.1) | Loopback only | `dexdrop-bench` on real Wi-Fi/Ethernet |
| 20 GB file (11.1, 11.3) | Not run | Needs a machine with the space |
| Interop matrix (13.1) | Partial | In CI (run 45180003839, all green on the redesigned UI): Android emulator and macOS app receive by code and invitation, then send from the app with Pause and Resume to a CLI; the iOS Simulator receives by code and invitation; Windows and Linux apps receive from the CLI. Real cross-device pairs (Android ⇄ Windows, macOS ⇄ Android, …) need hardware |
| Network matrix (13.2) | Partial | Isolation, VPN, network switches: engine tests and docs only |
| Security tests (13.4) | Done | Adversarial and engine tests; relay review waits for remote mode |
| Windows engine tests | Watch | Twice a transfer test timed out on Windows CI waiting for the receiver to complete (different tests; never on Linux or macOS): a suspected Windows-only stall. The test harness now prints the last engine events on a timeout to locate it |

## Tests

- `dexdrop-core`: 36 unit tests (protocol bounds, path traversal and reserved
  names including `CONIN$` and `COM¹`, invitations and codes, journals (log replay, stale and torn lines),
  bounded "keep both" naming, manifests, identity).
- `dexdrop-net`: 23 end-to-end tests over real TLS on loopback (consent,
  decline, no silent overwrite, skip policy, identical files already
  there not sent again, "keep both" never merging into a
  folder of the same transfer, pause/resume from the verified offset, cancel
  cleanup, wrong-peer pinning, trusted auto-accept, invalid invitations, code
  pairing by SPAKE2 including a fake device over real mDNS, 500 small files
  byte-for-byte, a source edited or deleted mid-transfer, automatic resume
  through a link that drops mid-transfer, and a destination disk that fills
  up on a real RAM disk), 12 adversarial tests (malicious manifests and
  names, oversized frames, offer floods, out-of-place chunks, results, probes and hashes,
  symbolic links planted at temporary names, corruption retried once and
  never saved) and 2 pairing unit tests.
- `dexdrop-app`: 9 headless UI tests on NSH's engine (phone layout): the first-launch local-network explanation (once), home, tabs, re-selecting a tab, the Receive page with its code and QR code, the Send page, keyboard shortcuts, identity reset through the system alert, and a 44 pt minimum hit-target check.

## Security review follow-up

All findings of the internal review are fixed and covered by tests. The
receive journal is a snapshot plus an append-only log of changed files: for
200,000 files a periodic save takes about 3 ms instead of rewriting 32 MB
(`cargo run --release -p dexdrop-core --example journal_cost`). Open:

- A local user who can swap folders inside the destination between the
  symbolic-link check and the final rename could redirect one file. Temporary
  files are created with `O_EXCL | O_NOFOLLOW`; a full fix needs `openat`-style
  directory handles on every platform.

## What's next, in order

1. **Real devices.** Run on an Android phone and an iPhone (free Personal
   Team) against a Windows, macOS and Linux computer on one Wi-Fi: the PRD
   13.1 matrix, discovery and setup times (11.1), goodput against `iperf3`.
2. **Android background transfers** (PRD 8.5): a foreground service or
   user-initiated job, once NSH's packaging can declare it, or by embedding
   NSH in a Gradle project; and a release keystore (P5-002).
3. **Phones: folders and share-sheet entry** (FR-031 on phones, FR-035): need
   NSH picker and packaging support.
4. **Camera QR** (FR-024): VisionKit on iOS, tested on a device.
5. **Desktop polish**: GNOME/KDE Wayland checks, a 20 GB transfer.
6. **Identity keys**: macOS Keychain with signed builds, Android Keystore.
7. **Internet transfers in production:** run the project's own `iroh-relay` and point the apps at it; a mobile-data check (FR-072); measure real NATs. Then the browser client (Phase 7).
