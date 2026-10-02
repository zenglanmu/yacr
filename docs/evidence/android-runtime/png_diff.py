#!/usr/bin/env python3
"""Compare two PNGs of identical size and report changed-pixel fraction.

Uses the same pure-stdlib decoder as png_analyze.py.
"""
import sys
from png_analyze import read_png, rgb


def main():
    a_path, b_path = sys.argv[1], sys.argv[2]
    wa, ha, ca, cta, pa, ba = read_png(a_path)
    wb, hb, cb, ctb, pb, bb = read_png(b_path)
    assert (wa, ha, ca) == (wb, hb, cb), "size mismatch"
    changed = 0
    total = wa * ha
    # Bounding box of changes.
    minx, miny, maxx, maxy = wa, ha, -1, -1
    for y in range(ha):
        ra = y * wa
        for x in range(wa):
            pa_px = rgb(ca, cta, pa, ba, ra + x)
            pb_px = rgb(cb, ctb, pb, bb, ra + x)
            if pa_px != pb_px:
                changed += 1
                if x < minx: minx = x
                if x > maxx: maxx = x
                if y < miny: miny = y
                if y > maxy: maxy = y
    print(f"changed {changed}/{total} = {changed / total:.5f} ({changed * 100.0 / total:.2f}%)")
    if changed:
        print(f"changed bbox x=[{minx},{maxx}] y=[{miny},{maxy}]")


if __name__ == "__main__":
    main()
