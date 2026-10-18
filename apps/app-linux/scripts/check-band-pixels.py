#!/usr/bin/env python3
"""Machine pixel fence for the live box-selection rubber band.

Given a screenshot of the running host, count how many pixels look like the
CROSSING box-selection band (and, for contrast, how many look like the WINDOW
band), then report one line:

    band-pixels: green=<n> window-blue=<n> total=<n>

The process exits non-zero when the expected CROSSING (green) count is 0, so a
run whose screenshot contains no band pixels fails the fence instead of being
eyeballed. This is the out-of-band counterpart of the box-evidence report's
`box_overlay_channel=none`: the overlay is not projected into `cad_status`, so
the screenshot is the only machine-readable channel for it.

Exact colours and tolerance
---------------------------
The host paints the band with the linear-space constants in
`crates/cad-app/src/render_scene/overlay/types.rs`:

  * CROSSING (`BOX_SELECT_CROSSING_COLOR`) = [0.35, 0.85, 0.45]
  * WINDOW   (`BOX_SELECT_WINDOW_COLOR`)   = [0.29, 0.56, 0.95]

The wgpu surface is sRGB, so those linear constants are sRGB-encoded before they
reach the pixels. The reference *display* colours this checker looks for are
therefore:

  * green       = sRGB(0.35, 0.85, 0.45) = (160, 237, 179)
  * window-blue = sRGB(0.29, 0.56, 0.95) = (147, 197, 249)

The band can be alpha-blended over the CAD background, so its pixels vary in
brightness (a darker green over dark geometry, a bright green over light paper).
To count the band at any blend level, matching is done on **chromaticity** (the
colour's direction) rather than on absolute brightness. For green, a pixel
counts when:

  * the green channel is the largest (green is the dominant channel), and
  * the green channel is at least the brightness floor (40/255, so near-black
    pixels are ignored), and
  * its ratios r/g and b/g are within ``TOLERANCE`` of the reference ratios
    (160/237, 179/237) = (0.6751, 0.7553).

The window-blue test is symmetric around (147, 197, 249) using r/b and g/b
(0.5904, 0.7912). The default ``TOLERANCE = 0.06`` is the maximum per-ratio
deviation; it comfortably covers the observed blend levels of the real band
while excluding neutral greys (ratio 1.0) and the other band colour.

Inputs
------
The screenshot may be either file the X11 grabber path produces:

  * a binary PPM (`P6`, 8-bit) — what
    `apps/app-linux/scripts/x11-window-grab.c` writes directly, and
  * an 8-bit PNG (grey / RGB / RGBA / palette, non-interlaced) — what
    `scripts/collect-evidence.sh` converts the PPM to via Pillow.

Usage
-----
    check-band-pixels.py [--file PATH] [--tolerance T] [PATH ...]

``--file`` may be repeated and bare positional paths are accepted too. With a
single input the output is exactly the single ``band-pixels: ...`` line above;
with several inputs each line is prefixed with ``<path>: `` so the results stay
attributable.

Standard library only. English messages.
"""
from __future__ import annotations

import struct
import sys
import zlib

# The sRGB-encoded display colours derived from the linear overlay constants
# (see the module docstring). Ratios are (r/g, b/g) for green and (r/b, g/b)
# for window-blue.
GREEN_RGB = (160, 237, 179)
GREEN_RATIOS = (GREEN_RGB[0] / GREEN_RGB[1], GREEN_RGB[2] / GREEN_RGB[1])
WINDOW_RGB = (147, 197, 249)
WINDOW_RATIOS = (WINDOW_RGB[0] / WINDOW_RGB[2], WINDOW_RGB[1] / WINDOW_RGB[2])

DEFAULT_TOLERANCE = 0.06
BRIGHTNESS_FLOOR = 40

_PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
_PNG_CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}


class ImageError(Exception):
    """A screenshot could not be decoded."""


def _decode_png(data: bytes) -> tuple[int, int, list[tuple[int, int, int]]]:
    if data[:8] != _PNG_SIGNATURE:
        raise ImageError("not a PNG (bad signature)")
    pos = 8
    width = height = bit_depth = color_type = interlace = 0
    idat = bytearray()
    palette = b""
    while pos + 8 <= len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        chunk_type = data[pos + 4:pos + 8]
        chunk = data[pos + 8:pos + 8 + length]
        if len(chunk) != length:
            raise ImageError("truncated PNG chunk")
        pos += 12 + length  # length + type + data + CRC
        if chunk_type == b"IHDR":
            width, height, bit_depth, color_type, _, _, interlace = struct.unpack(
                ">IIBBBBB", chunk
            )
        elif chunk_type == b"PLTE":
            palette = chunk
        elif chunk_type == b"IDAT":
            idat += chunk
        elif chunk_type == b"IEND":
            break
    if width == 0 or height == 0:
        raise ImageError("PNG has no IHDR")
    if bit_depth != 8:
        raise ImageError(f"unsupported PNG bit depth {bit_depth} (only 8-bit)")
    if interlace != 0:
        raise ImageError("unsupported interlaced PNG")
    if color_type not in _PNG_CHANNELS:
        raise ImageError(f"unsupported PNG colour type {color_type}")
    if color_type == 3 and not palette:
        raise ImageError("palette PNG without a PLTE chunk")

    channels = _PNG_CHANNELS[color_type]
    stride = width * channels
    raw = zlib.decompress(bytes(idat))
    if len(raw) < height * (stride + 1):
        raise ImageError("PNG pixel data shorter than the header implies")

    previous = bytearray(stride)
    pixels: list[tuple[int, int, int]] = []
    offset = 0
    for _ in range(height):
        filter_type = raw[offset]
        offset += 1
        scanline = bytearray(raw[offset:offset + stride])
        offset += stride
        _unfilter(filter_type, scanline, previous, channels)
        previous = scanline
        for x in range(width):
            px = scanline[x * channels:x * channels + channels]
            pixels.append(_to_rgb(color_type, px, palette))
    return width, height, pixels


