#!/usr/bin/env bash
# Builds the Android bridges (crates/dexdrop-net/android/...java) into
# crates/dexdrop-net/android/nsd.dex (and the direct link's permissions
# into direct-link.aar, below), which dexdrop-net's Cargo.toml lists
# as an Android library ([package.metadata.nsh.android] libraries): `nsh
# package` carries it in the APK, and src/jvm.rs loads its classes with the
# application's class loader. Run after changing the Java source and commit
# the result.
#
#   ANDROID_HOME=/opt/android-sdk scripts/build-nsd-dex.sh
set -euo pipefail
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/android-sdk}}
platform=$(ls -d "$sdk"/platforms/android-* | sort -V | tail -1)
tools=$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)
here=$(cd "$(dirname "$0")/.." && pwd)
src="$here/crates/dexdrop-net/android"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

javac --release 8 -Xlint:-options -cp "$platform/android.jar" -d "$work/classes" \
  "$src"/dev/dexdrop/*.java
"$tools/d8" --release --min-api 26 --lib "$platform/android.jar" --output "$work" \
  $(find "$work/classes" -name '*.class')
cp "$work/classes.dex" "$src/nsd.dex"
echo "wrote $src/nsd.dex ($(wc -c <"$src/nsd.dex") bytes)"

# The direct link's permissions with their attributes, which a crate's
# `permissions` list cannot carry (names only): NEARBY_WIFI_DEVICES never
# used for location (else Android 13+ asks for location too), and fine
# location only up to Android 12, where Wi-Fi Direct needs it. An .aar's
# manifest permissions join the app's (https://nsh.dev/docs/nshm.md); its
# classes.jar is empty (the classes are in nsd.dex).
mkdir -p "$work/aar"
cat >"$work/aar/AndroidManifest.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="dev.dexdrop.directlink">
    <uses-sdk android:minSdkVersion="26"/>
    <uses-permission android:name="android.permission.NEARBY_WIFI_DEVICES"
        android:usesPermissionFlags="neverForLocation"/>
    <uses-permission android:name="android.permission.ACCESS_FINE_LOCATION"
        android:maxSdkVersion="32"/>
</manifest>
XML
mkdir -p "$work/empty/META-INF"
printf 'Manifest-Version: 1.0\n' >"$work/empty/META-INF/MANIFEST.MF"
(cd "$work/empty" && zip -q -X -D "$work/aar/classes.jar" META-INF/MANIFEST.MF)
rm -f "$src/direct-link.aar"
(cd "$work/aar" && zip -q -X -D "$src/direct-link.aar" AndroidManifest.xml classes.jar)
echo "wrote $src/direct-link.aar ($(wc -c <"$src/direct-link.aar") bytes)"
