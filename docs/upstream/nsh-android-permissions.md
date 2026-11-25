# NSH and Android permissions / platform access: already solved upstream

> **Done with NSH 0.7.0 (2026-11-25).** The release ships all three
> (`android.permissions`/`queries`/`libraries`/`application`, `ndk-context`
> set by NSH, `nsh::permissions`), and DexDrop adopted them: the APK patch
> and `app/android/res` are gone, the engine's manifest needs are in
> `crates/dexdrop-net/Cargo.toml`, the bridges ship as an Android library,
> and the engine reads the VM from `ndk-context` (ADR-006 in
> `docs/ARCHITECTURE.md`). DexDrop kept `NsdManager` over mdns-sd with a
> multicast lock (no battery cost, one less permission). The rest of this
> page is the history that led there.

We drafted a request to NSH (`longbridge/nsh`) for Rust apps on Android to
declare manifest permissions and reach Android APIs. Searching its issues
before filing showed the same requests already done on `main` (all closed as
completed, 19–20 Nov 2026), so **no new issue is needed**:

| Issue | What it adds | In |
|---|---|---|
| [#2500](https://github.com/longbridge/nsh/issues/2500) | `android.permissions` in `nsh.json`, written as `<uses-permission>` | nightly (checked: `nsh 0.6.0 f577c51 nightly 2026-11-22` reads `permissions`, `queries`, `libraries`, `manifestPlaceholders` under `android`) |
| [#2686](https://github.com/longbridge/nsh/issues/2686) | NSH sets `ndk-context` (the `JavaVM` and the application `Context`) before app code runs | `main`; DexDrop pins the crates.io crates (`=0.6.1`), so it arrives with the next release |
| [#2531](https://github.com/longbridge/nsh/issues/2531) | `nsh_platform_android::permissions::{state, request}` for runtime permissions from Rust | `main`, next release |

v0.6.1 (what DexDrop uses) is the latest release and has none of them; the
docs at https://nsh.dev/llms-full.txt do not describe them yet.

Not requested: embedding a Rust root in Android's `NshView` (today it runs
TSX bundles and signed packages only). With #2500 and #2686 DexDrop does not
need it, and NSH asks for one focused issue per concrete use case.

## What DexDrop does now, and on the next NSH release

Now (v0.6.1): discovery uses the system's `NsdManager` through a small Java
bridge loaded from memory (`crates/dexdrop-net/src/discovery/nsd.rs`). It
needs no permission. It finds the `JavaVM` with `JNI_GetCreatedJavaVMs`
(public from Android 12; older versions fall back to mdns-sd) and the
`Context` with `ActivityThread.currentApplication()` by reflection — the
same workaround #2686 describes in 14 other crates.

Nightly does not help yet: it builds Rust apps against the crates.io `nsh-*`
crates (v0.6.0 for the 2026-11-22 nightly, older than our 0.6.1), so #2686
is not in them; only the CLI's `android.permissions` is. Checked by
packaging DexDrop with that nightly.

On the first release that ships both #2500 (CLI) and #2686 (crates), the
Android workaround can go entirely:

1. Bump the CLI in CI and the `nsh-*` crates.
2. `nsh.json`: `"android": { "permissions": ["CHANGE_WIFI_MULTICAST_STATE"] }`.
3. Take a `WifiManager.MulticastLock` through JNI with the VM and Context
   from `ndk_context::android_context()` (plain calls, no callbacks, so no
   Java), held while the app is in front: multicast costs battery.
4. Use mdns-sd on Android as on Linux and Windows; delete
   `crates/dexdrop-net/android/` (bridge and dex), `scripts/build-nsd-dex.sh`
   and the `JNI_GetCreatedJavaVMs` / reflection lookups. The Android 12 floor
   goes with them. NSH sets the context itself, so DexDrop must not call
   `initialize_android_context`.

Keeping `NsdManager` (with only the VM/Context lookup replaced) stays an
option if the multicast lock's battery cost matters more than one discovery
code path.

## Also patched in the APK meanwhile

Listing other apps (Send › Apps) needs `<queries>` for launcher activities
and opening received APKs needs `REQUEST_INSTALL_PACKAGES`.
`scripts/patch-android-manifest.sh` adds both to the APK `nsh package`
makes (apktool, then zipalign and apksigner with the same debug key), in
CI after packaging. With `android.queries` and `android.permissions` in a
released CLI, they move to `nsh.json` and the script goes.

## ndk-context for Iroh (Android)

Iroh's DNS resolver (`n0-dns-resolver`) and network watcher (`netdev`) read
the Java VM and application `Context` from `ndk-context`, and panic in a
background task when it is not set — which NSH 0.6.1 does not do. The
engine sets it before starting Iroh (`dexdrop_net::jvm::ensure_ndk_context`,
from the `JNI_GetCreatedJavaVMs` VM and `ActivityThread.currentApplication()`);
without a VM (the CLI run from a shell) internet transfers stay off instead
of panicking. NSH's next release sets the context itself (#2686): remove
the call then, as initializing it twice panics.

## nshm (NSH modules), checked 2026-11-24

https://nsh.dev/versions/main/docs/nshm/ documents, on `main` (not in
v0.6.1: its `nsh` has no `nsh add` and does not read crate metadata), what
the issues above add and more. Mapped to DexDrop:

| DexDrop today (v0.6.1) | With the release that ships nshm | Action then |
|---|---|---|
| `scripts/patch-android-manifest.sh` (apktool) adds `<queries>` for launcher apps and `REQUEST_INSTALL_PACKAGES` | `nsh.json` `android.permissions`, `android.queries` (and `android.application` for `<service>`, `<receiver>`, `<provider>`, `<activity>`, `<meta-data>`) | Move both into `nsh.json`; delete the script and its CI step |
| `jvm.rs` `ensure_ndk_context()` sets `ndk-context` from `ActivityThread` | NSH sets it when the first `NshView` is made; setting it again **panics** | Delete `ensure_ndk_context` and its call in `remote.rs` in the same commit as the upgrade |
| Java bridges (`NsdBridge`, `AppsBridge`) shipped as an embedded `nsd.dex`, loaded with `InMemoryDexClassLoader` | `android.libraries`: `.jar`, `.aar`, `.dex` packaged with the app (no `d8` run on `.dex`) | Ship the bridges as a library; keep the loader only if it stays simpler |
| Android background transfers (PRD 8.5 / 10.2): no `<service>` possible | `android.application` declares a `<service>`; its Java code goes in `android.libraries` | A foreground service of type `dataSync` (plus the `FOREGROUND_SERVICE` permissions) keeping transfers alive with a progress notification — the page does not cover foreground services, so prove it on a device first |
| Share-sheet entry (FR-035) | iOS: prebuilt `.appex` in `ios.extensions`; Android: an `<activity>` with an intent filter in `android.application` (intent filters not stated) | A share extension handing files to the app; needs Swift/Java beyond NSH |
| Runtime permissions | `nsh::permissions::{request, state}` | Use when a permission is added (camera) |

Not solved by nshm: keeping the device awake, connection type, battery,
background work on iOS (`UIBackgroundModes` exists, but no mode keeps a
long transfer going), picking folders on phones, and videos that live only
in the Photos library (`nsh-extra`'s `photos` returns re-encoded JPEG
images, not videos or originals).

Usable now: `nsh-extra` 0.6.0 is on crates.io (`camera`, `photos`, `push`,
`speech` features, against `nsh-core` 0.6). Its `camera::take_photo` returns
a JPEG that DexDrop's QR import (FR-025) could read: a way to scan another
device's invitation (FR-024) without a live viewfinder. With v0.6.1 the
app would declare `NSCameraUsageDescription` itself (`ios.infoPlist`) and
the Android camera `<queries>` entry in the manifest patch.
