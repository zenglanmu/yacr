#!/usr/bin/env python3
"""Verify the committed missing-font fallback assets.

The repository ships exactly one CJK fallback face, ``NotoSansCJKsc-Regular.otf``,
plus its OFL text and the ``fonts.json`` entry every host catalog reads. A
truncated/corrupted font, a silently dropped catalogue entry or a wrong licence
file would still let the build pass and only fail on a device, so this checker
pins the published facts:

* the font file exists with the exact byte count and SHA-256 recorded in
  ``fonts/SOURCE.md`` and ``THIRD_PARTY_NOTICES.md``;
* ``fonts/fonts.json`` still lists that file under every documented alias;
* ``fonts/NotoSansCJK-OFL.txt`` is the SIL OFL 1.1 text with the Adobe
  copyright/reserved-name line and no Debian packaging or GPL text.

Standard library only: this runs in the ``core-quality`` CI job.
"""
from __future__ import annotations

import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
FONTS_DIR = ROOT / "fonts"
FONT_FILE = FONTS_DIR / "NotoSansCJKsc-Regular.otf"
CATALOG = FONTS_DIR / "fonts.json"
LICENSE = FONTS_DIR / "NotoSansCJK-OFL.txt"

FONT_SIZE = 16437364
FONT_SHA256 = "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b"
EXPECTED_NAMES = (
    "Noto Sans CJK SC",
    "NotoSansCJKsc",
    "NotoSansCJKsc-Regular",
    "Noto Sans CJK",
    "Noto Sans SC",
    "noto sans cjk sc",
)

OFL_HEADER = "SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007"
OFL_COPYRIGHT = (
    "Copyright 2014-2021 Adobe (http://www.adobe.com/), "
    "with Reserved Font Name 'Source'."
)
OFL_SECTIONS = (
    "PREAMBLE",
    "DEFINITIONS",
    "PERMISSION & CONDITIONS",
    "TERMINATION",
    "DISCLAIMER",
)
# A Debian copyright stanza or the GPL appendix means the wrong slice was copied.
OFL_FORBIDDEN = ("Files: debian/", "License: GPL", "GNU General Public License")


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_font_bytes() -> list[str]:
    if not FONT_FILE.is_file():
        return [f"missing bundled fallback font: {FONT_FILE.relative_to(ROOT)}"]
    problems: list[str] = []
    size = FONT_FILE.stat().st_size
    if size != FONT_SIZE:
        problems.append(
            f"{FONT_FILE.relative_to(ROOT)}: expected {FONT_SIZE} bytes, got {size}"
        )
    digest = sha256(FONT_FILE)
    if digest != FONT_SHA256:
        problems.append(
            f"{FONT_FILE.relative_to(ROOT)}: expected SHA-256 {FONT_SHA256}, got {digest}"
        )
    return problems


def check_catalog() -> list[str]:
    if not CATALOG.is_file():
        return [f"missing font catalogue: {CATALOG.relative_to(ROOT)}"]
    try:
        entries = json.loads(CATALOG.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"{CATALOG.relative_to(ROOT)}: cannot parse: {error}"]
    if not isinstance(entries, list):
        return [f"{CATALOG.relative_to(ROOT)}: top level must be a JSON array"]

    matches = [
        entry
        for entry in entries
        if isinstance(entry, dict) and entry.get("file") == FONT_FILE.name
    ]
    if not matches:
        return [f"{CATALOG.relative_to(ROOT)}: no entry for {FONT_FILE.name}"]
    if len(matches) > 1:
        return [
            f"{CATALOG.relative_to(ROOT)}: {FONT_FILE.name} must appear exactly once "
            f"(found {len(matches)})"
        ]

    entry = matches[0]
    names = entry.get("name")
    problems: list[str] = []
    if not isinstance(names, list):
        return [f"{CATALOG.relative_to(ROOT)}: {FONT_FILE.name} entry has no name array"]
    missing = [name for name in EXPECTED_NAMES if name not in names]
    if missing:
        problems.append(
            f"{CATALOG.relative_to(ROOT)}: {FONT_FILE.name} is missing aliases {missing}"
        )
    if entry.get("type") != "mesh":
        problems.append(
            f"{CATALOG.relative_to(ROOT)}: {FONT_FILE.name} type must be 'mesh'"
        )
    return problems


def check_license() -> list[str]:
    if not LICENSE.is_file():
        return [f"missing OFL text: {LICENSE.relative_to(ROOT)}"]
    text = LICENSE.read_text(encoding="utf-8")
    problems: list[str] = []
    if OFL_COPYRIGHT not in text:
        problems.append(f"{LICENSE.relative_to(ROOT)}: missing Adobe copyright line")
    if OFL_HEADER not in text:
        problems.append(f"{LICENSE.relative_to(ROOT)}: missing OFL 1.1 header")
    for section in OFL_SECTIONS:
        if section not in text:
            problems.append(f"{LICENSE.relative_to(ROOT)}: missing OFL section {section!r}")
    for fragment in OFL_FORBIDDEN:
        if fragment in text:
            problems.append(
                f"{LICENSE.relative_to(ROOT)}: contains non-OFL text {fragment!r}"
            )
    return problems


def main() -> int:
    problems: list[str] = []
    for check in (check_font_bytes, check_catalog, check_license):
        try:
            problems.extend(check())
        except Exception as error:  # noqa: BLE001 - report, never crash into a pass
            problems.append(f"{check.__name__}: {error}")
    if problems:
        print("font-assets: FAILED", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(
        "font-assets OK: "
        f"{FONT_FILE.name} {FONT_SIZE} bytes sha256 {FONT_SHA256}; "
        f"{len(EXPECTED_NAMES)} catalogue aliases; OFL-1.1 + Adobe copyright"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
