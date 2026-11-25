# Packaging DexDrop

How DexDrop is packaged for each platform, following NSH's packaging guide
(https://nsh.dev/docs/packaging.md) for NSH 0.7.0. CI packages every target
on every push (`.github/workflows/ci.yml`); the commands below are the ones
it runs.

## Metadata (`app/nsh.json`)

| Key | Value | Notes |
|---|---|---|
| `name` | `DexDrop` | Display name, output file names |
| `id` | `dev.dexdrop.app` | Bundle id and Android package; also where the app keeps its data, so it never changes |
| `version` | `0.1.0` | `CFBundleShortVersionString`, Android `versionName`; raise it for each release |
| `build` | `1`, CI passes `--build <run number>` | `CFBundleVersion`, Android `versionCode`: grows with every build, so a newer APK installs over an older one |
| `icon` | one file per platform (below) | |
| `ios.launchScreen`, `android.launchScreen` | off-white `#fbfaf8` / near-black `#070707` (the theme's backgrounds), the icon at 96 pt | On Android the window's background from the tap on, and the system splash screen (Android 12+) |
| `application.quitMode` | `lastWindowClosed` | Closing the window quits on every computer, macOS included (ADR-006) |
| `fileTypes` | one type, any file (`mimeTypes: ["*/*"]`, `contentTypes: ["public.data"]`) | Android's share sheet, macOS's Open With and Dock icon, iOS's "Open in": the files go to Send (FR-035) |
| `macos.infoPlist.LSApplicationCategoryType` | `public.app-category.utilities` | The App Store's and Finder's category |
| `android.targetSdk` | `35` | |
| `android.permissions` | `REQUEST_INSTALL_PACKAGES` | Opening a received APK hands it to the system's installer |
| `ios.infoPlist.NSCameraUsageDescription` | why DexDrop uses the camera | Scan QR (FR-024); `nsh-extra`'s `camera` feature would add a generic text |

The engine's own Android needs are in `crates/dexdrop-net/Cargo.toml`
(`[package.metadata.nsh.android]`), which `nsh` adds to every package of
an app that links it: the Java bridges (`libraries`: `android/nsd.dex`),
the `<queries>` entry for launcher apps (Send › Apps), and the foreground
service that
keeps transfers going (`application`: `TransferService`, type `dataSync`,
with `FOREGROUND_SERVICE` and `FOREGROUND_SERVICE_DATA_SYNC`). Likewise
`nsh-extra`'s `camera` adds the camera apps' `<queries>`.

No permission to read photos, videos or storage, on either phone
(ADR-008): Google Play restricts `READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO` to
apps whose core use needs them, and the system's pickers need none; CI
fails an APK that asks for them, and the simulator check an app that
declares `NSPhotoLibraryUsageDescription`. The Play Console still asks
what the `dataSync` foreground service does (a user-started file
transfer) and why the app installs packages (`REQUEST_INSTALL_PACKAGES`:
received APKs, a file-transfer app's core use). Check a package's with
`aapt2 dump xmltree --file AndroidManifest.xml DexDrop.apk`; CI does.

## Icons

The artwork is a blue rounded square with two white arrows, given in two
versions: on a transparent background and on white. NSH takes one square
PNG per platform, ideally 1024 × 1024, and makes every size from it.

| File | Platforms | What it is |
|---|---|---|
| `app/icon.png` | Windows, Linux, web | The transparent artwork filling the canvas with a 4 % margin. Windows gets `.ico` sizes 16–256 embedded in `DexDrop.exe` (only when packaged on Windows, as CI does); Linux `dev.dexdrop.app.png` named by the `.desktop` file; web the favicon, Apple touch icon and manifest icons |
| `app/icon-macos.png` | macOS | The artwork at 824 px on a transparent 1024 canvas: Apple's icon grid, which NSH does not apply itself |
| `app/icon-ios.png` | iOS | The white-background artwork, opaque (the App Store refuses icons with transparency); iOS rounds the corners |
| `app/launch.png` | iOS launch screen | 288 px, drawn at 96 pt on 3x screens |
| `app/icon-android-foreground.png`, `app/icon-android-monochrome.png` | Android 8+ | `icon.android`'s adaptive icon: the artwork in the 66 dp safe zone of a 432 px layer on a white `background`, and a monochrome layer (the rounded square with the arrows cut out) for Android 13's themed icons |

To change the icon, replace the two source images and regenerate the files
above at the same sizes (the transparent one trimmed to its rounded square
first).

## Commands

| Target | Command (CI) | Output |
|---|---|---|
| Linux | `nsh package app --target linux --build N` | `DexDrop/` and `DexDrop.tar.gz`; needs GTK 4.14+, Pango, WebKitGTK 6.0 on the target |
| Windows | `nsh package app --target windows --build N` | `DexDrop\`, `.zip`, `.iss` and the per-user `DexDrop-setup.exe` (Inno Setup 6, on GitHub's Windows runners) |
| macOS | `nsh package app --target macos --build N` | `DexDrop.app`, ad hoc signed |
| iOS simulator | `nsh package app --target ios-simulator --build N` | `DexDrop.app` |
| Android | `nsh package app --target android --abi arm64-v8a,armeabi-v7a,x86_64 --build N [--keystore … --key-alias …]` | `DexDrop.apk` |
| Web | `nsh package app --target web --build N` | `DexDrop/` |

## Signing

**Android.** Without a release keystore, NSH signs with a debug key that
CI creates afresh on every runner: each APK then has a different
signature, and Android refuses to install one over another (uninstalling
first loses the app's data). With the project's release keystore, every
build installs over the last. To set it up, once:

```sh
keytool -genkeypair -v -keystore dexdrop-release.jks -alias upload \
  -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 dexdrop-release.jks   # the value of ANDROID_KEYSTORE_BASE64
```

then add three repository secrets (Settings › Secrets and variables ›
Actions): `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEY_ALIAS` (`upload`) and
`NSH_KEYSTORE_PASSWORD`. Keep the `.jks` file and its password out of the
repository and somewhere safe: an app's updates must be signed with the
same key for its whole life. CI decodes it to the runner's temporary
folder, passes it to `nsh package --keystore`, and deletes it.

For Google Play: `--format aab` (the upload key registered in Play App
Signing) and `nsh publish android`.

**iOS devices** need an Apple Distribution identity and an App Store
provisioning profile for `dev.dexdrop.app`: `nsh package app --target ios
--sign "Apple Distribution: …" --profile AppStore.mobileprovision`, then
`nsh publish ios` (App Store Connect API key, kept out of the repository).
Not set up yet.

**macOS** builds are ad hoc signed: fine on the Mac that built them;
distribution needs a Developer ID signature and notarization. Not set up
yet.

## Before a release

1. Raise `version` in `app/nsh.json` (CI raises `build`).
2. Android: the release keystore secrets are set (`apksigner verify
   --print-certs DexDrop.apk` names the key: `upload`, not
   `androiddebugkey`).
3. iOS and macOS: the signing above, once there is an Apple developer
   account.
4. Update `docs/RELEASE_NOTES.md`.
