#!/usr/bin/env bash
# Build MacGuitar for Android phones (arm64): an APK in target/android/.
#
#   scripts/build-android.sh            build MacGuitar-<version>-android-arm64.apk
#   scripts/build-android.sh --install  also install it on the phone adb sees
#
# Needs the Android SDK with an NDK (ANDROID_HOME, ANDROID_NDK_ROOT), Java,
# and cargo-apk (cargo install cargo-apk). The APK is signed with the debug
# key of this machine, ~/.android/debug.keystore, unless
# CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD name
# another: an update installs over a version signed with the same key.
set -euo pipefail

cd "$(dirname "$0")/.."

install=false
for arg in "$@"; do
  case "$arg" in
    --install) install=true ;;
    *) echo "unknown option: $arg" >&2; exit 1 ;;
  esac
done

: "${ANDROID_HOME:=/opt/homebrew/share/android-commandlinetools}"
if [ -z "${ANDROID_NDK_ROOT:-}" ]; then
  ANDROID_NDK_ROOT=$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -n 1)
fi
export ANDROID_HOME ANDROID_NDK_ROOT

if [ -z "${CARGO_APK_RELEASE_KEYSTORE:-}" ]; then
  keystore="$HOME/.android/debug.keystore"
  if [ ! -f "$keystore" ]; then
    mkdir -p "$(dirname "$keystore")"
    keytool -genkeypair -keystore "$keystore" -storepass android -alias androiddebugkey \
      -keypass android -keyalg RSA -keysize 2048 -validity 10000 \
      -dname "CN=Android Debug,O=Android,C=US" >/dev/null
  fi
  export CARGO_APK_RELEASE_KEYSTORE="$keystore"
  export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
fi

rustup target add aarch64-linux-android >/dev/null 2>&1 || true
cargo apk build --release --lib

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
out=target/android
apk="$out/MacGuitar-$version-android-arm64.apk"
mkdir -p "$out"
cp target/release/apk/MacGuitar.apk "$apk"
echo "built $apk"

if [ "$install" = true ]; then
  "$ANDROID_HOME/platform-tools/adb" install -r "$apk"
  echo "installed on the phone"
fi
