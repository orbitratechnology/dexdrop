# DexDrop design

The app's design standards, kept with its code as NSH's Design Guides ask
(`.agents/skills/nsh-design-guides`). Pages are built only from the `ui`
layer (`src/ui/`); a look the layer lacks becomes a variant there first.

## Thesis

One task: get files from this device to another one, privately. Every
screen shows what is moving where. The app is quiet black and white on
the platform's own grouped surfaces. Color is kept for the main action
(the app icon's blue), for status, and for the hues that tell what a file
is or where it comes from.

## Targets

| Family | Platforms | Fit |
|---|---|---|
| Mobile | iOS, Android | The system's tab bar on iOS. On Android, a drawn bar above the navigation. Sheets come up from the bottom. Back goes back. Hit targets are 44 pt on iOS and 48 dp on Android (`ui::hit`). The system's switch and segmented control. |
| Desktop | macOS, Windows, Linux | The app's own title bar (`ui::window_bar`), a sidebar when wide, a drawn tab bar when narrow. Compact pointer controls (36 pt), tooltips on icon-only buttons. Hover washes. Keyboard shortcuts (`mod+1..3`, `mod+o`, Escape, Shift+F10 for context menus). The window is declared `main`, 1040 × 720 at first, at least 360 × 560. |
| Web | Browser preview | Shares the desktop's design; the engine is off. |

## Tokens

The colors come from `theme.json`. Code reads them through `theme()` and
`ui::tokens` and never writes a raw color.

- **shadcn roles:** `background`, `foreground`, `card`, `primary`,
  `muted`, `accent`, `border`, `ring`, `destructive`, `sidebar`, …
- **Status:** `success`, `warning` (with `destructive`) through
  `ui::Tone`.
- **Main action:** `primary-from` → `primary-to` (the icon's blue gradient)
  and `primary-hover`.
- **Over color and media:** `on-color` (text on a colored fill),
  `scrim`, `media-backdrop`, `media-chip`, `media-chip-light`, `on-media`,
  `on-media-muted`.
- **What a file is:** `kind-*` (photo, video, music, archive, pdf, doc,
  text, sheet, table, slides, code, web, markdown, font, book, disk,
  vector, database, calendar, contact, android, apple, windows, linux,
  package, folder, file). Each has a light value and a lighter dark value,
  so a mark keeps at least 3:1 on its card.
- **Where it comes from:** `source-photos`, `source-videos`,
  `source-files`, `source-apps`, `source-folder` (Apple's system colors).
- **OS marks:** `os-windows`, `os-android`, `os-linux`.
- **QR code:** `qr-dark` on `qr-light`, the same in both appearances, so
  scanners read it.

Documented exceptions: the avatar gradients (`ui::AVATAR`) are data. A
device chooses one by index and tells its peers, so they stay with the
avatar code. Every stop keeps its white initial above 3:1.

| Scale | Values |
|---|---|
| Spacing (`ui::space`) | 4, 8, 12, 16, 24, 32 |
| Type (`ui::size`) | caption 12, small 14, body 16, heading 17, section 18, title 22, headline 28, display 34, figure 44. Nothing reads below 12. |
| Radius (`ui::radius`, `theme.json` `radius-*`) | xs 4 (marks), sm 8, md 12 (controls, thumbnails), lg 16 (groups, cards), xl 20 (tiles, fields), 2xl 24 (dialogs, sheets, the QR code), full (capsules, circles). Shapes drawn in proportion (an app icon's squircle, a circle `side / 2`) keep their geometry. |
| Motion (`ui::motion`) | instant 120 ms (hover, color), fast 180 ms (press, a control's state), base 240 ms (sliding, swapping, a progress fill), slow 360 ms (a dialog or sheet opening), slower 500 ms. Soft settle `cubic-bezier(0.22, 1, 0.36, 1)` for presses. Distances 4 / 8 / 16 pt. Press scales 0.97 (buttons, rows, tiles) and 0.95 (icon buttons, tabs). Loops: spin 800, pulse 1400, ping 2400. |

Reduced motion: NSH makes transitions instant and skips motions. The app
also drops press scales (`ui::controls::pressed`), jumps the segmented
selection, shows the radar's still circles, and pulses instead of spinning.

## The `ui` layer

| Module | Components |
|---|---|
| `tokens` | the scales above, semantic colors, `Tone`, `Source` |
| `layout` | `hit`, `mobile`, tab-bar heights, `above_tab_bar`, `content_width`, `own_title_bar`, `window_bar` |
| `icons` | `Icon` (one stroke SVG family, `assets/icons`), `icon`, `glyph`, `file_badge`, `kind_glyph`, `soft_glyph`, `os_badge`, `source_badge` |
| `text` | `title`, `body`, `strong`, `muted`, `section` |
| `controls` | `pressable` (ring and eased states), `btn`, `icon_btn`, `btn_then`, `hero_btn`, `round_btn`, `captioned_btn`, `link_btn`, `chip`, `field`, `search_field` + `search_bar`, `progress`, `tick`, `toggle`, `switch_row`, `segmented`, `sliding_selection`, `ring` |
| `surfaces` | `group`, `flat_group`, `tile`, `row`, `nav_row`, `separator`, `pill`, `dot`, `art`, `empty` |
| `people` | `face`, `avatar`, `far_avatar`, `badge_disc`, `route_label` |
| `indicators` | `progress_ring`, `radar`, `spinner`, `skeleton`, `skeleton_rows`, `qr` |
| `dialogs` | `ask`, `confirm` (the system's alerts), `sheet` (`nsh_base::sheet`), `dialog` and `cover` (`nsh_base::dialog`), `context_menu` + `Action` (`nsh_base::context_menu`), `tooltip` (`nsh_base::tooltip`) |

Each control wraps its `nsh_base` counterpart: `button`, `input`,
`progress`, `switch`, `toggle_group` + `toggle`, `sheet`, `dialog`, and
`alert_dialog`. The phones use the native switch and the iOS segmented
control. Controls take variants (`Primary`, `Secondary`, `Plain`,
`Destructive`), sizes and states, never colors or radii.

### States

| State | Look |
|---|---|
| Hover | A wash: the variant's fill one step deeper, or `muted` at 50% on rows, in `instant`. Pictures fade to 92%. |
| Pressed | Scale 0.97 (0.95 for icon buttons and tabs) on the soft settle, and a deeper wash. |
| Focus | A 2 pt `ring` outline on keyboard focus (`focus_visible`). Fields and search bars ring their border. |
| Selected | Filled chips and ticks, tinted tabs, the sliding segment, and `selected` for screen readers. |
| Disabled | 40% opacity, with no hover and no press. |
| Loading | Skeletons in the shape of what comes, a spinner when the shape is unknown, and a progress bar or ring when the extent is known. |

## Motion inventory

Each screen's state changes, the tool and tokens for each, and what
reduced motion does. Each line was checked in the headless engine, where
the styles and motions are mounted. A screenshot on a device is the
remaining check.

### Shell (tabs, sidebar, overlays)

| Change | Motion | Reduced |
|---|---|---|
| Tab or sidebar item: hover and press | Wash `instant`; tab press 0.95 `fast` | Wash only |
| Current tab changes | One selection slides to it (`ui::sliding_selection`, `base`, ease-in-out): Android's pill along the drawn bar, the sidebar's background down the list; tints fade `fast`. The iOS bar is native | Jumps |
| Context menu (Forget, Remove) | Grows from where it opens, from 0.97 and transparent, `slow`; leaves in `base`; a sheet at the bottom on phones | Appears at once |
| Page pushed or popped | The native stack's transition | Platform's |
| Toast arrives and leaves | Slides 16 pt from its edge and fades `fast`; leaves to 0.95. Its clock runs only while toasts show | Instant |
| Tooltip (Look again) | Fades in `fast` on the desktops while the pointer rests, or with the focus | Appears at once |
| Window title bar | None (the desktops' own bar is drawn by the app; it does not move) | — |
| Diagnostics dialog | `ui::dialog`: the modal fades, the card rises (FadeInUp) | Appears at once |
| Local-network cover | `ui::cover`: it fades, the content rises | Appears at once |
| History badge count changes | No motion (a count) | — |

### Home

| Change | Motion | Reduced |
|---|---|---|
| Send / Receive: hover and press | Gradient deepens `instant`; scale 0.97 `fast`, soft settle | Deepens only |
| Status line: visible or hidden | Dot and words change color `fast` | Instant |
| Looking for devices | Radar pings every `ping`, three rings a third apart | Three still circles |
| A device appears | Row enters with its list | Instant |
| Device row: hover and press | Wash `instant`; press wash | Same |

### Send

| Change | Motion | Reduced |
|---|---|---|
| Source tiles: press | Scale 0.97 `fast`; pictures fade to 92% on hover | No scale |
| A file is added or removed | Selection row or tile appears and leaves with its list | Instant |
| Code field focus | Border becomes `ring` in `instant` | Instant |
| Device list: send progress | `ui::progress` fill eases `base` | Instant |
| Sent / waiting | Glyph changes color (`soft_glyph`) | Instant |

### Library (media browser)

| Change | Motion | Reduced |
|---|---|---|
| Category segmented control | One selection slides `base`, ease-in-out | Jumps |
| Album chip: on or off | Fill and text color `fast`; press 0.97 | Color only |
| Picture chosen | Tick fills `fast`; the light wash and ring appear | Instant |
| Thumbnails loading | Skeleton pulses (`pulse`), then the image draws over it | Still skeleton |
| Search field focus | Border becomes `ring` in `instant` | Instant |
| Add bar appears | Rises with the selection count | Instant |

### Apps (Android)

| Change | Motion | Reduced |
|---|---|---|
| Row: hover and press | Wash | Same |
| App ticked | Tick fills `fast` | Instant |
| List loading | Skeleton rows pulse | Still |

### Receive

| Change | Motion | Reduced |
|---|---|---|
| Waiting | Radar pings around the avatar | Still circles |
| Code refreshes | Text changes in place (a value) | — |
| Copy invitation: press | Scale 0.97 | — |

### Incoming request (sheet)

| Change | Motion | Reduced |
|---|---|---|
| Request arrives | `ui::sheet` slides up from the bottom while the scrim fades | Appears at once |
| Options open | Options group appears below | Instant |
| Accept, Decline: press | Scale 0.97 | — |
| Answered or cancelled | Sheet slides down, the scrim fades out | Instant |

### Transfer (progress)

| Change | Motion | Reduced |
|---|---|---|
| Progress | Ring redraws per tick; bars ease `base` | Bars jump |
| Finished or failed | Result pops in (ZoomIn) | Appears |
| Pause / Resume / Stop: hover and press | Wash, scale 0.95 | No scale |
| Pause ↔ Resume | The new control grows in (ZoomIn): a glyph swap | Instant |

### History

| Change | Motion | Reduced |
|---|---|---|
| Row: hover and press | Wash | Same |
| Running row's ring | Redraws with progress | — |
| Entry removed | Leaves with its list | Instant |

### Settings and profile

| Change | Motion | Reduced |
|---|---|---|
| Switch | The native one on phones; on desktops the thumb slides and the track colors in 150 ms; it moves at once, before the engine saves | Instant |
| Name field focus | Border becomes `ring` in `instant` | Instant |
| Avatar color or emoji chosen | Choice shadow and check appear; press 0.95 | No scale |
| Confirmations (reset identity, forget) | The system's alert | Platform's |

## Accessibility and keyboard

Every control is a button, toggle, switch, input or tab. Each has a name,
and live values are part of its label (the status line and the device
chip say whether the device is visible). Tab reaches every control, and
the ring shows where the focus is.

Escape has one owner. The request's sheet declines, a dialog closes, the
toasts dismiss the newest, and only when nothing is over the page does
Escape go back (`shell::escape_goes_back`). Arrow keys move inside the
segmented control.

Actions that a long press or a right click offers (Forget a known device
on Home, Remove a past transfer) are context menus (`ui::context_menu` on
`nsh_base::context_menu`), so Shift+F10 or the Menu key on the focused item
opens them too (VoiceOver's "show menu" on macOS), and the arrows and Return
choose. Everything that removes something asks first: Forget, Remove,
Clear History, Reset identity, Stop a transfer, Replace files.

## Checks

- Headless tests in `app/tests` drive the real UI:
  - `design.rs`: hit targets, drawn tabs, widths.
  - `ui.rs`: the dialog and Escape.
  - `perf.rs`: mount-item budgets.
- Headless pictures, light and dark: `docs/screenshots/headless`.
- Real platforms, light and dark: Home from `app/e2e/app.test.ts` into
  each CI target's `shots-<target>/`.
- Devices: `scripts/*-check.sh` and `app/e2e/app.test.ts`.
