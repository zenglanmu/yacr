#!/usr/bin/env bash
# Reproducible Android APK build for yacr.
#
# Pinned toolchain (see docs/build.md, docs/validation.md):
#   Rust 1.99.0, JDK 17, Android build-tools 34.0.0, platform android-34/30,
#   NDK 27.0.12077973, cargo-apk 0.10.0.
#
# After `cargo apk build` this script post-processes the APK to add the SAF
# mediation shim (a small Java class, `apps/app-android/android/dev/yacr/app/`):
#   javac -> d8 (classes.dex) -> inject into the APK zip -> zipalign -> re-sign.
# The manifest already names the shim class as the launcher activity
# (`[package.metadata.android.application.activity]`), so no binary manifest edit
# is needed. The post-processing is mandatory: the manifest sets
# `android:hasCode="true"` and names `dev.yacr.app.YacrNativeActivity`, so an APK
# without the injected `classes.dex` would fail at launch with a
# `ClassNotFoundException`. There is no skip switch.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"
: "${JAVA_HOME:?set JAVA_HOME to a JDK 17}"
export ANDROID_NDK_HOME ANDROID_NDK_ROOT="$ANDROID_NDK_HOME" ANDROID_NDK="$ANDROID_NDK_HOME" JAVA_HOME
export PATH="$HOME/.cargo/bin:$JAVA_HOME/bin:$PATH"

TARGET="${TARGET:-aarch64-linux-android}"
BUILD_TOOLS="${BUILD_TOOLS:-$ANDROID_HOME/build-tools/34.0.0}"
PLATFORM="${ANDROID_PLATFORM:-$ANDROID_HOME/platforms/android-34/android.jar}"
# Matches the shipped manifest's `android:minSdkVersion` (23); keep in sync with
# `[package.metadata.android] min_sdk_version` in apps/app-android/Cargo.toml.
MIN_SDK="${MIN_SDK:-23}"

command -v cargo-apk >/dev/null || { echo "install cargo-apk 0.10.0: cargo install cargo-apk --version 0.10.0 --locked"; exit 1; }

# Development-only release signing key (gitignored). cargo-apk needs it for a
# release build; generate it on demand with the documented dev password.
KEYSTORE="$REPO_ROOT/apps/app-android/dev-release.jks"
if [[ ! -f "$KEYSTORE" ]]; then
    echo "Generating development keystore $KEYSTORE"
    "$JAVA_HOME/bin/keytool" -genkeypair -keystore "$KEYSTORE" -alias yacr \
        -keyalg RSA -keysize 2048 -validity 10000 -storepass yacrdev -keypass yacrdev \
        -dname "CN=yacr dev, O=yacr"
fi

rustup target add "$TARGET"
cargo check --target "$TARGET" -p cad-ui-slint --locked
cargo check --target "$TARGET" -p app-android --locked
cargo apk build -p app-android --target "$TARGET" --lib "$@"

# --- locate the produced APK ---------------------------------------------------
PROFILE=debug
for arg in "$@"; do
    [[ "$arg" == "--release" ]] && PROFILE=release
done
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
if [[ "$TARGET_DIR" != /* ]]; then
    TARGET_DIR="$PWD/$TARGET_DIR"
fi
APK="$TARGET_DIR/$PROFILE/apk/yacr.apk"
if [[ ! -f "$APK" ]]; then
    echo "APK not found at $APK" >&2
    exit 1
fi

# --- SAF shim post-processing (mandatory; see the header) ----------------------
JAVA_SRC_DIR="$REPO_ROOT/apps/app-android/android"
WORK_DIR="${SAF_WORK_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/yacr-saf.XXXXXX")}"
CLASSES_DIR="$WORK_DIR/classes"
DEX_DIR="$WORK_DIR/dex"
mkdir -p "$CLASSES_DIR" "$DEX_DIR"

echo "Compiling the SAF shim with javac"
mapfile -t JAVA_SOURCES < <(find "$JAVA_SRC_DIR" -name '*.java')
if [[ "${#JAVA_SOURCES[@]}" -eq 0 ]]; then
    echo "no Java sources under $JAVA_SRC_DIR" >&2
    exit 1
fi
"$JAVA_HOME/bin/javac" -source 8 -target 8 -Xlint:-options \
    -classpath "$PLATFORM" -d "$CLASSES_DIR" "${JAVA_SOURCES[@]}"

echo "Dexing the SAF shim with d8"
mapfile -t CLASS_FILES < <(find "$CLASSES_DIR" -name '*.class')
"$BUILD_TOOLS/d8" --lib "$PLATFORM" --min-api "$MIN_SDK" \
    --output "$DEX_DIR" "${CLASS_FILES[@]}"
[[ -f "$DEX_DIR/classes.dex" ]] || { echo "d8 produced no classes.dex" >&2; exit 1; }

echo "Injecting classes.dex into the APK"
UNSIGNED="$WORK_DIR/unsigned.apk"
ALIGNED="$WORK_DIR/aligned.apk"
cp "$APK" "$UNSIGNED"
(cd "$DEX_DIR" && zip -q "$UNSIGNED" classes.dex)

echo "Aligning and re-signing the APK"
"$BUILD_TOOLS/zipalign" -f 4 "$UNSIGNED" "$ALIGNED"
"$BUILD_TOOLS/apksigner" sign \
    --ks "$KEYSTORE" --ks-pass pass:yacrdev --key-pass pass:yacrdev --ks-key-alias yacr \
    --out "$APK" "$ALIGNED"
"$BUILD_TOOLS/apksigner" verify "$APK"

echo "Verifying the manifest declares the SAF shim activity and hasCode"
MANIFEST_DUMP="$WORK_DIR/manifest.txt"
"$BUILD_TOOLS/aapt2" dump xmltree --file AndroidManifest.xml "$APK" > "$MANIFEST_DUMP"
grep -q 'dev.yacr.app.YacrNativeActivity' "$MANIFEST_DUMP" \
    || { echo "manifest does not declare dev.yacr.app.YacrNativeActivity" >&2; exit 1; }
grep -q 'android:hasCode[^=]*=true' "$MANIFEST_DUMP" \
    || { echo "manifest does not set android:hasCode=true" >&2; exit 1; }

echo "APK: $APK"
