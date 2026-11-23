# Request to NSH: let Rust apps on Android declare permissions and reach Android APIs

Draft of a feature request for NSH (Native Shell, `longbridge/nsh`), to file
with its maintainers. DexDrop works around the gap today (below), so it is
not blocking. Checked against NSH v0.6.1 (the latest release) and the docs at
https://nsh.dev/llms-full.txt.

## What the docs say today

- A packaged Android app declares `INTERNET`, plus `POST_NOTIFICATIONS`,
  `RECORD_AUDIO` and `CAMERA` when NSH's own features need them; the
  `android` keys of `nsh.json` are `targetSdk` and `usesCleartextTraffic`
  (Packaging → "Camera, Bluetooth and other hardware", Project structure →
  `nsh.json`).
- For anything else the documented route is: "An Android app that uses other
  hardware embeds NSH in its own Gradle project … That project owns
  `AndroidManifest.xml` … and the app reaches everything else through
  `host.call`."
- But on Android the embedding view, `NshView` from `nsh.aar`, runs **TSX
  bundles and signed packages only** (Platforms → "Embed in a native app"),
  while on iOS, macOS, Windows and Linux `AppView` also mounts a Rust root.
  "An app written in Rust ships on Android as the APK `nsh package --target
  android` builds."

So an app written in Rust has no documented way on Android to declare a
permission or reach platform APIs: it cannot add to the packaged manifest,
and it cannot be embedded in a project that owns one.

## DexDrop's case

Local-network discovery (DNS-SD). Android's Wi-Fi stack drops incoming
multicast unless the app holds a `WifiManager.MulticastLock`, which needs
`android.permission.CHANGE_WIFI_MULTICAST_STATE`. A pure-Rust mDNS responder
(`mdns-sd`, which DexDrop uses on Windows and Linux) therefore cannot rely on
receiving on Android.

## Requests (any one unblocks Rust apps; 1 is the smallest)

1. **`android.permissions` in `nsh.json`**, written as `<uses-permission>`
   next to the ones NSH adds, the counterpart of `ios.infoPlist` /
   `macos.infoPlist`. Validate names (dotted identifiers) and warn on runtime
   (dangerous) permissions the app must still request. Optionally an
   `android.manifest` fragment for `<application>` children (intent filters
   for a share target, `<service>` for a foreground service).
2. **Access to Android from a packaged Rust app**: public accessors in
   `nsh-platform-android` for the process's `JavaVM` and the application
   `Context` (and the activity), so an app's crate can call Android APIs the
   way an iOS or macOS crate calls the system's frameworks. Today they are
   private (`java::init`), so an app must use `JNI_GetCreatedJavaVMs`
   (public only from Android 12, though NSH supports Android 8) and
   `ActivityThread.currentApplication()` by reflection.
3. **A Rust root in Android embedding**: an `NshView` (or a Rust
   `AppView`, like `nsh-platform-ios`) that mounts a Rust root, so Rust apps
   can follow the documented "embed in your own Gradle project" route.

## DexDrop's workaround meanwhile

DexDrop uses the system's DNS-SD service, `NsdManager`, which does the
multicast work in the system and needs no extra permission. A small Java
bridge (callbacks into a queue) is embedded in the Rust library as a dex and
loaded with `InMemoryDexClassLoader`; the Rust side gets the VM from
`JNI_GetCreatedJavaVMs` and the Context by reflection inside the bridge
(`crates/dexdrop-net/src/discovery/nsd.rs`). On Android 8–11 it falls back
to mdns-sd. Request 2 would remove both workarounds and the Android 12
floor.
