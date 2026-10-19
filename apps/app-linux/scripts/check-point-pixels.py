#!/usr/bin/env python3
"""Machine pixel fence for the visible POINT markers.

Given a screenshot of the running host, count how many pixels look like the
POINT marker cross (ADR 0015 decision E), then report one line:

    point-markers: <n>

The process exits non-zero when the count is 0, so a run whose screenshot
contains no POINT marker fails the fence instead of being eyeballed. This is the
out-of-band counterpart of the draw-evidence report: `cad_status` exposes no
per-entity geometry, so the screenshot is the only machine-readable channel that
the created POINT is actually **visible**.

Exact colour and tolerance
--------------------------
The host paints the marker with the linear-space constant in
`crates/cad-app/src/render_scene/overlay/point_markers.rs`:

  * POINT_MARKER_COLOR = [0.80, 0.90, 1.0]

The wgpu surface is sRGB (`Rgba8UnormSrgb`/`Bgra8UnormSrgb`), so that linear
constant is sRGB-encoded before it reaches the pixels — exactly the treatment the
box-selection band constants get in `check-band-pixels.py`. The reference
*display* colour this checker looks for is therefore:

  * marker = sRGB(0.80, 0.90, 1.0) = (231, 243, 255)

The CAD render target clears to **pure opaque black** and the marker is opaque
(`alpha = 1.0`), so an anti-aliased marker pixel is the marker colour scaled by
some blend factor over black; scaling toward black preserves chromaticity
exactly. Matching is therefore done on **chromaticity** (the colour's direction)
rather than on absolute brightness, using the same approach as the band checker:
a pixel counts as a marker when

  * the blue channel is the largest (blue is the dominant channel), and
  * the blue channel is at least the brightness floor (40/255, so near-black
    pixels are ignored), and
  * its ratios r/b and g/b are within ``TOLERANCE`` of the reference ratios
    (231/255, 243/255) = (0.9059, 0.9529).

Tolerance choice
----------------
Unlike the box-selection band, whose green/blue constants are far from neutral
(r/g 0.68 and r/b 0.59), the POINT marker is a **pale, near-neutral** blue-white:
its r/b ratio (0.9059) differs from a neutral grey (1.0) by only 0.094. The band
checker's default 0.06 would therefore also admit the CAD grid/axis greys: on a
real host screenshot the grid line is (157, 160, 165), ratios (0.9515, 0.9697),
only 0.046 from the marker ratio. The default is consequently tightened to
``TOLERANCE = 0.03`` — the marker's own anti-aliased pixels cluster at ratios
0.88..0.907 (their r/b deviation is <= 0.03), while the grid grey's 0.046
deviation is excluded. This is a discrimination requirement of the near-neutral
colour, not a tuned threshold: at 0.06 the counter is dominated by grid pixels
(2733 vs the 126 real marker pixels measured on the same screenshot).

Inputs
------
The screenshot may be either file the X11 grabber path produces:

  * a binary PPM (`P6`, 8-bit) — what
    `apps/app-linux/scripts/x11-window-grab.c` writes directly, and
  * an 8-bit PNG (grey / RGB / RGBA / palette, non-interlaced) — what
    `scripts/collect-evidence.sh` converts the PPM to via Pillow.

The PNG/PPM decoder is imported from the sibling `check-band-pixels.py`, so both
checkers share one decoder.

Usage
-----
    check-point-pixels.py [--file PATH] [--tolerance T] [PATH ...]

``--file`` may be repeated and bare positional paths are accepted too. With a
single input the output is exactly the single ``point-markers: ...`` line above;
with several inputs each line is prefixed with ``<path>: `` so the results stay
attributable.

Standard library only. English messages.
"""
from __future__ import annotations

import importlib.util
import os
import sys

# Reuse the sibling band checker's image decoder (one decoder for both fences).
# Bytecode caching is suppressed so loading the sibling never writes a
# `__pycache__` directory into the checked-out tree.
_HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "check_band_pixels", os.path.join(_HERE, "check-band-pixels.py")
)
if _spec is None or _spec.loader is None:  # pragma: no cover - defensive
    raise SystemExit("error: cannot load sibling check-band-pixels.py decoder")
_band = importlib.util.module_from_spec(_spec)
_prev_dont_write_bytecode = sys.dont_write_bytecode
sys.dont_write_bytecode = True
try:
    _spec.loader.exec_module(_band)
finally:
    sys.dont_write_bytecode = _prev_dont_write_bytecode


def _srgb_encode(channel: float) -> float:
    """Encode one linear [0, 1] channel to sRGB [0, 1]."""
    if channel <= 0.0031308:
        return 12.92 * channel
    return 1.055 * (channel ** (1.0 / 2.4)) - 0.055


def _display_rgb(linear: tuple[float, float, float]) -> tuple[int, int, int]:
    return tuple(round(_srgb_encode(channel) * 255.0) for channel in linear)


# The sRGB-encoded display colour derived from the linear marker constant
# (see the module docstring). Ratios are (r/b, g/b): blue is the dominant
# channel.
POINT_MARKER_LINEAR = (0.80, 0.90, 1.0)
POINT_RGB = _display_rgb(POINT_MARKER_LINEAR)
POINT_RATIOS = (POINT_RGB[0] / POINT_RGB[2], POINT_RGB[1] / POINT_RGB[2])

DEFAULT_TOLERANCE = 0.03


def _is_point_marker(
    pixel: tuple[int, int, int],
    ratios: tuple[float, float],
    tolerance: float,
) -> bool:
    r, g, b = pixel
    if b < _band.BRIGHTNESS_FLOOR:
        return False
    if b < max(pixel):
        return False
    return (
        abs(r / b - ratios[0]) <= tolerance
        and abs(g / b - ratios[1]) <= tolerance
    )


def count_point_markers(
    pixels: list[tuple[int, int, int]], tolerance: float
) -> int:
    """Return the number of pixels that look like a POINT marker."""
    count = 0
    for pixel in pixels:
        if _is_point_marker(pixel, POINT_RATIOS, tolerance):
            count += 1
    return count


def _parse_args(argv: list[str]) -> tuple[list[str], float]:
    files: list[str] = []
    tolerance = DEFAULT_TOLERANCE
    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg == "--file":
            if index + 1 >= len(argv):
                raise SystemExit("error: --file expects a path")
            files.append(argv[index + 1])
            index += 2
        elif arg == "--tolerance":
            if index + 1 >= len(argv):
                raise SystemExit("error: --tolerance expects a number")
            try:
                tolerance = float(argv[index + 1])
            except ValueError:
                raise SystemExit(
                    f"error: --tolerance expects a number, got {argv[index + 1]!r}"
                )
            index += 2
        elif arg in ("-h", "--help"):
            print(__doc__)
            raise SystemExit(0)
        elif arg.startswith("-"):
            raise SystemExit(f"error: unknown argument {arg!r}")
        else:
            files.append(arg)
            index += 1
    if not files:
        raise SystemExit("error: no screenshot path given (use --file PATH)")
    return files, tolerance


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    files, tolerance = _parse_args(argv)
    multi = len(files) > 1
    status = 0
    for path in files:
        try:
            _width, _height, pixels = _band.load_pixels(path)
        except (_band.ImageError, OSError) as error:
            print(f"{path}: error: {error}", file=sys.stderr)
            status = 2
            continue
        count = count_point_markers(pixels, tolerance)
        prefix = f"{path}: " if multi else ""
        print(f"{prefix}point-markers: {count} total={len(pixels)}")
        if count == 0:
            status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
