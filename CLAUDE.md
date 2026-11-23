# DexDrop: notes for working on this repository

Cross-platform peer-to-peer file transfer: a Rust engine
(`crates/dexdrop-core`, `crates/dexdrop-net`), an NSH 0.6.1 app (`app/`,
Rust only), a CLI (`crates/dexdrop-cli`). Product requirements:
`docs/STATUS.md` (audit against the PRD), `docs/ARCHITECTURE.md`
(decisions).

## Testing: NSH's recommendations are the primary way, on every platform

Follow `docs/TESTING.md` (from https://nsh.dev/docs/testing-and-trace.md):

- Every UI change gets a **headless** test in `app/tests` (Rust,
  `nsh_testing::Headless`). Platform-specific UI is tested headless by
  setting `EngineConfig::environment` (os, platform) and `viewport`, not by
  waiting for a device.
- Flows worth checking on real platforms go in **`app/e2e/app.test.ts`**:
  one file for Linux, Windows, macOS, the iOS simulator and the Android
  emulator (`nsh test e2e/app.test.ts --target …`).
- Select by **test id** (`id:send`, set with `.test_id("send")`); add new
  ids to the table in `docs/TESTING.md`.
- **No sleeps**: wait for what should appear; use the manual clock
  (`Clock::manual`, `fast_forward`) for timers.
- Guard performance with **mount-item counts** (`tests/perf.rs`), not
  timings.
- agent-device scripts (`scripts/*-check.sh`) only for what crosses apps:
  system pickers, the CLI as the other device, network discovery.
- Before pushing: `cargo fmt --all`,
  `cargo clippy --workspace --all-targets --features dexdrop-net/test-relay`,
  `cargo test --workspace --features dexdrop-net/test-relay` (~25 s).

## Keep the docs current

When tests, selectors, CI jobs or measured timings change, update
`docs/TESTING.md` in the same commit. When a feature or requirement's
state changes, update `docs/STATUS.md`; when a design decision changes,
`docs/ARCHITECTURE.md`.

## Platform notes

- NSH 0.6.1 cannot declare Android manifest entries: CI patches the APK
  (`scripts/patch-android-manifest.sh`). It does not set `ndk-context`
  either: the engine does, before Iroh (`crates/dexdrop-net/src/jvm.rs`).
  Both go when NSH's next release covers them
  (`docs/upstream/nsh-android-permissions.md`).
- Icons are SVG assets (`asset!("icons/…")`, `app/assets/icons`); `cargo run`
  lays them out for the desktop runner (`dev_assets`).
