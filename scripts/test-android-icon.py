#!/usr/bin/env python3
"""Static contract for the Android launcher icon wiring.

The APK is built by ``cargo apk`` with the Android SDK/NDK, so it cannot be
built here. This guard rail instead asserts the pieces cargo-apk consumes:

  * ``[package.metadata.android] resources`` points at the ``res/`` folder;
  * ``[package.metadata.android.application]`` sets ``icon`` and ``label``;
  * ``res/`` holds a launcher PNG at every density with the expected pixel size;
  * an adaptive-icon definition (API 26+) and its background colour exist.

It checks files and metadata only; it is not evidence that an APK was built or
that the icon renders on a device.
"""
from __future__ import annotations

import pathlib
import struct
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
CARGO = ROOT / "apps" / "app-android" / "Cargo.toml"
RES = ROOT / "apps" / "app-android" / "res"

DENSITIES = {
    "mdpi": 1.0,
    "hdpi": 1.5,
    "xhdpi": 2.0,
    "xxhdpi": 3.0,
    "xxxhdpi": 4.0,
}


def android_metadata(text: str) -> dict:
    return tomllib.loads(text)["package"]["metadata"]["android"]


def check_metadata(text: str) -> list[str]:
    """Return one problem string per missing manifest-icon guarantee."""
    android = android_metadata(text)
    application = android.get("application", {})
    problems = []
    if android.get("resources") != "res":
        problems.append("resources must point at res/")
    if application.get("icon") != "@mipmap/ic_launcher":
        problems.append("application.icon must be @mipmap/ic_launcher")
    if not application.get("label"):
        problems.append("application.label must be set")
    return problems


def png_size(path: pathlib.Path) -> tuple[int, int]:
    """Read the IHDR width/height without any image library."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


class AndroidIconContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.text = CARGO.read_text(encoding="utf-8")

    def test_manifest_points_at_resources_and_launcher_icon(self) -> None:
        self.assertEqual(check_metadata(self.text), [])
        self.assertTrue(android_metadata(self.text)["application"]["label"])

    def test_launcher_pngs_exist_at_every_density(self) -> None:
        for qualifier, scale in DENSITIES.items():
            with self.subTest(density=qualifier):
                path = RES / f"mipmap-{qualifier}" / "ic_launcher.png"
                self.assertTrue(path.is_file(), f"missing {path.relative_to(ROOT)}")
                self.assertEqual(png_size(path), (round(48 * scale),) * 2)

    def test_adaptive_foreground_pngs_are_sized_to_the_safe_zone(self) -> None:
        for qualifier, scale in DENSITIES.items():
            with self.subTest(density=qualifier):
                path = RES / f"mipmap-{qualifier}" / "ic_launcher_foreground.png"
                self.assertTrue(path.is_file(), f"missing {path.relative_to(ROOT)}")
                self.assertEqual(png_size(path), (round(108 * scale),) * 2)

    def test_adaptive_icon_and_colour_resources_exist(self) -> None:
        adaptive = RES / "mipmap-anydpi-v26" / "ic_launcher.xml"
        colors = RES / "values" / "colors.xml"
        self.assertTrue(adaptive.is_file(), "missing adaptive-icon definition")
        self.assertTrue(colors.is_file(), "missing colours resource")
        adaptive_text = adaptive.read_text(encoding="utf-8")
        self.assertIn("@color/ic_launcher_background", adaptive_text)
        self.assertIn("@mipmap/ic_launcher_foreground", adaptive_text)
        self.assertIn('name="ic_launcher_background"', colors.read_text(encoding="utf-8"))

    def test_each_manifest_guarantee_is_required(self) -> None:
        for fragment in ('icon = "@mipmap/ic_launcher"', 'resources = "res"', 'label = "yacr"'):
            with self.subTest(fragment=fragment):
                mutated = self.text.replace(fragment, "")
                self.assertNotEqual(check_metadata(mutated), [], f"mutation not detected: {fragment}")


if __name__ == "__main__":
    unittest.main()
