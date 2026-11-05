#!/usr/bin/env python3
"""Text contracts for the live macOS release packaging path.

``apps/app-macos`` is a real Tauri 2 host (transparent WKWebView + native CAD
NSView presented by wgpu/Metal) with release executable ``yacr-macos``, so
``scripts/package-macos-release.sh`` must be a real, fail-closed pipeline rather
than the retirement stub it used to be. These tests pin the fragments that make
that pipeline coherent:

* macOS-only guard, architecture selection (universal via ``lipo``, arm64,
  x86_64) and installed rust-target validation;
* the React frontend is built before ``cargo build`` because
  ``tauri::generate_context!`` embeds ``apps/desktop-ui/dist`` at compile time;
* ``cargo build --release --locked`` for ``cad-cli-tools`` and ``app-macos``;
* ``Yacr.app`` staging with ``io.github.zenglanmu.yacr``, ``yacr-macos``, dwg/dxf document
  types and macOS 11.0;
* Mach-O architecture checks via ``lipo -archs`` and system-only linkage via
  ``otool -L`` (no ``@rpath``/Homebrew dylibs);
* font assembly through ``scripts/fetch-fonts.sh`` (or committed ``fonts/``
  with ``fonts.json``), and tar.gz + sha256 artifacts under
  ``target/macos/<arch>/dist``.

This is a text check only: it runs no cargo/npm/tar/lipo command and makes no
network access. It is *not* evidence that a package was (or can be) built; the
script refuses to run anywhere but macOS.
"""
from __future__ import annotations

import json
import os
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-macos-release.sh"

APP_ID = "io.github.zenglanmu.yacr"
BINARY = "yacr-macos"
CJK_FALLBACK = "NotoSansCJKsc-Regular.otf"

# Each (label, literal) pair is a required fragment of the live pipeline. The
# mutation tests delete them one at a time, so a silent regression is reported.
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("fail-closed shell options", "set -euo pipefail"),
    ("loud failure", "exit 1"),
    ("macOS-only guard (uname)", "uname -s"),
    ("macOS-only guard (Darwin)", "Darwin"),
    ("architecture selection", "MACOS_ARCH"),
    ("universal target arm64", "aarch64-apple-darwin"),
    ("universal target x86_64", "x86_64-apple-darwin"),
    ("installed rust-target validation", "rustup target list --installed"),
    ("frontend install", "npm ci"),
    ("frontend build", "npm run build"),
    ("frontend reuse guard", "SKIP_UI_BUILD"),
    ("frontend entry point", "index.html"),
    ("vendor plugin bundle", "plugins/tch.js"),
    ("locked release build", "cargo build --release --locked"),
    ("embedded frontend protocol", "tauri/custom-protocol"),
    ("macOS host crate", "-p app-macos"),
    ("headless CLI crate", "-p cad-cli-tools"),
    ("macOS executable", BINARY),
    ("universal binary combine", "lipo -create"),
    ("Mach-O architecture check", "lipo -archs"),
    ("Mach-O dependency inspection", "otool -L"),
    ("system-only dependency root /usr/lib", "/usr/lib/"),
    ("system-only dependency root /System/Library", "/System/Library/"),
    (
        "bundle identifier",
        f"<key>CFBundleIdentifier</key><string>{APP_ID}</string>",
    ),
    (
        "bundle executable",
        f"<key>CFBundleExecutable</key><string>{BINARY}</string>",
    ),
    ("document types", "<string>dwg</string><string>dxf</string>"),
    ("minimum system version", "<key>LSMinimumSystemVersion</key><string>11.0</string>"),
    ("app icon", "assets/yacr-icon.icns"),
    ("font catalogue assembly", "fetch-fonts.sh"),
    ("font catalogue contract", "fonts.json"),
    ("font toggle", "WITH_FONTS"),
    ("artifact staging root", "target/macos/${ARCH_LABEL}/dist"),
    ("archive", "tar -czf"),
    ("checksum", "sha256"),
    ("unsigned disclosure", "unsigned"),
    ("notarization disclosure", "not notarized"),
    ("packaging manifest", "PACKAGE.txt"),
)


