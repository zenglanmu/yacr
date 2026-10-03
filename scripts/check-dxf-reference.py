#!/usr/bin/env python3
"""Offscreen DXF render check against a committed reference image.

This drives the *real* release CLI through ``scan`` -> ``build-representation``
-> ``render`` (native wgpu/Vulkan, normally Mesa lavapipe) on a DXF fixture and
checks that the pipeline still produces the expected structure and a non-empty
frame. It then decodes the rendered PNG and the committed reference PNG and
reports coarse ink-coverage / bounding-box statistics plus side-by-side and
overlay images for *human* review.

It deliberately does **not** assert a pixel-fidelity score (no SSIM/IoU
acceptance): the reference was rendered by a different program with a different
viewport, palette and margin convention, and this renderer is known to omit
elements the reference contains. The hard assertions are structural
(import/representation/frame smoke); graphical correctness is a manual call made
from the emitted artefacts and recorded in docs/.

Usage:
  python3 scripts/check-dxf-reference.py \
      fixtures/dxf/qcad-flange/flange.dxf \
      --reference fixtures/dxf/qcad-flange/flange.png \
      --out /tmp/opencode/yacr-dxf-reference

Stdlib only (zlib/struct): no Pillow/numpy dependency. Non-zero exit on a failed
hard assertion; reference statistics are reported, never invented.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
import subprocess
import time
import zlib
from collections import Counter
from pathlib import Path

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
DEFAULT_ICD = "/usr/share/vulkan/icd.d/lvp_icd.json"


# ---------------------------------------------------------------------------
# Minimal 8-bit non-interlaced PNG reader/writer
# ---------------------------------------------------------------------------

_CHANNELS = {0: 1, 2: 3, 4: 2, 6: 4}


def read_png(path: Path):
    """Return (width, height, channels, bytearray) for an 8-bit PNG."""
    data = path.read_bytes()
    if data[:8] != PNG_SIGNATURE:
        raise ValueError(f"{path}: not a PNG")
    pos = 8
    idat = bytearray()
    width = height = depth = color = None
    while pos + 8 <= len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, depth, color, comp, filt, interlace = struct.unpack(
                ">IIBBBBB", chunk
            )
            if depth != 8 or interlace != 0 or color not in _CHANNELS:
                raise ValueError(
                    f"{path}: unsupported PNG (depth={depth} color={color} "
                    f"interlace={interlace})"
                )
        elif kind == b"IDAT":
            idat += chunk
        elif kind == b"IEND":
            break
    if width is None:
        raise ValueError(f"{path}: missing IHDR")
    channels = _CHANNELS[color]
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    pixels = bytearray(width * height * channels)
    prev = bytearray(stride)
    src = 0
    for y in range(height):
        ftype = raw[src]
        src += 1
        line = bytearray(raw[src : src + stride])
        src += stride
        if ftype == 1:
            for x in range(channels, stride):
                line[x] = (line[x] + line[x - channels]) & 0xFF
        elif ftype == 2:
            for x in range(stride):
                line[x] = (line[x] + prev[x]) & 0xFF
        elif ftype == 3:
            for x in range(stride):
                left = line[x - channels] if x >= channels else 0
                line[x] = (line[x] + ((left + prev[x]) >> 1)) & 0xFF
        elif ftype == 4:
            for x in range(stride):
                a = line[x - channels] if x >= channels else 0
                b = prev[x]
                c = prev[x - channels] if x >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 0xFF
        elif ftype != 0:
            raise ValueError(f"{path}: unknown filter {ftype}")
        pixels[y * stride : (y + 1) * stride] = line
        prev = line
    return width, height, channels, pixels


def write_png(path: Path, width: int, height: int, pixels: bytearray) -> None:
    """Write an 8-bit RGBA PNG from a flat bytearray."""

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    stride = width * 4
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw += pixels[y * stride : (y + 1) * stride]
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    path.write_bytes(
        PNG_SIGNATURE
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 6))
        + chunk(b"IEND", b"")
    )


# ---------------------------------------------------------------------------
# Image statistics
# ---------------------------------------------------------------------------

Ink = tuple  # (mask: bytearray, width, height, bbox, coverage, colors)


def ink_mask(width: int, height: int, channels: int, pixels: bytearray, threshold: int = 48) -> Ink:
    """Foreground = pixels far from the most common (background) colour."""
    step = max(1, (width * height) // 200000)
    samples = Counter()
    for i in range(0, width * height, step):
        o = i * channels
        samples[tuple(pixels[o : o + channels])] += 1
    bg = samples.most_common(1)[0][0]
    mask = bytearray(width * height)
    minx, miny, maxx, maxy = width, height, -1, -1
    colors = set()
    count = 0
    for y in range(height):
        row = y * width
        for x in range(width):
            o = (row + x) * channels
            dist = 0
            for c in range(min(3, channels)):
                dist += abs(pixels[o + c] - bg[c])
            if channels == 4 and pixels[o + 3] < 16:
                continue
            if dist > threshold:
                mask[row + x] = 1
                count += 1
                colors.add(tuple(pixels[o : o + channels]))
                if x < minx:
                    minx = x
                if x > maxx:
                    maxx = x
                if y < miny:
                    miny = y
                if y > maxy:
                    maxy = y
    bbox = None if maxx < 0 else (minx, miny, maxx, maxy)
    return mask, width, height, bbox, count / (width * height), colors


def normalized_bbox(bbox, width, height):
    if bbox is None:
        return None
    return tuple(round(v, 4) for v in (bbox[0] / width, bbox[1] / height, bbox[2] / width, bbox[3] / height))


def downsample(mask: bytearray, width: int, height: int, gw: int = 64, gh: int = 48) -> list:
    grid = [0] * (gw * gh)
    for y in range(height):
        gy = min(gh - 1, y * gh // height)
        row = y * width
        for x in range(width):
            if mask[row + x]:
                grid[gy * gw + min(gw - 1, x * gw // width)] += 1
    return grid


# ---------------------------------------------------------------------------
# CLI phases
# ---------------------------------------------------------------------------

def run_cli(cli: str, operation: str, sample: Path, env: dict, extra: list) -> dict:
    command = [cli, operation, str(sample), *extra]
    started = time.monotonic()
    proc = subprocess.run(command, env=env, capture_output=True, text=True, timeout=660)
    elapsed = round(time.monotonic() - started, 3)
    if proc.returncode:
        raise RuntimeError(
            f"{operation} exited {proc.returncode}: {proc.stderr.strip()}"
        )
    try:
        document = json.loads(proc.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"{operation} stdout is not JSON: {error}") from error
    document["_seconds"] = elapsed
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("input", nargs="?", default="fixtures/dxf/qcad-flange/flange.dxf", type=Path)
    parser.add_argument("--reference", type=Path, default=Path("fixtures/dxf/qcad-flange/flange.png"))
    parser.add_argument("--out", type=Path, default=Path("/tmp/opencode/yacr-dxf-reference"))
    parser.add_argument("--cli", default="target/release/cad-cli-tools")
    parser.add_argument("--width", type=int, default=1024)
    parser.add_argument("--height", type=int, default=768)
    parser.add_argument("--icd", default=DEFAULT_ICD)
    parser.add_argument("--any-adapter", action="store_true",
                        help="accept any adapter instead of requiring CPU/llvmpipe")
    parser.add_argument("--font", action="append", default=[],
                        help="register a CAD text font for build/render as name=path "
                             "(repeatable); without it text stays an unresolved placeholder")
    parser.add_argument("--any-units", action="store_true",
                        help="accept any drawing unit instead of requiring Millimetre")
    args = parser.parse_args()

    sample = args.input
    if not sample.is_file():
        parser.error(f"input DXF not found: {sample}")
    args.out.mkdir(parents=True, exist_ok=True)

    summary: dict = {
        "input": str(sample),
        "input_sha256": hashlib.sha256(sample.read_bytes()).hexdigest(),
        "size": [args.width, args.height],
        "icd": args.icd,
        "fonts": args.font,
        "reference": str(args.reference) if args.reference.is_file() else None,
    }
    font_args = [item for font in args.font for item in ("--font", font)]
    env = dict(os.environ)
    if args.icd:
        env["VK_ICD_FILENAMES"] = args.icd

    try:
        scan = run_cli(args.cli, "scan", sample, env, [])
        (args.out / "scan.json").write_text(json.dumps(scan, indent=2))
        assert scan["operation"] == "scan" and scan["schema_version"] == 1
        assert scan["entities"] > 0, "no entities imported"
        assert scan["model_entities"] > 0, "no model-space entities"
        if not args.any_units:
            assert scan["units"] == "Millimeter", f"unexpected units {scan['units']!r}"
        completeness = scan["completeness"]
        assert completeness["status"] in {"complete", "partial"}, completeness
        if completeness["status"] == "partial":
            assert completeness["items"], "Partial without a reason"
        summary["scan"] = {
            "entities": scan["entities"],
            "model_entities": scan["model_entities"],
            "layers": scan["layers"],
            "units": scan["units"],
            "completeness": completeness,
            "seconds": scan["_seconds"],
        }

        build = run_cli(args.cli, "build-representation", sample, env, font_args)
        (args.out / "build-representation.json").write_text(json.dumps(build, indent=2))
        assert build["failures"] == [], f"representation failures: {build['failures']}"
        assert build["primitives"] > 0, "empty representation"
        assert sum(build["kind_counts"].values()) > 0
        summary["build_representation"] = {
            "primitives": build["primitives"],
            "vertices": build["vertices"],
            "kind_counts": build["kind_counts"],
            "seconds": build["_seconds"],
        }

        png = args.out / f"{sample.stem}.png"
        render = run_cli(
            args.cli,
            "render",
            sample,
            env,
            ["--png", str(png), "--width", str(args.width), "--height", str(args.height), *font_args],
        )
        (args.out / "render.json").write_text(json.dumps(render, indent=2))
        adapter = render["adapter"]
        assert adapter["backend"], "adapter.backend must be non-empty"
        if not args.any_adapter:
            assert adapter["backend"] == "vulkan", f"expected Vulkan, got {adapter}"
            assert adapter["device_type"] == "cpu", f"expected CPU adapter, got {adapter}"
            assert adapter["driver"] == "llvmpipe", f"expected llvmpipe, got {adapter}"
        assert render["pixels"]["non_background"] > 0, "blank frame"
        assert render["png"]["bytes"] > 0 and png.read_bytes()[:8] == PNG_SIGNATURE
        summary["render"] = {
            "adapter": adapter,
            "pixels": render["pixels"],
            "frame": render["frame"],
            "seconds": render["_seconds"],
        }

        # ---- reference comparison (coarse, human-reviewed) ----
        rendered = read_png(png)
        rmusk = ink_mask(*rendered)
        summary["rendered_stats"] = {
            "size": [rendered[0], rendered[1]],
            "coverage": round(rmusk[4], 6),
            "bbox_normalized": normalized_bbox(rmusk[3], rendered[0], rendered[1]),
            "distinct_ink_colors": len(rmusk[5]),
        }
        # A frame that is blank or a solid fill is a bug regardless of the reference.
        assert 0.0005 < rmusk[4] < 0.5, f"implausible ink coverage {rmusk[4]:.4f}"

        if args.reference.is_file():
            ref = read_png(args.reference)
            fmusk = ink_mask(*ref)
            grid_r = downsample(rmusk[0], rendered[0], rendered[1])
            grid_f = downsample(fmusk[0], ref[0], ref[1])
            inter = sum(1 for a, b in zip(grid_r, grid_f) if a and b)
            union = sum(1 for a, b in zip(grid_r, grid_f) if a or b)
            covered = sum(1 for a in grid_r if a)
            summary["reference_stats"] = {
                "path": str(args.reference),
                "sha256": hashlib.sha256(args.reference.read_bytes()).hexdigest(),
                "size": [ref[0], ref[1]],
                "coverage": round(fmusk[4], 6),
                "bbox_normalized": normalized_bbox(fmusk[3], ref[0], ref[1]),
                "coarse_iou_64x48": round(inter / union, 4) if union else None,
                "rendered_hits_reference": round(inter / covered, 4) if covered else None,
                "note": "different viewport/palette/added sheet elements; not a fidelity score",
            }
            _write_review_images(args.out, ref, fmusk, rendered, rmusk)
            summary["visual_acceptance"] = (
                "manual: compare rendered vs reference ignoring colour; the importer "
                "marks AcDbDimension/AcDbMText Partial, so annotations and title-block "
                "text are expected to be absent"
            )
        else:
            summary["reference_stats"] = None
            summary["visual_acceptance"] = "NOT RUN: reference image not found"

        summary["result"] = "passed"
    except Exception as error:  # noqa: BLE001 - report, do not fake success
        summary["result"] = "failed"
        summary["error"] = str(error)
        (args.out / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False))
        print(json.dumps(summary, indent=2, ensure_ascii=False))
        return 1

    (args.out / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False))
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0


def _write_review_images(out: Path, ref, fmusk, rendered, rmusk) -> None:
    """Side-by-side (reference | render) and overlay, all binarized."""
    rw, rh = rendered[0], rendered[1]
    fw, fh = ref[0], ref[1]
    # Normalise to the rendered canvas size for the side-by-side.
    grid_f = downsample(fmusk[0], fw, fh, rw, rh)
    gap = 16
    side = bytearray((rw * 2 + gap) * rh * 4)
    for i in range((rw * 2 + gap) * rh):
        side[i * 4 : i * 4 + 4] = bytes((255, 255, 255, 255))
    for y in range(rh):
        for x in range(rw):
            if grid_f[y * rw + x]:
                o = (y * (rw * 2 + gap) + x) * 4
                side[o : o + 4] = bytes((0, 0, 0, 255))
            if rmusk[0][y * rw + x]:
                o = (y * (rw * 2 + gap) + rw + gap + x) * 4
                side[o : o + 4] = bytes((0, 0, 0, 255))
    write_png(out / "review-side-by-side.png", rw * 2 + gap, rh, side)

    overlay = bytearray(rw * rh * 4)
    for i in range(rw * rh):
        refing = grid_f[i]
        rening = rmusk[0][i]
        if refing and rening:
            color = (0, 0, 0, 255)
        elif refing:
            color = (255, 120, 120, 255)
        elif rening:
            color = (120, 120, 255, 255)
        else:
            color = (255, 255, 255, 255)
        overlay[i * 4 : i * 4 + 4] = bytes(color)
    write_png(out / "review-overlay.png", rw, rh, overlay)


if __name__ == "__main__":
    raise SystemExit(main())
