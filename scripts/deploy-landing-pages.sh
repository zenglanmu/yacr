#!/usr/bin/env bash
# deploy-landing-pages.sh — publish the static yacr landing page to Cloudflare
# Pages by direct upload.
#
# The page lives in `apps/landing/` as plain static files (no build step). This
# script validates exactly the directory it is about to publish and then hands
# that same directory to wrangler. It never edits the bundle, never substitutes a
# placeholder, and never reports success it did not get from wrangler.
#
# Public entry point: `https://$PROJECT.pages.dev` (project `yacr` by default).
# This script does NOT manage a custom domain: attaching a domain is Cloudflare
# side configuration, not part of the upload contract.
#
# Prerequisites:
#   CLOUDFLARE_API_TOKEN   token with "Cloudflare Pages: Edit".
#   CLOUDFLARE_ACCOUNT_ID  account id.
#
# Usage: scripts/deploy-landing-pages.sh
# Env:
#   DIST               directory to publish (default: <repo>/apps/landing)
#   CF_PAGES_PROJECT   Pages project (default: yacr)
#   CF_PAGES_BRANCH    production branch (default: main)
#   WRANGLER_VERSION   npm version/dist-tag (default: 4)
#
# Honesty rules:
#   * every expected file is asserted non-empty before anything is uploaded;
#   * `downloads.json` must parse as JSON, and when its `cdnBase` is null/empty
#     the deployment still proceeds with the GitHub fallback download links, but
#     prints an explicit warning (a manual page deploy is allowed before the R2
#     mirror exists);
#   * a missing credential, a missing/invalid bundle or a wrangler failure is a
#     loud non-zero exit — there is no "probably fine" path;
#   * a successful upload is **deployment** evidence only. It is not runtime, GPU
#     or pixel evidence.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${DIST:-$ROOT/apps/landing}"
PROJECT="${CF_PAGES_PROJECT:-yacr}"
BRANCH="${CF_PAGES_BRANCH:-main}"
WRANGLER_VERSION="${WRANGLER_VERSION:-4}"
API="https://api.cloudflare.com/client/v4"

: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is not set}"
: "${CLOUDFLARE_ACCOUNT_ID:?CLOUDFLARE_ACCOUNT_ID is not set}"

for tool in curl jq python3 npx; do
  command -v "$tool" >/dev/null 2>&1 \
    || { echo "required tool not found: $tool" >&2; exit 1; }
done

[ -d "$DIST" ] || { echo "landing directory does not exist: $DIST" >&2; exit 1; }

# --- the static bundle contract --------------------------------------------

# Exactly these files, each non-empty: a stale or partial directory must never
# be published.
for file in index.html styles.css main.js downloads.json; do
  [ -s "$DIST/$file" ] \
    || { echo "missing or empty $DIST/$file; refusing to publish a partial site" >&2; exit 1; }
done

grep -q 'styles.css' "$DIST/index.html" \
  || { echo "$DIST/index.html does not reference styles.css" >&2; exit 1; }
grep -q 'main.js' "$DIST/index.html" \
  || { echo "$DIST/index.html does not reference main.js" >&2; exit 1; }

# Parse the JSON instead of guessing with grep: a malformed download manifest
# would leave the page with no working download links.
python3 -m json.tool "$DIST/downloads.json" >/dev/null \
  || { echo "$DIST/downloads.json is not valid JSON" >&2; exit 1; }

# At least one non-empty asset (the page's own images/icons, not a bundle).
[ -d "$DIST/assets" ] \
  || { echo "missing $DIST/assets; the page has no assets" >&2; exit 1; }
first_asset="$(find "$DIST/assets" -type f -size +0c -print -quit)"
if [ -z "$first_asset" ]; then
  echo "no non-empty file under $DIST/assets; refusing to publish" >&2
  exit 1
fi

CDN_BASE="$(python3 - "$DIST/downloads.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
value = data.get("cdnBase")
print(value if isinstance(value, str) else "")
PY
)"
if [ -z "$CDN_BASE" ]; then
  echo "WARNING: downloads.json cdnBase is null/empty; this deployment will carry" >&2
  echo "         GitHub fallback links only (the R2 mirror is not configured)." >&2
fi

if command -v node >/dev/null 2>&1; then
  node --check "$DIST/main.js" \
    || { echo "$DIST/main.js failed 'node --check'" >&2; exit 1; }
else
  echo "WARNING: node not found; skipping 'node --check $DIST/main.js'" >&2
fi

# --- idempotent Pages project creation -------------------------------------

AUTH_HEADER="Authorization: Bearer $CLOUDFLARE_API_TOKEN"
response="$(curl -sS -w '\n%{http_code}' \
  -H "$AUTH_HEADER" \
  "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects/$PROJECT")"
body="${response%$'\n'*}"
status="${response##*$'\n'}"

if [ "$status" = "404" ]; then
  echo "Pages project '$PROJECT' not found; creating it (production branch '$BRANCH')"
  response="$(curl -sS -w '\n%{http_code}' \
    -X POST \
    -H "$AUTH_HEADER" \
    -H 'Content-Type: application/json' \
    --data "{\"name\":\"$PROJECT\",\"production_branch\":\"$BRANCH\"}" \
    "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects")"
  body="${response%$'\n'*}"
  status="${response##*$'\n'}"
  if [ "$status" != "200" ] && [ "$status" != "201" ]; then
    echo "failed to create Pages project '$PROJECT' (HTTP $status):" >&2
    printf '%s\n' "$body" >&2
    exit 1
  fi
  printf '%s' "$body" | jq -e '.success' >/dev/null \
    || { echo "Cloudflare rejected the Pages project creation:" >&2; printf '%s\n' "$body" >&2; exit 1; }
elif [ "$status" = "200" ]; then
  printf '%s' "$body" | jq -e '.success' >/dev/null \
    || { echo "Pages project lookup failed:" >&2; printf '%s\n' "$body" >&2; exit 1; }
else
  echo "failed to look up Pages project '$PROJECT' (HTTP $status):" >&2
  printf '%s\n' "$body" >&2
  exit 1
fi

# --- deploy ----------------------------------------------------------------

# No pipe: a wrangler failure must keep its exit code and fail this script.
npx --yes "wrangler@$WRANGLER_VERSION" pages deploy "$DIST" \
  --project-name "$PROJECT" \
  --branch "$BRANCH" \
  --commit-dirty=true

echo "deployed the landing page to https://$PROJECT.pages.dev"
echo "NOTE: a successful upload is deployment evidence only; it is not runtime,"
echo "      GPU or pixel acceptance."
