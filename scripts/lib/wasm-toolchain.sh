#!/usr/bin/env bash
# wasm-toolchain.sh — shared toolchain resolution, pin checks and size
# optimization for the wasm CAD session (`app-web`) build entry points.
#
# The same wasm binary feeds two entry points, so the fragile parts live **once**
# here instead of being copied per script:
#
#   scripts/build-canvas-wasm.sh  desktop `dom-canvas` glue into
#                                 apps/desktop-ui/public/canvas (web + generated
#                                 public bootstrap module)
#   scripts/build-web.sh          standalone browser bundle into apps/app-web/dist
#
# This file is **sourced, never executed**. The caller must set `YACR_REPO_ROOT`
# (absolute) and `YACR_SCRIPT_NAME` (diagnostic prefix) before sourcing.
#
# Contract exposed to callers:
#   yacr_die <message>            stderr diagnostic with the caller's prefix, exit 1
#   yacr_init_wasm_toolchain      resolves cargo + wasm-bindgen and verifies that the
#                                 CLI version equals the `Cargo.toml` pin exactly;
#                                 sets YACR_CARGO_BIN, YACR_WASM_BINDGEN_BIN,
#                                 YACR_WASM_BINDGEN_VERSION, YACR_WASM_FILE
#   yacr_build_app_web_wasm       `cargo build -p app-web --target wasm32-unknown-unknown
#                                 --profile wasm --locked` (the size profile in
#                                 `Cargo.toml`), then asserts the .wasm exists
#   yacr_bindgen_web <out-dir>    wasm-bindgen `--target web --no-typescript` into
#                                 <out-dir>, asserts both artifacts, then runs
#                                 `yacr_optimize_wasm` on the module
#   yacr_optimize_wasm <wasm>     `wasm-opt -Oz --strip-debug --strip-producers` in
#                                 place; a module still over the 25 MiB Cloudflare
#                                 Pages per-file limit is a hard error
#   yacr_rotate_generated_dir <dir> <expected> <label>
#                                 removes + recreates a **generated** directory, and
#                                 refuses any path other than <expected>
#   yacr_report_artifact <file>   one size line; a missing/empty file is a hard error
#
# Honesty rule: nothing here skips silently or substitutes a stub. A missing
# toolchain, a moved pin, a version mismatch or a missing artifact is a loud
# non-zero exit, and a successful build is artifact evidence only — not runtime,
# GPU or pixel evidence.
# shellcheck shell=bash

readonly YACR_WASM_TARGET="wasm32-unknown-unknown"
# The pin these entry points were written against. The workspace pin is re-read
# below and must still agree: if someone moves `wasm-bindgen` in `Cargo.toml`,
# these scripts have to be re-checked rather than silently producing a mismatched
# module.
readonly YACR_PINNED_WASM_BINDGEN="0.2.129"
# `wasm-opt` (binaryen) shrinks the bindgen output; Cloudflare Pages rejects any
# single file over 25 MiB, so `web-deploy` cannot publish a larger module. The
# binaryen release is pinned and CI downloads exactly that archive (SHA-256
# checked) instead of taking whatever the runner image ships.
readonly YACR_PINNED_WASM_OPT="120"
# Cloudflare Pages per-file upload ceiling: 25 MiB (26,214,400 bytes).
readonly YACR_MAX_WASM_BYTES=26214400
# Custom size profile (`[profile.wasm]` in the workspace manifest); its output
# lands in `target/<target>/wasm/`, not `.../release/`.
readonly YACR_WASM_PROFILE="wasm"

yacr_die() {
  printf '%s: %s\n' "${YACR_SCRIPT_NAME:-wasm-toolchain.sh}" "$*" >&2
  exit 1
}

# The repository root is set by the caller; fall back to the conventional
# `REPO_ROOT` both entry points already define.
if [ -z "${YACR_REPO_ROOT:-}" ]; then
  YACR_REPO_ROOT="${REPO_ROOT:-}"
fi
[ -n "$YACR_REPO_ROOT" ] \
  || yacr_die "YACR_REPO_ROOT (or REPO_ROOT) must be set before sourcing scripts/lib/wasm-toolchain.sh"
readonly YACR_REPO_ROOT

