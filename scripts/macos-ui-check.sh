#!/usr/bin/env bash
# Drives the packaged macOS app with agent-device on a macOS host, through
# its own UI: walks the sections, finds the dexdrop CLI on the same Mac,
# receives a file sent with the pairing code (accepted in the app), then
# sends a file picked in the system's Open panel to a slow CLI receiver,
# pausing and resuming it from the app.
#
#   scripts/macos-ui-check.sh <DexDrop.app> <dexdrop CLI> <output dir>
#
# agent-device drives the app; AppleScript (System Events) answers what
# belongs to macOS: the local-network prompt and the Open panel's "Go to
# folder" field. Screenshots, accessibility snapshots and a summary go to the
# output dir.
set -uo pipefail

app_src=${1:?DexDrop.app}
cli=${2:?dexdrop CLI}
out=${3:-dist/macos-ui}
mkdir -p "$out"
summary="$out/summary.md"
: >"$summary"
work=$(mktemp -d)
app_name=DexDrop

note() { echo "$*" | tee -a "$summary"; }
cleanup() {
  pkill -f "$cli" 2>/dev/null || true
  osascript -e "tell application \"$app_name\" to quit" >/dev/null 2>&1 || true
  launchctl unsetenv DEXDROP_STATE_DIR
  launchctl unsetenv DEXDROP_DOWNLOAD_DIR
}
fail() {
  note "FAILED: $*"
  cp -R "$HOME/.agent-device/sessions" "$out/agent-device-sessions" 2>/dev/null || true
  log show --last 5m --style compact --predicate "process == \"$app_name\"" >"$out/app-log.txt" 2>&1 || true
  cleanup
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
  exit 1
}
ad() { agent-device "$@" --platform macos; }
step() { # step <name>: screenshot and full accessibility snapshot
  ad screenshot "$out/$1.png" >/dev/null 2>&1 || screencapture -x "$out/$1.png" 2>/dev/null || true
  ad snapshot --force-full >"$out/$1.txt" 2>&1 || true
}
# click1 <label>: a button with that label first (a view around a button can
# carry the same label), then anything with it (tabs, sidebar items).
click1() {
  ad click "role=button label=\"$1\"" --settle >/dev/null 2>&1 ||
    ad click "label=\"$1\"" --settle >/dev/null 2>&1
}
tap() { click1 "$1"; }

# Answers "Allow DexDrop to find devices on local networks?" if it shows.
allow_local_network() {
  local _ shown
  for _ in $(seq 1 "${1:-10}"); do
    shown=$(osascript <<'EOF' 2>/dev/null
tell application "System Events"
  repeat with proc in (every process)
    try
      repeat with w in (every window of proc)
        if exists (button "Allow" of w) then
          click button "Allow" of w
          return name of proc
        end if
      end repeat
    end try
  end repeat
end tell
return ""
EOF
    )
    [ -n "$shown" ] && { note "- Answered \"Allow\" to the local-network prompt"; return 0; }
    sleep 1
  done
  return 0
}

note "## macOS app check (agent-device)"
note ""
t0=$(date +%s)

# Installed where Launch Services finds it, with state and downloads of its
# own (Launch Services passes launchd's environment to the apps it opens).
mkdir -p "$HOME/Applications"
rm -rf "$HOME/Applications/$app_name.app"
cp -R "$app_src" "$HOME/Applications/" || fail "could not install the app"
launchctl setenv DEXDROP_STATE_DIR "$work/app"
launchctl setenv DEXDROP_DOWNLOAD_DIR "$work/received"
note "- App: \`$(basename "$app_src")\` in ~/Applications, state in a fresh folder"

# agent-device's XCTest runner, as for iOS; its daemon can stall while a CI
# host is busy: one retry.
ad prepare ios-runner --timeout 600000 >/dev/null 2>&1 ||
  { sleep 20; ad prepare ios-runner --timeout 600000; } || fail "agent-device could not prepare its runner"
note "- agent-device runner ready at $(($(date +%s) - t0)) s"

ad open "$app_name" || fail "the app did not open"
# The first launch says why the local network is used (FR-004); the
# system's prompt follows Continue.
if ad wait 'label="Continue"' 60000 >/dev/null 2>&1; then
  step 00-intro
  tap Continue || fail "could not tap Continue on the first screen"
  note "- First launch: the local-network explanation, then Continue"
fi
allow_local_network 10
ad wait text "Nearby" 60000 >/dev/null || { step 00-launch-failed; fail "the home screen did not appear"; }
step 01-nearby
note "- Launched: home shown (Send, Receive, Nearby)"

for section in History Settings Share; do
  tap "$section" || { step "02-$section-failed"; fail "could not open $section"; }
done
step 02-sections
note "- Sidebar: History, Settings and back to Share"

# Discovery: does the app list a receiver running on this Mac?
"$cli" --state "$work/cli-r" receive --name "CI Mac CLI" --dir "$work/cli-r-dl" >"$out/cli-receive.log" 2>&1 &
receiver=$!
if ad wait text "CI Mac CLI" 30000 >/dev/null 2>&1; then
  step 03-nearby-sees-cli
  note "- Discovery: the app lists the CLI (\"CI Mac CLI\")"
else
  step 03-nearby-no-cli
  kill "$receiver" 2>/dev/null
  fail "discovery: the app does not list the CLI"
fi
kill "$receiver" 2>/dev/null
wait "$receiver" 2>/dev/null

