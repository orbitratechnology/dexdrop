# DexDrop: notes for working on this repository

Cross-platform peer-to-peer file transfer: a Rust engine
(`crates/dexdrop-core`, `crates/dexdrop-net`), an NSH 0.7.0 app (`app/`,
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

- Android manifest entries live with the code that needs them: the
  engine's (Java bridges `android/nsd.dex`, `<queries>`, the foreground
  service)
  in `crates/dexdrop-net/Cargo.toml` `[package.metadata.nsh.android]`, the
  app's in `app/nsh.json` (`android.permissions`, `icon.android`,
  `android.launchScreen`). Nothing patches the APK.
- NSH sets `ndk-context` before app code runs and panics if it is set
  again: never call `initialize_android_context`. The app calls
  `dexdrop_net::use_android_context()`; the engine (`jvm.rs`, the Java
  bridges, `android_service.rs`) and the identity keystore (Android
  Keystore) read it from then on. After changing the Java in
  `crates/dexdrop-net/android/`, run `scripts/build-nsd-dex.sh` and commit
  `nsd.dex`.
- No broad photo permission on phones (`docs/ARCHITECTURE.md` ADR-008):
  no `READ_MEDIA_*`/storage permissions on Android, no photo library
  access on iOS; phones pick photos, videos and files with the system's
  pickers. CI's APK check and the simulator check fail otherwise.
- System features with an NSH fake are tested headless through it
  (`nsh_extra::camera::set_backend` in `tests/camera.rs`).
- On the desktops the root declares its window (`window().id("main")`);
  `main_window()` is the invisible app host, so per-window things (taskbar
  progress) use `current_window()`. `application.quitMode` is
  `lastWindowClosed` (`docs/ARCHITECTURE.md` ADR-006).
- Icons are SVG assets (`asset!("icons/…")`, `app/assets/icons`); `cargo run`
  lays them out for the desktop runner (`dev_assets`).
