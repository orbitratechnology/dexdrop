#!/usr/bin/env bash
# CI's Android checks on the running emulator: NSH's e2e suite
# (app/e2e/app.test.ts), then the agent-device check. Both run whatever the
# other does; the step fails if either failed.
#
#   scripts/android-checks.sh <DexDrop.apk> <dexdrop CLI for the emulator> <out dir>
set -uo pipefail

apk=$1 cli=$2 out=$3
mkdir -p "$out"
out=$(cd "$out" && pwd) # nsh runs in app/
root=$(cd "$(dirname "$0")/.." && pwd)

# Space on the device, before and after: NSH's test host has been refused
# with INSUFFICIENT_STORAGE.
adb shell df -h /data > "$out/device-storage.txt" 2>&1

# NSH's test host is a debug build: with full debug info its libnsh.so
# came to 1.2 GB an ABI (3.7 GB for the three), more than the emulator's
# 5 GB free could install. Line numbers are not needed to drive the UI.
export CARGO_PROFILE_DEV_DEBUG=0

e2e=0
(cd "$root/app" && nsh test e2e/app.test.ts --target android --junit "$out/e2e-android.xml") \
  2>&1 | tee "$out/e2e-android.log" || e2e=1
if [ "$e2e" != 0 ]; then
  {
    echo "--- after the e2e run"
    adb shell df -h /data
    for host in "$root"/app/.nsh/test/android/*.apk; do
      [ -f "$host" ] || continue
      ls -la "$host"
      unzip -l "$host" | sort -k1 -n | tail -8
    done
  } >> "$out/device-storage.txt" 2>&1
fi
# The check below starts from a device like a user's: without NSH's host
# (its space, its notifications, its process).
for pkg in $(adb shell pm list packages 2>/dev/null | tr -d '\r' | sed -n 's/^package://p' | grep -i nsh); do
  adb uninstall "$pkg" > /dev/null 2>&1 || true
done
adb shell cmd statusbar collapse > /dev/null 2>&1 || true

check=0
bash "$root/scripts/android-emu-check.sh" "$apk" "$cli" "$out" || check=1

echo "e2e: $([ $e2e = 0 ] && echo passed || echo FAILED); agent-device check: $([ $check = 0 ] && echo passed || echo FAILED)"
cat "$out/device-storage.txt"
[ $e2e = 0 ] && [ $check = 0 ]
