#!/usr/bin/env bash
# build-canvas-wasm.sh — build the wasm CAD session and post-process it into the
# desktop UI's public canvas directory.
#
# Two JS bindings are emitted from the **one** wasm binary:
#
#   app_web.js            wasm-bindgen `--target web` (ES module). Used by the
#                         standalone browser shell `apps/app-web/web`, which
#                         imports it as a module with no bundler in between.
#   app_web_nomodules.js  wasm-bindgen `--target no-modules` (classic script).
#                         This is the file `apps/desktop-ui/src/canvas-host.mjs`
#                         injects with a `<script src>` tag. It is a classic
#                         script because Vite 8's dev transform rewrites a dynamic
#                         import into `import(__vite__injectQuery(url, 'import'))`
#                         — `/canvas/app_web.js?import`, which the dev server
#                         answers with HTTP 500 (error-overlay HTML) for a
#                         public-dir file, and `@vite-ignore` does not stop it.
#                         `eval` is not an option (the host CSP has no
#                         `'unsafe-eval'`).
#
# Both are **generated artifacts**, not source: Vite copies
# `apps/desktop-ui/public/**` verbatim into `dist/`, so they have to exist in
# `apps/desktop-ui/public/canvas/` before `npm run build` (or a dev server run)
# for a `dom-canvas` host to have anything to load. Both generated trees are
# git-ignored, and the 9.5 MB wasm binary is **not** duplicated: the no-modules
# glue is initialised by the loader with the explicit existing
# `app_web_bg.wasm` URL.
#
# Honesty rules for this entry point:
#   * it never skips silently — a missing target, a missing `wasm-bindgen`, or a
#     version mismatch is a loud non-zero exit;
#   * it never substitutes a stub for a real build, so a successful run means the
#     real `app_web.wasm` was compiled from this tree (`--locked`, no network);
#   * a successful build is **not** runtime or pixel evidence: it says the
#     artifact exists, nothing about a browser or a GPU.
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

readonly CRATE_MANIFEST="$REPO_ROOT/Cargo.toml"
readonly WORKSPACE_LOCK="$REPO_ROOT/Cargo.lock"
readonly OUT_DIR="$REPO_ROOT/apps/desktop-ui/public/canvas"
readonly WASM_FILE="$REPO_ROOT/target/wasm32-unknown-unknown/release/app_web.wasm"
readonly WASM_TARGET="wasm32-unknown-unknown"
readonly ESM_JS="app_web.js"
readonly NOMODULES_JS="app_web_nomodules.js"
readonly WASM_ARTIFACT="app_web_bg.wasm"
# The pin this script was written against. The workspace pin is re-read below and
# must still agree: if someone moves `wasm-bindgen` in `Cargo.toml`, this script
# has to be re-checked rather than silently producing a mismatched module.
readonly PINNED_WASM_BINDGEN="0.2.129"

die() {
  printf 'build-canvas-wasm.sh: %s\n' "$*" >&2
  exit 1
}

# --- toolchain -------------------------------------------------------------

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ]; then
  if command -v cargo >/dev/null 2>&1; then
    CARGO_BIN="$(command -v cargo)"
  elif [ -x "$HOME/.cargo/bin/cargo" ]; then
    CARGO_BIN="$HOME/.cargo/bin/cargo"
  else
    die "cargo not found (install the toolchain in rust-toolchain.toml or set CARGO)"
  fi
fi
readonly CARGO_BIN

WASM_BINDGEN_BIN="${WASM_BINDGEN:-}"
if [ -z "$WASM_BINDGEN_BIN" ]; then
  if command -v wasm-bindgen >/dev/null 2>&1; then
    WASM_BINDGEN_BIN="$(command -v wasm-bindgen)"
  elif [ -x "$HOME/.cargo/bin/wasm-bindgen" ]; then
    WASM_BINDGEN_BIN="$HOME/.cargo/bin/wasm-bindgen"
  else
    die "wasm-bindgen CLI not found; install the pinned version with: cargo install wasm-bindgen-cli --version $PINNED_WASM_BINDGEN"
  fi
fi
readonly WASM_BINDGEN_BIN

[ -f "$CRATE_MANIFEST" ] || die "workspace manifest missing: $CRATE_MANIFEST"
[ -f "$WORKSPACE_LOCK" ] || die "workspace lockfile missing: $WORKSPACE_LOCK"

# The Cargo pin: `wasm-bindgen = "=0.2.129"` in the workspace dependency table.
cargo_pin="$(
  sed -n 's/^[[:space:]]*wasm-bindgen[[:space:]]*=[[:space:]]*"=\([0-9][0-9.]*\)".*/\1/p' \
    "$CRATE_MANIFEST" | head -n 1
)"
[ -n "$cargo_pin" ] || die "no exact \`wasm-bindgen = \"=x.y.z\"\` pin found in $CRATE_MANIFEST"
if [ "$cargo_pin" != "$PINNED_WASM_BINDGEN" ]; then
  die "wasm-bindgen pin moved: Cargo.toml says $cargo_pin, this script expects $PINNED_WASM_BINDGEN (re-review the wasm contract first)"
