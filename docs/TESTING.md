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
| Engine | `cargo test -p dexdrop-core -p dexdrop-net --features dexdrop-net/test-relay` | ~10 s | protocol, transfers, resume, security, the relay |
| UI headless, iPhone | `cargo test -p dexdrop-app --test ui` | ~6 s | flows on NSH's engine (`nsh_testing::Headless`, a 390 × 844 iPhone by default) |
| UI headless, Android | `cargo test -p dexdrop-app --test android` | ~2 s | Android layouts and features: `EngineConfig::environment` set to Android at 320 × 640 |
| UI headless, device settings | `cargo test -p dexdrop-app --test device` | ~3 s | the user's 12/24-hour clock, large text: `EngineConfig::environment` (`locale_settings`, `font_scale`) and `set_environment` |
| Performance budgets | `cargo test -p dexdrop-app --test perf` | ~7 s | mount items per step: idle screens send nothing, long lists cost what short ones do, tabs are not rebuilt |
| Every platform, real app | `cd app && nsh test e2e/app.test.ts --target <linux\|macos\|windows\|ios\|android>` | 2–6 s a test once built (CI: Windows ~2 s, macOS 3–6 s, Linux under Xvfb ~27 s); building the development host takes 3–7 min on CI, ~0.5 s when reused | the development host through the platform's input, with the same `id:` selectors |
| Cross-app, CI | `scripts/{android-emu,ios-sim,macos-ui}-check.sh` (agent-device) | 5–30 min | system pickers, the CLI as the other device, discovery over the network, the share sheet |

Run the first four before every push (about 25 s together):

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
| `copy-invitation` | Receive |
| `change-avatar`, `rename` | Settings |
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
  several runs (`dexdrop bench`).
- **Filter.** `cargo test -p dexdrop-app <name>`, `nsh test … --filter <name>`.
- **Reuse builds.** `nsh test` builds the development host once and reuses
  it (~0.5 s on later runs).

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
| Package macOS and iOS | `nsh test … --target macos`, `--target ios`, after the checks (the e2e first launch starts discovery in NSH's host; its system prompt must not be the one the checks answer) | desktop check, agent-device macOS and iOS checks (run first) |
| Package Android | `scripts/android-checks.sh`: `nsh test … --target android` on the emulator, then the agent-device check whatever the first did (the step fails if either did; free space and the host APK's size go in `device-storage.txt`) | agent-device emulator check |

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
