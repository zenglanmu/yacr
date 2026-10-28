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
# Prerequisites:
#   CLOUDFLARE_API_TOKEN   token with "Cloudflare Pages: Edit"; add "Zone: DNS:
#                          Edit" for the target zone to let Pages create the
#                          custom-domain DNS record automatically.
#   CLOUDFLARE_ACCOUNT_ID  account id.
#
# Usage: scripts/deploy-cloudflare-pages.sh
# Env:
#   CF_PAGES_PROJECT   Pages project (default: yacr-examples)
#   CF_PAGES_BRANCH    production branch (default: main)
#   CF_PAGES_DOMAIN    optional custom domain to attach, e.g. yacr.example.com
#   WRANGLER_VERSION   npm version/dist-tag (default: 4)
#   DIST               bundle to deploy (default: <repo>/apps/app-web/dist)
#   BUNDLE_KIND        which bundle contract `DIST` must satisfy (default: harness)
#                        harness — `scripts/build-web.sh`: index.html, main.js,
#                                  worker.js, style.css, pkg/app_web.js|wasm
#                        ui      — `scripts/build-web-ui.sh`: index.html,
#                                  assets/*.js, canvas/app_web.js|wasm, bootstrap
#   SKIP_BUILD=1       deploy the existing bundle without rebuilding it
#
# Honesty rules:
#   * every expected artifact is asserted non-empty before anything is uploaded,
#     and the exact directory that passed those checks is the one deployed;
#   * a missing credential, a missing bundle or a wrangler failure is a loud
#     non-zero exit — there is no "probably fine" path;
#   * a successful upload is **deployment** evidence only. It is not runtime,
#     GPU or pixel evidence: CI's browser smoke runs separately and under
#     software rendering (see `.github/workflows/build.yml`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${DIST:-$ROOT/apps/app-web/dist}"
BUNDLE_KIND="${BUNDLE_KIND:-harness}"
PROJECT="${CF_PAGES_PROJECT:-yacr-examples}"
BRANCH="${CF_PAGES_BRANCH:-main}"
DOMAIN="${CF_PAGES_DOMAIN:-}"
WRANGLER_VERSION="${WRANGLER_VERSION:-4}"
API="https://api.cloudflare.com/client/v4"

: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is not set}"
: "${CLOUDFLARE_ACCOUNT_ID:?CLOUDFLARE_ACCOUNT_ID is not set}"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  "$ROOT/scripts/build-web.sh"
fi

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

if [ -n "$DOMAIN" ]; then
  echo "attaching custom domain '$DOMAIN'"
  response="$(curl -sS "${auth[@]}" -H "Content-Type: application/json" -X POST \
    --data "{\"name\":\"$DOMAIN\"}" \
    "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects/$PROJECT/domains")"
  python3 - "$response" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
if data.get("success"):
    result = data.get("result") or {}
    print(f"  domain: {result.get('name')} status={result.get('status', 'pending')}")
    if result.get("validation_data"):
        print(f"  validation: {result['validation_data']}")
else:
    print("  attach failed:", data.get("errors"), file=sys.stderr)
    for error in data.get("errors") or []:
        message = str(error.get("message", "")).lower()
        if "already added" in message or "already exists" in message:
            sys.exit(0)
    sys.exit(1)
PY
  echo "  note: a domain whose zone is outside this account stays 'pending' until"
  echo "  a CNAME to $PROJECT.pages.dev is added at the DNS provider."
fi

echo "done: https://$PROJECT.pages.dev"
echo "note: an upload is deployment evidence only, not runtime/GPU/pixel evidence."
