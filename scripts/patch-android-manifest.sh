#!/usr/bin/env bash
# Adds to the manifest of the APK NSH packages what v0.6.1's nsh.json cannot
# declare, then signs it again with the same debug key:
#
# - <queries> for launcher activities: on Android 11+ an app sees other
#   apps only if it says which; DexDrop lists them to send their APKs.
# - REQUEST_INSTALL_PACKAGES: opening a received APK hands it to the
#   system's installer, which asks the user before installing.
#
# NSH's next release declares both in nsh.json (android.queries,
# android.permissions; see docs/upstream/nsh-android-permissions.md); this
# script goes then.
#
#   scripts/patch-android-manifest.sh dist/android/DexDrop.apk
set -euo pipefail
apk=$(realpath "${1:?apk}")
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/android-sdk}}
tools=$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)
keystore=${DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}
cache=${XDG_CACHE_HOME:-$HOME/.cache}/dexdrop-tools
version=2.10.0
sum=c0350abbab5314248dfe2ee0c907def4edd14f6faef1f5d372d3d4abd28f0431
jar="$cache/apktool_$version.jar"

mkdir -p "$cache"
if ! echo "$sum  $jar" | sha256sum -c --status 2>/dev/null; then
  curl -fsSL -o "$jar" "https://github.com/iBotPeaches/Apktool/releases/download/v$version/apktool_$version.jar"
  echo "$sum  $jar" | sha256sum -c --status || { echo "apktool checksum mismatch" >&2; exit 1; }
fi
[ -f "$keystore" ] || { echo "no debug keystore at $keystore" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# -s: classes.dex stays as it is; only resources and the manifest are read.
java -jar "$jar" d -s -f -o "$work/src" "$apk" >/dev/null
python3 - "$work/src/AndroidManifest.xml" <<'EOF'
import sys
path = sys.argv[1]
s = open(path, encoding="utf-8").read()
queries = '<intent><action android:name="android.intent.action.MAIN"/></intent>'
permission = '<uses-permission android:name="android.permission.REQUEST_INSTALL_PACKAGES"/>'
if 'android.intent.action.MAIN"/></intent></queries>' not in s and queries not in s:
    if "<queries>" in s:
        s = s.replace("<queries>", "<queries>" + queries, 1)
    else:
        s = s.replace("<application", "<queries>" + queries + "</queries><application", 1)
if permission not in s:
    s = s.replace("<application", permission + "<application", 1)
open(path, "w", encoding="utf-8").write(s)
EOF
java -jar "$jar" b -o "$work/unsigned.apk" "$work/src" >/dev/null
"$tools/zipalign" -p -f 4 "$work/unsigned.apk" "$work/aligned.apk"
"$tools/apksigner" sign --ks "$keystore" --ks-pass pass:android \
  --ks-key-alias androiddebugkey --key-pass pass:android \
  --out "$apk" "$work/aligned.apk"
"$tools/apksigner" verify "$apk"
"$tools/aapt2" dump xmltree --file AndroidManifest.xml "$apk" | grep -q REQUEST_INSTALL_PACKAGES
echo "patched $(basename "$apk"): queries for launcher apps, REQUEST_INSTALL_PACKAGES"