fi

# `wasm-bindgen --version` prints e.g. `wasm-bindgen 0.2.129`.
cli_version="$("$WASM_BINDGEN_BIN" --version | awk '{print $NF}')"
if [ "$cli_version" != "$cargo_pin" ]; then
  die "wasm-bindgen CLI is $cli_version but the workspace pins $cargo_pin; run: cargo install wasm-bindgen-cli --version $cargo_pin --locked"
fi

# --- output directory ------------------------------------------------------

# Only ever a generated, git-ignored directory under the UI package: refuse
# anything else before deleting, so a mangled variable cannot wipe source.
case "$OUT_DIR" in
  "$REPO_ROOT"/apps/desktop-ui/public/canvas) ;;
  *) die "refusing to write outside apps/desktop-ui/public/canvas: $OUT_DIR" ;;
esac
if [ -d "$OUT_DIR" ]; then
  printf 'removing previous artifact directory: %s\n' "$OUT_DIR"
  rm -rf -- "$OUT_DIR"
fi
mkdir -p -- "$OUT_DIR"

# --- build -----------------------------------------------------------------

printf 'building app-web for %s (release, locked)\n' "$WASM_TARGET"
"$CARGO_BIN" build -p app-web --target "$WASM_TARGET" --release --locked
[ -f "$WASM_FILE" ] || die "cargo reported success but $WASM_FILE is missing"

printf 'post-processing with wasm-bindgen %s (--target web)\n' "$cli_version"
"$WASM_BINDGEN_BIN" \
  --target web \
  --no-typescript \
  --out-dir "$OUT_DIR" \
  "$WASM_FILE"

# Second binding: the classic script the desktop transport injects. Emitted into a
# throwaway directory and copied as JS only — its sibling `_bg.wasm` is byte-identical
# to the one above, and shipping two 9.5 MB binaries would be pure waste.
NOMODULES_DIR="$(mktemp -d "${TMPDIR:-/tmp}/yacr-canvas-nomodules.XXXXXX")"
cleanup_nomodules() { rm -rf -- "$NOMODULES_DIR"; }
trap cleanup_nomodules EXIT

printf 'post-processing with wasm-bindgen %s (--target no-modules)\n' "$cli_version"
"$WASM_BINDGEN_BIN" \
  --target no-modules \
  --no-typescript \
  --out-name app_web \
  --out-dir "$NOMODULES_DIR" \
  "$WASM_FILE"
[ -s "$NOMODULES_DIR/$ESM_JS" ] || die "wasm-bindgen produced no $ESM_JS in $NOMODULES_DIR"
cp -- "$NOMODULES_DIR/$ESM_JS" "$OUT_DIR/$NOMODULES_JS"
[ -s "$OUT_DIR/$NOMODULES_JS" ] || die "copied classic script is missing or empty: $OUT_DIR/$NOMODULES_JS"
# Guard the whole point of the second target: it must not be an ES module.
if grep -qE '^[[:space:]]*(import|export)[[:space:]]' "$OUT_DIR/$NOMODULES_JS"; then
  die "$NOMODULES_JS looks like an ES module; the classic-script contract is broken (--target no-modules expected)"
fi

for artifact in "$OUT_DIR/$ESM_JS" "$OUT_DIR/$WASM_ARTIFACT" "$OUT_DIR/$NOMODULES_JS"; do
  [ -s "$artifact" ] || die "expected artifact missing or empty: $artifact"
done

# Exactly one wasm binary: the no-modules glue must initialise from this one (the
# loader passes its URL explicitly), so a second `*_bg.wasm` would be dead weight.
wasm_count=0
while IFS= read -r -d '' found; do
  wasm_count=$((wasm_count + 1))
done < <(find "$OUT_DIR" -maxdepth 1 -type f -name '*_bg.wasm' -print0)
[ "$wasm_count" -eq 1 ] || die "expected exactly one *_bg.wasm in $OUT_DIR, found $wasm_count"

# --- report ----------------------------------------------------------------

printf 'artifacts in %s\n' "$OUT_DIR"
for artifact in "$OUT_DIR/$ESM_JS" "$OUT_DIR/$WASM_ARTIFACT" "$OUT_DIR/$NOMODULES_JS"; do
  bytes="$(stat -c '%s' "$artifact")"
  printf '  %-22s %10s bytes (%s)\n' "$(basename "$artifact")" "$bytes" "$(numfmt --to=iec --suffix=B "$bytes")"
done
printf 'note: a built artifact is not runtime, GPU or pixel evidence.\n'
