#!/usr/bin/env bash
# Drives the iOS simulator build with agent-device on a macOS host:
# launches the app, walks its main screens, then sends it a real file from
# the dexdrop CLI on the same Mac and accepts it in the app.
#
#   scripts/ios-sim-check.sh <dir with DexDrop.app> <dexdrop CLI> <output dir>
#
# Screenshots, accessibility snapshots and a summary go to the output dir.
set -uo pipefail

dist=${1:-dist/ios-simulator}
cli=${2:-target/release/dexdrop}
out=${3:-dist/ios-screens}
bundle=dev.dexdrop.app
mkdir -p "$out"
summary="$out/summary.md"
: >"$summary"

note() { echo "$*" | tee -a "$summary"; }
fail() {
  note "FAILED: $*"
  # What the system's Bonjour logged, for discovery problems.
  if [ -n "${udid:-}" ]; then
    xcrun simctl spawn "$udid" log show --last 5m --style compact \
      --predicate 'process == "DexDrop" OR process == "mDNSResponder"' \
      >"$out/sim-log.txt" 2>&1 || true
    log show --last 5m --style compact --predicate 'process == "mDNSResponder"' 2>/dev/null |
      grep -i -E 'dexdrop|denied' | tail -200 >"$out/mac-mdns-log.txt" || true
  fi
  # agent-device's own request logs explain a failed command.
  cp -R "$HOME/.agent-device/sessions" "$out/agent-device-sessions" 2>/dev/null || true
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
  exit 1
}
ad() { agent-device "$@" --platform ios --udid "$udid"; }
step() { # step <name>: screenshot and full accessibility snapshot
  ad screenshot "$out/$1.png" >/dev/null 2>&1 || note "(no screenshot for $1)"
  ad snapshot >"$out/$1.txt" 2>&1 || true
}
tap() { # tap <label>: scroll down once if it is not on screen yet
  ad click "label=\"$1\"" --settle >/dev/null 2>&1 && return 0
  ad scroll down >/dev/null 2>&1
  ad click "label=\"$1\"" --settle >/dev/null 2>&1
}

app=$(find "$dist" -maxdepth 2 -name '*.app' -type d | head -1)
[ -n "$app" ] || fail "no .app under $dist"
note "## iOS Simulator check (agent-device)"
note ""
note "- App: \`$(basename "$app")\` ($bundle)"

t0=$(date +%s)
# Boot with simctl (about a minute on a fresh runner): a simulator already
# booted, else an iPhone 16, else any iPhone, on the newest iOS runtime.
udid=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
found = []
for runtime, devices in json.load(sys.stdin)["devices"].items():
    if "iOS" not in runtime:
        continue
    version = tuple(int(p) for p in runtime.rsplit("iOS-", 1)[1].split("-") if p.isdigit())
    for d in devices:
        if "iPhone" in d["name"]:
            found.append((d["state"] == "Booted", d["name"] == "iPhone 16", version, d["udid"]))
print(max(found)[3] if found else "")
')
[ -n "$udid" ] || fail "no iPhone simulator available"
xcrun simctl bootstatus "$udid" -b >/dev/null || fail "the simulator did not boot"
note "- Simulator: $(xcrun simctl list devices -j | python3 -c "import json,sys; print(next(d['name'] for v in json.load(sys.stdin)['devices'].values() for d in v if d['udid']=='$udid'))") ($udid), booted in $(($(date +%s) - t0)) s"
# agent-device's daemon can stall while a CI host is busy: one retry.
ad install "$bundle" "$app" || { sleep 20; ad install "$bundle" "$app"; } ||
  fail "the app did not install"
ad prepare ios-runner --timeout 600000 || { sleep 20; ad prepare ios-runner --timeout 600000; } ||
  fail "agent-device could not prepare its runner"
note "- App installed, agent-device runner ready at $(($(date +%s) - t0)) s"

ad open "$bundle" || fail "the app did not open"
ad alert accept >/dev/null 2>&1 || true
ad wait text "Nearby" 30000 >/dev/null || { step 00-launch-failed; fail "the Nearby screen did not appear"; }
step 01-nearby
note "- Launched: Nearby screen shown"

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

# Discovery the other way: does the app list a receiver running on this Mac?
rstate=$(mktemp -d)
rdir=$(mktemp -d)
"$cli" --state "$rstate" receive --name "CI Mac" --dir "$rdir" >"$out/cli-receive.log" 2>&1 &
receiver=$!
# What the Mac's own Bonjour (mDNSResponder) lists meanwhile.
dns-sd -B _dexdrop._tcp local. >"$out/dns-sd-browse.log" 2>&1 &
browser=$!
if ad wait text "CI Mac" 20000 >/dev/null 2>&1; then
  step 02-nearby-sees-mac
  note "- Discovery: the app sees the Mac (\"CI Mac\")"
