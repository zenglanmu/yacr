#!/usr/bin/env python3
"""Static + mutation contracts for scripts/fetch-fonts.sh.

The font packer runs on Linux, Windows and macOS CI. It previously read the
UTF-8 ``fonts.json`` catalogue with ``pathlib.Path.read_text()``, which uses the
platform default encoding (cp1252 on Windows) and aborted the Windows release
with ``UnicodeDecodeError`` on a non-ASCII font name. This guard rail asserts the
catalogue is always read and written as UTF-8, that a failed download still fails
the build, and that the committed default face is merged.

Each guarantee is deleted in turn and the checker must report it, so the contract
cannot silently degrade into an empty success. This checks the script text only;
it is not evidence that a catalogue was downloaded.
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/fetch-fonts.sh"

# (human label, fragment that must appear in the script).
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("utf-8 read of the catalogue", 'read_text(encoding="utf-8")'),
    (
        "utf-8 write of the merged catalogue",
        r'write_text(json.dumps(entries, indent=2) + "\n", encoding="utf-8")',
    ),
    ("missing font fails the build", "sys.exit(1)"),
    ("fallback download source", "FONT_FALLBACK_BASE_URL"),
    ("committed default face merge", 'extra / "fonts.json"'),
)


def check_script(text: str) -> list[str]:
    """Return one problem string per missing packaging guarantee."""
    return [
        f"missing {label}: {fragment!r}"
        for label, fragment in REQUIRED_FRAGMENTS
        if fragment not in text
    ]


class FetchFontsContracts(unittest.TestCase):
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

    def test_platform_default_encoding_cannot_return(self) -> None:
        # Replacing the explicit UTF-8 read with the encoding-less form must be
        # detected, because that is exactly the Windows regression.
        mutated = self.text.replace(
            'read_text(encoding="utf-8")', "read_text()"
        )
        self.assertNotEqual(check_script(mutated), [])

    def test_failure_cannot_be_folded_into_success(self) -> None:
        mutated = self.text.replace("sys.exit(1)", "pass")
        problems = check_script(mutated)
        self.assertTrue(problems)
        self.assertTrue(any("fails the build" in problem for problem in problems))


if __name__ == "__main__":
    unittest.main()
