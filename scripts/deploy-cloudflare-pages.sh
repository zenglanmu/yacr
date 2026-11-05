#!/usr/bin/env bash
# deploy-cloudflare-pages.sh — publish a built browser bundle to Cloudflare
# Pages by direct upload.
#
# Two bundles are supported (`BUNDLE_KIND`):
#   harness — `apps/app-web/dist`, the standalone vanilla page built by
#             `scripts/build-web.sh` (ADR 0024; what CI's `web-smoke` drives);
#   ui      — `apps/app-web/dist-ui`, the shared React chrome built by
#             `scripts/build-web-ui.sh` with `__YACR_RUNTIME__=web` (the mobile
#             responsive site, and what the `yacr-examples` project serves).
#
# This script only validates what it is about to publish and uploads it — it
# never edits the bundle, never substitutes a placeholder and never reports a
# success it did not get from wrangler.
#
# The published entry point is the project's own `pages.dev` hostname
# (`https://$CF_PAGES_PROJECT.pages.dev`). This script never attaches, detaches or
# otherwise manages a custom domain: a custom-domain DNS record is Cloudflare-side
# configuration, not part of the upload contract.
#
# Prerequisites:
#   CLOUDFLARE_API_TOKEN   token with "Cloudflare Pages: Edit".
#   CLOUDFLARE_ACCOUNT_ID  account id.
#
# Usage: scripts/deploy-cloudflare-pages.sh
# Env:
#   CF_PAGES_PROJECT   Pages project (default: yacr-examples)
#   CF_PAGES_BRANCH    production branch (default: main)
#   WRANGLER_VERSION   npm version/dist-tag (default: 4)
#   DIST               bundle to deploy (default: <repo>/apps/app-web/dist)
#   BUNDLE_KIND        which bundle contract `DIST` must satisfy (default: harness)
#                        harness — `scripts/build-web.sh`: index.html, main.js,
#                                  worker.js, style.css, pkg/app_web.js|wasm
#                        ui      — `scripts/build-web-ui.sh`: index.html,
#                                  assets/*.js, canvas/app_web.js|wasm, bootstrap,
#                                  plugins/tch.js
#   SKIP_BUILD=1       deploy the existing bundle without rebuilding it
#   SKIP_FONTS=1       reuse an existing $DIST/fonts instead of downloading it
#
# Fonts are **not** part of either build artifact: the page fetches the faces it
# needs lazily, after a document opens, from same-origin `fonts/`. This script
# therefore assembles the full CDN package into `$DIST/fonts` with
# `scripts/fetch-fonts.sh` (mlightcad catalogue + faces, plus the committed
# osifont/Noto) before anything is validated or uploaded — for both bundle kinds,
# so the site root always serves `fonts/`.
#
# Honesty rules:
#   * every expected artifact is asserted non-empty before anything is uploaded,
#     and the exact directory that passed those checks is the one deployed;
#   * a missing credential, a missing bundle, a failed font download or a wrangler
#     failure is a loud non-zero exit — there is no "probably fine" path;
#   * a successful upload is **deployment** evidence only. It is not runtime,
#     GPU or pixel evidence: CI's browser smoke runs separately and under
#     software rendering (see `.github/workflows/build.yml`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${DIST:-$ROOT/apps/app-web/dist}"
BUNDLE_KIND="${BUNDLE_KIND:-harness}"
PROJECT="${CF_PAGES_PROJECT:-yacr-examples}"
BRANCH="${CF_PAGES_BRANCH:-main}"
WRANGLER_VERSION="${WRANGLER_VERSION:-4}"
API="https://api.cloudflare.com/client/v4"

: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is not set}"
: "${CLOUDFLARE_ACCOUNT_ID:?CLOUDFLARE_ACCOUNT_ID is not set}"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  "$ROOT/scripts/build-web.sh"
fi

# --- the CDN font package --------------------------------------------------

# Neither bundle carries fonts: the page loads them lazily from same-origin
# `fonts/` after a document opens. Assemble the **full** package (mlightcad
# catalogue + every face, merged with the committed osifont/Noto) at the site
# root, so both bundle kinds publish the same `fonts/` URL space.
FONT_DEST="$DIST/fonts"
NOTO_FILE="NotoSansCJKsc-Regular.otf"
if [ "${SKIP_FONTS:-0}" = "1" ]; then
  echo "SKIP_FONTS=1: reusing the existing $FONT_DEST (no download)"
