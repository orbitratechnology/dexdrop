# Testing and the development loop

DexDrop tests the way NSH recommends
([Testing and trace](https://nsh.dev/docs/testing-and-trace.md)): headless
first, on NSH's real engine, in seconds; the same tests through the
platform's own input on every target; device scripts only for what crosses
apps. **This is the primary way to test on every platform.** Keep this page
current whenever tests, selectors, CI jobs or timings change.

## The layers, fastest first

| Layer | Command | Time | Covers |
|---|---|---|---|
| Engine | `cargo test -p dexdrop-core -p dexdrop-net --features dexdrop-net/test-relay` | ~10 s | protocol, transfers, resume, security, the relay, picked copies deleted once nothing selects or sends them (a send group's when its selection is cleared); referenced items (docs/FILE_SELECTION.md) streamed from a provider that is not a file, uneven reads, nothing read to offer, pause and resume (`tests/assets.rs`); the Android bridge's lines and permissions (`src/assets.rs`), an APK sent in place under its name |
| UI headless, media browser | `cargo test -p dexdrop-app --test library` | ~7 s | Send's media browser on Android (ADR-009) against a stand-in provider registered as `android-media`: photos chosen and added as references, nothing opened; a library of 5000 listed a page at a time (under 200 tiles exist); Videos, search, Large files; the permission card before listing (`DEXDROP_MEDIA_ACCESS=none`); `screenshots_of_the_browser` draws six screens into `target/shots/library` (or `DEXDROP_SHOTS`) |
| UI headless, iPhone | `cargo test -p dexdrop-app --test ui` | ~6 s | flows on NSH's engine (`nsh_testing::Headless`, a 390 × 844 iPhone by default) |
| UI headless, Android | `cargo test -p dexdrop-app --test android` | ~2 s | Android layouts and features: `EngineConfig::environment` set to Android at 320 × 640: Send's choices in thumb reach (`send_offers_its_choices_in_thumb_reach`: a row each at the foot of the screen above the tab bar, at least 48 pt, the heading clear of them; once a file is chosen, four side by side above Choose device, and the file's row shows its size once read), a selection of 130 files laid out in the page (120 rows, the rest counted, below Choose device's bar), Send in the icon's blue |
| UI headless, toasts | `cargo test -p dexdrop-app --test toasts` | ~2 s | NSH's toast stack: an alert, timed out by the manager on the manual clock, Escape dismisses the newest before going back |
| UI headless, keyboard and long presses | `cargo test -p dexdrop-app --test keys` | ~3 s | Home's S / R / arrows, Escape back; a past transfer removed and a device forgotten by a long press, through the system's alert (`answer_dialogs_myself`) |
| UI headless, layout promises | `cargo test -p dexdrop-app --test design` | ~4 s | Send and Receive within a thumb's reach above the tab bar, above the tabs a narrow Windows window draws (and Receive opens), Home's header (icon and name), "Nearby" and the radar centered on iPhone, Android and Mac, the radar's still circles with less motion, History's and Settings' titles centered, illustrations in the empty states and the first launch, Android's drawn white tabs (Settings opens), headings and the status pill taking their rows' width, a picked copy deleted when taken out of the selection, where received files go (Mac, iPhone), the segmented filter as a toggle group outside iOS |
| UI headless, Send's Recent | `cargo test -p dexdrop-app --test recent` | ~3 s | a computer's newest files as rows (from `DEXDROP_RECENT_DIRS` headless; a tap adds, another takes out); no Recent and no photo prompt on Android or iPhone (`phones_show_no_recent_and_ask_for_no_photos`, ADR-008) |
| UI headless, camera | `cargo test -p dexdrop-app --test camera` | ~4 s | Scan QR (FR-024): absent without a camera; with `nsh_extra::camera::set_backend` answering a photo of a QR invitation, the code page's input gets the invitation |
| UI headless, several devices | `cargo test -p dexdrop-app --test multi` | ~3 s | a Mac-sized app knowing two devices: files dropped, each device tapped, the list staying, the group's progress; on mobile data (`set_environment`, `Environment::network` expensive) sending over the internet asks first, Cancel sends nothing, Wi-Fi does not ask (FR-072) |
| UI headless, files handed over | `cargo test -p dexdrop-app --test opened` | ~2 s | files shared to DexDrop (Android's share sheet, Open With): `Root::url_schemes(["file"])` as `nsh` sets it from `fileTypes`, then `open_with_url("file://…")` while running and `Headless::start_with_link` at launch; they join Send's selection (FR-035) |
| UI headless, device settings | `cargo test -p dexdrop-app --test device` | ~3 s | the user's 12/24-hour clock, large text: `EngineConfig::environment` (`locale_settings`, `font_scale`) and `set_environment` |
| Performance budgets | `cargo test -p dexdrop-app --test perf` | ~7 s | mount items per step: idle screens send nothing, long lists cost what short ones do, tabs are not rebuilt |
| Every platform, real app | `cd app && nsh test e2e/app.test.ts --target <linux\|macos\|windows\|ios\|android>` | 2–6 s a test once built (CI: Windows ~2 s, macOS 3–6 s, Linux under Xvfb ~27 s); building the development host takes 3–7 min on CI, ~0.5 s when reused | the development host through the platform's input, with the same `id:` selectors |
| Cross-app, CI | `scripts/{android-emu,ios-sim,macos-ui}-check.sh` (agent-device) | 5–30 min | system pickers, the CLI as the other device, discovery over the network, the share sheet; Pause from the app (the chunk in flight lands, up to 4 MiB, then the received size must hold for 4 s) |

The iOS provider (`crates/dexdrop-photos`) type-checks on Linux too,
with a stand-in SDK for the C builds of `ring` and `blake3` (a
`TargetConditionals.h`, `assert.h` and `string.h` of a few lines in
`$SDK/usr/include`):

```sh
SDKROOT=$SDK CC_aarch64_apple_ios=clang \
  CFLAGS_aarch64_apple_ios="-target arm64-apple-ios15 -ffreestanding" \
  cargo clippy -p dexdrop-photos --target aarch64-apple-ios
```

### Pictures of headless screens

NSH's headless engine lays the app out but draws nothing (`screenshot()`
needs `--target`). `app/tests/common/shot.rs` draws what it laid out: a
`Camera` attached right after `Headless::start` records the mounted styles
(background, radius, border, opacity, tint), images and screen configs;
`shoot` writes the screen as SVG and PNG (resvg, 2×, the system's fonts):
each view's frame and style, the app's SVG icons tinted, picture files,
text at its size, weight and color in its laid-out lines, the system's
header bar from the screen's config. Not drawn: shadows, blur, transforms
(a rotated icon shows unrotated), native controls' own drawing; text is
shaped by resvg, not the platform. `docs/screenshots/headless/` holds the
media browser's (`cargo test -p dexdrop-app --test library screenshots`).

Run the headless layers before every push (about 25 s together):

```sh
cargo test --workspace --features dexdrop-net/test-relay
```

## Writing tests

1. **Headless first.** A UI change gets a headless test in `app/tests`
   (Rust, `nsh_testing::Headless`). It runs in seconds on any machine and
   fails with the texts on screen.
2. **Emulate the platform, not a device.** For a layout or feature that
   depends on the platform, set `config.environment.os`,
   `environment.platform` and `viewport` (see `tests/android.rs`). CI's
   emulator once took 30 minutes to report Send's tiles stacked on a small
   phone; `tests/android.rs` reports it in 1.4 s.
3. **One file for every platform.** A flow worth seeing on real platforms
   goes in `app/e2e/app.test.ts` (TypeScript, `nsh/test`); the same file
   runs on Linux, Windows, macOS, the iOS simulator and the Android
   emulator. Each `open` is a fresh engine and storage (`beforeEach`).
4. **Device scripts last.** agent-device scripts only for what NSH's runner
   cannot reach: other apps (document pickers, the share sheet), a second
   device (the CLI), real network discovery.

### Selectors

Prefer, in order: `id:` (a test id) › `role:button[Label]` › `label:` ›
`text:`. Test ids hold across platforms, wording and languages; on iOS they
are the `accessibilityIdentifier`, on Android the view id, so agent-device,
XCUITest and Espresso can use them too. Set them with `.test_id("…")`,
kebab-case:

| Id | Control |
|---|---|
| `send`, `receive` | Home's two actions |
| `pick-photos`, `pick-videos`, `pick-files`, `pick-apps`, `pick-folder` | Send's tiles (`pick-files` on every platform, though computers say "Choose files") |
| `next` | "Choose device" / "Send to …" |
| `accept`, `decline` | the incoming request |
| `pause`, `resume`, `retry`, `done` | a transfer's controls |
| `send-to-all`, `see-progress` | the device list (several devices) |
| `brand`, `radar` | Home's centered header (the app's icon and name) and the radar looking for devices |
| `tab-bar` | the tabs the app draws (Android, narrow windows) |
| `header-title` | a tab's centered title (History, Settings) |
| `art-history`, `art-trusted`, `art-drop`, `art-devices`, `art-done`, `art-failed`, `art-waiting`, `art-apps` | the illustrations |
| `kinds`, `recent` | Send's choices (phones: the dock at the foot of the screen; computers: the row at the top) and its Recent section (computers) |
| `scan-qr` | the code page's "Scan QR" (phones with a camera) |
| `library`, `library-search`, `library-sort`, `select-all`, `library-summary`, `library-add`, `allow-media` | the media browser (ADR-009): the page, its search and order, Select all, the count and size at the foot, Add, the permission card's button |
| `pause-all`, `resume-all` | a group's progress |
| `copy-invitation` | Receive |
| `change-avatar`, `rename`, `save-to`, `save-to-downloads` | Settings |
| `local-network-continue` | the first-launch local-network explanation (macOS, iOS) |

Add an id when a test needs a control; add it to this table.

### Notes for `nsh test`

- **First launch on Apple platforms** shows the local-network explanation
  over Home, once the engine has started. `e2e/app.test.ts` waits for the
  engine (Home's "Visible as …" line, a `text:/…/` regex selector), then
  presses `id:local-network-continue` if it is there, as a person would.
- **No debug info in the test hosts** (`CARGO_PROFILE_DEV_DEBUG=0` on
  every e2e step). NSH builds them with cargo's `dev` profile; with full
  debug info Android's came to 3.7 GB (1.2 GB of `libnsh.so` per ABI) and
  the emulator refused it (INSUFFICIENT_STORAGE, 5 GB free). Without, it
  is 752 MB and installs in 6 s; the Linux host is 311 MB instead of
  1.1 GB. It did not make CI's builds shorter (compiling dominates). Set
  it locally too when the host is only driven, not debugged.
- **Desktop runs declare a window.** Since NSH 0.7.0 a Rust root on the
  desktops shows nothing until it declares `window()`; headless runs as
  macOS or Windows (`tests/design.rs`, `tests/multi.rs`) caught it
  (an empty screen) before any platform run.
- **Environment variables do not reach the app** under `nsh test` unless
  `nsh.json` declares them in `capabilities.process.env`, and declaring any
  capability makes NSH refuse everything undeclared. DexDrop declares none,
  so `DEXDROP_NO_DISCOVERY` works in the headless tests (same process) but
  not in `nsh test`: the e2e suite runs with discovery on.

### Speed

- **Never sleep.** Wait for what should appear (`wait_for`, `waitFor`),
  not for time.
- **Manual clock for timers.** `config.clock = Clock::manual(..)` and
  `app.fast_forward(..)` / `run_for(..)` run the app's timers at once
  (`tests/perf.rs`: an idle test went from ~15 s to ~1 s).
- **Count, don't time.** `app.observe` sees each step's mount batch; the
  counts are deterministic, timings on shared machines are not. Budgets
  live in `tests/perf.rs`. Timings need `--release` and the minimum of
  several runs (`dexdrop bench`; `--to N` sends to N receivers at once).
- **Filter.** `cargo test -p dexdrop-app <name>`, `nsh test … --filter <name>`.
- **Reuse builds.** `nsh test` builds the development host once and reuses
  it (~0.5 s on later runs). `--jobs N` (NSH 0.7.0) runs test files side by
  side; DexDrop has one e2e file, so it gains nothing yet.

## The development loop

- `cargo test …` headless after each change (seconds).
- `cd app && nsh test e2e/app.test.ts --target linux` to see it on a real
  platform; `xvfb-run -a` where there is no display.
- `nsh run --target linux` (or `ios`, `android`) rebuilds and restarts on
  Rust changes (no hot reload for Rust; state starts fresh).
- native-sim (`.github/workflows/native-sim.yml`) streams the iOS
  simulator build to a browser for manual checks.

## CI

| Job | NSH | Then |
|---|---|---|
| Engine tests (Linux, Windows, macOS) | `cargo test` of the engine crates | the transfer benchmark |
| App UI tests (Linux) | `cargo test -p dexdrop-app` (iPhone, Android, performance) | — |
| Package Linux | `nsh test e2e/app.test.ts --target linux` (xvfb) | desktop check |
| Package Windows | `nsh test … --target windows` | desktop check |
| Package macOS and iOS | — (the e2e runs in its own job, below) | desktop check, agent-device macOS and iOS checks (publishing the simulator build for native-sim may fail, as GitHub's API did in run 92, without skipping them; the macOS check answers the runner's own local-network prompt before the Open panel); the iOS check also fails an app whose `Info.plist` lacks `NSPhotoLibraryUsageDescription`, then grants the simulator's photos, adds a photo of its own (`scripts/fixtures/dexdrop-check.jpg`, `simctl addmedia`) and chooses them in the media browser (`08-library` shows what Photos opened; a warning, not a failure, naming whether the browser asked for access, listed nothing or did not open) |
| App e2e (macOS, iOS simulator) | `nsh test … --target macos`, then `--target ios`, on a runner of its own beside packaging (45 min each): `nsh test` builds the app again for NSH's development host, which after an hour of packaging on the same runner did not finish within 25 and 30 min (run 91, NSH 0.7.0); on its own runner it takes 25.7 min on macOS and 22.8 min for the iOS simulator, building included (run 92, no cache yet); its own runner also keeps the e2e's local-network prompt away from the agent-device checks | — |
| Package Android | the packaged APK's manifest read back with `aapt2` (`REQUEST_INSTALL_PACKAGES`, the launcher `<queries>`, the camera apps' `<queries>`, the share sheet's `SEND_MULTIPLE` filter, `FOREGROUND_SERVICE_DATA_SYNC` and `TransferService`, the launcher icon, the Java bridges' classes in the dex, `AssetsBridge` and `PickerActivity` included; the granular `READ_MEDIA_*` and never `MANAGE_EXTERNAL_STORAGE`, ADR-009), then `scripts/android-checks.sh` (the agent-device part also checks the identity key is in the Android Keystore, reading the app's storage with the emulator image's `su` since the APK is not debuggable, `TransferService` runs in the foreground while sending, the installed app requests `READ_MEDIA_IMAGES` and not `MANAGE_EXTERNAL_STORAGE` (`dumpsys package`), and a photo pushed into MediaStore (`scripts/fixtures/dexdrop-check.jpg`, a real JPEG so the system makes its thumbnail) is chosen in the media browser and added with no copy in the app's own storage (`/data/data/<pkg>`, `Android/data/<pkg>`; `14-library`, `15-library-added`), and the app did not crash: a crash fails the check with the exception from logcat's crash buffer, as run 90's `ForegroundServiceDidNotStartInTimeException` did): `nsh test … --target android` on the emulator, then the agent-device check whatever the first did (the step fails if either did; free space and the host APK's size go in `device-storage.txt`; `launcher.png` shows the launcher's app list with the adaptive icon) | agent-device emulator check |

JUnit reports (`--junit`) go in each job's check artifact
(`desktop-check-*`, `e2e-apple`, `android-check`).

Measured in CI run 64: on Linux the e2e step takes 4.7 min (2.8 min
building the development host, 3 tests at ~27 s under Xvfb), on Windows
7 min (6.8 min building, 3 tests at ~2 s), on macOS 4.5 min (4.2 min
building, 3–6 s a test). Run 65, after the agent-device checks: macOS
11 min (10.7 min building, 2.5–10 s a test), iOS simulator 11.7 min
(10.6 min building, 7–10 s a test). On Android `nsh test` builds the development
host for all three ABIs (~10 min), installs it in 6 s and runs the three
tests in 2.6–4.2 s each (run 67). Run 67's e2e steps: Linux 7.5 min,
Windows 8.8 min, macOS 12 min.

Run 69 (b2ae85c) was the first with every job green, the e2e suite on all
five targets included. Its e2e steps: Linux 14 min (a slow runner; 4.7–7.5
min before), Windows 7 min, macOS 11.7 min, iOS simulator 6.6 min,
Android 7.3 min (3.1–3.4 s a test), then the Android agent-device check in
92 s. Build times dominate everywhere; the tests themselves take seconds.
