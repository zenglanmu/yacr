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
#   yacr_build_app_web_wasm [slim|full]
#                                 `cargo build -p app-web --target
#                                 wasm32-unknown-unknown --profile wasm --locked`
#                                 (the size profile in `Cargo.toml`). `slim`
#                                 (default) is the deployed browser bundle:
#                                 DWG/DXF connector lanes only, then asserts the
#                                 .wasm exists and fails closed when the resolved
#                                 tree regained a `cad-app` `formats-*` feature or
#                                 a connector crate. `full` passes `--features
#                                 full-formats` for the desktop `dom-canvas`
#                                 carrier and skips both the leak guard and the
#                                 20 MiB budget
#   yacr_bindgen_web <out-dir>    wasm-bindgen `--target web --no-typescript` into
#                                 <out-dir>, asserts both artifacts, then runs
#                                 `yacr_optimize_wasm` on the module
#   yacr_optimize_wasm <wasm>     `wasm-opt -Oz --strip-debug --strip-producers` in
#                                 place; a slim module over the 20 MiB DWG/DXF-only
#                                 budget, or any module over the 25 MiB Cloudflare
#                                 Pages per-file limit, is a hard error
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
readonly YACR_PINNED_WASM_BINDGEN="0.2.130"
# `wasm-opt` (binaryen) shrinks the bindgen output; Cloudflare Pages rejects any
# single file over 25 MiB, so `web-deploy` cannot publish a larger module. The
# binaryen release is pinned and CI downloads exactly that archive (SHA-256
# checked) instead of taking whatever the runner image ships.
readonly YACR_PINNED_WASM_OPT="120"
# Cloudflare Pages per-file upload ceiling: 25 MiB (26,214,400 bytes).
readonly YACR_MAX_WASM_BYTES=26214400
# Headroom budget for the DWG/DXF-only bundle: 20 MiB. This is **not** the publish
# ceiling. The module was ~24 MiB when every connector was linked; trimming the
# connector set is what buys the margin, so a silent return to the full set must
# fail loudly here, well before Cloudflare's 25 MiB per-file limit is reached.
readonly YACR_WASM_BUDGET_BYTES=20971520
# Which connector set the last `yacr_build_app_web_wasm` asked for. `1` (slim,
# DWG/DXF only) is the deployed browser bundle and the fail-closed default;
# `yacr_build_app_web_wasm full` clears it for the desktop `dom-canvas` carrier.
YACR_WASM_SLIM=1
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

  # The Cargo pin: `wasm-bindgen = "=0.2.130"` in the workspace dependency table.
  cargo_pin="$(
    sed -n 's/^[[:space:]]*wasm-bindgen[[:space:]]*=[[:space:]]*"=\([0-9][0-9.]*\)".*/\1/p' \
      "$manifest" | head -n 1
  )"
  [ -n "$cargo_pin" ] || yacr_die "no exact \`wasm-bindgen = \"=x.y.z\"\` pin found in $manifest"
  if [ "$cargo_pin" != "$YACR_PINNED_WASM_BINDGEN" ]; then
    yacr_die "wasm-bindgen pin moved: Cargo.toml says $cargo_pin, these entry points expect $YACR_PINNED_WASM_BINDGEN (re-review the wasm contract first)"
  fi

  # `wasm-bindgen --version` prints e.g. `wasm-bindgen 0.2.130` (first line only:
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
  local formats="${1:-slim}"
  local -a extra=()
  case "$formats" in
    # The deployed browser bundle: `cad-app`'s default features stay off, so the
    # module links the DWG/DXF lanes only and the 20 MiB budget applies.
    slim) YACR_WASM_SLIM=1 ;;
    # The desktop `dom-canvas` carrier: no per-file size ceiling and the same
    # capabilities as the native carriers, so `app-web`'s `full-formats` turns
    # every connector back on. Neither the leak guard nor the budget applies.
    full) YACR_WASM_SLIM=0; extra=(--features full-formats) ;;
    *) yacr_die "yacr_build_app_web_wasm takes 'slim' (the deployed browser bundle) or 'full' (the desktop dom-canvas carrier), got '$formats'" ;;
  esac
  [ -n "${YACR_CARGO_BIN:-}" ] || yacr_die "yacr_build_app_web_wasm called before yacr_init_wasm_toolchain"
  printf 'building app-web for %s (profile %s, %s formats, locked)\n' \
    "$YACR_WASM_TARGET" "$YACR_WASM_PROFILE" "$formats"
  "$YACR_CARGO_BIN" build -p app-web --target "$YACR_WASM_TARGET" \
    --profile "$YACR_WASM_PROFILE" --locked ${extra[@]+"${extra[@]}"}
  [ -f "$YACR_WASM_FILE" ] || yacr_die "cargo reported success but $YACR_WASM_FILE is missing"
  if [ "$YACR_WASM_SLIM" = 1 ]; then
    yacr_assert_app_web_slim
  fi
}

