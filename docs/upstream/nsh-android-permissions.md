# Request to NSH: Android manifest permissions in `nsh.json`

Draft of a feature request for NSH (Native Shell), to file with its
maintainers. DexDrop works around the gap today (below), so this is not
blocking, but it would remove the workaround and help other apps.

## Summary

Allow `nsh.json` to add Android manifest permissions, the way `ios.infoPlist`
and `macos.infoPlist` already add Info.plist keys:

```json
{
  "android": {
    "targetSdk": 35,
    "permissions": ["android.permission.CHANGE_WIFI_MULTICAST_STATE"]
  }
}
```

## Why

In NSH 0.6.1 the `android` section accepts `targetSdk` and
`usesCleartextTraffic`, and the generated manifest declares only `INTERNET`,
`POST_NOTIFICATIONS`, and camera/microphone permissions derived from
capabilities. An app that needs any other permission cannot be packaged
with `nsh package --target android`.

DexDrop's case: local-network discovery with mDNS. Android's Wi-Fi stack
drops incoming multicast unless the app holds a `WifiManager.MulticastLock`,
which requires `android.permission.CHANGE_WIFI_MULTICAST_STATE` (a normal,
install-time permission). Without it, a pure-Rust mDNS responder (such as
`mdns-sd`, which DexDrop uses on Windows and Linux) cannot reliably receive
on Android.

Other common needs the same key would cover: `ACCESS_NETWORK_STATE`,
`ACCESS_WIFI_STATE`, `FOREGROUND_SERVICE` (+ a type) for long-running work,
`NEARBY_WIFI_DEVICES`, `VIBRATE`, `WAKE_LOCK`.

## Proposal

1. `android.permissions`: a list of permission names written as
   `<uses-permission android:name="…"/>`, merged with the ones NSH adds
   itself and de-duplicated. Optionally accept objects for attributes
   (`{"name": "…", "maxSdkVersion": 32}` or `"usesPermissionFlags"`).
2. Validation: reject names that are not dotted identifiers (avoids XML
   injection), and warn on dangerous (runtime) permissions that the app
   must still request at run time.
3. Optionally, a more general escape hatch for later needs:
   `android.manifest` with XML fragments for `<manifest>` and
   `<application>` (intent filters for a share target, `<service>`
   declarations), like `ios.infoPlist` for Info.plist.

## DexDrop's workaround meanwhile

DexDrop uses the system's DNS-SD service, `NsdManager`, which needs no extra
permission: a small Java bridge (callbacks into a queue) is embedded in the
Rust library as a dex file and loaded with `InMemoryDexClassLoader`, and the
Rust side reaches the VM through `JNI_GetCreatedJavaVMs` (public from
Android 12). See `crates/dexdrop-net/src/discovery/nsd.rs`. It would also
help if NSH exposed the process's `JavaVM` and application `Context` to Rust
apps (for example `nsh_platform_android::java_vm()` and `::context()`), so
apps don't need `JNI_GetCreatedJavaVMs` or `ActivityThread` reflection.