# 1. Receiving with the pairing code, read from its screen-reader label, and
#    accepting in the app.
tap Receive || fail "could not open the Receive page"
ad wait text "Pairing code" 10000 >/dev/null 2>&1 || { step 04-no-sheet; fail "the Receive page did not open"; }
step 04-receive-code
code=$(grep -oE 'Pairing code: [0-9A-Z ,]+' "$out/04-receive-code.txt" | head -1 | sed 's/^Pairing code://' | tr -cd 'A-Z0-9')
[ ${#code} -ge 8 ] || fail "the pairing code is not readable by screen readers"
code="${code:0:4}-${code:4:4}"
note "- Receive page shows code $code, readable by screen readers"
payload="$work/payload.bin"
head -c 3000000 /dev/urandom >"$payload"
"$cli" --state "$work/cli-s" send "$code" "$payload" >"$out/cli-send-code.log" 2>&1 &
sender=$!
for _ in $(seq 1 30); do
  grep -q 'to accept' "$out/cli-send-code.log" 2>/dev/null && break
  kill -0 "$sender" 2>/dev/null || break
  sleep 1
done
grep -q 'to accept' "$out/cli-send-code.log" || { kill "$sender" 2>/dev/null; fail "the CLI did not reach the app with the code (cli-send-code.log)"; }
# The request shows over the Receive page (leaving it would end the code).
accepted=no
for _ in $(seq 1 20); do
  tap Accept && { accepted=yes; break; }
  sleep 1
done
[ "$accepted" = yes ] || { step 05-no-accept; kill "$sender" 2>/dev/null; fail "could not accept the request in the app"; }
step 05-accepted
for _ in $(seq 1 60); do kill -0 "$sender" 2>/dev/null || break; sleep 1; done
if kill -0 "$sender" 2>/dev/null; then
  kill "$sender" 2>/dev/null
  step 06-stuck
  fail "the CLI did not finish sending within 60 s (cli-send-code.log)"
fi
wait "$sender" || fail "the CLI reported a failed transfer (cli-send-code.log)"
cmp -s "$payload" "$work/received/payload.bin" || fail "payload.bin is missing or differs in the app's download folder"
step 06-received
note "- 3 MB sent with the code, accepted in the app, received byte-identical"

# 2. Sending from the app, with Pause and Resume: a file picked in the Open
#    panel goes to a CLI receiver that accepts on its own and reads at 1 MB/s:
#    24 MB, about 24 s, so Pause is still there to tap (8 MB, two chunks,
#    finished first once the sender read ahead).
outgoing="$work/dexdrop-out.bin"
head -c 24000000 /dev/urandom >"$outgoing"
"$cli" --max-rate 1 --state "$work/inbox-state" receive --yes --name "CI Mac Inbox" --dir "$work/inbox" >"$out/cli-inbox.log" 2>&1 &
inbox=$!
tap Share || true # re-selecting the tab leaves the finished transfer for home
ad wait text "CI Mac Inbox" 30000 >/dev/null 2>&1 || { step 07-no-inbox; fail "discovery: the app does not list the receiving CLI"; }
tap Send || { step 07-no-send-page; fail "could not open Send"; }
tap "Choose files" || fail "could not open the Open panel"
sleep 2
osascript >/dev/null 2>&1 <<EOF || true
tell application "$app_name" to activate
tell application "System Events"
  keystroke "g" using {command down, shift down}
  delay 1
  keystroke "$outgoing"
  delay 1
  key code 36
  delay 1
  key code 36
end tell
EOF
ad wait text "24.0 MB" 20000 >/dev/null 2>&1 || ad wait text "dexdrop-out.bin" 5000 >/dev/null 2>&1 ||
  { step 07-not-selected; fail "the file picked in the Open panel is not selected"; }
step 07-selected
note "- Picked \`dexdrop-out.bin\` (24 MB) in the Open panel"
tap "Choose device" || { step 07-no-next; fail "could not tap Choose device"; }
ad wait 'label="Send to CI Mac Inbox"' 15000 >/dev/null 2>&1 || true
tap "Send to CI Mac Inbox" || { step 07-no-send; fail "could not tap Send"; }
# The list stays (more devices can be tapped); on to the progress.
ad wait 'label="See progress"' 15000 >/dev/null 2>&1 || true
tap "See progress" || { step 07-no-progress; fail "could not tap See progress"; }
ad wait 'label="Pause"' 30000 >/dev/null 2>&1 || { step 08-not-sending; fail "the transfer did not start (no Pause button)"; }
tap Pause || fail "could not tap Pause"
ad wait text "Paused" 15000 >/dev/null 2>&1 || { step 08-not-paused; fail "the transfer did not pause"; }
step 08-paused
held() { cat "$work"/inbox/.dexdrop-out.bin.*dexdrop-part "$work/inbox/dexdrop-out.bin" 2>/dev/null | wc -c | tr -d ' '; }
# The chunk already on its way when Pause went (4 MiB, about 4 s at the
# receiver's 1 MB/s cap) still lands: wait for the size to settle (up to
# 16 s), then check that nothing more comes.
before=$(held)
for _ in $(seq 1 8); do
  sleep 2
  now=$(held)
  [ "$now" = "$before" ] && break
  before=$now
done
sleep 4
after=$(held)
[ "$before" = "$after" ] || fail "the receiver kept getting data while paused ($before -> $after bytes)"
note "- Paused from the app: the receiver held at $after of 24000000 bytes"
tap Resume || fail "could not tap Resume"
for _ in $(seq 1 60); do
  [ -f "$work/inbox/dexdrop-out.bin" ] && break
  sleep 1
done
step 09-resumed
kill "$inbox" 2>/dev/null
wait "$inbox" 2>/dev/null
cmp -s "$outgoing" "$work/inbox/dexdrop-out.bin" || fail "the file did not arrive intact after Resume (cli-inbox.log)"
note "- Resumed from the app: the CLI received \`dexdrop-out.bin\` byte-identical"

ad close >/dev/null 2>&1 || true
cleanup
note ""
note "Passed in $(($(date +%s) - t0)) s."
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
exit 0
