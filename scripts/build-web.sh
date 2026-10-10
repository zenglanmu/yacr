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

# App icon set and web app manifest, from the single source of truth in
# `assets/` (never regenerated here), so a deployed build is self-contained.
mkdir -p "$DIST/assets/icons"
cp "$ROOT/assets/yacr-icon.svg" "$DIST/assets/yacr-icon.svg"
cp "$ROOT/assets/icons/yacr-256.png" "$ROOT/assets/icons/yacr-512.png" "$DIST/assets/icons/"
cp apps/app-web/web/manifest.webmanifest "$DIST/manifest.webmanifest"

# Build stamp = content hash of the wasm. main.js, index.html and the generated
# pkg/yacr.js all request the wasm/glue under this stamp, so a redeploy can never
# link a stale cached pkg/yacr.js against a newer pkg/yacr_bg.wasm (the classic
# "import object field ... is not a Function" LinkError).
BUILD_STAMP="$(python3 - "$DIST/pkg/yacr_bg.wasm" <<'PY'
import hashlib, sys
print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest()[:16])
PY
)"
python3 - "$DIST/index.html" "$BUILD_STAMP" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
if "__YACR_BUILD__" not in text:
    raise SystemExit("index.html is missing the __YACR_BUILD__ placeholder")
path.write_text(text.replace("__YACR_BUILD__", sys.argv[2]), encoding="utf-8")
PY
echo "web build: build stamp $BUILD_STAMP"

# Copy the single-source-of-truth catalogs so the JS host localizes its own
# chrome from the same JSON the Rust `MessageSource` embeds (N01 host sync).
mkdir -p "$DIST/i18n"
cp crates/cad-ui-slint/i18n/zh-CN.json "$DIST/i18n/zh-CN.json"
cp crates/cad-ui-slint/i18n/en.json "$DIST/i18n/en.json"

# Self-hosted third-party fonts. Downloaded (never committed: see
# docs/fonts.md) so a deployed `web-dist/` is self-contained, and the committed
# `fonts/` package (QCAD osifont) is merged in. `WITH_FONTS=0` keeps the CDN
# base and leaves the directory out.
if [ "$WITH_FONTS" = "1" ]; then
  "$ROOT/scripts/fetch-fonts.sh" "$DIST/fonts"
else
  echo "web build: WITH_FONTS=0, using font base $YACR_FONT_BASE_URL (no local copy)"
fi

# Report the module size so regressions are visible in CI logs.
WASM_SIZE=$(stat -c %s "$DIST/pkg/yacr_bg.wasm")
echo "web build: $DIST (wasm ${WASM_SIZE} bytes, profile ${PROFILE}, font base ${YACR_FONT_BASE_URL})"
echo "serve with: scripts/serve-web.py --directory $DIST"
