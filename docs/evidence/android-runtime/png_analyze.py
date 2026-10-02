#!/usr/bin/env python3
"""Pure-stdlib PNG decoder + region analyzer for the yacr Android evidence.

Reports image size and, for named rectangular regions (fractions of the image),
the number of distinct 24-bit colours and the fraction of pixels differing from
the region's modal colour. A flat background region has ~1 distinct colour; a
rendered CAD canvas has many.
"""
import sys, zlib, struct
from collections import Counter


def read_png(path):
    with open(path, "rb") as f:
        data = f.read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos = 8
    idat = bytearray()
    width = height = bit_depth = color_type = interlace = None
    palette = None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        ctype = data[pos + 4:pos + 8]
        chunk = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if ctype == b"IHDR":
            width, height, bit_depth, color_type, _comp, _filt, interlace = struct.unpack(
                ">IIBBBBB", chunk
            )
        elif ctype == b"IDAT":
            idat += chunk
        elif ctype == b"PLTE":
            palette = chunk
        elif ctype == b"IEND":
            break
    assert interlace == 0, "interlaced PNG not supported"
    assert bit_depth == 8, f"unsupported bit depth {bit_depth}"

    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color_type]
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    out = bytearray(height * stride)
    prev = bytearray(stride)

    def paeth(a, b, c):
        p = a + b - c
        pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
        if pa <= pb and pa <= pc:
            return a
        if pb <= pc:
            return b
        return c

    rp = 0
    for y in range(height):
        ftype = raw[rp]
        rp += 1
        line = bytearray(raw[rp:rp + stride])
        rp += stride
        if ftype == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif ftype == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                c = prev[i - channels] if i >= channels else 0
                line[i] = (line[i] + paeth(a, prev[i], c)) & 0xFF
        line = bytes(line)
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return width, height, channels, color_type, palette, bytes(out)


def rgb(channels, color_type, palette, buf, idx):
    i = idx * channels
    if color_type == 6:
        return buf[i], buf[i + 1], buf[i + 2]
    if color_type == 2:
        return buf[i], buf[i + 1], buf[i + 2]
    if color_type == 0:
        v = buf[i]
        return v, v, v
    if color_type == 4:
        v = buf[i]
        return v, v, v
    if color_type == 3:
        p = buf[i] * 3
        return palette[p], palette[p + 1], palette[p + 2]
    raise ValueError(color_type)


def region_stats(width, height, channels, color_type, palette, buf, name, box):
    x0, y0, x1, y1 = box
    cnt = Counter()
    n = 0
    for y in range(y0, y1):
        row = y * width
        for x in range(x0, x1):
            cnt[rgb(channels, color_type, palette, buf, row + x)] += 1
            n += 1
    distinct = len(cnt)
    modal, modal_n = cnt.most_common(1)[0]
    nonmodal = (n - modal_n) / n if n else 0.0
    print(f"region {name:16s} box=({x0},{y0})-({x1},{y1}) px={n:8d} distinct_colors={distinct:5d} "
          f"modal={modal} nonmodal_frac={nonmodal:.4f}")
    return distinct, nonmodal


def main():
    path = sys.argv[1]
    w, h, ch, ct, pal, buf = read_png(path)
    print(f"image {path}: {w}x{h} channels={ch} color_type={ct}")
    # Regions as fractions (x0,y0,x1,y1). Canvas is the big middle band.
    regions = {
        "top_bar": (0.0, 0.0, 1.0, 0.03),
        "canvas_upper": (0.30, 0.15, 1.0, 0.35),
        "canvas_mid": (0.30, 0.30, 1.0, 0.55),
        "canvas_lower": (0.0, 0.55, 1.0, 0.75),
        "bottom_status": (0.0, 0.97, 1.0, 1.0),
    }
    for name, (fx0, fy0, fx1, fy1) in regions.items():
        region_stats(w, h, ch, ct, pal, buf, name,
                     (int(fx0 * w), int(fy0 * h), int(fx1 * w), int(fy1 * h)))


if __name__ == "__main__":
    main()
