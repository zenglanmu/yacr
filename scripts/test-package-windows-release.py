#!/usr/bin/env python3
"""Static + mutation contracts for scripts/package-windows-release.sh.

The packaging script is a heavy cross-compile (it links the Slint GUI for
``x86_64-pc-windows-gnu``), so it is not run in CI. This guard rail instead
asserts the script keeps its packaging guarantees:

  * it builds and stages *both* the headless CLI and the GUI host as PE32+ exe;
  * it only relocates MinGW runtime DLLs that the exes import and that are not
    Windows system libraries, and refuses to ship an unresolved import;
  * it assembles the CAD font catalogue into ``fonts/`` (or the committed set);
  * it emits a zip and a sha256;
  * an optional Wine smoke exercises the documented argument-parse path.

Each guarantee is deleted in turn and the checker must report it, so the contract
cannot silently degrade into an empty success. This checks the script text only;
it is not evidence that a package was built or that its GUI runs on Windows.
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-windows-release.sh"

# (human label, fragment that must appear in the script).
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("windows gnu local target", "x86_64-pc-windows-gnu"),
    ("msvc crt-static link", "target-feature=+crt-static"),
    ("headless CLI + GUI build", "cargo build --release --locked --target \"$TARGET\" -p cad-cli-tools -p app-windows"),
    ("stage CLI binary", 'cp "$CLI" "$STAGE/bin/cad-cli-tools.exe"'),
    ("stage GUI binary", 'cp "$GUI" "$STAGE/bin/yacr.exe"'),
    ("PE32+ machine check", "--machine"),
    ("relocate non-system DLLs", 'copy_runtime_dlls "$STAGE/bin/yacr.exe" "$STAGE/bin"'),
    ("refuse unresolved import", "is neither crt-static nor available to bundle"),
    ("font catalogue assembly", '"$ROOT/scripts/fetch-fonts.sh" "$STAGE/fonts"'),
    ("optional Wine smoke gate", "YACR_WINDOWS_SMOKE"),
    ("wine smoke asserts parse error", "--bogus"),
    ("zip output", "zipfile.ZipFile"),
    ("sha256 output", "hashlib.sha256"),
)


def check_script(text: str) -> list[str]:
    """Return one problem string per missing packaging guarantee."""
    return [
        f"missing {label}: {fragment!r}"
        for label, fragment in REQUIRED_FRAGMENTS
        if fragment not in text
    ]


class PackageWindowsReleaseContracts(unittest.TestCase):
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

    def test_gui_binary_cannot_be_dropped_silently(self) -> None:
        mutated = self.text.replace('cp "$GUI" "$STAGE/bin/yacr.exe"', "")
        problems = check_script(mutated)
        self.assertTrue(problems)
        self.assertTrue(any("GUI" in problem for problem in problems))

    def test_import_resolution_cannot_be_dropped_silently(self) -> None:
        mutated = self.text.replace(
            "      echo \"ERROR: $exe imports non-system '$name'; it is neither crt-static nor available to bundle\" >&2\n"
            "      return 1\n",
            "",
        )
        self.assertNotEqual(check_script(mutated), [])


if __name__ == "__main__":
    unittest.main()
