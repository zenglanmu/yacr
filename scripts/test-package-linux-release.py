#!/usr/bin/env python3
"""Static contracts for the re-enabled Linux (Flatpak) release packaging.

The Linux host is the React/Tauri app at ``apps/app-linux`` (binary ``yacr``).
The packaging path was retired during the Slint removal and is now re-enabled
with a *prebuilt payload* manifest, so these tests pin the properties that make
the path coherent:

* ``packaging/flatpak`` exists with manifest + desktop entry + AppStream data;
* the manifest targets ``dev.yacr.app``, runtime ``org.gnome.Platform`` 51 and
  ``command: yacr``, ships the payload to ``/app/bin/yacr`` + ``/app/fonts``
  (the host auto-loads ``<exe dir>/../fonts``), and packages no source tree;
* the desktop entry launches ``yacr %F`` and claims the DWG/DXF media types
  (``%U`` would hand the host ``scheme://`` URIs, which it refuses);
* the script builds the frontend, builds the binary **with**
  ``tauri/custom-protocol`` (else it embeds the dev URL), and drives
  ``flatpak-builder`` + ``flatpak build-bundle`` with loud failures;
* the metainfo declares ``yacr`` and ``image/vnd.dwg``.

This is a text check: it runs no flatpak/npm/cargo command and makes no network
access. It is *not* evidence that a bundle was (or can be) built.
"""
from __future__ import annotations

import os
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-linux-release.sh"
FLATPAK_DIR = ROOT / "packaging/flatpak"
MANIFEST = FLATPAK_DIR / "dev.yacr.app.yml"
DESKTOP = FLATPAK_DIR / "dev.yacr.app.desktop"
METAINFO = FLATPAK_DIR / "dev.yacr.app.metainfo.xml"

APP_ID = "dev.yacr.app"
BINARY = "yacr"


def check_manifest(text: str) -> list[str]:
    problems: list[str] = []
    required = (
        f"app-id: {APP_ID}",
        "runtime: org.gnome.Platform",
        "runtime-version: '51'",
        "sdk: org.gnome.Sdk",
        f"command: {BINARY}",
        "buildsystem: simple",
        f"install -Dm755 bin/{BINARY} /app/bin/{BINARY}",
        "mkdir -p /app/fonts",
        "cp -a fonts/. /app/fonts/",
        "/app/share/applications/" + APP_ID + ".desktop",
        "/app/share/metainfo/" + APP_ID + ".metainfo.xml",
        "/app/share/icons/hicolor/scalable/apps/" + APP_ID + ".svg",
        "/app/share/icons/hicolor/256x256/apps/" + APP_ID + ".png",
        "type: dir",
        "path: ../../target/flatpak/payload",
    )
    for fragment in required:
        if fragment not in text:
            problems.append(f"manifest missing {fragment!r}")
    for arg in (
        "--share=ipc",
        "--socket=wayland",
        "--socket=fallback-x11",
        "--device=dri",
        "--filesystem=home",
    ):
        if arg not in text:
            problems.append(f"finish-args missing {arg!r}")
    if text.count("buildsystem:") != 1:
        problems.append("manifest must declare exactly one module buildsystem")
    if "- name: yacr" not in text:
        problems.append("manifest module must be named 'yacr'")
    # No compiling and no network sources: the payload is assembled on the host.
    for fragment in ("cargo build", "buildsystem: cargo", "type: git", "type: archive"):
        if fragment in text:
            problems.append(f"manifest must not build/compile in the sandbox: {fragment!r}")
    if re.search(r"^\s*(?:-\s*)?path:.*ui/dist", text, re.MULTILINE):
        problems.append("manifest must not ship ui/dist; the frontend is embedded in the binary")
    return problems


def check_desktop(text: str) -> list[str]:
    problems: list[str] = []
    required = (
        "Type=Application",
        "Name=Yacr",
        f"Exec={BINARY} %F",
        f"Icon={APP_ID}",
        "Terminal=false",
        "Categories=Graphics;Engineering;",
        "Keywords=",
    )
    for fragment in required:
        if fragment not in text:
            problems.append(f"desktop entry missing {fragment!r}")
    if "Exec=yacr %U" in text or "%u" in text:
        problems.append("Exec must use %F: the host refuses arguments containing '://'")
    mime_line = next(
        (line for line in text.splitlines() if line.startswith("MimeType=")), ""
    )
    for mime in ("image/vnd.dwg", "image/vnd.dxf"):
        if mime not in mime_line:
            problems.append(f"MimeType missing {mime}")
    if not text.startswith("[Desktop Entry]"):
        problems.append("desktop entry must start with [Desktop Entry]")
    return problems


