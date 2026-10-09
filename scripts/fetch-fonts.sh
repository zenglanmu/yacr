#!/usr/bin/env bash
# Assemble the CAD font package into a target directory (platform-agnostic).
#
# This is the single implementation behind every host's bundled fonts. It
# downloads the `mlightcad/cad-data` catalogue plus the requested faces into
# `DEST`, merges the committed `fonts/` package (QCAD osifont) into the same
# directory and leaves `DEST/fonts.json` as the host catalog. Web, Android and
# desktop packaging are just consumers that pick a different DEST; there is no
# platform-specific logic here.
#
# Usage:
#   scripts/fetch-fonts.sh <DEST> [FILE|STEM|CATALOG-NAME ...]
#
# With no explicit faces the whole catalogue is fetched. A subset matches
# catalogue files by file name, file stem or catalog name; the same subset can
# be passed through the FONTS environment variable (space separated). The full
# `fonts.json` is always written so each host can still resolve any drawing
# font name.
#
# Env:
#   FONT_BASE_URL           primary source (default: mlightcad/cad-data via jsDelivr)
#   FONT_FALLBACK_BASE_URL  second source tried when the primary fails
#                           (default: raw.githubusercontent.com; reachable from
#                           GitHub Actions, where jsDelivr throttles shared IPs)
#   FONT_RETRIES            attempts per source before falling back (default: 3)
#   FONTS                   optional space-separated subset (same as positional args)
#   FORCE                   1 to re-download files that already exist
#
# Licensing: the mlightcad fonts are third-party and are NOT committed to this
# repository; downloading and redistributing them is the deployer's
# responsibility. The committed package merged here is `fonts/` (QCAD osifont,
# GPL-3 with font exception; provenance in `fonts/SOURCE.md`). See docs/fonts.md.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:?usage: scripts/fetch-fonts.sh <DEST> [FONTS...]}"
shift || true
export FONT_BASE_URL="${FONT_BASE_URL:-https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts}"
export FONT_FALLBACK_BASE_URL="${FONT_FALLBACK_BASE_URL:-https://raw.githubusercontent.com/mlightcad/cad-data/main/fonts}"
export FONT_RETRIES="${FONT_RETRIES:-3}"

if [ -z "${PYTHON:-}" ]; then
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1; then
      PYTHON="$candidate"
      break
    fi
  done
fi
[ -n "$PYTHON" ] || { echo "python3 (or python) is required" >&2; exit 1; }
[ -n "$FONT_BASE_URL" ] || { echo "FONT_BASE_URL is empty" >&2; exit 1; }

# Face subset: positional arguments win, then the FONTS env var; otherwise the
# whole catalogue. The python downloader reads FONTS.
if [ "$#" -gt 0 ]; then
  export FONTS="$*"
fi

mkdir -p "$DEST"

"$PYTHON" - "$FONT_BASE_URL" "$FONT_FALLBACK_BASE_URL" "$FONT_RETRIES" "$DEST" <<'PY'
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

# The full catalogue is always written so each host can resolve every drawing
# font name even when only a subset of faces was downloaded.
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

# Merge the committed font package (`fonts/`, currently QCAD osifont.ttf) into
# the target directory so it carries the default outline face even when the
# catalogue does not.
if [ -d "$ROOT/fonts" ]; then
  "$PYTHON" - "$DEST" "$ROOT/fonts" <<'PY'
import json, pathlib, sys
dest, extra = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
catalog_path = dest / "fonts.json"
extra_catalog = extra / "fonts.json"
if extra_catalog.exists():
    entries = []
    if catalog_path.exists():
        entries = json.loads(catalog_path.read_text())
    added = [e for e in json.loads(extra_catalog.read_text())
             if all(str(e.get("file")) != str(x.get("file")) for x in entries)]
    entries.extend(added)
    catalog_path.write_text(json.dumps(entries, indent=2) + "\n")
    for font in added:
        src = extra / str(font["file"])
        if src.exists():
            (dest / str(font["file"])).write_bytes(src.read_bytes())
            print(f"  merged {'':<22} {font['file']} ({src.stat().st_size} bytes)")
PY
fi