# Fail closed when the **resolved** app-web tree is not the DWG/DXF-only one.
#
# `apps/app-web/Cargo.toml` is one edge; Cargo unifies features per build, so the
# bundle can regain the full connector set two ways that the manifest text alone
# does not show:
#
# 1. a **shared crate** that depends on `cad-app` without
#    `default-features = false` re-enables every `formats-*` feature. The same
#    happens when a member writes `default-features = false` but the
#    `[workspace.dependencies]` entry omits the key: Cargo silently drops the
#    member's value (verified against a scratch workspace), so that entry has to
#    carry `default-features = false` and native carriers opt back in with
#    `default-features = true`;
# 2. an edge that links a connector crate directly (`rvt`, `cadmpeg-*`,
#    `ifc-lite-*`, `skpwright`, `cadkit-*`, `gltf`, `image`).
#
# Both checks read the real tree, not the manifest, because the leak is a
# *resolution* result. A `cargo tree` failure is loud on purpose: an unverifiable
# tree must never pass as a slim one.
yacr_assert_app_web_slim() {
  [ -n "${YACR_CARGO_BIN:-}" ] || yacr_die "yacr_assert_app_web_slim called before yacr_init_wasm_toolchain"
  local features_tree normal_tree leaked

  # `--edges features,normal,build --target <wasm>` is the wasm build's own unit
  # graph, on purpose: the default edge set also walks `dev`, and `cad-host`'s
  # `[dev-dependencies] cad-app` legitimately keeps the default `formats-all` so
  # its fixture tests can open STL/DGN/IFC. `cargo tree -e features` folds that
  # dev edge into the displayed resolution, so a dev-inclusive tree would fail
  # this gate on a correctly slim bundle.
  if ! features_tree="$("$YACR_CARGO_BIN" tree -p app-web --target "$YACR_WASM_TARGET" --edges features,normal,build -i cad-app --locked --offline)"; then
    yacr_die "cargo tree -e features -i cad-app failed; cannot prove the wasm bundle kept its DWG/DXF-only format set"
  fi
  if leaked="$(grep -E 'formats-' <<<"$features_tree")"; then
    printf '%s\n' "$leaked" >&2
    yacr_die "the app-web wasm bundle regained cad-app 'formats-*' features (lines above): fix apps/app-web/Cargo.toml (the cad-app edge must set default-features = false and name no formats-* feature) and every shared crate with a [dependencies.cad-app] edge the same way"
  fi

  if ! normal_tree="$("$YACR_CARGO_BIN" tree -p app-web --target "$YACR_WASM_TARGET" -e normal --locked --offline)"; then
    yacr_die "cargo tree -p app-web --target $YACR_WASM_TARGET failed; cannot prove the bundle dropped the other connectors"
  fi
  # `^<tree glyphs>*<crate> v<version>` anchors on a package line, so a crate that
  # merely contains one of these names (`cad-import-rvt`, `cad-export-gltf`) does
  # not match: the name must be followed by its version.
  if leaked="$(grep -E '^[│├└─ ]*(rvt|cadmpeg-[a-z0-9_-]+|ifc-lite-[a-z0-9_-]+|skpwright|cadkit-[a-z0-9_-]+|gltf|image) v[0-9]' <<<"$normal_tree")"; then
    printf '%s\n' "$leaked" >&2
    yacr_die "the app-web wasm bundle still links connector crates it must have dropped (lines above): this carrier is DWG/DXF only; check apps/app-web/Cargo.toml and every shared crate's [dependencies.cad-app] edge (formats-* features must stay off)"
  fi
}

# Shrink the bindgen output in place and enforce the size gates.
#
# `web-deploy` uploads this module to Cloudflare Pages, which rejects any file
# over 25 MiB. A module that optimizes above the limit is a hard error here (not
# a deploy-time surprise) with the size and the remedy, so the bundle can never
# be published partially or silently. The tighter 20 MiB budget catches a
# regression -- the full connector set creeping back in -- before it can reach
# the publish ceiling.
yacr_optimize_wasm() {
  local wasm="$1"
  local optimized="$wasm.opt"
  [ -n "${YACR_WASM_OPT_BIN:-}" ] || yacr_die "yacr_optimize_wasm called before yacr_init_wasm_toolchain"
  [ -s "$wasm" ] || yacr_die "wasm to optimize is missing or empty: $wasm"
  local before after
  before="$(stat -c '%s' "$wasm")"
  printf 'optimizing with wasm-opt %s (-Oz, %s bytes)\n' \
    "$YACR_WASM_OPT_VERSION" "$before"
  # `-Oz` includes binaryen's `merge-similar-functions`, which can emit a
  # `byn$mgfn-shared$…` function with >1000 parameters; V8 (browser and Node)
  # then refuses the module: "param count of 1023 exceeds internal limit of
  # 1000". Skipping that pass keeps the module loadable at a ~0.3 MB cost.
  "$YACR_WASM_OPT_BIN" \
    -Oz \
    --skip-pass=merge-similar-functions \
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
  if [ "$YACR_WASM_SLIM" = 1 ] && [ "$after" -gt "$YACR_WASM_BUDGET_BYTES" ]; then
    yacr_die "$wasm is $after bytes after wasm-opt -Oz, over the $YACR_WASM_BUDGET_BYTES-byte (20 MiB) budget for the DWG/DXF-only bundle: it regained connector formats; keep app-web's cad-app edge at default-features = false with no formats-* feature (and the same on every shared crate)"
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
