#!/usr/bin/env bash
# build-web.sh — build the standalone browser bundle for the wasm CAD session.
#
# `apps/app-web` is the real `surfaceMode: dom-canvas` CAD session compiled to
# wasm (ADR 0024): it imports the synthetic fixture, builds the display scene,
# renders it with the renderer's offscreen frame and presents that frame on a DOM
# `<canvas>` through the wasm-only WebGL2 presenter. The shell in
# `apps/app-web/web/` is dependency-free (no framework, no bundler, no second
# document) and imports the wasm-bindgen `--target web` output from `./pkg/`.
#
# What this script produces:
#
#   apps/app-web/web/pkg/   wasm-bindgen `--target web` output: `app_web.js` +
#                           `app_web_bg.wasm` (generated, git-ignored). This is
#                           exactly the `./pkg/` the shell's docs tell you to
#                           build when serving `apps/app-web/web/` directly.
#   apps/app-web/dist/      the deployable bundle: `index.html`, `main.js`,
#                           `style.css`, `pkg/app_web.js`, `pkg/app_web_bg.wasm`
#                           (generated, git-ignored). Serve it over HTTP — `file://`
#                           cannot instantiate wasm:
#                             python3 scripts/serve-web.py --directory apps/app-web/dist
#
# The wasm build, the wasm-bindgen pin check and the artifact reporting are
# shared with `scripts/build-canvas-wasm.sh` (the desktop `dom-canvas` glue)
# through `scripts/lib/wasm-toolchain.sh`, so the cargo/wasm-bindgen invocations
# exist once.
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing toolchain, a moved pin, a version
#     mismatch, a missing shell file or a missing/empty artifact is a loud
#     non-zero exit;
#   * it never emits a partial or stub bundle: the bundle directory is recreated
#     from scratch and every expected artifact is asserted non-empty afterwards;
#   * a successful build is **not** runtime, GPU or pixel evidence. It says the
#     bundle exists, nothing about a browser, a GPU, or that the drawing renders.
#     There is still **no** CI build and **no** publish path (`web-build`,
#     `web-deploy` and `scripts/deploy-cloudflare-pages.sh` remain retired).
#
# Usage:
#   bash scripts/build-web.sh
#
# Environment:
#   CARGO        cargo binary to use (default: `cargo` on PATH, else ~/.cargo/bin)
#   WASM_BINDGEN wasm-bindgen binary to use (default: PATH, else ~/.cargo/bin)
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"
# shellcheck source=scripts/lib/wasm-toolchain.sh
. "$REPO_ROOT/scripts/lib/wasm-toolchain.sh"

# Committed shell sources.
readonly WEB_SRC="$REPO_ROOT/apps/app-web/web"
readonly SHELL_FILES=(index.html main.js style.css)
# Generated, git-ignored outputs.
readonly PKG_DIR="$WEB_SRC/pkg"
readonly OUT_DIR="$REPO_ROOT/apps/app-web/dist"
readonly PKG_JS="app_web.js"
readonly PKG_WASM="app_web_bg.wasm"

die() {
  yacr_die "$*"
}

# --- source shell ----------------------------------------------------------

for file in "${SHELL_FILES[@]}"; do
  [ -s "$WEB_SRC/$file" ] || die "browser shell source missing or empty: $WEB_SRC/$file"
done

# --- toolchain + wasm ------------------------------------------------------

yacr_init_wasm_toolchain
yacr_build_app_web_wasm

# The shell imports `./pkg/app_web.js` next to itself, so the module is emitted
# into the shell directory rather than only into the assembled bundle: serving
# `apps/app-web/web/` directly keeps working as documented in `main.js`.
yacr_rotate_generated_dir "$PKG_DIR" "$WEB_SRC/pkg" "apps/app-web/web/pkg"
yacr_bindgen_web "$PKG_DIR"

# --- assemble the deployable bundle ---------------------------------------

yacr_rotate_generated_dir "$OUT_DIR" "$REPO_ROOT/apps/app-web/dist" "apps/app-web/dist"
mkdir -p -- "$OUT_DIR/pkg"
for file in "${SHELL_FILES[@]}"; do
  cp -- "$WEB_SRC/$file" "$OUT_DIR/$file"
done
cp -- "$PKG_DIR/$PKG_JS" "$OUT_DIR/pkg/$PKG_JS"
cp -- "$PKG_DIR/$PKG_WASM" "$OUT_DIR/pkg/$PKG_WASM"

# Every artifact must exist and be non-empty; a partial bundle is never reported
# as a success.
expected=(
  "$OUT_DIR/index.html"
  "$OUT_DIR/main.js"
  "$OUT_DIR/style.css"
  "$OUT_DIR/pkg/$PKG_JS"
  "$OUT_DIR/pkg/$PKG_WASM"
)
for artifact in "${expected[@]}"; do
  [ -s "$artifact" ] || die "bundle artifact missing or empty: $artifact"
done

# Nothing else may sit in the bundle: a stale/extra file would be served without
# anyone noticing it is not part of this build.
file_count="$(find "$OUT_DIR" -type f -printf '.' | wc -c)"
if [ "$file_count" -ne "${#expected[@]}" ]; then
  die "bundle contains $file_count files, expected ${#expected[@]} (unexpected file in $OUT_DIR): $(find "$OUT_DIR" -type f -printf '%P ')"
fi

# The shell must actually reference the module this build emitted, otherwise the
# bundle is deployable but blank.
grep -qF "./pkg/$PKG_JS" "$OUT_DIR/main.js" \
  || die "$OUT_DIR/main.js does not import ./pkg/$PKG_JS; the bundle would not load"
grep -qF "$PKG_WASM" "$OUT_DIR/pkg/$PKG_JS" \
  || die "$OUT_DIR/pkg/$PKG_JS does not reference $PKG_WASM; the emitted module is incomplete"

# --- report ----------------------------------------------------------------

printf 'artifacts in %s\n' "$OUT_DIR"
yacr_report_artifact "$OUT_DIR/index.html" "index.html"
yacr_report_artifact "$OUT_DIR/main.js" "main.js"
yacr_report_artifact "$OUT_DIR/style.css" "style.css"
yacr_report_artifact "$OUT_DIR/pkg/$PKG_JS" "pkg/$PKG_JS"
yacr_report_artifact "$OUT_DIR/pkg/$PKG_WASM" "pkg/$PKG_WASM"
printf 'serve with: python3 scripts/serve-web.py --directory %s\n' "$OUT_DIR"
printf 'note: a built bundle is not runtime, GPU or pixel evidence, and there is no CI/publish path.\n'
