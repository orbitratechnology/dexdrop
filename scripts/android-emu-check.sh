#!/usr/bin/env bash
# Drives the Android app in an emulator with agent-device: launches it,
# walks its main screens, checks discovery through the system's NsdManager
# against the dexdrop CLI running inside the same emulator, then sends the
# app real files from that CLI (with the pairing code when the CLI can see
# the app, and with the invitation link) and accepts them in the app.
#
#   scripts/android-emu-check.sh <app .apk> <dexdrop CLI for the emulator> <output dir>
#
# The emulator's network is behind its own NAT, so the CLI runs inside it
# (pushed with adb, run from a shell: it uses the mdns-sd responder, the
# app the system's service). Needs adb, agent-device and zbarimg (zbar-tools).
# Screenshots, accessibility snapshots, logcat and a summary go to the
# output dir.
set -uo pipefail

apk=${1:?apk}
cli=${2:?cli}
out=${3:-dist/android-check}
pkg=dev.dexdrop.app
dev=/data/local/tmp/dexdrop-check
mkdir -p "$out"
summary="$out/summary.md"
: >"$summary"
warnings=0

note() { echo "$*" | tee -a "$summary"; }
warn() {
  note "- WARNING: $*"
  warnings=$((warnings + 1))
}
collect_logs() {
  adb logcat -d -v time >"$out/logcat.txt" 2>&1 || true
  grep -E 'DexDropNsd|dexdrop|nsh' "$out/logcat.txt" >"$out/logcat-dexdrop.txt" 2>/dev/null || true
  cp -R "$HOME/.agent-device/sessions" "$out/agent-device-sessions" 2>/dev/null || true
}
fail() {
  note "FAILED: $*"
  collect_logs
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
  exit 1
}
serial=$(adb devices | awk 'NR > 1 && $2 == "device" { print $1; exit }')
[ -n "$serial" ] || fail "no emulator attached"
ad() { agent-device "$@" --platform android --serial "$serial"; }
sh_() { adb -s "$serial" shell "$@"; }
step() { # step <name>: screenshot and full accessibility snapshot
  ad screenshot "$out/$1.png" >/dev/null 2>&1 || note "(no screenshot for $1)"
  # --force-full: agent-device otherwise answers "unchanged" with no tree.
  ad snapshot --force-full >"$out/$1.txt" 2>&1 || true
}
tap() { # tap <label>: scroll down once if it is not on screen yet
  ad click "label=\"$1\"" --settle >/dev/null 2>&1 && return 0
  ad scroll down >/dev/null 2>&1
  ad click "label=\"$1\"" --settle >/dev/null 2>&1
}
# tap_only <label>: no scrolling. A scroll with nothing to scroll can act as
# the system's home gesture and send the app to the background (seen in CI).
tap_only() { ad click "label=\"$1\"" --settle >/dev/null 2>&1; }
# The app in front again if something sent it away.
front() { ad open "$pkg" >/dev/null 2>&1 || true; }

note "## Android emulator check (agent-device)"
note ""
note "- Emulator: $(sh_ getprop ro.product.model | tr -d '\r'), Android $(sh_ getprop ro.build.version.release | tr -d '\r') (API $(sh_ getprop ro.build.version.sdk | tr -d '\r')), $(sh_ getprop ro.product.cpu.abi | tr -d '\r')"
t0=$(date +%s)
adb -s "$serial" logcat -c || true

# The CLI inside the emulator.
sh_ "rm -rf $dev && mkdir -p $dev" || fail "could not prepare $dev"
adb -s "$serial" push "$cli" "$dev/dexdrop" >/dev/null || fail "could not push the CLI"
sh_ "chmod 755 $dev/dexdrop"
# `adb shell` joins its arguments with spaces: quote each for the device's
# shell.
dexdrop() { sh_ "$dev/dexdrop $(printf "%q " "$@")"; }

