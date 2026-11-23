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

On the release that includes #2686:

1. Bump the `nsh-*` crates and the CLI in CI.
2. In `nsd.rs`, take the VM and the Context from `ndk_context::android_context()`
   (pass the Context to the bridge instead of reflection), dropping
   `JNI_GetCreatedJavaVMs` and with it the Android 12 floor. NSH sets the
   context itself, so DexDrop must not call `initialize_android_context`.
3. Keep `NsdManager`: it needs no permission. `android.permissions` (#2500)
   becomes useful for later features (for example `ACCESS_NETWORK_STATE` to
   react to Wi-Fi changes, or a multicast lock if mdns-sd is ever wanted).
