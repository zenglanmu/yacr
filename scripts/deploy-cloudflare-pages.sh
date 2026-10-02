#!/usr/bin/env bash
# Build the yacr web bundle and publish it to Cloudflare Pages (direct upload).
#
# The bundle is self-contained: `build-web.sh` bakes in a same-origin font base
# and copies the third-party font catalogue into `DIST/fonts/` (see
# docs/fonts.md), so the page does not depend on the jsDelivr CDN.
#
# Prerequisites:
#   CLOUDFLARE_API_TOKEN   token with "Cloudflare Pages: Edit"; add "Zone: DNS:
#                          Edit" for the target zone to let Pages create the
#                          custom-domain DNS record automatically.
#   CLOUDFLARE_ACCOUNT_ID  account id (also read from the environment).
#
# Usage: scripts/deploy-cloudflare-pages.sh
# Env:
#   CF_PAGES_PROJECT   Pages project (default: yacr-examples)
#   CF_PAGES_BRANCH    production branch (default: main)
#   CF_PAGES_DOMAIN    custom domain to attach, e.g. yacr-examples.snakeheart.top
#   WRANGLER_VERSION   npm version/dist-tag (default: 4)
#   DIST               bundle to deploy (default: <repo>/web-dist)
#   SKIP_BUILD=1       deploy the existing bundle without rebuilding
#
# This publishes to a public URL: only run it when the artefacts are cleared for
# release (spec/audit: web-dist is a build output, not an automatic deploy).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${DIST:-$ROOT/web-dist}"
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

for required in index.html main.js style.css pkg/yacr.js pkg/yacr_bg.wasm; do
  [ -s "$DIST/$required" ] || { echo "missing $DIST/$required; run scripts/build-web.sh" >&2; exit 1; }
done

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
        if "already exists" in str(error.get("message", "")).lower():
            sys.exit(0)
    sys.exit(1)
PY
  echo "  note: a domain whose zone is outside this account stays 'pending' until"
  echo "  a CNAME to $PROJECT.pages.dev is added at the DNS provider."
fi

echo "done: https://$PROJECT.pages.dev"
