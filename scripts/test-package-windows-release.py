#!/usr/bin/env python3
"""Retirement contracts for scripts/package-windows-release.sh.

The Windows GUI was retired to a NOT IMPLEMENTED failure stub
(``apps/app-windows``, bin ``yacr-windows``), so there is nothing to package.
The packaging script now fails with ``NOT IMPLEMENTED`` instead of emitting a
stub release zip. Each guarantee is removed in turn and the checker must report
it, so the retirement cannot silently degrade into a published stub. The
``app-windows`` icon-embedding contract is a still-valid source contract and is
kept. Text only; it is not evidence that any package was (or can be) built.
"""
from __future__ import annotations

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-windows-release.sh"

FORBIDDEN_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("release cross-compile", "cargo build --release"),
    ("zip output", "zipfile.ZipFile"),
    ("font catalogue assembly", "fetch-fonts.sh"),
    ("relocated GUI binary", "yacr.exe"),
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


class PackageWindowsReleaseRetiredContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.script = SCRIPT.read_text(encoding="utf-8")

    def test_script_is_an_explicit_not_implemented_failure(self) -> None:
        self.assertEqual(check_retired_script(self.script), [])

    def test_not_implemented_diagnostic_is_required(self) -> None:
        self.assertNotEqual(
            check_retired_script(self.script.replace("NOT IMPLEMENTED", "")), []
        )

    def test_nonzero_exit_is_required(self) -> None:
        problems = check_retired_script(self.script.replace("exit 1", "exit 0"))
        self.assertTrue(any("non-zero" in problem for problem in problems))

    def test_each_forbidden_live_path_is_required_for_retirement(self) -> None:
        for label, fragment in FORBIDDEN_FRAGMENTS:
            with self.subTest(fragment=label):
                mutated = self.script + f"\n# {fragment}\n{fragment}\n"
                self.assertNotEqual(
                    check_retired_script(mutated), [],
                    f"mutation not detected: {label}",
                )

    def test_window_binary_build_script_embeds_the_shared_icon(self) -> None:
        # Still-valid source contract on the Windows host (the packaging script
        # itself is retired): `tauri-build` embeds the PE icon from the shared
        # asset for a Windows target, gated on the *target* os so other targets
        # never run the resource compiler, and the bundle icon is a
        # byte-identical copy of that shared asset so the two entry points
        # cannot drift apart.
        build_rs = (ROOT / "apps" / "app-windows" / "build.rs").read_text(encoding="utf-8")
        cargo = (ROOT / "apps" / "app-windows" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn("assets/yacr-icon.ico", build_rs)
        self.assertIn("tauri_build::build()", build_rs)
        self.assertIn("CARGO_CFG_TARGET_OS", build_rs)
        self.assertIn("tauri-build", cargo)

        shared = (ROOT / "assets" / "yacr-icon.ico").read_bytes()
        bundled = (ROOT / "apps" / "app-windows" / "icons" / "icon.ico").read_bytes()
        self.assertEqual(
            bundled,
            shared,
            "apps/app-windows/icons/icon.ico must stay a copy of assets/yacr-icon.ico",
        )


if __name__ == "__main__":
    unittest.main()
