#!/usr/bin/env bash
# upload-r2-downloads.sh — mirror a GitHub release's download assets to a
# Cloudflare R2 public bucket and regenerate the landing page download manifest.
#
# Flow:
#   1. resolve the release tag (`latest` -> `gh release view`);
#   2. download every asset with `gh release download`;
#   3. verify each expected artifact is non-empty and passes `sha256sum -c`;
#   4. create the bucket idempotently and enable its managed r2.dev domain;
#   5. upload every artifact to `releases/<tag>/` with wrangler v4 (`--remote`!);
#   6. HEAD the four primary packages through r2.dev;
#   7. atomically rewrite `<landing>/downloads.json` with the CDN base and hashes.
#
# Prerequisites:
#   CLOUDFLARE_API_TOKEN   token with "Workers R2 Storage Write".
#   CLOUDFLARE_ACCOUNT_ID  account id.
#   gh                     authenticated (CI passes GH_TOKEN); used to fetch assets.
#
# Usage: scripts/upload-r2-downloads.sh
# Env:
#   R2_BUCKET          bucket name (default: yacr-downloads)
#   RELEASE_TAG        release tag, or `latest` (default: latest)
#   GITHUB_REPOSITORY  owner/repo (default: zenglanmu/yacr)
#   LANDING_DIR        directory holding downloads.json (default: <repo>/apps/landing)
#   ASSETS_DIR         local asset staging directory (default:
#                      <repo>/target/landing-downloads/<tag>)
#   WRANGLER_VERSION   npm version/dist-tag (default: 4)
#   R2_PUBLIC_BASE     public download base written to `cdnBase`
#                      (default: https://dl.snakeheartgo.top). Set it to the
#                      literal `managed` to use the rate-limited r2.dev domain.
#
# Honesty rules:
#   * the preferred host is the bucket's custom domain (`dl.snakeheartgo.top`),
#     which can carry Cloudflare Cache Rules; the managed r2.dev domain is a
#     test / low-cost fallback with rate limits and no production SLA;
#   * the chosen base is only written after a real HEAD probe of every package;
#     a missing/unreachable custom domain is a loud non-zero exit, never a
#     silent downgrade to r2.dev;
#   * a failed checksum, upload or HEAD probe is a loud non-zero exit;
#   * tokens are never printed; a successful upload is distribution evidence only
#     (not runtime, GPU, device or pixel acceptance).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUCKET="${R2_BUCKET:-yacr-downloads}"
TAG="${RELEASE_TAG:-latest}"
REPO="${GITHUB_REPOSITORY:-zenglanmu/yacr}"
LANDING_DIR="${LANDING_DIR:-$ROOT/apps/landing}"
ASSETS_DIR_OVERRIDE="${ASSETS_DIR:-}"
WRANGLER_VERSION="${WRANGLER_VERSION:-4}"
PUBLIC_BASE="${R2_PUBLIC_BASE:-https://dl.snakeheartgo.top}"
API="https://api.cloudflare.com/client/v4"

die() {
  echo "$*" >&2
  exit 1
}

: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is not set}"
: "${CLOUDFLARE_ACCOUNT_ID:?CLOUDFLARE_ACCOUNT_ID is not set}"

for tool in curl jq gh sha256sum npx python3; do
  command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done

AUTH_HEADER="Authorization: Bearer $CLOUDFLARE_API_TOKEN"

if [ "$TAG" = "latest" ]; then
  TAG="$(gh release view --repo "$REPO" --json tagName -q .tagName)"
  [ -n "$TAG" ] || die "could not resolve the latest release tag for $REPO"
fi
VER="${TAG#v}"
[ -n "$VER" ] || die "release tag '$TAG' has no version part"

ASSETS_DIR="${ASSETS_DIR_OVERRIDE:-$ROOT/target/landing-downloads/$TAG}"

LINUX="yacr-${VER}-linux-x86_64.flatpak"
WINDOWS="yacr-${VER}-windows-x86_64.zip"
MACOS="yacr-${VER}-macos-universal.tar.gz"
ANDROID="app-universal-release.apk"

required=(
  "$LINUX" "$LINUX.sha256" "$LINUX.gpg"
  "$WINDOWS" "$WINDOWS.sha256"
  "$MACOS" "$MACOS.sha256"
  "$ANDROID" "$ANDROID.sha256"
  "SIGNING.txt"
)

# --- fetch -----------------------------------------------------------------

echo "downloading release '$TAG' from $REPO into $ASSETS_DIR"
rm -rf "$ASSETS_DIR"
mkdir -p "$ASSETS_DIR"
gh release download "$TAG" --repo "$REPO" --dir "$ASSETS_DIR"

for file in "${required[@]}"; do
  [ -s "$ASSETS_DIR/$file" ] \
    || die "release '$TAG' is missing a non-empty asset: $file"
done

# Every checksum file the release ships must verify against its sibling.
mapfile -t SUM_FILES < <(find "$ASSETS_DIR" -maxdepth 1 -type f -name '*.sha256' | sort)
[ "${#SUM_FILES[@]}" -gt 0 ] || die "release '$TAG' ships no .sha256 file"
for sum in "${SUM_FILES[@]}"; do
  ( cd "$ASSETS_DIR" && sha256sum -c "$(basename "$sum")" )
done

# --- bucket + managed r2.dev domain ---------------------------------------