ad install "$pkg" "$apk" || { sleep 15; ad install "$pkg" "$apk"; } || fail "the app did not install"
# Android 13+ asks before an app may post notifications; answer up front.
sh_ pm grant "$pkg" android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true
note "- App installed ($(basename "$apk"))"
ad open "$pkg" --relaunch >/dev/null || fail "the app did not open"
ad alert accept >/dev/null 2>&1 || true
ad wait text "Nearby" 60000 >/dev/null || { step 00-launch-failed; fail "the Nearby screen did not appear"; }
step 01-nearby
note "- Launched: Nearby screen shown ($(($(date +%s) - t0)) s)"

for tab in Transfers Settings; do
  if tap "$tab" && ad wait text "$tab" 10000 >/dev/null; then
    step "02-$(echo "$tab" | tr '[:upper:]' '[:lower:]')"
    note "- $tab tab shown"
  else
    step "02-$tab-failed"
    fail "could not open the $tab tab"
  fi
done
tap Nearby || fail "could not return to Nearby"

# Which discovery the app uses: the bridge logs to logcat.
sleep 3
if adb -s "$serial" logcat -d -s DexDropNsd:I | grep -q 'browsing for'; then
  note "- The app browses with the system's NsdManager (logcat: DexDropNsd)"
else
  fail "the app did not log NsdManager browsing (see logcat.txt)"
fi

# Discovery: does the app list a receiver run by the CLI?
dexdrop --state "$dev/r" receive --name "CI Emulator CLI" --dir "$dev/received" >"$out/cli-receive.log" 2>&1 &
receiver=$!
if ad wait text "CI Emulator CLI" 30000 >/dev/null 2>&1; then
  step 03-nearby-sees-cli
  note "- Discovery: the app sees the CLI (\"CI Emulator CLI\")"
else
  step 03-nearby-no-cli
  sh_ "pkill -f '$dev/dexdrop'" >/dev/null 2>&1 || true
  kill "$receiver" 2>/dev/null
  fail "discovery: the app does not list the CLI (\"CI Emulator CLI\")"
fi
sh_ "pkill -f '$dev/dexdrop'" >/dev/null 2>&1 || true
kill "$receiver" 2>/dev/null
wait "$receiver" 2>/dev/null

# open_sheet <step>: opens "Receive with a code".
open_sheet() {
  local _
  tap Nearby >/dev/null 2>&1 || true
  for _ in 1 2 3; do
    ad wait 'label="Receive with a code"' 5000 >/dev/null 2>&1 || true
    if tap "Receive with a code" && ad wait text "Pairing code" 5000 >/dev/null 2>&1; then
      step "$1"
      return 0
    fi
  done
  step "$1-failed"
  fail "the receive sheet did not open"
}

# send_with <target> <label> <file on the emulator>: sends from the CLI,
# accepts in the app, waits for the CLI to finish; 2 when the request
# never reached the app.
send_with() {
  dexdrop --state "$dev/s" send "$1" "$3" >"$out/cli-send-$2.log" 2>&1 &
  local sender=$! accepted=no connected=no _
  for _ in $(seq 1 30); do
    if grep -q 'to accept' "$out/cli-send-$2.log" 2>/dev/null; then
      connected=yes
      break
    fi
    kill -0 "$sender" 2>/dev/null || break
    sleep 1
  done
  if [ "$connected" != yes ]; then
    sh_ "pkill -f '$dev/dexdrop'" >/dev/null 2>&1 || true
    kill "$sender" 2>/dev/null
    wait "$sender" 2>/dev/null
    return 2
  fi
  tap_only Done || true
  for _ in $(seq 1 30); do
    if tap_only Accept; then
      accepted=yes
      break
    fi
    kill -0 "$sender" 2>/dev/null || break
    front
    tap_only Transfers || true
    sleep 2
  done
  if [ "$accepted" != yes ]; then
    sh_ "pkill -f '$dev/dexdrop'" >/dev/null 2>&1 || true
    kill "$sender" 2>/dev/null
    wait "$sender" 2>/dev/null
    return 2
  fi
  step "05-accepted-$2"
  for _ in $(seq 1 60); do
    kill -0 "$sender" 2>/dev/null || break
    sleep 1
  done
  if kill -0 "$sender" 2>/dev/null; then
    sh_ "pkill -f '$dev/dexdrop'" >/dev/null 2>&1 || true
    kill "$sender"
    step "06-stuck-$2"
    fail "the CLI did not finish sending within 60 s (cli-send-$2.log)"
  fi
  wait "$sender" || {
    step "06-failed-$2"
    fail "the CLI reported a failed transfer (cli-send-$2.log)"
  }
  step "06-received-$2"
}

