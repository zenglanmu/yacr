#!/usr/bin/env python3
"""Static + mutation contracts for scripts/package-macos-release.sh.

The packaging script is a native macOS release build (it links the Slint GUI as
a Mach-O executable and combines architectures with ``lipo``), so it cannot run
on the Linux development host. This guard rail instead asserts the script keeps
its packaging guarantees:

  * it refuses to run off macOS (Apple SDK/linker are required);
  * it builds and stages *both* the headless CLI and the GUI host as Mach-O;
  * a universal build combines ``aarch64-apple-darwin`` and
    ``x86_64-apple-darwin`` with ``lipo``;
  * it refuses to ship a package whose executables link a non-system library;
  * it stages a self-contained ``Yacr.app`` bundle (Info.plist + executable);
  * it assembles the CAD font catalogue into the bundle;
  * it emits a tar.gz and a sha256;
  * an optional smoke exercises the documented argument-parse path.

Each guarantee is deleted in turn and the checker must report it, so the contract
cannot silently degrade into an empty success. This checks the script text only;
it is not evidence that a package was built or that its GUI runs on macOS.
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-macos-release.sh"

# (human label, fragment that must appear in the script).
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("macos host guard", '$(uname -s)" != "Darwin"'),
    ("arm64 apple target", "aarch64-apple-darwin"),
    ("x86_64 apple target", "x86_64-apple-darwin"),
    ("universal lipo combine", "lipo -create"),
    ("headless CLI + GUI build", 'cargo build --release --locked --target "$target" -p cad-cli-tools -p app-macos'),
    ("stage GUI into the app bundle", 'lipo_thin "yacr-macos" "$GUI_BUILT"'),
    ("stage CLI binary", 'lipo_thin "cad-cli-tools" "$CLI_BUILT"'),
    ("app bundle Info.plist", "Yacr.app/Contents/Info.plist"),
    ("bundle executable metadata", "CFBundleExecutable"),
    ("font catalogue assembly", '"$ROOT/scripts/fetch-fonts.sh"'),
    ("reject non-system Mach-O dependency", "links non-system library"),
    ("optional macOS smoke gate", "YACR_MACOS_SMOKE"),
    ("smoke asserts parse error", "--bogus"),
    ("tar.gz output", 'tar -czf "${NAME}.tar.gz"'),
    ("sha256 output", "shasum -a 256"),
)


def check_script(text: str) -> list[str]:
    """Return one problem string per missing packaging guarantee."""
    return [
        f"missing {label}: {fragment!r}"
        for label, fragment in REQUIRED_FRAGMENTS
        if fragment not in text
    ]


class PackageMacosReleaseContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.text = SCRIPT.read_text(encoding="utf-8")

    def test_real_script_satisfies_every_guarantee(self) -> None:
        self.assertEqual(check_script(self.text), [])

    def test_each_guarantee_is_required(self) -> None:
        for label, fragment in REQUIRED_FRAGMENTS:
            with self.subTest(guarantee=label):
                mutated = self.text.replace(fragment, "")
                self.assertNotEqual(
                    check_script(mutated), [], f"mutation not detected: {label}"
                )

    def test_gui_bundle_cannot_be_dropped_silently(self) -> None:
        mutated = self.text.replace('lipo_thin "yacr-macos" "$GUI_BUILT"', "")
        problems = check_script(mutated)
        self.assertTrue(problems)
        self.assertTrue(any("GUI" in problem for problem in problems))

    def test_universal_arch_cannot_be_dropped_silently(self) -> None:
        mutated = self.text.replace("aarch64-apple-darwin", "").replace(
            "x86_64-apple-darwin", ""
        )
        problems = check_script(mutated)
        self.assertTrue(problems)
        self.assertTrue(any("apple target" in problem for problem in problems))


if __name__ == "__main__":
    unittest.main()
