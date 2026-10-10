#!/usr/bin/env python3
"""Static + mutation contracts for the Linux Flatpak packaging.

The packaging script is a heavy, host-specific release build (it links the Slint
GUI, assembles the font catalogue and runs flatpak-builder), so it is not run in
this checker. Instead this guard rail asserts the packaging keeps its guarantees:

  * scripts/package-linux-release.sh builds and stages *both* the headless CLI
    and the GUI host, then emits a single-file Flatpak bundle;
  * the Flatpak manifest packages the prebuilt binaries under /app, points at the
    freedesktop runtime/sdk, and keeps the GPU/display finish-args;
  * the desktop entry, AppStream metainfo and icons all use the manifest app-id;
  * the script never falls back to the old tarball.

Each guarantee is deleted in turn and the checker must report it, so the contract
cannot silently degrade into an empty success. This checks text only; it is not
evidence that a bundle was built, installed or that its GUI renders.
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-linux-release.sh"
MANIFEST = ROOT / "packaging/flatpak/dev.yacr.app.yml"
DESKTOP = ROOT / "packaging/flatpak/dev.yacr.app.desktop"
METAINFO = ROOT / "packaging/flatpak/dev.yacr.app.metainfo.xml"

APP_ID = "dev.yacr.app"

# (human label, fragment that must appear in the packaging script).
SCRIPT_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("headless CLI build", "cargo build --release --offline --locked -p cad-cli-tools"),
    ("GUI host build", "cargo build --release --offline --locked -p app-linux --bin yacr-linux"),
    ("stage CLI binary", 'cp "$BIN" "$PAYLOAD/bin/cad-cli-tools"'),
    ("stage GUI binary", 'cp "$GUI" "$PAYLOAD/bin/yacr-linux"'),
    ("assemble CAD fonts", '"$ROOT/scripts/fetch-fonts.sh" "$PAYLOAD/fonts"'),
    ("prepared font dir override", 'FONTS_DIR must contain fonts.json'),
    ("copy prepared fonts", 'cp -R "$FONTS_DIR"/. "$PAYLOAD/fonts/"'),
    ("verify GUI loads", "--headless requires --output NEW_DIRECTORY"),
    ("refuse unresolved libraries", "grep -q 'not found'"),
    ("flatpak-builder required", "flatpak-builder not found"),
    ("flatpak-builder invocation", '--repo="$REPO"'),
    ("single-file bundle", 'flatpak build-bundle "$REPO" "$BUNDLE" "$APP_ID"'),
    ("app id", f'APP_ID="{APP_ID}"'),
    ("committed manifest path", 'MANIFEST="packaging/flatpak/dev.yacr.app.yml"'),
    ("optional DWG smoke", "YACR_TEST_DWG"),
    ("optional sandbox smoke", "YACR_FLATPAK_SMOKE"),
    ("flatpak artifact", 'BUNDLE="${DIST}/${NAME}.flatpak"'),
)

# (human label, fragment that must appear in the Flatpak manifest).
MANIFEST_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("app id", f"app-id: {APP_ID}"),
    ("runtime", "runtime: org.freedesktop.Platform"),
    ("runtime version", "runtime-version: '26.08'"),
    ("sdk", "sdk: org.freedesktop.Sdk"),
    ("command", "command: yacr-linux"),
    ("ipc share", "--share=ipc"),
    ("wayland socket", "--socket=wayland"),
    ("x11 fallback socket", "--socket=fallback-x11"),
    ("dri device", "--device=dri"),
    ("home filesystem", "--filesystem=home"),
    ("install GUI", "install -Dm755 bin/yacr-linux /app/bin/yacr-linux"),
    ("install CLI", "install -Dm755 bin/cad-cli-tools /app/bin/cad-cli-tools"),
    ("install fonts", "- cp -a fonts/. /app/fonts/"),
    ("install desktop", f"/app/share/applications/{APP_ID}.desktop"),
    ("install metainfo", f"/app/share/metainfo/{APP_ID}.metainfo.xml"),
    ("install scalable icon", f"/app/share/icons/hicolor/scalable/apps/{APP_ID}.svg"),
    ("install png icon", f"/app/share/icons/hicolor/256x256/apps/{APP_ID}.png"),
    ("prebuilt payload source", "path: ../../target/flatpak/payload"),
)


def check_script(text: str) -> list[str]:
    """Return one problem string per missing packaging guarantee."""
    problems = [
        f"missing {label}: {fragment!r}"
        for label, fragment in SCRIPT_FRAGMENTS
        if fragment not in text
    ]
    if ".tar.gz" in text:
        problems.append("script still references a .tar.gz artifact")
    return problems


def check_manifest(text: str) -> list[str]:
    return [
        f"missing {label}: {fragment!r}"
        for label, fragment in MANIFEST_FRAGMENTS
        if fragment not in text
    ]


class PackageLinuxFlatpakContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.script = SCRIPT.read_text(encoding="utf-8")
        self.manifest = MANIFEST.read_text(encoding="utf-8")
        self.desktop = DESKTOP.read_text(encoding="utf-8")
        self.metainfo = METAINFO.read_text(encoding="utf-8")

    def test_script_satisfies_every_guarantee(self) -> None:
        self.assertEqual(check_script(self.script), [])

    def test_each_script_guarantee_is_required(self) -> None:
        for label, fragment in SCRIPT_FRAGMENTS:
            with self.subTest(guarantee=label):
                mutated = self.script.replace(fragment, "")
                self.assertNotEqual(
                    check_script(mutated), [], f"mutation not detected: {label}"
                )

    def test_gui_binary_cannot_be_dropped_silently(self) -> None:
        mutated = self.script.replace('cp "$GUI" "$PAYLOAD/bin/yacr-linux"', "")
        problems = check_script(mutated)
        self.assertTrue(problems)
        self.assertTrue(any("GUI" in problem for problem in problems))

    def test_tarball_cannot_creep_back(self) -> None:
        mutated = self.script + '\ntar -C "$DIST" -czf x.tar.gz "$NAME"\n'
        self.assertTrue(any(".tar.gz" in p for p in check_script(mutated)))

    def test_manifest_satisfies_every_guarantee(self) -> None:
        self.assertEqual(check_manifest(self.manifest), [])

    def test_each_manifest_guarantee_is_required(self) -> None:
        for label, fragment in MANIFEST_FRAGMENTS:
            with self.subTest(guarantee=label):
                mutated = self.manifest.replace(fragment, "")
                self.assertNotEqual(
                    check_manifest(mutated), [], f"mutation not detected: {label}"
                )

    def test_desktop_and_metainfo_use_the_manifest_app_id(self) -> None:
        self.assertIn(f"Icon={APP_ID}", self.desktop)
        self.assertIn(f"<id>{APP_ID}</id>", self.metainfo)
        self.assertIn(f"<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>", self.metainfo)


if __name__ == "__main__":
    unittest.main()
