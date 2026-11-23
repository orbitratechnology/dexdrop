# NSH and Android permissions / platform access: already solved upstream

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
