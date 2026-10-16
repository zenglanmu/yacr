#!/usr/bin/env python3
"""Retirement contracts for the Linux release packaging.

The old Flatpak pipeline built the retired Slint ``yacr-linux`` headless host.
It is disabled pending a validated replacement: ``scripts/package-linux-release.sh``
now fails with ``NOT IMPLEMENTED`` and the Flatpak manifest/desktop/metainfo under
``packaging/flatpak`` are retired, so the release path cannot invent an untested
distributable. Each guarantee is removed in turn and the checker must report it,
so the retirement cannot silently degrade into a published stub. Text only; it is
not evidence that any package was (or can be) built.
"""
from __future__ import annotations

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-linux-release.sh"
FLATPAK_DIR = ROOT / "packaging/flatpak"

# Live packaging/publish signatures that must NOT remain in a retired script.
FORBIDDEN_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("flatpak-builder invocation", "flatpak-builder"),
    ("single-file bundle", "flatpak build-bundle"),
    ("release cargo build", "cargo build --release"),
    ("retired Slint GUI binary", "yacr-linux"),
    ("font catalogue assembly", "fetch-fonts.sh"),
)


def check_retired_script(text: str) -> list[str]:
    problems: list[str] = []
    if "NOT IMPLEMENTED" not in text:
        problems.append("missing the NOT IMPLEMENTED diagnostic")
    if not re.search(r"^exit 1\s*$", text, re.MULTILINE):
        problems.append("must exit non-zero ('exit 1')")
    for label, fragment in FORBIDDEN_FRAGMENTS:
        if fragment in text:
            problems.append(f"still contains live packaging path {label}: {fragment!r}")
    return problems


class PackageLinuxReleaseRetiredContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.script = SCRIPT.read_text(encoding="utf-8")

    def test_script_is_an_explicit_not_implemented_failure(self) -> None:
        self.assertEqual(check_retired_script(self.script), [])

    def test_not_implemented_diagnostic_is_required(self) -> None:
        mutated = self.script.replace("NOT IMPLEMENTED", "")
        self.assertNotEqual(check_retired_script(mutated), [])

    def test_nonzero_exit_is_required(self) -> None:
        mutated = self.script.replace("exit 1", "exit 0")
        problems = check_retired_script(mutated)
        self.assertTrue(any("non-zero" in problem for problem in problems))

    def test_each_forbidden_live_path_is_required_for_retirement(self) -> None:
        for label, fragment in FORBIDDEN_FRAGMENTS:
            with self.subTest(fragment=label):
                mutated = self.script + f"\n# {fragment}\n{fragment}\n"
                self.assertNotEqual(
                    check_retired_script(mutated), [],
                    f"mutation not detected: {label}",
                )

    def test_flatpak_packaging_is_retired(self) -> None:
        # The manifest + desktop + metainfo are gone; keeping any of them would
        # claim a distributable that is not built or validated.
        self.assertFalse(FLATPAK_DIR.exists(), "packaging/flatpak must be retired")


if __name__ == "__main__":
    unittest.main()
