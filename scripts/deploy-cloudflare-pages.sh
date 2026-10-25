#!/usr/bin/env bash
# deploy-cloudflare-pages.sh — publish `apps/app-web/dist` (the wasm CAD session
# shell) to Cloudflare Pages by direct upload.
#
# The bundle is built by `scripts/build-web.sh` (ADR 0024). This script only
# validates what it is about to publish and uploads it — it never edits the
# bundle, never substitutes a placeholder and never reports a success it did not
# get from wrangler.
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

# The bundle contract from `scripts/build-web.sh`: exactly these files, each
# non-empty. A stale or partial directory must never be published.
required=(
  index.html
  main.js
  worker.js
  style.css
  pkg/app_web.js
  pkg/app_web_bg.wasm
)
for artifact in "${required[@]}"; do
  [ -s "$DIST/$artifact" ] || { echo "missing $DIST/$artifact; run scripts/build-web.sh" >&2; exit 1; }
done
# The shell must load the module this bundle ships, otherwise the deployment is
# a blank page nobody notices.
grep -qF './pkg/app_web.js' "$DIST/main.js" \
  || { echo "$DIST/main.js does not import ./pkg/app_web.js; refusing to publish a blank page" >&2; exit 1; }

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
