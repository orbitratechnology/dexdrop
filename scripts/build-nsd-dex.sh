#!/usr/bin/env bash
# Builds the Android NSD bridge (crates/dexdrop-net/android/...java) into
# crates/dexdrop-net/android/nsd.dex, which dexdrop-net's Cargo.toml lists
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
