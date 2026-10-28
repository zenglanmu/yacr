#!/usr/bin/env bash
# build-web-ui.sh — build the **shared React chrome** as a plain browser bundle.
#
# `apps/desktop-ui` is one React tree for every host (AGENTS.md). Its normal
# `dist/` is the Tauri hosts' artifact: the chrome plus the generated
# `public/canvas/` bootstrap, loaded by a WebView that provides the Tauri IPC
# bridge. A plain browser has no bridge at all, so the same tree is built once
# more with the runtime switched to `web` (`YACR_RUNTIME=web`, injected by
# `vite.config.mjs`) and routed through `src/host/web-adapter.ts` onto the four
# `shell_*` exports of the wasm CAD session.
#
# What this script produces (all **generated**, git-ignored):
#
#   apps/desktop-ui/public/canvas/   the wasm session artifacts, via
#                                    `scripts/build-canvas-wasm.sh`
#                                    (`app_web.js`, `app_web_bg.wasm`,
#                                    `bootstrap.js`)
#   apps/app-web/dist-ui/            the chrome bundle: `index.html`, `assets/*`,
#                                    and a verbatim copy of `canvas/`
#
# `apps/app-web/dist/` (the standalone vanilla harness) is **not** touched: it is
# a different artifact built by `scripts/build-web.sh`, and the two must not
# overwrite each other.
#
# The bundle is served over HTTP, not `file://` (wasm cannot be instantiated from
# a file URL), and the scripts themselves do not run a server:
#
#   python3 scripts/serve-web.py --directory apps/app-web/dist-ui
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing `node_modules`, a missing build entry
#     point or a missing/empty artifact is a loud non-zero exit;
#   * it asserts the artifacts a browser needs (the chrome bundle, the wasm
#     module, the wasm binary, at least one emitted JS asset), so a bundle that
#     would load a blank page is never reported as a success;
#   * it never substitutes a stub for the wasm session and never fabricates a
#     command answer: everything the chrome cannot do in a browser is an explicit
#     `unsupported-runtime: <command>` at runtime (`src/host/web-adapter.ts`);
#   * no network: the wasm build is `--locked` and the UI build reuses the
#     checked-in `package-lock.json` install already on disk;
#   * a successful build is artifact evidence only — not runtime, GPU or pixel
#     evidence, and not a browser acceptance.
#
# Usage:
#   bash scripts/build-web-ui.sh
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
readonly OUT_DIR="$REPO_ROOT/apps/app-web/dist-ui"
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

# --- preconditions ---------------------------------------------------------

[ -f "$UI_DIR/package.json" ] || die "UI package manifest missing: $UI_DIR/package.json"
[ -d "$UI_DIR/node_modules" ] \
  || die "dependencies not installed: $UI_DIR/node_modules (run 'npm ci' in $UI_DIR; this script does not use the network)"
command -v "$NPM_BIN" >/dev/null 2>&1 || die "$NPM_BIN not found on PATH (set NPM)"
grep -qF '"build:web"' "$UI_DIR/package.json" \
  || die "$UI_DIR/package.json has no build:web script (this entry point would build nothing)"

# --- 1. the wasm session ---------------------------------------------------

# The chrome loads its canvas session from `/canvas/bootstrap.js`, which
# `apps/desktop-ui/public/` serves verbatim. Vite copies `public/**` into the
# output, so this has to exist *before* the UI build; the shared script asserts
# the four artifacts itself and exits non-zero on any failure.
printf '==> wasm CAD session artifacts (scripts/build-canvas-wasm.sh)\n'
bash "$REPO_ROOT/scripts/build-canvas-wasm.sh" \
  || die "scripts/build-canvas-wasm.sh failed; no UI bundle was produced"

# --- 2. the chrome bundle --------------------------------------------------

printf '==> React chrome, runtime=web (npm run build:web)\n'
(
  cd "$UI_DIR"
  "$NPM_BIN" run build:web
) || die "npm run build:web failed; no UI bundle was produced"

# --- 3. artifact guards ----------------------------------------------------

[ -d "$OUT_DIR" ] || die "build reported success but $OUT_DIR does not exist"
[ -s "$OUT_DIR/index.html" ] || die "bundle entry missing or empty: $OUT_DIR/index.html"
[ -s "$OUT_DIR/canvas/$ESM_JS" ] || die "canvas module missing or empty: $OUT_DIR/canvas/$ESM_JS"
[ -s "$OUT_DIR/canvas/$WASM_ARTIFACT" ] || die "canvas binary missing or empty: $OUT_DIR/canvas/$WASM_ARTIFACT"
[ -s "$OUT_DIR/canvas/$BOOTSTRAP_JS" ] || die "canvas bootstrap missing or empty: $OUT_DIR/canvas/$BOOTSTRAP_JS"

# At least one emitted JS asset: the bundle itself. A build that only copied the
# public directory would render nothing.
asset_count=0
while IFS= read -r -d '' asset; do
  [ -s "$asset" ] || die "emitted asset is empty: $asset"
  asset_count=$((asset_count + 1))
done < <(find "$OUT_DIR/assets" -maxdepth 1 -type f -name '*.js' -print0 2>/dev/null || true)
[ "$asset_count" -ge 1 ] || die "no emitted JS asset in $OUT_DIR/assets (the chrome bundle is missing)"

# The runtime switch must actually be in the bundle: a `web` build that still
# carries the `tauri` constant would call the Tauri IPC in a browser and fail at
# the first poll. The literal is what `vite.config.mjs` injects.
grep -qF 'unsupported-runtime: ' "$OUT_DIR/assets"/*.js \
  || die "the built bundle has no web transport marker; it was probably not built with YACR_RUNTIME=web"

# The generated bundle must not shadow the standalone harness.
[ -d "$REPO_ROOT/apps/app-web/dist" ] && printf 'note: apps/app-web/dist (standalone harness) left untouched.\n'

# --- report ----------------------------------------------------------------

printf 'bundle in %s\n' "$OUT_DIR"
report_artifact "$OUT_DIR/index.html" "index.html"
report_artifact "$OUT_DIR/canvas/$ESM_JS" "canvas/$ESM_JS"
report_artifact "$OUT_DIR/canvas/$WASM_ARTIFACT" "canvas/$WASM_ARTIFACT"
report_artifact "$OUT_DIR/canvas/$BOOTSTRAP_JS" "canvas/$BOOTSTRAP_JS"
printf '  %-28s %s file(s)\n' "assets/*.js" "$asset_count"
printf 'serve with: python3 scripts/serve-web.py --directory apps/app-web/dist-ui\n'
printf 'note: a built bundle is not runtime, GPU or pixel evidence, and not a browser acceptance.\n'
