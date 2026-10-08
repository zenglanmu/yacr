#!/usr/bin/env bash
# Reproducible browser build for yacr (spec v2.0 §9.2).
#
# Output: web-dist/ = static files (index.html, main.js, style.css, pkg/*),
# deployable to any static host serving application/wasm over HTTPS.
#
# Pinned: Rust 1.99.0 (rust-toolchain.toml), wasm-bindgen-cli 0.2.129
# (must match the wasm-bindgen crate version in Cargo.lock).
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="${PROFILE:-release}"
DIST="${DIST:-$ROOT/web-dist}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
WASM="$TARGET_DIR/wasm32-unknown-unknown/$PROFILE/app_web.wasm"

# Font source baked into the wasm at compile time (`apps/app-web/src/browser/fonts.rs`
# reads `YACR_FONT_BASE_URL`). The default is a path relative to the page, so a
# deployed build serves the third-party fonts from `$DIST/fonts/` (copied below)
# instead of reaching out to jsDelivr. Build a CDN-backed bundle with
# `YACR_FONT_BASE_URL=https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts/`
# and `WITH_FONTS=0`.
export YACR_FONT_BASE_URL="${YACR_FONT_BASE_URL:-fonts/}"
WITH_FONTS="${WITH_FONTS:-1}"

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
cp -R apps/app-web/web/host "$DIST/host"
mkdir -p "$DIST/ui-font"
cp crates/cad-ui-slint/fonts/YacrUI-Regular.otf crates/cad-ui-slint/fonts/OFL.txt "$DIST/ui-font/"
cp apps/app-web/web/style.css "$DIST/style.css"

# Copy the single-source-of-truth catalogs so the JS host localizes its own
# chrome from the same JSON the Rust `MessageSource` embeds (N01 host sync).
mkdir -p "$DIST/i18n"
cp crates/cad-ui-slint/i18n/zh-CN.json "$DIST/i18n/zh-CN.json"
cp crates/cad-ui-slint/i18n/en.json "$DIST/i18n/en.json"

# Self-hosted third-party fonts. Downloaded (never committed: see
# docs/fonts.md) so a deployed `web-dist/` is self-contained. `WITH_FONTS=0`
# keeps the CDN base and leaves the directory out.
if [ "$WITH_FONTS" = "1" ]; then
  "$ROOT/scripts/fetch-web-fonts.sh" "$DIST/fonts"
else
  echo "web build: WITH_FONTS=0, using font base $YACR_FONT_BASE_URL (no local copy)"
fi

# Report the module size so regressions are visible in CI logs.
WASM_SIZE=$(stat -c %s "$DIST/pkg/yacr_bg.wasm")
echo "web build: $DIST (wasm ${WASM_SIZE} bytes, profile ${PROFILE}, font base ${YACR_FONT_BASE_URL})"
echo "serve with: scripts/serve-web.py --directory $DIST"
