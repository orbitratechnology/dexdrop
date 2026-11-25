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
| Engine | `cargo test -p dexdrop-core -p dexdrop-net --features dexdrop-net/test-relay` | ~10 s | protocol, transfers, resume, security, the relay, picked copies deleted once nothing selects or sends them (a send group's when its selection is cleared) |
| UI headless, iPhone | `cargo test -p dexdrop-app --test ui` | ~6 s | flows on NSH's engine (`nsh_testing::Headless`, a 390 × 844 iPhone by default) |
| UI headless, Android | `cargo test -p dexdrop-app --test android` | ~2 s | Android layouts and features: `EngineConfig::environment` set to Android at 320 × 640: Send's four square choices in a row, a selection of 130 files laid out in the page (120 rows, the rest counted, below Choose device's bar), Send in the icon's blue |
| UI headless, toasts | `cargo test -p dexdrop-app --test toasts` | ~2 s | NSH's toast stack: an alert, timed out by the manager on the manual clock, Escape dismisses the newest before going back |
| UI headless, keyboard and long presses | `cargo test -p dexdrop-app --test keys` | ~3 s | Home's S / R / arrows, Escape back; a past transfer removed and a device forgotten by a long press, through the system's alert (`answer_dialogs_myself`) |
| UI headless, layout promises | `cargo test -p dexdrop-app --test design` | ~4 s | Send and Receive within a thumb's reach above the tab bar, above the tabs a narrow Windows window draws (and Receive opens), Home's header (icon and name), "Nearby" and the radar centered on iPhone, Android and Mac, the radar's still circles with less motion, History's and Settings' titles centered, illustrations in the empty states and the first launch, Android's drawn white tabs (Settings opens), headings and the status pill taking their rows' width, a picked copy deleted when taken out of the selection, where received files go (Mac, iPhone), the segmented filter as a toggle group outside iOS |
| UI headless, Send's Recent | `cargo test -p dexdrop-app --test recent` | ~4 s | a computer's newest files as rows (a tap adds, another takes out), Android's newest photos and videos as squares four across (from `DEXDROP_RECENT_DIRS` headless), an iPhone's photo library (the same folder standing in for PhotoKit): a tap exports the original into `staging` and selects it, another takes it out |
| UI headless, camera | `cargo test -p dexdrop-app --test camera` | ~4 s | Scan QR (FR-024): absent without a camera; with `nsh_extra::camera::set_backend` answering a photo of a QR invitation, the code page's input gets the invitation |
| UI headless, several devices | `cargo test -p dexdrop-app --test multi` | ~3 s | a Mac-sized app knowing two devices: files dropped, each device tapped, the list staying, the group's progress; on mobile data (`set_environment`, `Environment::network` expensive) sending over the internet asks first, Cancel sends nothing, Wi-Fi does not ask (FR-072) |
| UI headless, files handed over | `cargo test -p dexdrop-app --test opened` | ~2 s | files shared to DexDrop (Android's share sheet, Open With): `Root::url_schemes(["file"])` as `nsh` sets it from `fileTypes`, then `open_with_url("file://…")` while running and `Headless::start_with_link` at launch; they join Send's selection (FR-035) |
| UI headless, device settings | `cargo test -p dexdrop-app --test device` | ~3 s | the user's 12/24-hour clock, large text: `EngineConfig::environment` (`locale_settings`, `font_scale`) and `set_environment` |
| Performance budgets | `cargo test -p dexdrop-app --test perf` | ~7 s | mount items per step: idle screens send nothing, long lists cost what short ones do, tabs are not rebuilt |
| Every platform, real app | `cd app && nsh test e2e/app.test.ts --target <linux\|macos\|windows\|ios\|android>` | 2–6 s a test once built (CI: Windows ~2 s, macOS 3–6 s, Linux under Xvfb ~27 s); building the development host takes 3–7 min on CI, ~0.5 s when reused | the development host through the platform's input, with the same `id:` selectors |
| Cross-app, CI | `scripts/{android-emu,ios-sim,macos-ui}-check.sh` (agent-device) | 5–30 min | system pickers, the CLI as the other device, discovery over the network, the share sheet; Pause from the app (the chunk in flight lands, up to 4 MiB, then the received size must hold for 4 s) |

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
| `art-history`, `art-trusted`, `art-drop`, `art-devices`, `art-done`, `art-failed`, `art-waiting`, `art-apps`, `art-photos` | the illustrations |
| `kinds`, `recent`, `allow-recent` | Send's row of choices, its Recent section, and the phones' "Show recent photos" |
| `scan-qr` | the code page's "Scan QR" (phones with a camera) |
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
| Package macOS and iOS | `nsh test … --target macos`, `--target ios`, after the checks (the e2e first launch starts discovery in NSH's host; its system prompt must not be the one the checks answer) | desktop check, agent-device macOS and iOS checks (run first); the iOS check also grants Photos (`simctl privacy`), adds a photo (`simctl addmedia`) and picks it in Send's Recent, whose original must land in `staging` |
| Package Android | the packaged APK's manifest read back with `aapt2` (`REQUEST_INSTALL_PACKAGES`, the media permissions, the launcher `<queries>`, the camera apps' `<queries>`, the share sheet's `SEND_MULTIPLE` filter, `FOREGROUND_SERVICE_DATA_SYNC` and `TransferService`, the launcher icon, the Java bridges' classes in the dex), then `scripts/android-checks.sh` (the agent-device part also checks the identity key is in the Android Keystore, `TransferService` runs in the foreground while sending, and "Show recent photos" brings the system's prompt): `nsh test … --target android` on the emulator, then the agent-device check whatever the first did (the step fails if either did; free space and the host APK's size go in `device-storage.txt`; `launcher.png` shows the launcher's app list with the adaptive icon; `13-recent.png`: a photo added to the camera folder shows in Send's Recent) | agent-device emulator check |

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