def _unfilter(filter_type: int, line: bytearray, previous: bytearray, bpp: int) -> None:
    n = len(line)
    if filter_type == 0:
        return
    if filter_type == 1:  # Sub
        for i in range(bpp, n):
            line[i] = (line[i] + line[i - bpp]) & 0xFF
    elif filter_type == 2:  # Up
        for i in range(n):
            line[i] = (line[i] + previous[i]) & 0xFF
    elif filter_type == 3:  # Average
        for i in range(n):
            left = line[i - bpp] if i >= bpp else 0
            line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
    elif filter_type == 4:  # Paeth
        for i in range(n):
            left = line[i - bpp] if i >= bpp else 0
            up = previous[i]
            up_left = previous[i - bpp] if i >= bpp else 0
            line[i] = (line[i] + _paeth(left, up, up_left)) & 0xFF
    else:
        raise ImageError(f"unsupported PNG filter type {filter_type}")


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def _to_rgb(color_type: int, px: bytes, palette: bytes) -> tuple[int, int, int]:
    if color_type == 0:
        return (px[0], px[0], px[0])
    if color_type == 2:
        return (px[0], px[1], px[2])
    if color_type == 3:
        index = px[0] * 3
        return (palette[index], palette[index + 1], palette[index + 2])
    if color_type == 4:
        return (px[0], px[0], px[0])
    return (px[0], px[1], px[2])  # color_type 6 (RGBA)


def _decode_ppm(data: bytes) -> tuple[int, int, list[tuple[int, int, int]]]:
    if not data.startswith(b"P6"):
        raise ImageError("not a binary PPM (expected P6)")
    fields: list[bytes] = []
    pos = 2
    while len(fields) < 3:
        while pos < len(data) and data[pos:pos + 1].isspace():
            pos += 1
        if pos < len(data) and data[pos:pos + 1] == b"#":
            while pos < len(data) and data[pos:pos + 1] not in (b"\n", b"\r"):
                pos += 1
            continue
        start = pos
        while pos < len(data) and not data[pos:pos + 1].isspace():
            pos += 1
        if start == pos:
            raise ImageError("malformed PPM header")
        fields.append(data[start:pos])
    pos += 1  # single whitespace after maxval
    width, height, maxval = (int(f) for f in fields)
    if maxval != 255:
        raise ImageError(f"unsupported PPM maxval {maxval} (only 255)")
    body = data[pos:]
    if len(body) < width * height * 3:
        raise ImageError("PPM pixel data shorter than the header implies")
    pixels = [
        (body[i], body[i + 1], body[i + 2])
        for i in range(0, width * height * 3, 3)
    ]
    return width, height, pixels


def load_pixels(path: str) -> tuple[int, int, list[tuple[int, int, int]]]:
    """Return ``(width, height, pixels)`` for a PPM (P6) or 8-bit PNG file."""
    with open(path, "rb") as handle:
        data = handle.read()
    if data.startswith(_PNG_SIGNATURE):
        return _decode_png(data)
    if data.startswith(b"P6"):
        return _decode_ppm(data)
    raise ImageError("unknown image format (expected binary PPM 'P6' or PNG)")


def _is_band(
    pixel: tuple[int, int, int],
    dominant: int,
    ratios: tuple[float, float],
    tolerance: float,
) -> bool:
    r, g, b = pixel
    if pixel[dominant] < BRIGHTNESS_FLOOR:
        return False
    if pixel[dominant] < max(pixel):
        return False
    other_a, other_b = (
        (pixel[0], pixel[2]) if dominant == 1 else (pixel[0], pixel[1])
    )
    value = pixel[dominant]
    return (
        abs(other_a / value - ratios[0]) <= tolerance
        and abs(other_b / value - ratios[1]) <= tolerance
    )


def count_bands(
    pixels: list[tuple[int, int, int]], tolerance: float
) -> tuple[int, int]:
    """Return ``(green_count, window_blue_count)`` for the given pixels."""
    green = blue = 0
    for pixel in pixels:
        if _is_band(pixel, 1, GREEN_RATIOS, tolerance):
            green += 1
        elif _is_band(pixel, 2, WINDOW_RATIOS, tolerance):
            blue += 1
    return green, blue


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
                raise SystemExit(f"error: --tolerance expects a number, got {argv[index+1]!r}")
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
            width, height, pixels = load_pixels(path)
        except (ImageError, OSError, zlib.error) as error:
            print(f"{path}: error: {error}", file=sys.stderr)
            status = 2
            continue
        green, blue = count_bands(pixels, tolerance)
        prefix = f"{path}: " if multi else ""
        print(
            f"{prefix}band-pixels: green={green} window-blue={blue} total={len(pixels)}"
        )
        if green == 0:
            status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
