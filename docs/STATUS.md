# Status against the PRD

Legend: **Done** (implemented and tested), **Partial**, **Open**.
"Verified" means exercised in this repository's tests or by running the app
on Linux; other platforms are built in CI but not yet run on devices.

## Platforms (PRD 8.1, Phase 0 exit criteria)

| Target | Build | Run / validate | Notes |
|---|---|---|---|
| Linux x86_64 | Done (`nsh package --target linux`, 15 MB tarball) | Verified: two app instances discover each other and transfer; CLI ⇄ app; 300 MB byte-identical | Needs GTK 4.14+ (Ubuntu 24.04+) |
| Android | Done, here and in CI (APK, arm64-v8a + armeabi-v7a, debug-signed, targetSdk 35) | Open: no device or emulator available | Manifest has `INTERNET`; mDNS needs `CHANGE_WIFI_MULTICAST_STATE` + a multicast lock or an NSD bridge, which NSH's standalone packaging cannot declare yet (P0-015). Invitations and direct addresses work without discovery. |
| Web | Done (static site, 7 MB) | Verified in Chromium | Browser preview only: states that browsers cannot discover or connect directly (FR-084) |
| Windows | Done in CI (`nsh package --target windows`) | Engine tests and throughput bench pass on Windows in CI; the app has not been run interactively | |
| macOS | CI on `macos-15` (free for this public repository) | Building in CI; not yet run by hand | Unsigned, local use only |
| iOS | CI on `macos-15` (`ios-simulator`) | Verified in the iOS Simulator by agent-device on every CI run: launch, tabs, and a 3 MB file from the CLI received byte-identical (`scripts/ios-sim-check.sh`, screenshots in `docs/screenshots/ios/`) | `NSLocalNetworkUsageDescription` and `NSBonjourServices` set in `nsh.json`; Bonjour adapter via Network framework is open |

## Functional requirements

| ID | Status | Notes |
|---|---|---|
| FR-001 No account | Done | |
| FR-002 Cryptographic identity | Done | Ed25519, persisted; OS keystore storage open |
| FR-003 Device name | Done | Generic default `Linux PC 4F2A`, renamable |
| FR-004 Local-network permission copy | Partial | iOS plist text set; Android flow open |
| FR-005 Empty states | Done | Searching, unavailable, browser preview |
| FR-010 – FR-012 Discovery, info, dedupe | Done (desktop) | mDNS; deduped by peer ID |
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
| Desktop shortcuts (PRD 7.3) | Done | Ctrl/Cmd+O files, Ctrl/Cmd+Shift+O folder, Ctrl/Cmd+1–3 sections, Ctrl/Cmd+, settings, Esc closes sheets |
| System notifications (P4-006) | Done | When a transfer finishes, fails or arrives |
| FR-040 – FR-046 Receiving | Done (phones: "Share or save" exports received files through the system share sheet) | Consent, details, destination, keep-both/skip/replace (confirmed), space check, path safety, atomic commit |
| FR-047 Per-device receive policy | Done | |
| FR-050 – FR-056 Lifecycle | Done | Queue states, throttled progress, rate after samples, ETA after 3 s, pause/resume, reconnect with backoff, cancel cleans temp files, verify-and-retry |
| FR-057 Many files | Done for 250k entries | Paged manifest, 250k entry cap; 20,000 files transfer with 40 MB peak memory for both ends (`dexdrop bench --files 20000`) |
| FR-058 Concurrent streams | Done (adapted) | One encrypted stream per transfer with up to 64 pipelined file requests; files that fit in one chunk are read, written, synced and committed in single blocking calls, 8 commits in parallel |
| FR-059 Duplicate detection | Done | Only for a name and size already at the destination; the receiver compares hashes, the sender never sees the receiver's |
| FR-060 History | Done | Local, capped, clearable |
| FR-070 – FR-074 Settings | Done | Metered-data switch is stored but not yet enforced |
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
- `dexdrop-app`: 6 headless UI tests on NSH's engine (phone layout), including keyboard shortcuts and a 44 pt minimum hit-target check.

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

1. Run the macOS app and the Windows build interactively; extend the iOS Simulator check (sending from the app, pause/resume).
2. Android: a native NSD/multicast-lock bridge and SAF file access (needs an
   NSH packaging extension), then device testing.
3. iOS: Network-framework Bonjour adapter; Files/Photos picker; simulator tests.
4. Store the identity key in the OS keystore.
5. Run `dexdrop-bench` on real Wi-Fi/Ethernet with loss and latency (loopback results are in docs/ARCHITECTURE.md).
6. Remote mode (Iroh with a project-owned relay) as a separately gated phase.
