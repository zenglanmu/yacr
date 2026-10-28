#!/usr/bin/env bash
# build-android-ui.sh — build the **shared React chrome** for the Android host.
#
# `apps/desktop-ui` is one React tree for every host (AGENTS.md). For Android it
# is built once more with the runtime switched to `android` (`YACR_RUNTIME=android`,
# injected by `vite.config.mjs`): the page runs inside the Tauri WebView, the Tauri
# IPC bridge **is** present, and `src/host/invoke.ts` routes the two shell-only
# canvas reports to Tauri while every CAD command goes to the wasm `dom-canvas`
# session (ADR 0024, iron rule 8).
#
# What this script produces (all **generated**, git-ignored):
#
#   apps/desktop-ui/public/canvas/   the wasm session artifacts, via
#                                    `scripts/build-canvas-wasm.sh`
#                                    (`app_web.js`, `app_web_bg.wasm`,
#                                    `bootstrap.js`)
#   apps/app-android/dist/           the chrome bundle: `index.html`, `assets/*`,
#                                    and a verbatim copy of `canvas/`
#
# The Android artifact has its **own** directory on purpose: `npm run build`
# (the desktop hosts' `apps/desktop-ui/dist`) and this bundle must never clobber
# each other, because the two are built with different runtime constants.
#
# The bundle is embedded in the APK by `tauri android build` (Tauri serves it from
# its custom protocol), so no server is involved. This script is also the
# `beforeBuildCommand` of `apps/app-android/src-tauri/tauri.conf.json`, which is
# why it never assumes a working directory (`BASH_SOURCE` resolves the repo root).
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing `node_modules`, a missing build entry
#     point or a missing/empty artifact is a loud non-zero exit;
#   * it asserts the artifacts the WebView needs (the chrome bundle, the wasm
#     module, the wasm binary, at least one emitted JS asset), so a bundle that
#     would load a blank page is never reported as a success;
#   * it never substitutes a stub for the wasm session and never fabricates a
#     command answer;
#   * no network: the wasm build is `--locked` and the UI build reuses the
#     checked-in `package-lock.json` install already on disk;
#   * a successful build is **artifact evidence only** — not runtime, device, GPU
#     or pixel evidence, and not an Android acceptance.
#
# Usage:
#   bash scripts/build-android-ui.sh
#
# Environment:
#   CARGO        cargo binary to use (passed through to the wasm build)
#   WASM_BINDGEN wasm-bindgen binary to use (passed through to the wasm build)
#   NPM          npm binary to use (default: `npm` on PATH)
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"

readonly UI_DIR="$REPO_ROOT/apps/desktop-ui"
readonly OUT_DIR="$REPO_ROOT/apps/app-android/dist"
readonly ESM_JS="app_web.js"
readonly WASM_ARTIFACT="app_web_bg.wasm"
readonly BOOTSTRAP_JS="bootstrap.js"
readonly NPM_BIN="${NPM:-npm}"

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
  || die "dependencies not installed: $UI_DIR/node_modules (run 'npm ci' in $UI_DIR; this script does not use the network)"
command -v "$NPM_BIN" >/dev/null 2>&1 || die "$NPM_BIN not found on PATH (set NPM)"
grep -qF '"build:android"' "$UI_DIR/package.json" \
  || die "$UI_DIR/package.json has no build:android script (this entry point would build nothing)"
grep -qF -- '--outDir ../app-android/dist' "$UI_DIR/package.json" \
  || die "$UI_DIR/package.json build:android must write ../app-android/dist (never the desktop dist/)"

# --- 1. the wasm session ---------------------------------------------------

# The chrome loads its canvas session from `/canvas/bootstrap.js`, which
# `apps/desktop-ui/public/` serves verbatim. Vite copies `public/**` into the
# output, so this has to exist *before* the UI build; the shared script asserts
# the four artifacts itself and exits non-zero on any failure.
printf '==> wasm CAD session artifacts (scripts/build-canvas-wasm.sh)\n'
bash "$REPO_ROOT/scripts/build-canvas-wasm.sh" \
  || die "scripts/build-canvas-wasm.sh failed; no Android bundle was produced"

# --- 2. the chrome bundle --------------------------------------------------

printf '==> React chrome, runtime=android (npm run build:android)\n'
(
  cd "$UI_DIR"
  "$NPM_BIN" run build:android
) || die "npm run build:android failed; no Android bundle was produced"

# --- 3. artifact guards ----------------------------------------------------

[ -d "$OUT_DIR" ] || die "build reported success but $OUT_DIR does not exist"
[ -s "$OUT_DIR/index.html" ] || die "bundle entry missing or empty: $OUT_DIR/index.html"
[ -s "$OUT_DIR/canvas/$ESM_JS" ] || die "canvas module missing or empty: $OUT_DIR/canvas/$ESM_JS"
[ -s "$OUT_DIR/canvas/$WASM_ARTIFACT" ] || die "canvas binary missing or empty: $OUT_DIR/canvas/$WASM_ARTIFACT"
[ -s "$OUT_DIR/canvas/$BOOTSTRAP_JS" ] || die "canvas bootstrap missing or empty: $OUT_DIR/canvas/$BOOTSTRAP_JS"

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

# --- report ----------------------------------------------------------------

printf 'bundle in %s\n' "$OUT_DIR"
report_artifact "$OUT_DIR/index.html" "index.html"
report_artifact "$OUT_DIR/canvas/$ESM_JS" "canvas/$ESM_JS"
report_artifact "$OUT_DIR/canvas/$WASM_ARTIFACT" "canvas/$WASM_ARTIFACT"
report_artifact "$OUT_DIR/canvas/$BOOTSTRAP_JS" "canvas/$BOOTSTRAP_JS"
printf '  %-28s %s file(s)\n' "assets/*.js" "$asset_count"
printf 'note: a built bundle is not runtime, device, GPU or pixel evidence, and not an Android acceptance.\n'
