#!/usr/bin/env bash
# build-android-ui.sh — build the **shared React chrome** for the Android host.
#
# `apps/desktop-ui` is one React tree for every host (AGENTS.md). For Android it
# is built once more with the runtime switched to `android` (`YACR_RUNTIME=android`,
# injected by `vite.config.mjs`): the page runs inside the Tauri WebView, the Tauri
# IPC bridge **is** present, and `src/host/invoke.ts` routes every CAD command to
# the native `CadSession` over that bridge; this carrier is `native-below`
# (ADR 0027), not the wasm `dom-canvas` session.
#
# What this script produces (all **generated**, git-ignored):
#
#   apps/app-android/dist/           the chrome bundle: `index.html`, `assets/*`,
#                                    and **no `canvas/` wasm**: this carrier is
#                                    `native-below` (ADR 0027) and never
#                                    instantiates the wasm session
#   apps/app-android/font-assets/    the CAD font package every host ships:
#                                    `fonts.json`, the face files, and the
#                                    committed license texts
#
# The Android artifact has its **own** directory on purpose: `npm run build`
# (the desktop hosts' `apps/desktop-ui/dist`) and this bundle must never clobber
# each other, because the two are built with different runtime constants.
#
# The bundle is embedded in the APK by `tauri android build` (Tauri serves it from
# its custom protocol), so no server is involved. `apps/app-android/font-assets` is
# an assets `srcDir` of the generated Gradle project, so the font package travels in
# the APK too. This script is also the `beforeBuildCommand` of
# `apps/app-android/src-tauri/tauri.conf.json`, which is why it never assumes a
# working directory (`BASH_SOURCE` resolves the repo root).
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing `node_modules`, a missing build entry
#     point, a missing/empty artifact or a failed font package is a loud non-zero
#     exit;
#   * it asserts the artifacts the WebView needs (the chrome bundle and at least
#     one emitted JS asset), so a bundle that would load a blank page is never
#     reported as a success;
#   * it asserts the font package the native loader needs (`fonts.json` plus at
#     least one face), so an APK that would start without fonts is not a success;
#   * it never substitutes a stub for the native session and never fabricates a
#     command answer;
#   * network: the UI build reuses the checked-in `package-lock.json` install
#     already on disk, and **fonts are the one networked step by default** (`WITH_FONTS=1` downloads the full mlightcad catalogue via
#     `scripts/fetch-fonts.sh`). `WITH_FONTS=0` stages the committed `fonts/` only
#     (offline) and `FONTS_DIR=<dir>` reuses a prepared package without downloading;
#   * a successful build is **artifact evidence only** — not runtime, device, GPU
#     or pixel evidence, and not an Android acceptance.
#
# Usage:
#   bash scripts/build-android-ui.sh
#
# Environment:
#   NPM          npm binary to use (default: `npm` on PATH)
#   WITH_FONTS   1 (default) full mlightcad catalogue (network) |
#                0 committed `fonts/` only (offline)
#   FONTS_DIR    prepared font directory to copy instead of downloading (must
#                contain `fonts.json`; overrides WITH_FONTS)
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES
#                passed through to `scripts/fetch-fonts.sh`
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"

readonly UI_DIR="$REPO_ROOT/apps/desktop-ui"
readonly OUT_DIR="$REPO_ROOT/apps/app-android/dist"
readonly FONT_ASSETS="$REPO_ROOT/apps/app-android/font-assets"
readonly NPM_BIN="${NPM:-npm}"

export YACR_REPO_ROOT="$REPO_ROOT"
# shellcheck source=scripts/lib/font-package.sh
. "$REPO_ROOT/scripts/lib/font-package.sh"

die() {
  printf '%s: %s\n' "$YACR_SCRIPT_NAME" "$*" >&2
  exit 1
}

report_artifact() {
  local artifact="$1"
  local label="$2"
  [ -s "$artifact" ] || die "expected artifact missing or empty: $artifact"
  local bytes
  bytes="$(wc -c < "$artifact")"
  printf '  %-28s %s bytes\n' "$label" "$bytes"
}

# Which directory the caller is in is not part of the contract, but it is useful
# when this runs as a Tauri `beforeBuildCommand` (the CLI picks the directory).
printf '==> cwd=%s (invoked as %s)\n' "$(pwd -P)" "${BASH_SOURCE[0]}"

# --- preconditions ---------------------------------------------------------

[ -f "$UI_DIR/package.json" ] || die "UI package manifest missing: $UI_DIR/package.json"
[ -d "$UI_DIR/node_modules" ] \
  || die "dependencies not installed: $UI_DIR/node_modules (run 'npm ci' in $UI_DIR; this script installs nothing)"
command -v "$NPM_BIN" >/dev/null 2>&1 || die "$NPM_BIN not found on PATH (set NPM)"
grep -qF '"build:android"' "$UI_DIR/package.json" \
  || die "$UI_DIR/package.json has no build:android script (this entry point would build nothing)"
grep -qF -- '--outDir ../app-android/dist' "$UI_DIR/package.json" \
  || die "$UI_DIR/package.json build:android must write ../app-android/dist (never the desktop dist/)"

