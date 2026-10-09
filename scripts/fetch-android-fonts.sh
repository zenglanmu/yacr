#!/usr/bin/env bash
# Download the optional Android font package into `apps/app-android/assets/fonts/`.
#
# Fonts are third-party and their licence is NOT bundled with this repository, so
# the binaries are gitignored. Run this before building an APK that should shape
# text; `cargo apk` only packages the directory when `assets = "assets"` is set
# in `apps/app-android/Cargo.toml` (it is).
#
# Source: the same catalogue the web host uses —
#   mlightcad/cad-data via jsDelivr, base <DEFAULT_FONT_BASE_URL> (docs/fonts.md).
# The pack below is the common subset a typical drawing references (SHX text
# fonts plus a few CJK/outline faces). Override the set with FONTS="a b c".
set -euo pipefail

BASE="${FONT_BASE_URL:-https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/fonts}"
DEST="$(cd "$(dirname "$0")/.." && pwd)/apps/app-android/assets/fonts"

DEFAULT_FONTS=(
  # shape (SHX) fonts
  simplex.shx txt.shx romans.shx romand.shx romant.shx
  isocp.shx isocp2.shx isocp3.shx ltypeshp.shx
  gbcbig.shx hztxt.shx bigfont.shx
  # outline / CJK fonts
  arial.woff simsun.woff simhei.woff simkai.woff msyh.woff
  tahoma.woff verdana.woff msgothic.woff noto-sans-kr.woff
  gbgdt.woff SJQY.woff AIGDT.ttf
)

read -r -a FONTS <<<"${FONTS:-${DEFAULT_FONTS[*]}}"

command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }

mkdir -p "$DEST"
echo "Downloading ${#FONTS[@]} fonts into $DEST"
for f in "${FONTS[@]}"; do
  if curl -fsSL "$BASE/$f" -o "$DEST/$f"; then
    printf '  ok   %s\n' "$f"
  else
    printf '  FAIL %s (not in the catalogue?)\n' "$f" >&2
  fi
done

# The Android host reads the catalogue from `asset://fonts/fonts.json`.
if curl -fsSL "$BASE/fonts.json" -o "$DEST/fonts.json"; then
  echo "  ok   fonts.json"
else
  echo "  FAIL fonts.json" >&2
fi

# Merge the committed font package (`fonts/`, currently QCAD osifont.ttf) into
# the asset set so a packaged APK always carries the default outline face.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ -d "$ROOT/fonts" ]; then
  python3 - "$DEST" "$ROOT/fonts" <<'PY'
import json, pathlib, sys
dest, extra = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
catalog_path = dest / "fonts.json"
if (extra / "fonts.json").exists():
    entries = json.loads(catalog_path.read_text()) if catalog_path.exists() else []
    added = [e for e in json.loads((extra / "fonts.json").read_text())
             if all(str(e.get("file")) != str(x.get("file")) for x in entries)]
    entries.extend(added)
    catalog_path.write_text(json.dumps(entries, indent=2) + "\n")
    for font in added:
        src = extra / str(font["file"])
        if src.exists():
            (dest / str(font["file"])).write_bytes(src.read_bytes())
            print(f"  merged {font['file']}")
PY
fi

echo "Done. Rebuild with: ANDROID_HOME=... JAVA_HOME=... bash scripts/build-android.sh --release"
