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
#                           `worker.js`, `style.css`, `pkg/app_web.js`,
#                           `pkg/app_web_bg.wasm` and a staged `fonts/` package
#                           (the full mlightcad catalogue merged with the
#                           committed `fonts/`; `WITH_FONTS=1` by default,
#                           generated, git-ignored). The page still fetches only
#                           the faces the open drawing needs, lazily, from this
#                           same-origin `fonts/` — bundling removes the CDN
#                           dependency, it does not add a preload. Serve it over
#                           HTTP — `file://` cannot instantiate wasm:
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
#   * fonts: `WITH_FONTS=1` (default) downloads the full mlightcad catalogue via
#     `scripts/fetch-fonts.sh` (network); `WITH_FONTS=0` stages the committed
#     `fonts/` only (offline) and `FONTS_DIR=<dir>` reuses a prepared package
#     (must contain `fonts.json`) with no download;
#   * a successful build is **not** runtime, GPU or pixel evidence. It says the
#     bundle exists, nothing about a browser, a GPU, or that the drawing renders.
#     CI builds this bundle (`web-build`) and drives it headlessly under software
#     rendering (`web-smoke`, SwiftShader — not GPU evidence); publishing is
#     `web-deploy`, gated to `v*` tags or a manual dispatch and limited to the
#     artifact that job uploaded.
#
# Usage:
#   bash scripts/build-web.sh
#
# Environment:
#   CARGO        cargo binary to use (default: `cargo` on PATH, else ~/.cargo/bin)
#   WASM_BINDGEN wasm-bindgen binary to use (default: PATH, else ~/.cargo/bin)
#   WITH_FONTS   1 (default) stage the full CAD catalogue (needs network);
#                0 stage the committed `fonts/` only (offline)
#   FONTS_DIR    reuse a prepared font package directory (must contain
#                `fonts.json`); overrides WITH_FONTS and performs no download
#   FONT_BASE_URL / FONT_FALLBACK_BASE_URL / FONT_RETRIES
#                inherited by `scripts/fetch-fonts.sh` when downloading
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"
YACR_REPO_ROOT="$REPO_ROOT"
readonly YACR_REPO_ROOT
# shellcheck source=scripts/lib/wasm-toolchain.sh
. "$REPO_ROOT/scripts/lib/wasm-toolchain.sh"
# shellcheck source=scripts/lib/font-package.sh
. "$REPO_ROOT/scripts/lib/font-package.sh"

# Committed shell sources.
readonly WEB_SRC="$REPO_ROOT/apps/app-web/web"
readonly SHELL_FILES=(index.html main.js worker.js style.css)
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

# Both local Web bundles carry the same package the deployed site serves at its
# root: the full mlightcad catalogue merged with the committed `fonts/`
# (`WITH_FONTS=1`, default) or the committed package only (`WITH_FONTS=0`), or a
# prepared `FONTS_DIR`. The page still resolves `cad_fonts` lazily after an open
# and fetches only the needed faces from this same-origin `fonts/`.
printf '==> bundled font package (%s -> %s/fonts)\n' \
  "${FONTS_DIR:-WITH_FONTS=${WITH_FONTS:-1}}" "$OUT_DIR"
rm -rf -- "$OUT_DIR/fonts"
yacr_stage_fonts "$OUT_DIR/fonts" "web fonts" \
  || die "font package staging failed (WITH_FONTS=${WITH_FONTS:-1}${FONTS_DIR:+, FONTS_DIR=$FONTS_DIR})"
font_face_count="$(yacr_font_package_faces "$OUT_DIR/fonts")"
if [ -n "${FONTS_DIR:-}" ]; then
  font_source="prepared package $FONTS_DIR (no download)"
elif [ "${WITH_FONTS:-1}" = "0" ]; then
  font_source="committed fonts/ only (WITH_FONTS=0, offline)"
else
  font_source="full CAD catalogue (scripts/fetch-fonts.sh)"
fi