else
  step 02-nearby-no-mac
  kill "$browser" "$receiver" 2>/dev/null
  fail "discovery: the app does not see the Mac (\"CI Mac\")"
fi
kill "$browser" "$receiver" 2>/dev/null
wait "$receiver" 2>/dev/null

state=$(mktemp -d)
container=$(xcrun simctl get_app_container "$udid" "$bundle" data)

# open_sheet <step>: opens "Receive with a code". The device list may still
# be changing (a device leaving), which can move the button under a tap.
open_sheet() {
  local _
  # The button is on Nearby; accepting a transfer leaves the app on
  # Transfers.
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

# send_with <target> <label> <file>: sends, accepts in the app and waits for
# the CLI to finish; 2 when the request never reached the app.
send_with() {
  "$cli" --state "$state" send "$1" "$3" >"$out/cli-send-$2.log" 2>&1 &
  local sender=$! accepted=no connected=no _
  # Close the sheet only once the CLI is connected: "Done" also ends the
  # code and the invitation.
  for _ in $(seq 1 30); do
    if grep -q 'to accept' "$out/cli-send-$2.log" 2>/dev/null; then
      connected=yes
      break
    fi
    kill -0 "$sender" 2>/dev/null || break
    sleep 1
  done
  if [ "$connected" != yes ]; then
    kill "$sender" 2>/dev/null
    wait "$sender" 2>/dev/null
    return 2
  fi
  tap Done >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    if tap Accept; then
      accepted=yes
      break
    fi
    kill -0 "$sender" 2>/dev/null || break # the CLI gave up
    tap Transfers >/dev/null 2>&1 || true
    sleep 2
  done
  if [ "$accepted" != yes ]; then
    kill "$sender" 2>/dev/null
    wait "$sender" 2>/dev/null
    return 2
  fi
  step "04-accepted-$2"
  for _ in $(seq 1 60); do
    kill -0 "$sender" 2>/dev/null || break
    sleep 1
  done
  if kill -0 "$sender" 2>/dev/null; then
    kill "$sender"
    step "05-stuck-$2"
    fail "the CLI did not finish sending within 60 s (cli-send-$2.log)"
  fi
  wait "$sender" || {
    step "05-failed-$2"
    fail "the CLI reported a failed transfer (cli-send-$2.log)"
  }
  step "05-received-$2"
}

# received_intact <file>: byte-identical in the app's container.
received_intact() {
  local got
  got=$(find "$container" -name "$(basename "$1")" -type f | head -1)
  [ -n "$got" ] || fail "$(basename "$1") not found in the app's container"
  cmp -s "$1" "$got" || fail "$(basename "$1") differs from the file sent"
  echo "${got#"$container"/}"
}

# 1. The pairing code, read as screen readers get it ("Pairing code: 9 S V
#    Y, W F X Y"), sent from the Mac: needs discovery and the SPAKE2 proof.
open_sheet 03-receive-code
code=$(grep -oE 'Pairing code: [0-9A-Z ,]+' "$out/03-receive-code.txt" | head -1 | sed 's/^Pairing code://' | tr -cd 'A-Z0-9')
[ ${#code} -ge 8 ] || fail "the pairing code is not readable by screen readers"
code="${code:0:4}-${code:4:4}"
note "- Receive sheet shows code $code, readable by screen readers"
name=$(grep -oE 'iPhone [A-Z0-9]{4}' "$out/01-nearby.txt" | head -1)
"$cli" --state "$state" peers --wait 5 >"$out/cli-peers.log" 2>&1 || true
if [ -n "$name" ] && grep -q "$name" "$out/cli-peers.log"; then
  note "- Discovery: the Mac sees $name"
else
  fail "discovery: the Mac does not see ${name:-the app} (cli-peers.log)"
fi
payload="$out/payload.bin"
head -c 3000000 /dev/urandom >"$payload"
send_with "$code" code "$payload" ||
  fail "sending with the pairing code did not reach the app (cli-send-code.log)"
note "- 3 MB sent with the pairing code, accepted, received byte-identical (\`$(received_intact "$payload")\`)"

# 2. The invitation link from the same sheet ("Copy invitation").
open_sheet 06-receive-invitation
tap "Copy invitation" || fail "could not copy the invitation"
invitation=$(xcrun simctl pbpaste "$udid")
case "$invitation" in DEXDROP1:*) ;; *) fail "the clipboard holds no invitation" ;; esac
note2="$out/note.bin"
head -c 200000 /dev/urandom >"$note2"
send_with "$invitation" invitation "$note2" ||
  fail "sending with the invitation link did not reach the app (cli-send-invitation.log)"
note "- 200 KB sent with the invitation link, accepted, received byte-identical (\`$(received_intact "$note2")\`)"

ad close >/dev/null 2>&1 || true
note ""
note "Passed in $(($(date +%s) - t0)) s."
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
exit 0