def check_live_script(text: str) -> list[str]:
    problems: list[str] = []
    for label, fragment in REQUIRED_FRAGMENTS:
        if fragment not in text:
            problems.append(f"missing {label}: {fragment!r}")
    if "NOT IMPLEMENTED" in text:
        problems.append("packaging script must not be a NOT IMPLEMENTED stub")
    if "invalid.example" in text:
        problems.append("bundle identifier must not use invalid.example")
    if re.search(r"slint", text, re.IGNORECASE):
        problems.append("packaging script must not reference the retired Slint host")
    # The frontend is embedded by tauri::generate_context!, so it must be built
    # before cargo starts compiling the host.
    build = text.find("npm run build")
    cargo = text.find("cargo build --release --locked")
    if build == -1 or cargo == -1 or build > cargo:
        problems.append("the React frontend must be built before cargo build")
    return problems


class PackageMacosReleaseLiveContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.script = SCRIPT.read_text(encoding="utf-8")

    def test_script_is_a_live_packaging_pipeline(self) -> None:
        self.assertEqual(check_live_script(self.script), [])

    def test_script_is_executable(self) -> None:
        self.assertTrue(os.access(SCRIPT, os.X_OK), "packaging script must be executable")

    def test_not_implemented_reintroduction_is_reported(self) -> None:
        problems = check_live_script(self.script + "\n# NOT IMPLEMENTED\n")
        self.assertTrue(any("NOT IMPLEMENTED" in problem for problem in problems))

    def test_slint_reference_is_reported(self) -> None:
        problems = check_live_script(self.script + "\n# Slint host\n")
        self.assertTrue(any("Slint" in problem for problem in problems))

    def test_cf_bundle_identifier_must_stay_io_github_zenglanmu_yacr(self) -> None:
        mutated = self.script.replace(APP_ID, "invalid.example.yacr")
        problems = check_live_script(mutated)
        self.assertNotEqual(problems, [])
        self.assertTrue(
            any(APP_ID in problem or "CFBundleIdentifier" in problem for problem in problems)
        )

    def test_each_required_fragment_is_required(self) -> None:
        for label, fragment in REQUIRED_FRAGMENTS:
            with self.subTest(label=label):
                mutated = self.script.replace(fragment, "")
                problems = check_live_script(mutated)
                self.assertNotEqual(
                    problems, [], f"mutation not detected: {label}"
                )
                self.assertTrue(
                    any(fragment in problem for problem in problems),
                    f"mutation did not report the removed fragment: {label}",
                )

    def test_frontend_build_before_cargo_is_required(self) -> None:
        # Put a cargo build ahead of the frontend build; the ordering check must
        # report it even though both fragments are still present.
        mutated = "cargo build --release --locked\n" + self.script
        problems = check_live_script(mutated)
        self.assertTrue(
            any("before cargo build" in problem for problem in problems),
            f"ordering mutation not detected: {problems}",
        )

    def test_staged_fonts_expect_the_committed_cjk_fallback(self) -> None:
        # The app bundle's Resources/fonts is filled from the committed package
        # (WITH_FONTS=1 merges the fetched catalogue into it, WITH_FONTS=0 copies
        # it offline), so the Noto CJK fallback must be in that package.
        entries = json.loads(
            (ROOT / "fonts" / "fonts.json").read_text(encoding="utf-8")
        )
        self.assertIn(CJK_FALLBACK, {entry.get("file") for entry in entries})
        self.assertTrue((ROOT / "fonts" / CJK_FALLBACK).is_file())
        self.assertIn("fetch-fonts.sh", self.script)
        self.assertIn('cp -R "${ROOT}/fonts/." "$FONTS_STAGE/"', self.script)
        self.assertIn('[ -s "${FONTS_STAGE}/fonts.json" ]', self.script)


if __name__ == "__main__":
    unittest.main()