yacr_init_wasm_toolchain() {
  local manifest="$YACR_REPO_ROOT/Cargo.toml"
  local lock="$YACR_REPO_ROOT/Cargo.lock"
  local cargo_pin

  [ -f "$manifest" ] || yacr_die "workspace manifest missing: $manifest"
  [ -f "$lock" ] || yacr_die "workspace lockfile missing: $lock"

  if [ -n "${CARGO:-}" ]; then
    YACR_CARGO_BIN="$CARGO"
  elif command -v cargo >/dev/null 2>&1; then
    YACR_CARGO_BIN="$(command -v cargo)"
  elif [ -x "$HOME/.cargo/bin/cargo" ]; then
    YACR_CARGO_BIN="$HOME/.cargo/bin/cargo"
  else
    yacr_die "cargo not found (install the toolchain in rust-toolchain.toml or set CARGO)"
  fi

  if [ -n "${WASM_BINDGEN:-}" ]; then
    YACR_WASM_BINDGEN_BIN="$WASM_BINDGEN"
  elif command -v wasm-bindgen >/dev/null 2>&1; then
    YACR_WASM_BINDGEN_BIN="$(command -v wasm-bindgen)"
  elif [ -x "$HOME/.cargo/bin/wasm-bindgen" ]; then
    YACR_WASM_BINDGEN_BIN="$HOME/.cargo/bin/wasm-bindgen"
  else
    yacr_die "wasm-bindgen CLI not found; install the pinned version with: cargo install wasm-bindgen-cli --version $YACR_PINNED_WASM_BINDGEN"
  fi

  # The Cargo pin: `wasm-bindgen = "=0.2.129"` in the workspace dependency table.
  cargo_pin="$(
    sed -n 's/^[[:space:]]*wasm-bindgen[[:space:]]*=[[:space:]]*"=\([0-9][0-9.]*\)".*/\1/p' \
      "$manifest" | head -n 1
  )"
  [ -n "$cargo_pin" ] || yacr_die "no exact \`wasm-bindgen = \"=x.y.z\"\` pin found in $manifest"
  if [ "$cargo_pin" != "$YACR_PINNED_WASM_BINDGEN" ]; then
    yacr_die "wasm-bindgen pin moved: Cargo.toml says $cargo_pin, these entry points expect $YACR_PINNED_WASM_BINDGEN (re-review the wasm contract first)"
  fi

  # `wasm-bindgen --version` prints e.g. `wasm-bindgen 0.2.129` (first line only:
  # a wrong binary can print arbitrary multi-line output).
  YACR_WASM_BINDGEN_VERSION="$("$YACR_WASM_BINDGEN_BIN" --version | head -n 1 | awk '{print $NF}')"
  if [ "$YACR_WASM_BINDGEN_VERSION" != "$cargo_pin" ]; then
    yacr_die "wasm-bindgen CLI is $YACR_WASM_BINDGEN_VERSION but the workspace pins $cargo_pin; run: cargo install wasm-bindgen-cli --version $cargo_pin --locked"
  fi

  if [ -n "${WASM_OPT:-}" ]; then
    YACR_WASM_OPT_BIN="$WASM_OPT"
  elif command -v wasm-opt >/dev/null 2>&1; then
    YACR_WASM_OPT_BIN="$(command -v wasm-opt)"
  elif [ -x "$HOME/.cargo/bin/wasm-opt" ]; then
    YACR_WASM_OPT_BIN="$HOME/.cargo/bin/wasm-opt"
  else
    yacr_die "wasm-opt (binaryen version_$YACR_PINNED_WASM_OPT) not found; install that binaryen release (https://github.com/WebAssembly/binaryen/releases/tag/version_$YACR_PINNED_WASM_OPT) or set WASM_OPT"
  fi
  # `wasm-opt --version` prints e.g. `wasm-opt version 120`.
  YACR_WASM_OPT_VERSION="$("$YACR_WASM_OPT_BIN" --version | head -n 1 | awk '{print $3}')"
  if [ "$YACR_WASM_OPT_VERSION" != "$YACR_PINNED_WASM_OPT" ]; then
    yacr_die "wasm-opt is ${YACR_WASM_OPT_VERSION:-unknown} but these entry points pin binaryen $YACR_PINNED_WASM_OPT; install the pinned binaryen"
  fi

  YACR_WASM_FILE="$YACR_REPO_ROOT/target/$YACR_WASM_TARGET/$YACR_WASM_PROFILE/app_web.wasm"
}

