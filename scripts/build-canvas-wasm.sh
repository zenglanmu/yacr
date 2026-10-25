#!/usr/bin/env bash
# build-canvas-wasm.sh — build the wasm CAD session and post-process it into the
# desktop UI's public canvas directory.
#
# What lands in `apps/desktop-ui/public/canvas/` (all **generated**, git-ignored):
#
#   app_web.js       wasm-bindgen `--target web` ES module (the whole session).
#   app_web_bg.wasm  the one wasm binary; both JS files initialise from it.
#   bootstrap.js     a tiny **hand-written-by-this-script** ES module, served from
#                    `public/` so no bundler transforms it:
#
#                      import init, * as mod from './app_web.js';
#                      const ready = init(new URL('./app_web_bg.wasm', import.meta.url).href);
#                      globalThis.__YACR_CANVAS_MODULE__ = { ready, ...mod };
#
# Why a `public/` bootstrap module and not an import from the app bundle, and not
# the `--target no-modules` classic script:
#
#   * a dynamic `import()` of the artifact **inside the bundled tree** does not
#     survive Vite 8's dev transform: it becomes
#     `import(__vite__injectQuery(url, 'import'))`, i.e. it requests
#     `/canvas/app_web.js?import`, which the dev server answers with **HTTP 500**
#     (error-overlay HTML) for a public-dir file. `@vite-ignore` does not stop the
#     query injection. A file under `public/` is served verbatim, so the import
#     graph starts outside the bundler and is never rewritten;
#   * the `--target no-modules` classic script cannot be used: its glue starts with
#     `if (… && document.currentScript !== null) { … document.currentScript.src … }`
#     and WebKitGTK leaves `document.currentScript` **`undefined`** while a
#     dynamically inserted classic script runs. `undefined !== null` is true, the
#     top-level `let wasm_bindgen` never initialises, and every later
#     `typeof wasm_bindgen` throws the TDZ error
#     `ReferenceError: Cannot access 'wasm_bindgen' before initialization.`
#     (Verified in WebKitGTK 2.52.6, independent of Tauri.)
#
# `eval` is not an option either: the host CSP has no `'unsafe-eval'`.
#
# Vite copies `apps/desktop-ui/public/**` verbatim into `dist/`, so these files
# have to exist before `npm run build` (or a dev server run) for a `dom-canvas`
# host to have anything to load. Only one 9.5 MB wasm binary is shipped.
#
# The wasm build itself, the wasm-bindgen pin check and the artifact/size
# reporting are **shared** with `scripts/build-web.sh` (the standalone browser
# bundle) through `scripts/lib/wasm-toolchain.sh`.
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing target, a missing `wasm-bindgen`, a
#     version mismatch or a missing/empty artifact is a loud non-zero exit;
#   * it never substitutes a stub for a real build, so a successful run means the
#     real `app_web.wasm` was compiled from this tree (`--locked`, no network);
#   * a successful build is **not** runtime or pixel evidence: it says the
#     artifacts exist, nothing about a browser or a GPU.
#
# Usage:
#   bash scripts/build-canvas-wasm.sh
#
# Environment:
#   CARGO        cargo binary to use (default: `cargo` on PATH, else ~/.cargo/bin)
#   WASM_BINDGEN wasm-bindgen binary to use (default: PATH, else ~/.cargo/bin)
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
YACR_SCRIPT_NAME="$(basename -- "${BASH_SOURCE[0]}")"
# Toolchain resolution, the wasm-bindgen pin check, the `app-web` cargo build and
# the artifact guards are shared with `scripts/build-web.sh` (the browser bundle).
# shellcheck source=scripts/lib/wasm-toolchain.sh
. "$REPO_ROOT/scripts/lib/wasm-toolchain.sh"

readonly OUT_DIR="$REPO_ROOT/apps/desktop-ui/public/canvas"
readonly ESM_JS="app_web.js"
readonly WASM_ARTIFACT="app_web_bg.wasm"
readonly BOOTSTRAP_JS="bootstrap.js"
# The bootstrap module's content. Fixed on purpose: it is the one file the loader
# in `apps/desktop-ui/src/canvas-host.ts` waits on, and it must name the *existing*
# `app_web_bg.wasm` explicitly (the wasm-bindgen default would be derived from
# `app_web.js`'s own URL, which is the same file — spelling it out keeps the
# contract visible in one place).
readonly BOOTSTRAP_SOURCE='import init, * as mod from '\''./app_web.js'\'';
const ready = init(new URL('\''./app_web_bg.wasm'\'', import.meta.url).href);
globalThis.__YACR_CANVAS_MODULE__ = { ready, ...mod };'

die() {
  yacr_die "$*"
}

# --- toolchain -------------------------------------------------------------

yacr_init_wasm_toolchain

# --- output directory ------------------------------------------------------

# Only ever a generated, git-ignored directory under the UI package: refuse
# anything else before deleting, so a mangled variable cannot wipe source.
yacr_rotate_generated_dir \
  "$OUT_DIR" "$REPO_ROOT/apps/desktop-ui/public/canvas" "apps/desktop-ui/public/canvas"

# --- build -----------------------------------------------------------------

yacr_build_app_web_wasm
yacr_bindgen_web "$OUT_DIR"

printf 'writing %s (public/, so no bundler rewrites its imports)\n' "$BOOTSTRAP_JS"
printf '%s\n' "$BOOTSTRAP_SOURCE" > "$OUT_DIR/$BOOTSTRAP_JS"
[ -s "$OUT_DIR/$BOOTSTRAP_JS" ] || die "generated $BOOTSTRAP_JS is missing or empty: $OUT_DIR/$BOOTSTRAP_JS"

for artifact in "$OUT_DIR/$ESM_JS" "$OUT_DIR/$WASM_ARTIFACT" "$OUT_DIR/$BOOTSTRAP_JS"; do
  [ -s "$artifact" ] || die "expected artifact missing or empty: $artifact"
done

# Exactly one wasm binary: the bootstrap initialises from this one, so a second
# `*_bg.wasm` would be dead weight (and a sign a second target crept back in).
wasm_count=0
while IFS= read -r -d '' found; do
  wasm_count=$((wasm_count + 1))
done < <(find "$OUT_DIR" -maxdepth 1 -type f -name '*_bg.wasm' -print0)
[ "$wasm_count" -eq 1 ] || die "expected exactly one *_bg.wasm in $OUT_DIR, found $wasm_count"

# The bootstrap must import the module this build emitted and name the same binary.
grep -qF "./$ESM_JS" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not import ./$ESM_JS"
grep -qF "$WASM_ARTIFACT" "$OUT_DIR/$BOOTSTRAP_JS" \
  || die "$BOOTSTRAP_JS does not reference $WASM_ARTIFACT"

# --- report ----------------------------------------------------------------

printf 'artifacts in %s\n' "$OUT_DIR"
for artifact in "$OUT_DIR/$ESM_JS" "$OUT_DIR/$WASM_ARTIFACT" "$OUT_DIR/$BOOTSTRAP_JS"; do
  yacr_report_artifact "$artifact"
done
printf 'note: a built artifact is not runtime, GPU or pixel evidence.\n'
