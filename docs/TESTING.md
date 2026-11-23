# Testing and the development loop

Fast checks first; devices only for what needs them.

| Loop | Command | Time | What it covers |
|---|---|---|---|
| Engine | `cargo test -p dexdrop-core -p dexdrop-net --features dexdrop-net/test-relay` | ~10 s | protocol, transfers, resume, security, relay |
| UI, headless (iPhone) | `cargo test -p dexdrop-app --test ui` | ~6 s | flows on NSH's real engine (`nsh_testing::Headless`) |
| UI, headless (Android) | `cargo test -p dexdrop-app --test android` | ~2 s | Android-only layouts and features: `EngineConfig::environment` set to Android at a 320 × 640 phone |
| Performance | `cargo test -p dexdrop-app --test perf` | ~20 s | mount items per step (NSH's trace): idle screens send nothing, long lists cost what short ones do, tabs are not rebuilt |
| Real app, a platform | `cd app && nsh test e2e/tabs.test.ts --target linux` (`macos`, `ios`, `android`) | ~4 s built | the packaged runner through the platform's input, same selectors; `--junit` for CI |
| Devices in CI | `scripts/*-check.sh` (agent-device) | 5–30 min | what crosses apps: system pickers, the CLI as the other device, discovery, the share sheet |
| Watch and run | `nsh run --target linux` (or `ios`, `android`) | rebuild + restart | Rust changes restart the app; there is no hot reload for Rust |

Tips from NSH's testing guide that we use:

- **Emulate the platform headless.** `config.environment.os = Os::Android`,
  `platform = Some("android")` and `viewport` reproduce a phone's layout in
  seconds; CI's emulator once took half an hour to report that the Send
  tiles stacked on a 320 pt screen, which `tests/android.rs` now catches.
- **Manual clock.** `config.clock = Clock::manual(..)` and
  `app.fast_forward(..)` run the app's timers at once: the idle test went
  from ~15 s to ~1 s.
- **Count, don't time.** `app.observe` sees each step's mount batch; counts
  are deterministic where timings are not (`tests/perf.rs`).
- **Failures show the screen.** A failed step lists the texts on screen,
  which usually says what went wrong.