# received_intact <file on the emulator>: the receiver verified every file
# with BLAKE3 before the CLI could finish; compare bytes too where the app's
# private storage is readable (debuggable builds).
received_intact() {
  local name sum got
  name=$(basename "$1")
  sum=$(sh_ "sha256sum $1" | awk '{ print $1 }')
  got=$(sh_ "run-as $pkg sh -c 'find . -name $name -type f -exec sha256sum {} +'" 2>/dev/null | awk 'NR == 1 { print $1 }')
  if [ -z "$got" ]; then
    echo "verified with BLAKE3 by the app"
  elif [ "$got" = "$sum" ]; then
    echo "byte-identical in the app's storage"
  else
    fail "$name differs in the app's storage"
  fi
}

sh_ "head -c 3000000 /dev/urandom > $dev/payload.bin && head -c 200000 /dev/urandom > $dev/note.bin" ||
  fail "could not create the files to send"

# 1. The pairing code, read as screen readers get it; the CLI must find the
#    app through the app's NsdManager advertisement.
open_sheet 04-receive-code
code=$(grep -oE 'Pairing code: [0-9A-Z ,]+' "$out/04-receive-code.txt" | head -1 | sed 's/^Pairing code://' | tr -cd 'A-Z0-9')
[ ${#code} -ge 8 ] || fail "the pairing code is not readable by screen readers"
code="${code:0:4}-${code:4:4}"
note "- Receive sheet shows code $code, readable by screen readers"
name=$(grep -oE 'Android device [A-Z0-9]{4}' "$out/01-nearby.txt" | head -1)
dexdrop --state "$dev/s" peers --wait 6 >"$out/cli-peers.log" 2>&1 || true
if [ -n "$name" ] && grep -q "$name" "$out/cli-peers.log"; then
  note "- Discovery: the CLI sees $name (advertised by NsdManager)"
  send_with "$code" code "$dev/payload.bin" ||
    fail "sending with the pairing code did not reach the app (cli-send-code.log)"
  note "- 3 MB sent with the pairing code, accepted, $(received_intact "$dev/payload.bin")"
else
  fail "discovery: the CLI does not see ${name:-the app} (cli-peers.log)"
fi

# 2. The invitation link from the same sheet: carries the app's addresses,
#    so it works without discovery.
# Read from the sheet's QR code, as another device's camera would; the
# clipboard as a fallback (agent-device cannot read it on every image).
open_sheet 07-receive-invitation
invitation=$(zbarimg --raw -q "$out/07-receive-invitation.png" 2>/dev/null | grep -oE 'DEXDROP1:[A-Z0-9]+' | head -1)
how="read from the QR code"
if [ -z "$invitation" ]; then
  tap "Copy invitation" || fail "could not copy the invitation"
  invitation=$(ad clipboard read 2>/dev/null | tr -d '\r' | grep -oE 'DEXDROP1:[^[:space:]"]+' | head -1)
  how="copied with \"Copy invitation\""
fi
[ -n "$invitation" ] || fail "no invitation in the QR code or on the clipboard"
note "- Invitation $how"
send_with "$invitation" invitation "$dev/note.bin" ||
  fail "sending with the invitation link did not reach the app (cli-send-invitation.log)"
note "- 200 KB sent with the invitation link, accepted, $(received_intact "$dev/note.bin")"

# 3. Sending from the app, with Pause and Resume (only a sender can pause):
#    a file picked in Android's file picker goes to a CLI receiver that
#    accepts on its own and reads at 1 MB/s, so there is time to pause.
out_file=dexdrop-out.bin
sh_ "head -c 8000000 /dev/urandom > /sdcard/Download/$out_file" || fail "could not create /sdcard/Download/$out_file"
sh_ "content call --method scan_volume --uri content://media --arg external_primary" >/dev/null 2>&1 || true
dexdrop --max-rate 1 --state "$dev/inbox-state" receive --yes --name "CI Emulator Inbox" --dir "$dev/inbox" >"$out/cli-inbox.log" 2>&1 &
inbox=$!
front
tap_only Nearby || true
ad wait text "CI Emulator Inbox" 30000 >/dev/null 2>&1 || { step 08-no-inbox; fail "discovery: the app does not list the receiving CLI"; }
tap "Choose files" || fail "could not open the file picker"
picked=no
for _ in 1 2 3 4 5 6; do
  if tap_only "$out_file"; then
    picked=yes
    break
  fi
  # Not in the picker's first view: open Downloads from its roots.
  tap_only "Show roots" || true
  tap_only "Downloads" || true
  sleep 2
done
[ "$picked" = yes ] || { step 08-picker; fail "could not pick $out_file in the file picker"; }
tap_only "Select" || true # in the picker's selection mode
ad wait text "8.00 MB" 20000 >/dev/null 2>&1 || ad wait text "$out_file" 20000 >/dev/null 2>&1 ||
  { step 08-not-selected; fail "the picked file is not selected in the app"; }
step 08-selected
note "- Picked \`$out_file\` (8 MB) in Android's file picker"
tap_only Send || tap Send || { step 08-no-send; fail "could not tap Send for the CLI receiver"; }
ad wait 'label="Pause"' 30000 >/dev/null 2>&1 || { step 09-not-sending; fail "the transfer did not start (no Pause button)"; }
tap_only Pause || fail "could not tap Pause"
ad wait text "Paused" 15000 >/dev/null 2>&1 || { step 09-not-paused; fail "the transfer did not pause"; }
step 09-paused
size_at() { sh_ "cat $dev/inbox/.$out_file.*dexdrop-part $dev/inbox/$out_file 2>/dev/null | wc -c" | tr -d '\r '; }
before=$(size_at)
sleep 4
after=$(size_at)
[ "$before" = "$after" ] || fail "the receiver kept getting data while paused ($before -> $after bytes)"
note "- Paused from the app: the receiver held at $after of 8000000 bytes"
tap_only Resume || fail "could not tap Resume"
src_sum=$(sh_ "sha256sum /sdcard/Download/$out_file" | awk '{ print $1 }')
got_sum=
for _ in $(seq 1 60); do
  got_sum=$(sh_ "sha256sum $dev/inbox/$out_file 2>/dev/null" | awk '{ print $1 }')
  [ -n "$got_sum" ] && break
  sleep 1
done
step 10-resumed
sh_ "pkill -f '$dev/dexdrop'" >/dev/null 2>&1 || true
kill "$inbox" 2>/dev/null
wait "$inbox" 2>/dev/null
[ -n "$got_sum" ] || fail "the file did not arrive after Resume (cli-inbox.log)"
[ "$got_sum" = "$src_sum" ] || fail "the file the CLI received differs from the one picked"
note "- Resumed from the app: the CLI received \`$out_file\` byte-identical (SHA-256)"

collect_logs
ad close >/dev/null 2>&1 || true
note ""
if [ "$warnings" -gt 0 ]; then
  note "Passed with $warnings warning(s) in $(($(date +%s) - t0)) s."
else
  note "Passed in $(($(date +%s) - t0)) s."
fi
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
exit 0
