#!/usr/bin/env python3
# generate-icons.py -- rasterise the master app icon into per-platform assets.
#
# Source of truth (committed, never generated):
#   assets/yacr-icon.svg
# Generated (committed so packaging/CI needs no rasteriser at build time):
#   assets/yacr-icon.ico                        multi-resolution Windows icon
#   assets/yacr-icon.icns                       multi-resolution macOS icon
#   assets/icons/yacr-<size>.png                PNG set for the Linux hicolor
#                                               icon theme
#
# Android launcher art is **not** generated here any more: the Android host is a
# Tauri 2 app whose launcher icons come from `apps/app-android/src-tauri/icons/`
# (the shared PNG set below), and `tauri android init` turns them into the
# `gen/android` mipmaps. The cargo-apk `res/` tree this script used to write was
# deleted with that pipeline (docs/android-host.md).
#
# Requires `rsvg-convert` (librsvg2-bin) and Pillow:
#   sudo apt-get install -y librsvg2-bin
#   python3 -m pip install --user Pillow
#
# Usage: python3 scripts/generate-icons.py [--check]
#   --check  regenerate into a temp tree and fail if any committed asset
#            differs (CI-friendly drift guard).
import argparse
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SVG = ROOT / "assets" / "yacr-icon.svg"
ASSETS = ROOT / "assets"

PNG_SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]

def render_pngs(svg: pathlib.Path, outdir: pathlib.Path, sizes):
    pngs = {}
    for size in sizes:
        path = outdir / f"{size}.png"
        subprocess.run(
            ["rsvg-convert", "-w", str(size), "-h", str(size), str(svg), "-o", str(path)],
            check=True,
        )
        pngs[size] = path
    return pngs


def write_text(path: pathlib.Path, text: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def build(assets: pathlib.Path):
    """Render every derivative under the given assets root."""
    from PIL import Image

    (assets / "icons").mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        pngs = render_pngs(SVG, pathlib.Path(tmp), PNG_SIZES)
        Image.open(pngs[256]).convert("RGBA").save(
            assets / "yacr-icon.ico", format="ICO", sizes=[(s, s) for s in ICO_SIZES]
        )
        Image.open(pngs[1024]).convert("RGBA").save(assets / "yacr-icon.icns", format="ICNS")
        for size in PNG_SIZES:
            Image.open(pngs[size]).convert("RGBA").save(assets / "icons" / f"yacr-{size}.png")


def committed_paths():
    paths = [ASSETS / "yacr-icon.ico", ASSETS / "yacr-icon.icns"]
    paths += [ASSETS / "icons" / f"yacr-{s}.png" for s in PNG_SIZES]
    return paths


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    if not SVG.is_file():
        print(f"missing source icon: {SVG}", file=sys.stderr)
        return 1
    if subprocess.run(["which", "rsvg-convert"], capture_output=True).returncode != 0:
        print("rsvg-convert not found; install librsvg2-bin", file=sys.stderr)
        return 1

    if not args.check:
        build(ASSETS)
        for path in committed_paths():
            print(f"{path.relative_to(ROOT)}  {path.stat().st_size} bytes")
        return 0

    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        build(tmp / "assets")
        drift = []
        for path in committed_paths():
            fresh = tmp / path.relative_to(ROOT)
            if not path.is_file() or path.read_bytes() != fresh.read_bytes():
                drift.append(str(path.relative_to(ROOT)))
        if drift:
            print("icon assets out of date: " + ", ".join(drift), file=sys.stderr)
            return 1
        print("icon assets up to date")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
