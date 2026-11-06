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
#   FORCE                   1 to re-download files that already exist (and refresh
#                           the cache entry)
#   FONT_CACHE_DIR          shared download cache (default:
#                           ${XDG_CACHE_HOME:-$HOME/.cache}/yacr/fonts). A non-empty
#                           cached face is copied instead of re-downloaded, so
#                           repeated packaging runs (and the two Web bundles) reuse
#                           one download. An explicitly empty value disables it.
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
# `-` (not `:-`): an explicitly empty FONT_CACHE_DIR disables the shared cache.
FONT_CACHE_DIR="${FONT_CACHE_DIR-${XDG_CACHE_HOME:-$HOME/.cache}/yacr/fonts}"
export FONT_CACHE_DIR

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

"$PYTHON" - "$FONT_BASE_URL" "$FONT_FALLBACK_BASE_URL" "$FONT_RETRIES" "$DEST" "$FONT_CACHE_DIR" <<'PY'
import concurrent.futures
import json
import os
import sys
import tempfile
import time
import urllib.parse
import urllib.request

primary, fallback, retries, dest, cache = (
    sys.argv[1].rstrip("/"),
    sys.argv[2].rstrip("/"),
    int(sys.argv[3]),
    sys.argv[4],
    sys.argv[5].rstrip("/") if len(sys.argv) > 5 else "",
)
bases = [base for base in (primary, fallback) if base]
only = set(os.environ.get("FONTS", "").split())
force = os.environ.get("FORCE", "") == "1"
if cache:
    os.makedirs(cache, exist_ok=True)


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


def write_atomic(path: str, blob: bytes) -> None:
    """Write through a temp file: a killed build never leaves a truncated face."""
    directory = os.path.dirname(path) or "."
    handle_fd, temporary = tempfile.mkstemp(
        dir=directory, prefix=".yacr-font-", suffix=".tmp"
    )
    try:
        with os.fdopen(handle_fd, "wb") as handle:
            handle.write(blob)
        os.replace(temporary, path)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


def safe_name(name: str) -> bool:
    """A catalogue entry must be a bare file name (never a path)."""
    return bool(name) and "/" not in name and "\\" not in name and name not in (".", "..")


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
    if not safe_name(name):
        return name, 0, "FAIL", f"invalid catalogue file name: {name!r}"
    target = os.path.join(dest, name)
    cached = os.path.join(cache, name) if cache else None
    if os.path.exists(target) and not force:
        return name, os.path.getsize(target), "present", None
    if cached and not force:
        try:
            with open(cached, "rb") as handle:
                blob = handle.read()
        except OSError:
            blob = b""
        if blob:
            write_atomic(target, blob)
            return name, len(blob), "cache", cached
    try:
        blob, base = fetch(name)
    except Exception as exc:  # surfaced, never folded into a success
        return name, 0, "FAIL", str(exc)
    if cached:
        # Refresh the shared cache too, so the next run/destination is offline.
        write_atomic(cached, blob)
    write_atomic(target, blob)
    return name, len(blob), "ok", base


total = 0
failures = []
sources = {}
from_cache = 0
already_staged = 0
downloaded = 0
with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
    for name, size, status, detail in pool.map(download, entries):
        total += size
        if status == "FAIL":
            failures.append(f"{name}: {detail}")
            print(f"  {status:<24} {name}", file=sys.stderr)
        else:
            if status == "cache":
                from_cache += 1
            elif status == "ok":
                downloaded += 1
                sources[detail] = sources.get(detail, 0) + 1
            else:
                already_staged += 1
            print(f"  {status:<24} {name} ({size} bytes)")
    if len(entries) == 0:
        print("  (no entries matched FONTS)", file=sys.stderr)

print(f"fonts: {len(entries)} files, {round(total / 1024 / 1024, 2)} MiB -> {dest}")
if cache:
    print(
        f"font cache: {cache} "
        f"({from_cache} reused, {already_staged} already staged, {downloaded} downloaded)"
    )
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

# Merge the committed font package (`fonts/`, QCAD osifont + Noto Sans CJK SC)
# into the target directory so it carries the default faces and their licence
# texts even when the catalogue does not.
if [ -d "$ROOT/fonts" ]; then
  "$PYTHON" - "$DEST" "$ROOT/fonts" <<'PY'
import json, pathlib, sys
dest, extra = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
catalog_path = dest / "fonts.json"
extra_catalog = extra / "fonts.json"
if extra_catalog.exists():
    entries = []
    if catalog_path.exists():
        # The catalogue is UTF-8; `read_text()` would otherwise use the platform
        # default (cp1252 on Windows) and fail on non-ASCII font names.
        entries = json.loads(catalog_path.read_text(encoding="utf-8"))
    added = [e for e in json.loads(extra_catalog.read_text(encoding="utf-8"))
             if all(str(e.get("file")) != str(x.get("file")) for x in entries)]
    entries.extend(added)
    catalog_path.write_text(json.dumps(entries, indent=2) + "\n", encoding="utf-8")
    for font in added:
        src = extra / str(font["file"])
        if src.exists():
            (dest / str(font["file"])).write_bytes(src.read_bytes())
            print(f"  merged {'':<22} {font['file']} ({src.stat().st_size} bytes)")
# The OFL/GPL texts are part of the redistribution terms, not optional payload:
# every assembled package (Windows/macOS release, Web CDN, local staging) must
# carry them next to the faces even though they are not catalogue entries.
for name in ("NotoSansCJK-OFL.txt", "COPYING.GPL-3", "SOURCE.md", "README.md"):
    src = extra / name
    if src.exists():
        (dest / name).write_bytes(src.read_bytes())
        print(f"  merged {'':<22} {name} ({src.stat().st_size} bytes)")
PY
fi