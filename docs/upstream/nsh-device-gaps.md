# NSH: device features a file-transfer app still lacks

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
| Connection type and cost (Wi-Fi, cellular, offline; metered or "expensive", Android's data saver, iOS Low Data Mode) | Ask before a large transfer over the internet on mobile data (FR-072); say "No network" at once instead of after a timeout. |
| Battery level, charging, low-power mode | Hold large transfers on low battery unless charging; skip previews in low-power mode. |
| Background execution (iOS `beginBackgroundTask`, Android foreground service or user-initiated data transfer job) | Finish a transfer when the user switches apps (PRD 8.5, 10.2). The Android `<service>` also needs manifest support (`nsh-android-permissions.md`). |
| A progress notification (Android ongoing notification with a bar, iOS Live Activity) | Show a transfer's progress outside the app. |
| Free storage space | Refuse up front a transfer that cannot fit, instead of stopping when the disk fills (the engine handles the full disk today). |