# Every artifact must exist and be non-empty; a partial bundle is never reported
# as a success. The fixed artifacts are the six shell/module files.
expected=(
  "$OUT_DIR/index.html"
  "$OUT_DIR/main.js"
  "$OUT_DIR/worker.js"
  "$OUT_DIR/style.css"
  "$OUT_DIR/pkg/$PKG_JS"
  "$OUT_DIR/pkg/$PKG_WASM"
)
for artifact in "${expected[@]}"; do
  [ -s "$artifact" ] || die "bundle artifact missing or empty: $artifact"
done

# The bundle is a whitelist: the six artifacts above plus anything under
# `fonts/`. A stale/extra file anywhere else is served without anyone noticing
# it is not part of this build, so it is a hard error.
while IFS= read -r -d '' path; do
  rel="${path#"$OUT_DIR"/}"
  case "$rel" in
    index.html | main.js | worker.js | style.css) ;;
    "pkg/$PKG_JS" | "pkg/$PKG_WASM") ;;
    fonts/*) ;;
    *)
      die "unexpected file in $OUT_DIR: $rel (bundle is the six shell/pkg artifacts plus fonts/)"
      ;;
  esac
  [ -s "$path" ] || die "bundle artifact missing or empty: $path"
done < <(find "$OUT_DIR" -type f -print0)

# The font package is part of the bundle now: the catalog and at least one face
# must be present (the helper already refused an empty package, this guards the
# copy that landed in the bundle).
[ -s "$OUT_DIR/fonts/fonts.json" ] \
  || die "font package catalog missing or empty: $OUT_DIR/fonts/fonts.json"
[ "${font_face_count:-0}" -gt 0 ] \
  || die "font package contains no face files: $OUT_DIR/fonts"

# The shell must actually reference the module this build emitted, otherwise the
# bundle is deployable but blank.
grep -qF "./pkg/$PKG_JS" "$OUT_DIR/main.js" \
  || die "$OUT_DIR/main.js does not import ./pkg/$PKG_JS; the bundle would not load"
# The Worker entry must ship too, otherwise a capable browser silently falls back
# to the main-thread carrier instead of running isolated.
[ -s "$OUT_DIR/worker.js" ] || die "$OUT_DIR/worker.js missing; isolation would silently degrade"
grep -qF "$PKG_WASM" "$OUT_DIR/pkg/$PKG_JS" \
  || die "$OUT_DIR/pkg/$PKG_JS does not reference $PKG_WASM; the emitted module is incomplete"
# The main-thread carrier must install the package lazily, after an open: a shell
# that dropped the loader would leave every text drawing `unsupported-resources: fonts`.
grep -qF "ensureFontPackage" "$OUT_DIR/main.js" \
  || die "$OUT_DIR/main.js has no lazy ensureFontPackage()"
grep -qF "set_font_package" "$OUT_DIR/main.js" \
  || die "$OUT_DIR/main.js does not call set_font_package; text would be unshaped"
grep -qF "apply_font_package" "$OUT_DIR/main.js" \
  || die "$OUT_DIR/main.js does not call apply_font_package"
if grep -qF "await loadFontPackage()" "$OUT_DIR/main.js"; then
  die "$OUT_DIR/main.js still preloads the font package before start"
fi

# --- report ----------------------------------------------------------------

printf 'artifacts in %s\n' "$OUT_DIR"
yacr_report_artifact "$OUT_DIR/index.html" "index.html"
yacr_report_artifact "$OUT_DIR/main.js" "main.js"
yacr_report_artifact "$OUT_DIR/worker.js" "worker.js"
yacr_report_artifact "$OUT_DIR/style.css" "style.css"
yacr_report_artifact "$OUT_DIR/pkg/$PKG_JS" "pkg/$PKG_JS"
yacr_report_artifact "$OUT_DIR/pkg/$PKG_WASM" "pkg/$PKG_WASM"
printf '  %-28s %s face(s) (%s)\n' "fonts/*" "$font_face_count" "$font_source"
printf 'serve with: python3 scripts/serve-web.py --directory %s\n' "$OUT_DIR"
printf 'note: a built bundle is not runtime, GPU or pixel evidence; CI drives it headlessly under software rendering and publishes only on tags/dispatch.\n'
