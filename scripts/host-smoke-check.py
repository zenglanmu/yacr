#!/usr/bin/env python3
"""Check a headless Linux host smoke log for the CAD-presentation failure class.

`scripts/host-smoke-linux.sh` runs the real GTK/Wayland host with a software
Vulkan adapter and a real fixture. This checker is the *gate* over that run: it
reads only the host's stable `[probe]` lines, so it is pure text parsing and
`scripts/test-host-smoke-check.py` can prove it fails on the real defects.

What it rejects (the failures this repository has actually shipped):

* a presenter/refusal error (`cad present failed`, `refusing to stretch`,
  `native texture rejected`, `does not match the DOM CAD rect`, `cad bind FAILED`,
  a failed presenter construction, a failed session creation);
* **no presented frame at all** (the window would show the desktop);
* no fully rendered frame (`complete=current_scene` never observed);
* a **settled** geometry line whose `dom_physical` and `native_surface` disagree
  (the 3851x1645 vs 3852x1644 class: the DOM-derived render target and the
  configured native surface must be the same pixel count);
* the last presented frame's texture `size=` disagreeing with the settled
  `native_surface` (the presenter would have refused it).

Transient geometry lines are legitimate: a layout change prints the new
`dom_physical` before the presenter reconfigures, and the next line settles.
Only the **last non-zero** geometry line is required to match, so a real
mid-resize transient cannot fail the gate — a persistent disagreement can.

Exit status: 0 = pass, 1 = gate failure, 2 = usage/IO error.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

# Stable markers a healthy CAD-presenting run never emits. Keep these exact
# substrings: they are the host's own diagnostic phrases (see the app-linux host
# and `cad-render-wgpu::ProbeError`), not paraphrases.
FORBIDDEN = (
    "cad present failed",
    "present_texture failed",
    "refusing to stretch",
    "does not match the DOM CAD rect",
    "native texture rejected",
    "cad bind FAILED",
    "cad session creation FAILED",
    "cad surface presenter FAILED",
    "bridge wayland presenter FAILED",
    "CAD composition is unavailable",
    "probe presenter init failed",
)

GEOMETRY_RE = re.compile(
    r"\[probe\] geometry .*?dom_physical=(?P<dw>\d+)x(?P<dh>\d+)"
    r".*?native_surface=(?P<nw>\d+)x(?P<nh>\d+)"
)
FRAME_RE = re.compile(r"\[probe\] cad-frame presented .*?size=(?P<w>\d+)x(?P<h>\d+)")
COMPLETE_MARKER = "complete=current_scene"


def _first_line_with(text: str, needle: str) -> str:
    for line in text.splitlines():
        if needle in line:
            return line.strip()
    return ""


def check(text: str) -> tuple[bool, str, list[str]]:
    """Return `(ok, summary, findings)` for one host log."""
    findings: list[str] = []

    for marker in FORBIDDEN:
        if marker in text:
            findings.append(f"forbidden marker {marker!r}: {_first_line_with(text, marker)}")

    frames = [(int(m.group("w")), int(m.group("h"))) for m in FRAME_RE.finditer(text)]
    if not frames:
        findings.append("no '[probe] cad-frame presented' line: no CAD frame was presented")
    complete = sum(1 for line in text.splitlines() if COMPLETE_MARKER in line)
    if frames and complete == 0:
        findings.append("no presented frame reported complete=current_scene")

    geometry = [
        (
            (int(m.group("dw")), int(m.group("dh"))),
            (int(m.group("nw")), int(m.group("nh"))),
        )
        for m in GEOMETRY_RE.finditer(text)
    ]
    settled = [(dom, native) for dom, native in geometry if native != (0, 0)]
    if not geometry:
        findings.append("no '[probe] geometry' line: the host never reported its CAD rect")
    elif not settled:
        findings.append("every geometry line reported native_surface=0x0: the presenter never configured")
    else:
        dom, native = settled[-1]
        if dom != native:
            findings.append(
                "settled size mismatch: dom_physical={}x{} != native_surface={}x{} "
                "(the presenter must refuse such a texture)".format(*dom, *native)
            )
        if frames and frames[-1] != native:
            findings.append(
                "last presented texture size={}x{} != settled native_surface={}x{}".format(
                    *frames[-1], *native
                )
            )

    last_size = "{}x{}".format(*frames[-1]) if frames else "none"
    summary = (
        f"host-smoke-check: frames={len(frames)} complete={complete} "
        f"geometry={len(geometry)} settled={len(settled)} last_frame={last_size}"
    )
    return (not findings, summary, findings)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=pathlib.Path, help="host stderr log captured by the smoke")
    args = parser.parse_args(argv)
    try:
        text = args.log.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        print(f"host-smoke-check: cannot read {args.log}: {error}", file=sys.stderr)
        return 2
    ok, summary, findings = check(text)
    print(summary)
    if not ok:
        for finding in findings:
            print(f"FAIL: {finding}")
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