def check_script(text: str) -> list[str]:
    problems: list[str] = []
    required = (
        "set -euo pipefail",
        "npm ci",
        "npm run build",
        "cargo build --release --locked -p app-linux --bin yacr",
        "--features tauri/custom-protocol",
        "flatpak-builder",
        "--install-deps-from",
        "--force-clean",
        "flatpak build-bundle",
        "target/flatpak/payload",
        ".flatpak",
        "FLATPAK_REMOTE_DEPS",
        "FLATPAK_REMOTE_BUILD",
    )
    for fragment in required:
        if fragment not in text:
            problems.append(f"packaging script missing {fragment!r}")
    # `tauri build` would run its own bundler and re-introduce a second,
    # unverified packaging path; this script drives cargo/flatpak directly.
    if re.search(r"\btauri build\b", text):
        problems.append("packaging script must not invoke 'tauri build'")
    if "NOT IMPLEMENTED" in text:
        problems.append("packaging script must not be a NOT IMPLEMENTED stub")
    # A missing prerequisite must be fatal, never a silent success.
    if "die()" not in text or "exit 1" not in text:
        problems.append("packaging script must fail loudly (die/exit 1)")
    return problems


def check_metainfo(text: str) -> list[str]:
    problems: list[str] = []
    required = (
        f"<id>{APP_ID}</id>",
        '<launchable type="desktop-id">' + APP_ID + ".desktop</launchable>",
        f"<binary>{BINARY}</binary>",
        "<mediatype>image/vnd.dwg</mediatype>",
        "<mediatype>image/vnd.dxf</mediatype>",
        "<project_license>AGPL-3.0</project_license>",
        "<metadata_license>CC0-1.0</metadata_license>",
        '<release version="0.1.2"',
        "<name>Yacr</name>",
        "<summary>",
        "<description>",
    )
    for fragment in required:
        if fragment not in text:
            problems.append(f"metainfo missing {fragment!r}")
    return problems


class PackageLinuxReleaseContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest_text = MANIFEST.read_text(encoding="utf-8")
        self.desktop_text = DESKTOP.read_text(encoding="utf-8")
        self.metainfo_text = METAINFO.read_text(encoding="utf-8")
        self.script_text = SCRIPT.read_text(encoding="utf-8")

    def test_flathub_files_exist(self) -> None:
        for path in (MANIFEST, DESKTOP, METAINFO):
            with self.subTest(path=path.name):
                self.assertTrue(path.is_file(), f"missing packaging file: {path}")

    def test_manifest_declares_the_react_host(self) -> None:
        self.assertEqual(check_manifest(self.manifest_text), [])

    def test_manifest_rejects_a_sandbox_build(self) -> None:
        mutated = self.manifest_text + "\n      - cargo build --release\n"
        self.assertNotEqual(check_manifest(mutated), [])

    def test_manifest_rejects_shipping_ui_dist(self) -> None:
        mutated = self.manifest_text + "      - path: ../../ui/dist\n"
        self.assertNotEqual(check_manifest(mutated), [])

    def test_desktop_entry_launches_the_host(self) -> None:
        self.assertEqual(check_desktop(self.desktop_text), [])

    def test_desktop_entry_rejects_uri_arguments(self) -> None:
        mutated = self.desktop_text.replace("Exec=yacr %F", "Exec=yacr %U")
        self.assertNotEqual(check_desktop(mutated), [])

    def test_metainfo_declares_appstream_identity(self) -> None:
        self.assertEqual(check_metainfo(self.metainfo_text), [])

    def test_script_is_a_live_packaging_path(self) -> None:
        self.assertEqual(check_script(self.script_text), [])

    def test_script_is_executable(self) -> None:
        self.assertTrue(os.access(SCRIPT, os.X_OK), "packaging script must be executable")

    def test_script_requires_custom_protocol(self) -> None:
        mutated = self.script_text.replace("--features tauri/custom-protocol", "")
        self.assertNotEqual(check_script(mutated), [])

    def test_binary_names_agree_across_files(self) -> None:
        # command / Exec / <binary> must name the same executable, else the
        # desktop entry and AppStream data would launch nothing.
        exec_line = next(
            line for line in self.desktop_text.splitlines() if line.startswith("Exec=")
        )
        self.assertEqual(exec_line.split()[0], "Exec=" + BINARY)
        self.assertIn(f"command: {BINARY}", self.manifest_text)
        self.assertIn(f"<binary>{BINARY}</binary>", self.metainfo_text)


if __name__ == "__main__":
    unittest.main()
