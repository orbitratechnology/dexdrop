# agent-device on Linux: GTK 4 controls missing from snapshots

Draft of a bug report for agent-device (checked with agent-device 0.21.41,
`--platform linux`, X11 under Xvfb, AT-SPI 2.52, GTK 4.14, Ubuntu 24.04).
It keeps DexDrop from driving its Linux app through the UI in CI the way it
does on Android, iOS and macOS. The app itself exposes its controls
correctly (below).

## What happens

`agent-device snapshot --platform linux` on DexDrop (a GTK 4 app built with
Native Shell) lists headings, texts and the tab bar, but no push buttons,
switches or entries: they appear as unnamed `[group] "0.0"` nodes or not at
all, and every node is marked `[disabled]`. `click 'label="Receive with a
code"'` fails with "Selector did not match"; `find text "Receive with a
code" exists` finds nothing. Clicking a tab (`click 'role="tab"
label="Settings"'`) works.

## What AT-SPI actually reports

A direct dump with python3-gi (`Atspi.get_desktop(0)`, walking all
children) shows the controls, named and enabled:

```
'push button' name='Choose files'         states=['sensitive', 'showing', 'visible']
'push button' name='Receive with a code'  states=['sensitive', 'showing', 'visible']
'push button' name='Send with a code'     states=['sensitive', 'showing', 'visible']
```

## Causes in `linux/atspi-dump.py` and `dist/src/snapshot4.js`

1. **Depth limit.** `MAX_DEPTH = 12`, and `snapshot4.js` always passes
   `--max-depth 12` (`maxDepth ?? 12`; `snapshot --depth` does not reach it).
   GTK 4 apps that nest layout containers (Native Shell, many libadwaita
   apps) put their controls deeper than 12 levels, so they are cut off.
   Suggestion: a much higher default (64), or derive it from `--depth`.
2. **Enabled state.** `enabled = has_state(ENABLED)`, but GTK 4 reports
   `SENSITIVE` (and not `ENABLED`) for usable widgets, so everything is
   `[disabled]` and `hittable` may be false. Suggestion: treat
   `SENSITIVE or ENABLED` as enabled.

## Environment notes that may help others

- The AT-SPI bus must be running in the session: under `dbus-run-session`,
  start `/usr/libexec/at-spi-bus-launcher --launch-immediately` before the
  app.
- `python3` must be the interpreter `python3-gi` was built for (on Ubuntu
  24.04, 3.12); another `python3` first on `PATH` fails with "cannot import
  name '_gi' from partially initialized module 'gi'".
- `open <app>` needs `xdg-open` (package `xdg-utils`); input needs
  `xdotool` on X11.

## Reproduce

```sh
sudo apt-get install -y xvfb xdotool xdg-utils python3-gi gir1.2-atspi-2.0 at-spi2-core
dbus-run-session -- bash -c '
  Xvfb :77 -screen 0 1280x800x24 & export DISPLAY=:77
  /usr/libexec/at-spi-bus-launcher --launch-immediately &
  sleep 1; ./DexDrop & sleep 6
  agent-device open DexDrop --platform linux
  agent-device snapshot --platform linux        # no push buttons
  agent-device click "label=\"Choose files\"" --platform linux   # no match
'
```
