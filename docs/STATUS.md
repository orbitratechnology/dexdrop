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
- **Send:** Photos, Videos or Files on phones (Files or Folder on
  desktops, plus drag and drop), the selection with its size, then *Choose
  device*: a radar with the nearby devices, or *Use a code*.
- **Receive:** this device on a radar, ready for nearby devices, with its
  code and QR code for devices that cannot see it.
- **Incoming request:** a bottom sheet with the sender, what it sends, the
  verification number to compare and *Accept* / *Decline*; trust and
  what to do with existing files are under *Options*.
- **Transfer:** a progress ring with speed and time left, *Pause* /
  *Resume* (*Retry* after a failure), *Cancel* (confirmed with the system's
  alert), *Reconnecting* while a dropped connection is retried, and a check
  mark when done, with *Share or save* on phones and *Show in folder* on
  desktops.
- **Icons:** 36 original 24 × 24 line icons (`app/assets/icons/`), embedded
  as SVG; file types and settings rows have their own colors; devices get
  an avatar with a platform badge.

## Functional requirements

| ID | Status | Notes |
|---|---|---|
| FR-001 No account | Done | |
| FR-002 Cryptographic identity | Done | Ed25519; private key in the OS credential store on Windows, Linux (Secret Service) and iOS, app-private file elsewhere (docs/SECURITY.md); "Reset device identity" in Settings gives a new peer ID (PRD 9.9) |
| FR-003 Device name | Done | Generic default `Linux PC 4F2A`, renamable |
| FR-004 Local-network permission copy | Partial | iOS plist text set; Android flow open |
| FR-005 Empty states | Done | Searching, unavailable, browser preview |
| FR-010 – FR-012 Discovery, info, dedupe | Done (desktop, iOS Simulator, Android emulator) | System Bonjour (`dns_sd`) on macOS and iOS devices, the system's `NsdManager` on Android, mdns-sd on Windows, Linux and the iOS Simulator; deduped by peer ID; TXT updates in place when a code opens |
| FR-013 Network changes | Done | Re-advertises on address change (5 s poll) |
| FR-014 Denied permission | Partial | Fallbacks shown; platform permission flows open |
| FR-015 Trusted devices | Done | Trust on accept, forget, per-device auto-accept |
| FR-016 Connection quality | Done | Measured average speed (time spent transferring only) shown on finished transfers and as "last … MB/s" on each nearby device; nothing shown until measured |
| FR-020 – FR-023 QR, code, expiry, confirmation | Done | QR shown; single-use tokens; codes proven with SPAKE2 (never sent; a device faking an open code gets nothing), 10 min, 5 attempts; SAS on both sides; the receive sheet closes a used code and offers a new one |
| FR-024 Camera QR scanning | Open | Needs a native camera bridge |
| FR-025 QR from a picture | Done | Screenshot or photo, in the app and the CLI; paste from clipboard too |
| FR-030 – FR-034 Selecting and sending | Done (desktop) | Native file/folder dialogs, drag and drop; phones stage picked files |
| FR-035 Share sheet entry | Open | |
| FR-036 Drag and drop | Done | Desktop drop zone |
| Desktop shortcuts (PRD 7.3) | Done | Ctrl/Cmd+O files, Ctrl/Cmd+Shift+O folder, Ctrl/Cmd+1–3 tabs, Ctrl/Cmd+, settings, Esc goes back |
| System notifications (P4-006) | Done | When a transfer finishes, fails or arrives |
| FR-040 – FR-046 Receiving | Done (phones: "Share or save" exports received files through the system share sheet) | Consent, details, destination, keep-both/skip/replace (confirmed), space check, path safety, atomic commit |
| FR-047 Per-device receive policy | Done | |
| FR-050 – FR-056 Lifecycle | Done | Queue states, throttled progress, rate after samples, ETA after 3 s, pause/resume, reconnect with backoff, cancel cleans temp files, verify-and-retry |
| FR-057 Many files | Done for 250k entries | Paged manifest, 250k entry cap; 20,000 files transfer with 40 MB peak memory for both ends (`dexdrop bench --files 20000`) |
| FR-058 Concurrent streams | Done (adapted) | One encrypted stream per transfer with up to 64 pipelined file requests; files that fit in one chunk are read, written, synced and committed in single blocking calls, 8 commits in parallel |
| FR-059 Duplicate detection | Done | Only for a name and size already at the destination; the receiver compares hashes, the sender never sees the receiver's |
| FR-060 History | Done | Local, capped, clearable |
| FR-070 – FR-074 Settings | Done | No mobile-data switch shown: local transfers never use mobile data; the stored setting returns with remote transfers |
| FR-075 Diagnostics | Done | No names, paths or keys |
| FR-080 – FR-084 Remote / browser transfer | Open (P1/P2) | Browser preview explains the limitation |

## Tests

- `dexdrop-core`: 36 unit tests (protocol bounds, path traversal and reserved
  names including `CONIN$` and `COM¹`, invitations and codes, journals (log replay, stale and torn lines),
  bounded "keep both" naming, manifests, identity).
- `dexdrop-net`: 20 end-to-end tests over real TLS on loopback (consent,
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
- `dexdrop-app`: 7 headless UI tests on NSH's engine (phone layout): home, tabs, the Receive page with its code and QR code, the Send page, keyboard shortcuts, identity reset through the system alert, and a 44 pt minimum hit-target check.

## Security review follow-up

All findings of the internal review are fixed and covered by tests. The
receive journal is a snapshot plus an append-only log of changed files: for
200,000 files a periodic save takes about 3 ms instead of rewriting 32 MB
(`cargo run --release -p dexdrop-core --example journal_cost`). Open:

- A local user who can swap folders inside the destination between the
  symbolic-link check and the final rename could redirect one file. Temporary
  files are created with `O_EXCL | O_NOFOLLOW`; a full fix needs `openat`-style
  directory handles on every platform.

## Known gaps worth doing next

1. Run on an iPhone and by hand on Windows and macOS (CI drives sending, pause and resume from the app on Android and macOS).
2. Android: run on a phone (discovery with a computer); SAF file access and a
   share target (need an NSH manifest extension or an app that embeds `NshView`);
   transfers while the app is in the background (Android freezes cached apps, so
   long transfers likely need a foreground service, a manifest `<service>` NSH's
   packaging cannot declare yet; untested).
3. iOS: share sheet entry (FR-035), which needs a share extension NSH's packaging cannot add yet; camera QR scanning (FR-024) through VisionKit, testable only on a device. The Photos and Files pickers work (checked in the simulator).
4. Identity keys: macOS Keychain with signed releases, Android Keystore once NSH sets `ndk-context`.
5. Run `dexdrop-bench` on real Wi-Fi/Ethernet with loss and latency (loopback results are in docs/ARCHITECTURE.md).
6. Remote mode (Iroh with a project-owned relay) as a separately gated phase.