else
  echo "assembling the font package into $FONT_DEST (scripts/fetch-fonts.sh)"
  "$ROOT/scripts/fetch-fonts.sh" "$FONT_DEST" \
    || { echo "font package assembly failed; refusing to publish without fonts/" >&2; exit 1; }
fi
[ -s "$FONT_DEST/fonts.json" ] \
  || { echo "missing or empty $FONT_DEST/fonts.json; the page cannot resolve any face" >&2; exit 1; }
[ -s "$FONT_DEST/$NOTO_FILE" ] \
  || { echo "missing or empty $FONT_DEST/$NOTO_FILE; the default CJK fallback would be absent" >&2; exit 1; }
[ -s "$FONT_DEST/NotoSansCJK-OFL.txt" ] \
  || { echo "missing or empty $FONT_DEST/NotoSansCJK-OFL.txt; the OFL text must ship with the face" >&2; exit 1; }

# The two bundle contracts. Exactly these files, each non-empty: a stale or
# partial directory must never be published.
#   harness — `scripts/build-web.sh`: the standalone vanilla page (+ Worker
#             isolation) that CI's `web-smoke` drives.
#   ui      — `scripts/build-web-ui.sh`: the shared React chrome compiled with
#             `__YACR_RUNTIME__=web`; this is what the Pages site serves.
required=(
  index.html
  main.js
  worker.js
  style.css
  pkg/app_web.js
  pkg/app_web_bg.wasm
)
ui_required=(
  index.html
  canvas/app_web.js
  canvas/app_web_bg.wasm
  canvas/bootstrap.js
  plugins/tch.js
)
case "$BUNDLE_KIND" in
  harness) artifacts=("${required[@]}") ;;
  ui) artifacts=("${ui_required[@]}") ;;
  *)
    echo "unknown BUNDLE_KIND '$BUNDLE_KIND' (expected 'harness' or 'ui')" >&2
    exit 1
    ;;
esac
for artifact in "${artifacts[@]}"; do
  [ -s "$DIST/$artifact" ] || {
    echo "missing $DIST/$artifact (BUNDLE_KIND=$BUNDLE_KIND); run the matching build script" >&2
    exit 1
  }
done
# The shell must load the module this bundle ships, otherwise the deployment is
# a blank page nobody notices. Each kind has its own loader.
if [ "$BUNDLE_KIND" = "ui" ]; then
  grep -qF './assets/' "$DIST/index.html" \
    || { echo "$DIST/index.html references no ./assets/ bundle; refusing to publish a blank page" >&2; exit 1; }
  grep -qF './app_web.js' "$DIST/canvas/bootstrap.js" \
    || { echo "$DIST/canvas/bootstrap.js does not import ./app_web.js; refusing to publish a blank page" >&2; exit 1; }
  shopt -s nullglob
  ui_assets=("$DIST"/assets/*.js)
  shopt -u nullglob
  [ "${#ui_assets[@]}" -gt 0 ] \
    || { echo "$DIST/assets has no emitted .js bundle; refusing to publish a blank page" >&2; exit 1; }
else
  grep -qF './pkg/app_web.js' "$DIST/main.js" \
    || { echo "$DIST/main.js does not import ./pkg/app_web.js; refusing to publish a blank page" >&2; exit 1; }
fi

auth=(-H "Authorization: Bearer $CLOUDFLARE_API_TOKEN")

# Wrangler prompts before creating a project; create it deterministically first.
if ! curl -fsS "${auth[@]}" "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects/$PROJECT" >/dev/null 2>&1; then
  echo "creating Pages project '$PROJECT'"
  curl -fsS "${auth[@]}" -H "Content-Type: application/json" -X POST \
    --data "{\"name\":\"$PROJECT\",\"production_branch\":\"$BRANCH\"}" \
    "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects" >/dev/null
fi

echo "deploying $DIST to Pages project '$PROJECT' (branch $BRANCH)"
npx --yes "wrangler@$WRANGLER_VERSION" pages deploy "$DIST" \
  --project-name "$PROJECT" \
  --branch "$BRANCH" \
  --commit-dirty=true

echo "done: https://$PROJECT.pages.dev"
echo "note: an upload is deployment evidence only, not runtime/GPU/pixel evidence."
