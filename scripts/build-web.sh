#!/usr/bin/env bash
# Reproducible browser build for yacr (spec v2.0 §9.2).
#
# Output: web-dist/ = static files (index.html, main.js, style.css, pkg/*),
# deployable to any static host serving application/wasm over HTTPS.
#
# Pinned: Rust 1.98.1 (rust-toolchain.toml), wasm-bindgen-cli 0.2.129
# (must match the wasm-bindgen crate version in Cargo.lock).
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="${PROFILE:-release}"
DIST="${DIST:-$ROOT/web-dist}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
WASM="$TARGET_DIR/wasm32-unknown-unknown/$PROFILE/app_web.wasm"

cargo build -p app-web --target wasm32-unknown-unknown --profile "$PROFILE" --locked

command -v wasm-bindgen >/dev/null || {
  echo "install wasm-bindgen-cli 0.2.129: cargo install wasm-bindgen-cli --version 0.2.129 --locked" >&2
  exit 1
}

rm -rf "$DIST"
mkdir -p "$DIST/pkg"
wasm-bindgen --target web --no-typescript --out-name yacr --out-dir "$DIST/pkg" "$WASM"

cp apps/app-web/web/index.html "$DIST/index.html"
cp apps/app-web/web/main.js "$DIST/main.js"
cp apps/app-web/web/style.css "$DIST/style.css"

# Report the module size so regressions are visible in CI logs.
WASM_SIZE=$(stat -c %s "$DIST/pkg/yacr_bg.wasm")
echo "web build: $DIST (wasm ${WASM_SIZE} bytes, profile ${PROFILE})"
echo "serve with: scripts/serve-web.py --directory $DIST"