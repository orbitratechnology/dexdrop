# NSH: device features a file-transfer app still lacks

> **NSH 0.7.0 (2026-11-25):** `network()` now reports the link's kind and
> whether it is expensive or constrained; DexDrop asks before sending over
> the internet on such a link (FR-072), so that row is gone. 0.7.0 also
> lets the manifest declare a `<service>` (`android.application`), which a
> foreground service for background transfers needs; the service itself is
> still DexDrop's to write. The rest below is still missing in 0.7.0.

Checked against https://nsh.dev/docs/device-and-environment.md and the
module pages (`nsh`, `nsh/mobile`, `nsh/desktop`) on 2026-11-24, NSH 0.6.1.
DexDrop uses what exists: `lifecycle`, `wake_count` (look again and retry
after sleep), `locale_settings().hour_cycle` (History's times),
`font_scale` (stacked controls with large text), `haptic` (requests,
results), `set_badge` (requests waiting), the window's `progress` (taskbar
and Dock), notifications.

Not available, each with what DexDrop would do with it:

| Missing | What DexDrop would do |
|---|---|
| Keep the screen or device awake (iOS `isIdleTimerDisabled`, Android `FLAG_KEEP_SCREEN_ON`, a desktop power assertion) | Keep the device awake while a transfer is moving. Today a phone that locks mid-transfer suspends the app, and the transfer resumes only when it is opened again. |
| Battery level, charging, low-power mode | Hold large transfers on low battery unless charging; skip previews in low-power mode. |
| Background execution (iOS `beginBackgroundTask`, Android foreground service or user-initiated data transfer job) | Finish a transfer when the user switches apps (PRD 8.5, 10.2). The Android `<service>` also needs manifest support (`nsh-android-permissions.md`). |
| A progress notification (Android ongoing notification with a bar, iOS Live Activity) | Show a transfer's progress outside the app. |
| Free storage space | Refuse up front a transfer that cannot fit, instead of stopping when the disk fills (the engine handles the full disk today). |
