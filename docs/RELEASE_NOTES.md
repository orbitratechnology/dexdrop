# Release notes

## 0.1.0 — local beta (unreleased)

First build of DexDrop: local, direct, encrypted file transfer between your
devices, with no account and no cloud.

### What works

- Find devices on the same network automatically (desktops), or pair with
  an 8-character code, a QR/link invitation, or a picture of the QR code.
- Mutual TLS 1.3 between devices; both screens show the same 6-digit
  verification number. Every transfer asks the receiver first, unless you
  allowed a device you trust.
- Files and folders, including empty folders; every chunk and file checked
  with BLAKE3; nothing is overwritten unless you choose *Replace*.
- Pause, resume, automatic reconnect after a dropped connection, resume
  after restarting the app, and a clean stop when the receiving disk is
  full, all continuing from the last verified chunk.
- Desktop: native file and folder dialogs, drag and drop, keyboard
  shortcuts, system notifications, *Show in folder*.
- Phones: *Share or save* to export received files.
- Android: share files to DexDrop from any app's share sheet; they wait on
  *Send*, ready for a device (on a Mac: *Open With* or a drop on the Dock
  icon).
- Asks before sending to a device over the internet while on mobile data
  or with Data Saver / Low Data Mode on.
- Measured transfer speed per device; local history; anonymized diagnostics.
- `dexdrop` command-line tool for sending, receiving and benchmarking.

### Tested configurations

| What | Where | Result |
|---|---|---|
| Two app instances discovering each other and transferring (GTK app) | Ubuntu 24.04, x86_64 | Verified by hand, 300 MB byte-identical |
| CLI ⇄ app, code pairing, QR from picture | Ubuntu 24.04 | Verified by hand |
| Engine, protocol and adversarial tests (53+) | Ubuntu 24.04 and Windows Server (GitHub Actions) | Pass |
| Headless UI tests (phone layout) | Ubuntu 24.04 | Pass |
| Drop mid-transfer, disk full mid-transfer, edited source | Ubuntu 24.04 (proxy, RAM disk) | Pass |
| Throughput: 1 GiB file / 20,000 small files / 3 MB photos | Loopback, build container | 340–480 MB/s / 3,700 files/s / 620 MB/s |
| Linux, Windows, Android and web packages | GitHub Actions | Build |

### Known limitations

- **Android:** builds and transfers with codes and invitations; automatic
  discovery and the Storage Access Framework file picker need a native
  component the current Native Shell packaging cannot include. Not yet
  run on a physical device.
- **iOS and macOS:** not built yet (needs a Mac or a macOS CI runner). No
  App Store or TestFlight release; no paid Apple Developer Program
  membership is assumed.
- **Windows:** packaged in CI but not yet run interactively.
- **Web:** a browser preview that explains its limits; it cannot transfer.
- Local networks only: no internet transfer or relay in this release.
- The device key is stored in an app-private file (owner-only on Unix),
  not yet in the system keychain.
- Throughput on real Wi-Fi with loss and latency has not been measured.