# --- 1. no wasm session on this carrier ------------------------------------

# Android is `native-below` (ADR 0027): the native `CadSession` owns the
# document and the wasm `dom-canvas` session is never instantiated. Vite copies
# `public/**` verbatim, so a previously generated `apps/desktop-ui/public/canvas/`
# would leak a large unused wasm blob into the APK. Remove it before the build;
# section 3 asserts the bundle carries no `canvas/`. The shared generated source
# is deleted, so a later Linux `dom-canvas` package must rebuild it with
# `scripts/build-canvas-wasm.sh` first.
if [ -e "$UI_DIR/public/canvas" ]; then
  printf '==> dropping unused dom-canvas artifacts (native-below carrier)\n'
  rm -rf -- "$UI_DIR/public/canvas"
fi

# --- 2. the chrome bundle --------------------------------------------------

printf '==> React chrome, runtime=android (npm run build:android)\n'
(
  cd "$UI_DIR"
  "$NPM_BIN" run build:android
) || die "npm run build:android failed; no Android bundle was produced"

# --- 3. artifact guards ----------------------------------------------------

[ -d "$OUT_DIR" ] || die "build reported success but $OUT_DIR does not exist"
[ -s "$OUT_DIR/index.html" ] || die "bundle entry missing or empty: $OUT_DIR/index.html"
# This carrier is `native-below`; a `canvas/` directory means a stale
# `public/canvas/` was copied into an APK that can never use it.
[ ! -e "$OUT_DIR/canvas" ] \
  || die "unexpected $OUT_DIR/canvas: the native Android bundle must not ship the wasm session"
[ -s "$OUT_DIR/plugins/tch.js" ] \
  || die "vendor plugin missing or empty: $OUT_DIR/plugins/tch.js (is the plugins/tch submodule initialized?)"

# The desktop artifact must not have been rebuilt by accident: a bundle that
# wrote `apps/desktop-ui/dist` would route the two canvas reports to a Tauri host
# command set that does not exist on this carrier.
[ -d "$REPO_ROOT/apps/desktop-ui/dist" ] \
  && printf 'note: apps/desktop-ui/dist (desktop hosts) left untouched by this build.\n'

# At least one emitted JS asset: the bundle itself. A build that only copied the
# public directory would render nothing.
asset_count=0
while IFS= read -r -d '' asset; do
  [ -s "$asset" ] || die "emitted asset is empty: $asset"
  asset_count=$((asset_count + 1))
done < <(find "$OUT_DIR/assets" -maxdepth 1 -type f -name '*.js' -print0 2>/dev/null || true)
[ "$asset_count" -ge 1 ] || die "no emitted JS asset in $OUT_DIR/assets (the chrome bundle is missing)"

# The runtime switch must actually be in the bundle: an `android` build that
# still carried the `web` constant would route the two shell-only reports to the
# wasm adapter instead of the Tauri commands this host registers.
grep -qF 'unsupported-runtime: ' "$OUT_DIR/assets"/*.js \
  || die "the built bundle has no canvas transport marker; it was probably not built with YACR_RUNTIME=android"

# --- 4. the font package ---------------------------------------------------

# The generated Gradle project exposes `apps/app-android/font-assets` as an assets
# `srcDir`, so this package is what the native loader receives. It is assembled by
# the shared policy (`scripts/lib/font-package.sh`), i.e. the same package the other
# hosts ship: the full mlightcad catalogue by default (`WITH_FONTS=1`, network),
# the committed `fonts/` only with `WITH_FONTS=0` (offline), or a prepared
# `FONTS_DIR`. A failure here is loud — an APK without faces is not a build.
printf '==> CAD font package (apps/app-android/font-assets)\n'
rm -rf "$FONT_ASSETS"
yacr_stage_fonts "$FONT_ASSETS" "android font assets" || die "font package staging failed"
[ -s "$FONT_ASSETS/fonts.json" ] \
  || die "font package reported success but $FONT_ASSETS/fonts.json is missing or empty"
font_faces="$(yacr_font_package_faces "$FONT_ASSETS")"
[ "${font_faces:-0}" -gt 0 ] \
  || die "font package reported success but no font face landed in $FONT_ASSETS"

# --- report ----------------------------------------------------------------

printf 'bundle in %s\n' "$OUT_DIR"
report_artifact "$OUT_DIR/index.html" "index.html"
[ ! -e "$OUT_DIR/canvas" ] || die "bundle still carries canvas/ after the artifact guards"
report_artifact "$OUT_DIR/plugins/tch.js" "plugins/tch.js"
printf '  %-28s %s file(s)\n' "assets/*.js" "$asset_count"
printf 'fonts in %s\n' "$FONT_ASSETS"
report_artifact "$FONT_ASSETS/fonts.json" "fonts/fonts.json"
printf '  %-28s %s face(s)\n' "fonts faces" "$font_faces"
printf 'note: a built bundle is not runtime, device, GPU or pixel evidence, and not an Android acceptance.\n'
