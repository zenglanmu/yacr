#!/usr/bin/env bash
# render-smoke.sh -- headless render smoke test for the yacr CLI.
#
# Renders one DWG offscreen and verifies the produced PNG is a real, non-blank
# frame. On a machine with Mesa lavapipe, force the software adapter first:
#
#   export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
#   scripts/render-smoke.sh <input.dwg> <out.png> [width] [height]
#
# The CLI is taken from $YACR_CLI, or, when this script is inside a release
# package, from ../bin/cad-cli-tools.
set -euo pipefail

DWG="${1:?usage: render-smoke.sh <input.dwg> <out.png> [width] [height]}"
PNG="${2:?usage: render-smoke.sh <input.dwg> <out.png> [width] [height]}"
WIDTH="${3:-1280}"
HEIGHT="${4:-720}"

if [ -n "${YACR_CLI:-}" ]; then
  BIN="$YACR_CLI"
else
  HERE="$(cd "$(dirname "$0")" && pwd)"
  BIN="$HERE/../bin/cad-cli-tools"
fi
[ -x "$BIN" ] || { echo "ERROR: cad-cli-tools not found/executable: $BIN" >&2; exit 1; }
[ -f "$DWG" ] || { echo "ERROR: input DWG not found: $DWG" >&2; exit 1; }

rm -f "$PNG"
JSON="$("$BIN" render "$DWG" --png "$PNG" --width "$WIDTH" --height "$HEIGHT")"

if command -v python3 >/dev/null 2>&1; then
  printf '%s' "$JSON" | python3 -c '
import json, os, sys
doc = json.load(sys.stdin)
png = doc.get("png")
if not png:
    print("ERROR: render returned no png object", file=sys.stderr); sys.exit(1)
path, size = png["path"], png["bytes"]
if size <= 0 or not os.path.exists(path):
    print("ERROR: png missing or empty: " + path, file=sys.stderr); sys.exit(1)
with open(path, "rb") as fh:
    sig = fh.read(8)
if sig != b"\x89PNG\r\n\x1a\n":
    print("ERROR: not a PNG file", file=sys.stderr); sys.exit(1)
nonbg = doc["pixels"]["non_background"]
if nonbg <= 0:
    print("ERROR: rendered frame is blank (non_background=0)", file=sys.stderr); sys.exit(1)
adapter = doc["adapter"]
frame = doc["frame"]
pixels = doc["pixels"]
print("render-smoke OK: adapter=%s/%s draws=%s non_background=%s coverage=%.4f png=%sB"
      % (adapter["backend"], adapter["device_type"], frame["draw_calls"],
         nonbg, pixels["coverage"], size))
'
else
  # Minimal fallback without python3: signature + size + non-empty PNG/JSON.
  [ -s "$PNG" ] || { echo "ERROR: png missing or empty: $PNG" >&2; exit 1; }
  sig="$(head -c 8 "$PNG" | od -An -tx1 | tr -d ' \n')"
  [ "$sig" = "89504e470d0a1a0a" ] || { echo "ERROR: not a PNG file: $PNG" >&2; exit 1; }
  printf '%s' "$JSON" | grep -q '"non_background": *[1-9]' \
    || { echo "ERROR: rendered frame appears blank (non_background=0)" >&2; exit 1; }
  echo "render-smoke OK (no python3): png=$(wc -c < "$PNG")B"
fi
