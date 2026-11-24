#!/usr/bin/env bash
# Adds to the manifest of the APK NSH packages what v0.6.1's nsh.json cannot
# declare, then signs it again with the same debug key:
#
# - <queries> for launcher activities: on Android 11+ an app sees other
#   apps only if it says which; DexDrop lists them to send their APKs.
# - REQUEST_INSTALL_PACKAGES: opening a received APK hands it to the
#   system's installer, which asks the user before installing.
# - READ_MEDIA_IMAGES / READ_MEDIA_VIDEO (Android 13+), with
#   READ_MEDIA_VISUAL_USER_SELECTED (14+: "allow limited access"), and
#   READ_EXTERNAL_STORAGE before 13: Send's recent photos and videos,
#   asked for when the user taps "Show recent photos and videos".
# - An adaptive launcher icon (Android 8+: the launcher's shape, and
#   Android 13's themed icons) from app/android/res: NSH packages one flat
#   192 px icon, which launchers shrink onto a white disc.
#
# Signed with the release keystore when ANDROID_KEYSTORE is set (alias in
# ANDROID_KEY_ALIAS, password in NSH_KEYSTORE_PASSWORD, as `nsh package
# --keystore` takes them), else with NSH's debug key.
#
# NSH's next release declares the first two in nsh.json (android.queries,
# android.permissions; see docs/upstream/nsh-android-permissions.md); the
# icon stays here until NSH packages adaptive icons (docs/PACKAGING.md).
#
#   scripts/patch-android-manifest.sh dist/android/DexDrop.apk
set -euo pipefail
apk=$(realpath "${1:?apk}")
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/android-sdk}}
tools=$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)
keystore=${ANDROID_KEYSTORE:-${DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}}
if [ -n "${ANDROID_KEYSTORE:-}" ]; then
  alias=${ANDROID_KEY_ALIAS:?ANDROID_KEY_ALIAS}
  pass=${NSH_KEYSTORE_PASSWORD:?NSH_KEYSTORE_PASSWORD}
else
  alias=androiddebugkey
  pass=android
fi
res=$(dirname "$(realpath "$0")")/../app/android/res
cache=${XDG_CACHE_HOME:-$HOME/.cache}/dexdrop-tools
version=2.10.0
sum=c0350abbab5314248dfe2ee0c907def4edd14f6faef1f5d372d3d4abd28f0431
jar="$cache/apktool_$version.jar"

mkdir -p "$cache"
if ! echo "$sum  $jar" | sha256sum -c --status 2>/dev/null; then
  curl -fsSL -o "$jar" "https://github.com/iBotPeaches/Apktool/releases/download/v$version/apktool_$version.jar"
  echo "$sum  $jar" | sha256sum -c --status || { echo "apktool checksum mismatch" >&2; exit 1; }
fi
[ -f "$keystore" ] || { echo "no keystore at $keystore" >&2; exit 1; }

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
media = [
    '<uses-permission android:name="android.permission.READ_MEDIA_IMAGES"/>',
    '<uses-permission android:name="android.permission.READ_MEDIA_VIDEO"/>',
    '<uses-permission android:name="android.permission.READ_MEDIA_VISUAL_USER_SELECTED"/>',
    '<uses-permission android:name="android.permission.READ_EXTERNAL_STORAGE" android:maxSdkVersion="32"/>',
]
if 'android.intent.action.MAIN"/></intent></queries>' not in s and queries not in s:
    if "<queries>" in s:
        s = s.replace("<queries>", "<queries>" + queries, 1)
    else:
        s = s.replace("<application", "<queries>" + queries + "</queries><application", 1)
if permission not in s:
    s = s.replace("<application", permission + "<application", 1)
for p in media:
    name = p.split('"')[1]
    if name not in s:
        s = s.replace("<application", p + "<application", 1)
open(path, "w", encoding="utf-8").write(s)
EOF
# The adaptive icon: same name as NSH's flat icon, which stays for Android 7.
cp -R "$res"/. "$work/src/res/"
java -jar "$jar" b -o "$work/unsigned.apk" "$work/src" >/dev/null
"$tools/zipalign" -p -f 4 "$work/unsigned.apk" "$work/aligned.apk"
KS_PASS=$pass "$tools/apksigner" sign --ks "$keystore" --ks-pass env:KS_PASS \
  --ks-key-alias "$alias" --key-pass env:KS_PASS \
  --out "$apk" "$work/aligned.apk"
"$tools/apksigner" verify "$apk"
"$tools/aapt2" dump xmltree --file AndroidManifest.xml "$apk" | grep -q REQUEST_INSTALL_PACKAGES
"$tools/aapt2" dump xmltree --file AndroidManifest.xml "$apk" | grep -q READ_MEDIA_VISUAL_USER_SELECTED
"$tools/aapt2" dump xmltree --file res/mipmap-anydpi-v26/ic_launcher.xml "$apk" | grep -q monochrome
echo "patched $(basename "$apk"): queries for launcher apps, REQUEST_INSTALL_PACKAGES, media reading, adaptive icon ($alias)"