yacr_build_app_web_wasm() {
  [ -n "${YACR_CARGO_BIN:-}" ] || yacr_die "yacr_build_app_web_wasm called before yacr_init_wasm_toolchain"
  printf 'building app-web for %s (profile %s, locked)\n' \
    "$YACR_WASM_TARGET" "$YACR_WASM_PROFILE"
  "$YACR_CARGO_BIN" build -p app-web --target "$YACR_WASM_TARGET" \
    --profile "$YACR_WASM_PROFILE" --locked
  [ -f "$YACR_WASM_FILE" ] || yacr_die "cargo reported success but $YACR_WASM_FILE is missing"
}

# Shrink the bindgen output in place and enforce the publishable ceiling.
#
# `web-deploy` uploads this module to Cloudflare Pages, which rejects any file
# over 25 MiB. A module that optimizes above the limit is a hard error here (not
# a deploy-time surprise) with the size and the remedy, so the bundle can never
# be published partially or silently.
yacr_optimize_wasm() {
  local wasm="$1"
  local optimized="$wasm.opt"
  [ -n "${YACR_WASM_OPT_BIN:-}" ] || yacr_die "yacr_optimize_wasm called before yacr_init_wasm_toolchain"
  [ -s "$wasm" ] || yacr_die "wasm to optimize is missing or empty: $wasm"
  local before after
  before="$(stat -c '%s' "$wasm")"
  printf 'optimizing with wasm-opt %s (-Oz, %s bytes)\n' \
    "$YACR_WASM_OPT_VERSION" "$before"
  "$YACR_WASM_OPT_BIN" \
    -Oz \
    --strip-debug \
    --strip-producers \
    -o "$optimized" \
    "$wasm" \
    || yacr_die "wasm-opt failed on $wasm"
  [ -s "$optimized" ] || yacr_die "wasm-opt produced no output for $wasm"
  mv -f -- "$optimized" "$wasm"
  after="$(stat -c '%s' "$wasm")"
  if [ "$after" -gt "$YACR_MAX_WASM_BYTES" ]; then
    yacr_die "$wasm is $after bytes after wasm-opt -Oz, over the $YACR_MAX_WASM_BYTES-byte (25 MiB) Cloudflare Pages per-file limit; shrink the module (dependencies or [profile.wasm]) instead of deploying it"
  fi
  printf '  %-22s %10s -> %s bytes\n' "$(basename -- "$wasm")" "$before" "$after"
}

yacr_bindgen_web() {
  local out_dir="$1"
  [ -n "${YACR_WASM_BINDGEN_BIN:-}" ] || yacr_die "yacr_bindgen_web called before yacr_init_wasm_toolchain"
  [ -f "$YACR_WASM_FILE" ] || yacr_die "wasm input missing: $YACR_WASM_FILE"
  printf 'post-processing with wasm-bindgen %s (--target web)\n' "$YACR_WASM_BINDGEN_VERSION"
  "$YACR_WASM_BINDGEN_BIN" \
    --target web \
    --no-typescript \
    --out-dir "$out_dir" \
    "$YACR_WASM_FILE"
  [ -s "$out_dir/app_web.js" ] || yacr_die "wasm-bindgen produced no app_web.js in $out_dir"
  [ -s "$out_dir/app_web_bg.wasm" ] || yacr_die "wasm-bindgen produced no app_web_bg.wasm in $out_dir"
  yacr_optimize_wasm "$out_dir/app_web_bg.wasm"
}

yacr_rotate_generated_dir() {
  local dir="$1"
  local expected="$2"
  local label="$3"
  if [ "$dir" != "$expected" ]; then
    yacr_die "refusing to write outside $label: $dir"
  fi
  if [ -d "$dir" ]; then
    printf 'removing previous artifact directory: %s\n' "$dir"
    rm -rf -- "$dir"
  fi
  mkdir -p -- "$dir"
}

yacr_report_artifact() {
  local artifact="$1"
  local label="${2:-}"
  local bytes
  [ -s "$artifact" ] || yacr_die "expected artifact missing or empty: $artifact"
  if [ -z "$label" ]; then
    label="$(basename -- "$artifact")"
  fi
  bytes="$(stat -c '%s' "$artifact")"
  printf '  %-22s %10s bytes (%s)\n' \
    "$label" "$bytes" "$(numfmt --to=iec --suffix=B "$bytes")"
}
