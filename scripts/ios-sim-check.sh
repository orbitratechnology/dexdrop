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
ad install "$bundle" "$app" || fail "the app did not install"
ad prepare ios-runner --timeout 600000 || fail "agent-device could not prepare its runner"
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

# A real transfer. The app shows a pairing code and an invitation link; the
# CLI on this Mac sends a file with one of them and the app accepts it.
tap "Receive with a code" || fail "could not open Receive with a code"
ad wait text "Pairing code" 10000 >/dev/null 2>&1 || true
step 03-receive-code
# The code as screen readers get it ("Pairing code: 9 S V Y, W F X Y").
code=$(grep -oE 'Pairing code: [0-9A-Z ,]+' "$out/03-receive-code.txt" | head -1 | sed 's/^Pairing code://' | tr -cd 'A-Z0-9')
if [ ${#code} -ge 8 ]; then
  code="${code:0:4}-${code:4:4}"
  note "- Receive sheet shows code $code, readable by screen readers"
else
  code=""
  note "- WARNING: the pairing code is not readable by screen readers"
fi
tap "Copy invitation" || fail "could not copy the invitation"
invitation=$(xcrun simctl pbpaste "$udid")
case "$invitation" in DEXDROP1:*) ;; *) fail "the clipboard holds no invitation" ;; esac
tap Done >/dev/null 2>&1 || true

# Does the Mac see the app on the network? Code pairing depends on it.
state=$(mktemp -d)
name=$(grep -oE 'iPhone [A-Z0-9]{4}' "$out/01-nearby.txt" | head -1)
"$cli" --state "$state" peers --wait 5 >"$out/cli-peers.log" 2>&1 || true
if [ -n "$name" ] && grep -q "$name" "$out/cli-peers.log"; then
  note "- Discovery: the Mac sees $name"
else
  note "- WARNING: discovery: the Mac does not see ${name:-the app} (cli-peers.log)"
fi

payload="$out/payload.bin"
head -c 3000000 /dev/urandom >"$payload"

# send_with <target> <label>: 0 when sent, accepted and finished; 2 when the
# request never reached the app.
send_with() {
  "$cli" --state "$state" send "$1" "$payload" >"$out/cli-send-$2.log" 2>&1 &
  local sender=$! accepted=no
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
  step 04-accepted
  for _ in $(seq 1 60); do
    kill -0 "$sender" 2>/dev/null || break
    sleep 1
  done
  if kill -0 "$sender" 2>/dev/null; then
    kill "$sender"
    step 05-stuck
    fail "the CLI did not finish sending within 60 s"
  fi
  wait "$sender" || {
    step 05-failed
    fail "the CLI reported a failed transfer (cli-send-$2.log)"
  }
}

how=""
if [ -n "$code" ]; then
  if send_with "$code" code; then
    how="pairing code"
  else
    reason=$(grep -v '^ *$' "$out/cli-send-code.log" | tr '\r' '\n' | grep -v '%' | tail -2 | head -1)
    note "- WARNING: sending with the code did not reach the app: ${reason:-no request} (cli-send-code.log)"
  fi
fi
if [ -z "$how" ]; then
  send_with "$invitation" invitation || {
    step 04-no-request
    fail "the transfer request never appeared in the app (cli-send-invitation.log)"
  }
  how="invitation link"
fi
step 05-received
note "- Request shown and accepted in the app (sent with the $how)"

# The file must be byte-identical in the app's container.
container=$(xcrun simctl get_app_container booted "$bundle" data)
received=$(find "$container" -name payload.bin -type f | head -1)
[ -n "$received" ] || fail "payload.bin not found in the app's container"
cmp -s "$payload" "$received" || fail "the received file differs from the one sent"
note "- 3 MB sent from the CLI, received byte-identical in the app (\`${received#"$container"/}\`)"

ad close >/dev/null 2>&1 || true
note ""
note "Passed in $(($(date +%s) - t0)) s."
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
exit 0
