# DexDrop: notes for working on this repository

Cross-platform peer-to-peer file transfer: a Rust engine
(`crates/dexdrop-core`, `crates/dexdrop-net`), an NSH 0.7.1 app (`app/`,
Rust only), a CLI (`crates/dexdrop-cli`). Product requirements:
`docs/STATUS.md` (audit against the PRD), `docs/ARCHITECTURE.md`
(decisions).

## NSH's skills and guides

NSH's agent skills are in `.agents/skills` (`nsh`, `nsh-design-guides`;
`npx skills add longbridge/nsh` updates them, pinned in
`skills-lock.json`). Read their Coding Guides and Design Guides before
changing code or a screen. The app's design standards and each screen's
motion inventory are in `app/DESIGN.md` (ADR-010). Pages are built from
the `ui` layer (`app/src/ui/`), never from `nsh_base` controls directly.
Colors come only from `theme.json` tokens (`ui::tokens`); radius, type and
motion come from its scales. Overlays use `ui::sheet`, `ui::dialog`,
`ui::confirm`.

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
- Phones browse their media in the app and send items in place
  (`docs/ARCHITECTURE.md` ADR-009, `docs/FILE_SELECTION.md`): the granular
  `READ_MEDIA_*` on Android (never `MANAGE_EXTERNAL_STORAGE`) and PhotoKit
  on iOS; documents through the system's pickers in open mode. Selections
  hold reference paths (`dexdrop_core::asset`): read a selected item through
  `dexdrop_core::source`, never `std::fs`, and never copy it. CI's APK
  check and the simulator check expect the permissions. Google Play needs
  the photo and video permission declaration accepted before a release.
- System features with an NSH fake are tested headless through it
  (`nsh_extra::camera::set_backend` in `tests/camera.rs`).
- On the desktops the root declares its window (`window().id("main")`);
  `main_window()` is the invisible app host, so per-window things (taskbar
  progress) use `current_window()`. `application.quitMode` is
  `lastWindowClosed` (`docs/ARCHITECTURE.md` ADR-006).
- Icons are SVG assets (`asset!("icons/…")`, `app/assets/icons`); `cargo run`
  lays them out for the desktop runner (`dev_assets`).
