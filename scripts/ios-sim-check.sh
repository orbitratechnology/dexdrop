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
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
  exit 1
}
ad() { agent-device "$@" --platform ios; }
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
ad boot --pick newest --timeout 300000 || fail "the simulator did not boot"
ad install "$bundle" "$app" || fail "the app did not install"
ad prepare ios-runner --timeout 600000 || fail "agent-device could not prepare its runner"
note "- Simulator booted, app installed, runner ready in $(($(date +%s) - t0)) s"

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

# A real transfer: the app shows a pairing code, the CLI on this Mac sends
# a file with it, and the app accepts.
tap "Receive with a code" || fail "could not open Receive with a code"
ad wait text "Pairing code" 10000 >/dev/null 2>&1 || true
step 03-receive-code
code=$(grep -oE '[A-Z0-9]{4}-[A-Z0-9]{4}' "$out/03-receive-code.txt" | head -1)
[ -n "$code" ] || fail "no pairing code on screen"
note "- Receive sheet shows code $code"

payload="$out/payload.bin"
head -c 3000000 /dev/urandom >"$payload"
state=$(mktemp -d)
"$cli" --state "$state" send "$code" "$payload" >"$out/cli-send.log" 2>&1 &
sender=$!

tap Done >/dev/null 2>&1 || true
accepted=no
for _ in $(seq 1 30); do
  if tap Accept; then
    accepted=yes
    break
  fi
  tap Transfers >/dev/null 2>&1 || true
  sleep 2
done
step 04-request
[ "$accepted" = yes ] || {
  kill "$sender" 2>/dev/null
  fail "the transfer request never appeared in the app (see cli-send.log)"
}
note "- Request shown and accepted in the app"

for _ in $(seq 1 60); do
  kill -0 "$sender" 2>/dev/null || break
  sleep 1
done
if kill -0 "$sender" 2>/dev/null; then
  kill "$sender"
  step 05-stuck
  fail "the CLI did not finish sending within 60 s"
fi
wait "$sender" || { step 05-failed; fail "the CLI reported a failed transfer (see cli-send.log)"; }
step 05-received

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
