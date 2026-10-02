#!/usr/bin/env python3
"""Regenerate the OFL UI font (requires fonttools 4.61.1, not a build dependency).

Input: Noto Sans SC 2.004-H2, Google Fonts v41 text subset (provenance in
crates/cad-ui-slint/fonts/README.md). Only shell glyphs are bundled;
CAD drawing fonts remain a separate resource pipeline.
"""
import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
text = "".join(
    path.read_text()
    for pattern in (
        "crates/cad-ui-slint/i18n/*.json",
        "apps/app-web/src/**/*.rs",
        "apps/app-web/web/index.html",
        "crates/cad-app/src/host.rs",
    )
    for path in ROOT.glob(pattern)
)
unicodes = set(range(0x20, 0x100)) | set(range(0x3000, 0x3040)) | {ord(c) for c in text}
font = TTFont(sys.argv[1], recalcTimestamp=False)
options = subset.Options()
options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14, 16, 17]
options.name_legacy = True
options.name_languages = [0x409]
subsetter = subset.Subsetter(options=options)
subsetter.populate(unicodes=unicodes)
subsetter.subset(font)
# Give the modified font its own family name; preserve copyright/license records.
for record in font["name"].names:
    if record.nameID in (1, 4, 6, 16):
        value = "YacrUI-Regular" if record.nameID == 6 else "Yacr UI"
        record.string = value.encode(record.getEncoding())
if "CFF " in font:
    font["CFF "].cff.fontNames = ["YacrUI-Regular"]
    top = font["CFF "].cff.topDictIndex[0]
    top.FamilyName = "Yacr UI"
    top.FullName = "Yacr UI Regular"
output = ROOT / "crates/cad-ui-slint/fonts/YacrUI-Regular.otf"
output.parent.mkdir(parents=True, exist_ok=True)
font.save(output)
print(f"{output}: {output.stat().st_size} bytes, {len(font.getBestCmap())} codepoints")
