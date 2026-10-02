#!/usr/bin/env bash
# Download the third-party CAD font catalogue into a web distribution.
#
# The web host fetches `fonts.json` plus one file per font a drawing references
# from the compile-time base `YACR_FONT_BASE_URL` (default `fonts/`, i.e. next
# to the page). This script makes that directory self-contained so a deployed
# `web-dist/` does not depend on the jsDelivr CDN.
#
# Usage: scripts/fetch-web-fonts.sh [DEST]
#   DEST   target directory (default: <repo>/web-dist/fonts)
# Env:
#   FONT_BASE_URL           primary source (default: mlightcad/cad-data via jsDelivr)
#   FONT_FALLBACK_BASE_URL  second source tried when the primary fails
#                           (default: raw.githubusercontent.com; reachable from
#                           GitHub Actions, where jsDelivr throttles shared IPs)
#   FONT_RETRIES            attempts per source before falling back (default: 3)
#   FONTS                   optional space-separated subset (file, stem or catalog name)
#   FORCE                   set to 1 to re-download files that already exist
#
# Licensing: the fonts are third-party (Autodesk / Microsoft / open fonts) and
# are NOT committed to this repository; downloading and redistributing them is
# the deployer's responsibility. See docs/fonts.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-$ROOT/web-dist/fonts}"
export FONT_BASE_URL="${FONT_BASE_URL:-https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts}"
export FONT_FALLBACK_BASE_URL="${FONT_FALLBACK_BASE_URL:-https://raw.githubusercontent.com/mlightcad/cad-data/main/fonts}"
export FONT_RETRIES="${FONT_RETRIES:-3}"

command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }
[ -n "$FONT_BASE_URL" ] || { echo "FONT_BASE_URL is empty" >&2; exit 1; }

mkdir -p "$DEST"

python3 - "$FONT_BASE_URL" "$FONT_FALLBACK_BASE_URL" "$FONT_RETRIES" "$DEST" <<'PY'
import concurrent.futures
import json
import os
import sys
import time
import urllib.parse
import urllib.request

primary, fallback, retries, dest = (
    sys.argv[1].rstrip("/"),
    sys.argv[2].rstrip("/"),
    int(sys.argv[3]),
    sys.argv[4],
)
bases = [base for base in (primary, fallback) if base]
only = set(os.environ.get("FONTS", "").split())
force = os.environ.get("FORCE", "") == "1"


def fetch(path: str):
    """Fetch `path` from the first source that answers, retrying transient errors."""
    errors = []
    for base in bases:
        url = f"{base}/{urllib.parse.quote(path)}"
        # A normal User-Agent avoids WAFs that reject the default urllib one.
        request = urllib.request.Request(
            url,
            headers={"User-Agent": "yacr-build/1.0 (+https://github.com/zenglanmu/yacr)"},
        )
        for attempt in range(retries):
            try:
                with urllib.request.urlopen(request, timeout=60) as response:
                    return response.read(), base
            except Exception as exc:  # noqa: BLE001 - reported below, never hidden
                errors.append(f"{url}: {exc}")
                if attempt + 1 < retries:
                    time.sleep(0.5 * (2**attempt))
    raise RuntimeError("; ".join(errors))


catalog_bytes, catalog_base = fetch("fonts.json")
catalog = json.loads(catalog_bytes)
entries = [e for e in catalog if isinstance(e, dict) and str(e.get("file", "")).strip()]

if only:

    def wanted(entry):
        keys = {entry["file"], entry["file"].rsplit(".", 1)[0]}
        keys.update(entry.get("name") or [])
        return bool(keys & only)

    entries = [e for e in entries if wanted(e)]

with open(os.path.join(dest, "fonts.json"), "wb") as handle:
    handle.write(catalog_bytes)


def download(entry):
    name = entry["file"]
    target = os.path.join(dest, name)
    if os.path.exists(target) and not force:
        return name, os.path.getsize(target), "cached", None
    try:
        blob, base = fetch(name)
    except Exception as exc:  # surfaced, never folded into a success
        return name, 0, "FAIL", str(exc)
    with open(target, "wb") as handle:
        handle.write(blob)
    return name, len(blob), "ok", base


total = 0
failures = []
sources = {}
with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
    for name, size, status, detail in pool.map(download, entries):
        total += size
        if status == "FAIL":
            failures.append(f"{name}: {detail}")
            print(f"  {status:<24} {name}", file=sys.stderr)
        else:
            if status == "ok":
                sources[detail] = sources.get(detail, 0) + 1
            print(f"  {status:<24} {name} ({size} bytes)")
    if len(entries) == 0:
        print("  (no entries matched FONTS)", file=sys.stderr)

print(f"fonts: {len(entries)} files, {round(total / 1024 / 1024, 2)} MiB -> {dest}")
if sources:
    for base, count in sorted(sources.items()):
        print(f"font source: {base} ({count} files)")
else:
    print(f"font source: {catalog_base} (catalog only)")
if failures:
    print(f"fonts: {len(failures)} failed", file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    sys.exit(1)
PY
