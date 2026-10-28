#!/usr/bin/env bash
# build-android.sh — build the **Android host APK** (Tauri 2 carrier + wasm session).
#
# The Android host (`apps/app-android`) is a Tauri 2 WebView/asset carrier: the
# shared React chrome runs in the Android WebView and the wasm `dom-canvas` CAD
# session owns the document (ADR 0024, AGENTS.md iron rule 8). There is no native
# CAD surface and no Java/`cargo-apk` shim any more — the previous retirement stub
# (non-zero "NOT IMPLEMENTED") is gone with the retired pipeline.
#
# What this entry point does:
#
#   1. verifies the toolchain it actually needs (Node/npm, the Android SDK + NDK,
#      a JDK 17, the Tauri CLI installed in `apps/app-android`);
#   2. installs the host's npm dependencies if `node_modules` is missing;
#   3. runs `npx tauri android build --apk …`, which **first** runs the configured
#      `beforeBuildCommand` (`scripts/build-android-ui.sh` → wasm session artifacts
#      + `YACR_RUNTIME=android` chrome bundle into `apps/app-android/dist`), then
#      the Gradle project in `src-tauri/gen/android`, then reports the APK;
#   4. prints the APK path and size and says plainly that a **release** APK is
#      unsigned unless a keystore is configured.
#
# Honesty rules:
#   * a missing toolchain, a failed UI build, a failed Gradle build or a missing
#     APK is a loud non-zero exit — there is no stub and no "skipped" success;
#   * the artifact is a **build** result: it is not device, runtime, GPU or pixel
#     evidence, and an emulator run is not a real-device acceptance;
#   * no signing key is committed. `--debug` produces an installable debug APK
#     (signed with the local debug keystore); a release APK needs its own key.
#
# Usage:
#   bash scripts/build-android.sh [--debug] [--target <aarch64|armv7|i686|x86_64>] [-- <extra tauri args>]
#
# Defaults: release, `--target aarch64` (real devices). The local emulator here is
# x86_64, so an emulator run needs `--target x86_64`.
#
# Environment: ANDROID_HOME (required), NDK_HOME (required), JAVA_HOME (JDK 17,
# required), CARGO/WASM_BINDGEN/NPM (passed through to the UI build).
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"
readonly APP_DIR="$REPO_ROOT/apps/app-android"

die() {
  printf '%s: %s\n' "$YACR_SCRIPT_NAME" "$*" >&2
  exit 1
}

profile_args=()
target="aarch64"
extra=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --debug) profile_args+=(--debug); shift ;;
    # `tauri android build` is release by default and has no `--release` flag; the
    # flag is accepted here so a caller can be explicit without guessing.
    --release) shift ;;
    --target)
      [ "$#" -ge 2 ] || die "--target needs a value"
      target="$2"; shift 2 ;;
    --) shift; extra+=("$@"); break ;;
    -h|--help) sed -n '2,40p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

case "$target" in
  aarch64|armv7|i686|x86_64) ;;
  *) die "--target must be one of aarch64, armv7, i686, x86_64 (got '$target')" ;;
esac

# --- preconditions ---------------------------------------------------------

command -v node >/dev/null 2>&1 || die "node not found on PATH (the Tauri CLI and the UI build need it)"
command -v npm >/dev/null 2>&1 || die "npm not found on PATH"
[ -n "${ANDROID_HOME:-}" ] || die "ANDROID_HOME is not set (Android SDK)"
[ -d "$ANDROID_HOME" ] || die "ANDROID_HOME does not exist: $ANDROID_HOME"
[ -n "${NDK_HOME:-}" ] || die "NDK_HOME is not set (e.g. \$ANDROID_HOME/ndk/29.0.13846066)"
[ -d "$NDK_HOME" ] || die "NDK_HOME does not exist: $NDK_HOME"
[ -n "${JAVA_HOME:-}" ] || die "JAVA_HOME is not set (JDK 17 for Gradle)"
[ -x "$JAVA_HOME/bin/java" ] || die "JAVA_HOME/bin/java is missing: $JAVA_HOME"
"$JAVA_HOME/bin/java" -version 2>&1 | grep -q 'version "17' \
  || die "JDK 17 is required by this Gradle project (JAVA_HOME=$JAVA_HOME)"
[ -f "$APP_DIR/src-tauri/gen/android/settings.gradle" ] \
  || die "the generated Gradle project is missing; run 'npx tauri android init' in $APP_DIR"
[ -x "$APP_DIR/node_modules/.bin/tauri" ] || missing_cli=1
if [ "${missing_cli:-0}" = "1" ]; then
  printf '%s: installing the host npm dependencies (apps/app-android/node_modules missing)\n' "$YACR_SCRIPT_NAME"
  ( cd "$APP_DIR" && npm install --no-audit --no-fund ) \
    || die "npm install failed in $APP_DIR"
fi

# --- build -----------------------------------------------------------------

printf '==> tauri android build (apk, target=%s, profile=%s)\n' \
  "$target" "$([ "${#profile_args[@]}" -gt 0 ] && echo "${profile_args[0]#--}" || echo release)"
(
  cd "$APP_DIR"
  npx tauri android build --apk --target "$target" "${profile_args[@]}" "${extra[@]+"${extra[@]}"}"
) || die "tauri android build failed; no APK was produced"

# --- artifact guards -------------------------------------------------------

profile=release
[ "${#profile_args[@]}" -gt 0 ] && profile="${profile_args[0]#--}"
case "$profile" in
  debug) apk_dir="$APP_DIR/src-tauri/gen/android/app/build/outputs/apk/universal/debug" ;;
  *) apk_dir="$APP_DIR/src-tauri/gen/android/app/build/outputs/apk/universal/release" ;;
esac
[ -d "$apk_dir" ] || die "the build reported success but $apk_dir does not exist"
apk_count=0
while IFS= read -r -d '' apk; do
  apk_count=$((apk_count + 1))
  printf '  %-44s %s bytes\n' "$(basename -- "$apk")" "$(wc -c < "$apk")"
  printf '  %s\n' "$apk"
done < <(find "$apk_dir" -maxdepth 1 -type f -name '*.apk' -print0)
[ "$apk_count" -ge 1 ] || die "no .apk in $apk_dir (the build produced nothing installable)"

printf 'note: install with `adb install -r <apk>`; package id is dev.yacr.app.\n'
if [ "$profile" = "release" ]; then
  printf 'note: a release APK is UNSIGNED unless a keystore is configured; sign it before installing.\n'
fi
printf 'note: a built APK is not device, runtime, GPU or pixel evidence.\n'
