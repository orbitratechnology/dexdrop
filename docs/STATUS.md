# Status against the PRD

Legend: **Done** (implemented and tested), **Partial**, **Open**.
"Verified" means exercised in this repository's tests or by running the app
on Linux; other platforms are built in CI but not yet run on devices.

## Platforms (PRD 8.1, Phase 0 exit criteria)

| Target | Build | Run / validate | Notes |
|---|---|---|---|
| Linux x86_64 | Done (`nsh package --target linux`, 15 MB tarball) | Verified: two app instances discover each other and transfer; CLI ⇄ app; 300 MB byte-identical | Needs GTK 4.14+ (Ubuntu 24.04+) |
| Android | Done (APK, arm64-v8a + armeabi-v7a, debug-signed, targetSdk 35) | Open: no device or emulator available | Manifest has `INTERNET`; mDNS needs `CHANGE_WIFI_MULTICAST_STATE` + a multicast lock or an NSD bridge, which NSH's standalone packaging cannot declare yet (P0-015). Invitations and direct addresses work without discovery. |
| Web | Done (static site, 7 MB) | Verified in Chromium | Browser preview only: states that browsers cannot discover or connect directly (FR-084) |
| Windows | CI job | Open | Engine has no Unix-only code paths besides file permissions |
| macOS | CI job (Apple silicon) | Open | Unsigned, local use only |
| iOS | CI job (`ios-simulator`) | Open | `NSLocalNetworkUsageDescription` and `NSBonjourServices` set in `nsh.json`; Bonjour adapter via Network framework is open |

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
| FR-016 Connection quality | Open | |
| FR-020 – FR-023 QR, code, expiry, confirmation | Done | QR shown; single-use tokens/codes, 10 min, 5-attempt lock; SAS on both sides; the receive sheet closes a used code and offers a new one |
| FR-024 Camera QR scanning | Open | Needs a native camera bridge |
| FR-025 QR from a picture | Done | Screenshot or photo, in the app and the CLI; paste from clipboard too |
| FR-030 – FR-034 Selecting and sending | Done (desktop) | Native file/folder dialogs, drag and drop; phones stage picked files |
| FR-035 Share sheet entry | Open | |
| FR-036 Drag and drop | Done | Desktop drop zone |
| Desktop shortcuts (PRD 7.3) | Done | Ctrl/Cmd+O files, Ctrl/Cmd+Shift+O folder, Ctrl/Cmd+1–3 sections, Ctrl/Cmd+, settings, Esc closes sheets |
| System notifications (P4-006) | Done | When a transfer finishes, fails or arrives |
| FR-040 – FR-046 Receiving | Done | Consent, details, destination, keep-both/skip/replace (confirmed), space check, path safety, atomic commit |
| FR-047 Per-device receive policy | Done | |
| FR-050 – FR-056 Lifecycle | Done | Queue states, throttled progress, rate after samples, ETA after 3 s, pause/resume, reconnect with backoff, cancel cleans temp files, verify-and-retry |
| FR-057 Many files | Done for 250k entries | Paged manifest, 250k entry cap; 20,000 files transfer with 40 MB peak memory for both ends (`dexdrop bench --files 20000`) |
| FR-058 Concurrent streams | Done (adapted) | One encrypted stream per transfer with up to 64 pipelined file requests; files that fit in one chunk are read, written, synced and committed in single blocking calls, 8 commits in parallel |
| FR-059 Duplicate detection | Open | |
| FR-060 History | Done | Local, capped, clearable |
| FR-070 – FR-074 Settings | Done | Metered-data switch is stored but not yet enforced |
| FR-075 Diagnostics | Done | No names, paths or keys |
| FR-080 – FR-084 Remote / browser transfer | Open (P1/P2) | Browser preview explains the limitation |

## Tests

- `dexdrop-core`: 32 unit tests (protocol bounds, path traversal and reserved
  names, invitations and codes, journals, naming, manifests, identity).
- `dexdrop-net`: 10 end-to-end tests over real TLS on loopback (consent, decline,
  no silent overwrite, skip policy, pause/resume from the verified offset,
  cancel cleanup, wrong-peer pinning, trusted auto-accept, invalid
  invitations, a used code reported and refused a second time) and 5 adversarial session tests (malicious manifests, oversized
  frames, out-of-place chunks, corruption retried once, persistent corruption
  never saved).
- `dexdrop-app`: 5 headless UI tests on NSH's engine (phone layout), including keyboard shortcuts.

## Known gaps worth doing next

1. Run the CI matrix; fix whatever Windows/macOS/iOS packaging surfaces.
2. Android: a native NSD/multicast-lock bridge and SAF file access (needs an
   NSH packaging extension), then device testing.
3. iOS: Network-framework Bonjour adapter; Files/Photos picker; simulator tests.
4. Store the identity key in the OS keystore.
5. Benchmarks on real Wi-Fi/Ethernet against Quinn and Iroh (P0-012).
6. Remote mode (Iroh with a project-owned relay) as a separately gated phase.
