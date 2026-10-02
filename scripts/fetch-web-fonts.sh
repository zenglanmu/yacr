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
#   FONT_BASE_URL  source base (default: mlightcad/cad-data via jsDelivr)
#   FONTS          optional space-separated subset (file, stem or catalog name)
#   FORCE          set to 1 to re-download files that already exist
#
# Licensing: the fonts are third-party (Autodesk / Microsoft / open fonts) and
# are NOT committed to this repository; downloading and redistributing them is
# the deployer's responsibility. See docs/fonts.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-$ROOT/web-dist/fonts}"
export FONT_BASE_URL="${FONT_BASE_URL:-https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts}"

command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }

mkdir -p "$DEST"

python3 - "$FONT_BASE_URL" "$DEST" <<'PY'
import concurrent.futures
import json
import os
import sys
import urllib.parse
import urllib.request

base, dest = sys.argv[1].rstrip("/"), sys.argv[2]
only = set(os.environ.get("FONTS", "").split())
force = os.environ.get("FORCE", "") == "1"


def fetch(path: str) -> bytes:
    url = f"{base}/{urllib.parse.quote(path)}"
    with urllib.request.urlopen(url, timeout=60) as response:
        return response.read()


catalog_bytes = fetch("fonts.json")
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
        return name, os.path.getsize(target), "cached"
    try:
        blob = fetch(name)
    except Exception as exc:  # surfaced, never folded into a success
        return name, 0, f"FAIL {exc}"
    with open(target, "wb") as handle:
        handle.write(blob)
    return name, len(blob), "ok"


total = 0
failures = []
with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
    for name, size, status in pool.map(download, entries):
        total += size
        if status.startswith("FAIL"):
            failures.append(f"{name}: {status}")
            print(f"  {status:<24} {name}", file=sys.stderr)
        else:
            print(f"  {status:<24} {name} ({size} bytes)")

print(
    f"fonts: {len(entries)} files, {round(total / 1024 / 1024, 2)} MiB -> {dest}"
)
if failures:
    print(f"fonts: {len(failures)} failed", file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    sys.exit(1)
PY

echo "font source: ${FONT_BASE_URL} (copy served from $DEST)"