response="$(curl -sS -w '\n%{http_code}' \
  -X POST \
  -H "$AUTH_HEADER" \
  -H 'Content-Type: application/json' \
  --data "{\"name\":\"$BUCKET\"}" \
  "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets")"
body="${response%$'\n'*}"
status="${response##*$'\n'}"
if [ "$status" != "200" ] && [ "$status" != "409" ]; then
  error_code="$(printf '%s' "$body" | jq -r '.errors[0].code // empty' 2>/dev/null || true)"
  if [ "$error_code" = "10042" ]; then
    die "R2 is not enabled for this account (账户未开通 R2); enable R2 in the dashboard first"
  fi
  if [ "$error_code" != "10073" ]; then
    echo "failed to create R2 bucket '$BUCKET' (HTTP $status):" >&2
    printf '%s\n' "$body" >&2
    exit 1
  fi
fi

response="$(curl -sS -w '\n%{http_code}' \
  -X PUT \
  -H "$AUTH_HEADER" \
  -H 'Content-Type: application/json' \
  --data '{"enabled":true}' \
  "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets/$BUCKET/domains/managed")"
body="${response%$'\n'*}"
status="${response##*$'\n'}"
if [ "$status" != "200" ]; then
  echo "failed to enable the r2.dev public domain for '$BUCKET' (HTTP $status):" >&2
  printf '%s\n' "$body" >&2
  exit 1
fi
printf '%s' "$body" | jq -e '.success and .result.enabled' >/dev/null \
  || { echo "r2.dev managed domain was not enabled:" >&2; printf '%s\n' "$body" >&2; exit 1; }
DOMAIN="$(printf '%s' "$body" | jq -r '.result.domain')"
[ -n "$DOMAIN" ] || die "Cloudflare did not return an r2.dev domain"

# --- upload ----------------------------------------------------------------

mime_for() {
  case "$1" in
    *.flatpak) echo "application/octet-stream" ;;
    *.zip) echo "application/zip" ;;
    *.tar.gz) echo "application/gzip" ;;
    *.apk) echo "application/vnd.android.package-archive" ;;
    *.sha256 | *.txt) echo "text/plain; charset=utf-8" ;;
    *.gpg) echo "application/pgp-signature" ;;
    *) echo "application/octet-stream" ;;
  esac
}

for file in "${required[@]}"; do
  mime="$(mime_for "$file")"
  echo "uploading releases/$TAG/$file ($mime)"
  npx --yes "wrangler@$WRANGLER_VERSION" r2 object put "$BUCKET/releases/$TAG/$file" \
    --file="$ASSETS_DIR/$file" \
    --content-type="$mime" \
    --cache-control="public, max-age=31536000, immutable" \
    --remote
done

# The preferred public base is the bucket's custom domain (it can carry Cache
# Rules). The managed r2.dev host is only an explicit opt-in fallback.
if [ "$PUBLIC_BASE" = "managed" ]; then
  CDN_BASE="https://$DOMAIN/releases/$TAG"
  echo "WARNING: R2_PUBLIC_BASE=managed; using the rate-limited r2.dev domain" >&2
else
  CDN_BASE="$PUBLIC_BASE/releases/$TAG"
fi

# DNS/propagation can lag a few seconds; retry the probe before giving up.
for file in "$LINUX" "$WINDOWS" "$MACOS" "$ANDROID"; do
  echo "verifying $CDN_BASE/$file"
  curl -fsS -I --retry 10 --retry-delay 3 --retry-all-errors \
    "$CDN_BASE/$file" >/dev/null \
    || die "HEAD probe failed for $CDN_BASE/$file; bind the custom download domain in Cloudflare (R2 -> bucket -> Custom Domains) and add the Cache Rule, or set R2_PUBLIC_BASE=managed to fall back to r2.dev"
done

# --- manifest --------------------------------------------------------------

mkdir -p "$LANDING_DIR"
tmp="$(mktemp "$LANDING_DIR/.downloads.json.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
GITHUB_BASE="https://github.com/$REPO/releases/download/$TAG/"

python3 - "$ASSETS_DIR" "$tmp" "$TAG" "$CDN_BASE" "$GITHUB_BASE" \
  "$LINUX" "$WINDOWS" "$MACOS" "$ANDROID" <<'PY'
import json
import os
import sys

assets_dir, out, tag, cdn_base, github_base, *files = sys.argv[1:]
labels = ("linux", "windows", "macos", "android")


def entry(name: str) -> dict:
    path = os.path.join(assets_dir, name)
    with open(path + ".sha256", encoding="utf-8") as handle:
        sha256 = handle.readline().split()[0]
    if len(sha256) != 64:
        raise SystemExit(f"{path}.sha256 does not start with a SHA-256 digest")
    return {"file": name, "size": os.path.getsize(path), "sha256": sha256}


payload = {
    "release": tag,
    "cdnBase": cdn_base,
    "githubBase": github_base,
    "assets": {label: entry(name) for label, name in zip(labels, files)},
}
with open(out, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
PY
mv "$tmp" "$LANDING_DIR/downloads.json"
trap - EXIT

echo "CDN_BASE=$CDN_BASE"
echo "wrote $LANDING_DIR/downloads.json"
echo "NOTE: the custom domain is only a cache front; a successful upload is"
echo "      distribution evidence only, not runtime/GPU/device/pixel acceptance."
