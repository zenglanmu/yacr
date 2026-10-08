#!/usr/bin/env bash
# Reproducible Android APK build for yacr.
#
# Pinned toolchain (see docs/build.md, docs/validation.md):
#   Rust 1.99.0, JDK 17, Android build-tools 34.0.0, platform android-34/30,
#   NDK 27.0.12077973, cargo-apk 0.10.0.
set -euo pipefail

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"
: "${JAVA_HOME:?set JAVA_HOME to a JDK 17}"
export ANDROID_NDK_HOME ANDROID_NDK_ROOT="$ANDROID_NDK_HOME" ANDROID_NDK="$ANDROID_NDK_HOME" JAVA_HOME
export PATH="$HOME/.cargo/bin:$JAVA_HOME/bin:$PATH"

TARGET="${TARGET:-aarch64-linux-android}"

command -v cargo-apk >/dev/null || { echo "install cargo-apk 0.10.0: cargo install cargo-apk --version 0.10.0 --locked"; exit 1; }

rustup target add "$TARGET"
cargo check --target "$TARGET" -p cad-ui-slint --locked
cargo check --target "$TARGET" -p app-android --locked
cargo apk build -p app-android --target "$TARGET" --lib "$@"

echo "APK: target/*/apk/yacr.apk"
