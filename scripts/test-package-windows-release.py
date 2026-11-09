#!/usr/bin/env python3
"""Live-pipeline contracts for scripts/package-windows-release.sh.

``apps/app-windows`` is a real Tauri 2 / WebView2 host whose executable is
``yacr-windows.exe``, and the shared React UI in ``apps/desktop-ui`` is embedded
at compile time by ``tauri::generate_context!``. The packaging script must
therefore build the UI first, cross-build both release executables, verify the
PE32+ machine and embedded icon, bundle or reject non-system runtime DLLs,
assemble fonts plus the release documents, and stage a release tree whose
``PACKAGE.txt`` still carries the ``@@YACR_SIGNING_NOTE@@`` placeholder.

The zip + sha256 are written by ``scripts/finalize-windows-release.sh``, which CI
runs *after* the repository self-signed PFX has signed (or explicitly left
unsigned) the staged
executables. Both scripts are pinned here so the local UNSIGNED path and the CI
SIGNED path cannot drift apart. The executables live at the **root** of the
staged/extracted package directory (no ``bin/`` subdirectory); a ``bin/`` path
reappearing in either script is reported as a contract violation.

Every guarantee is checked by **removing it in turn** from the script text and
requiring the checker to report it, so the pipeline cannot silently drift back
to the retired "NOT IMPLEMENTED" stub or to the old ``yacr.exe`` binary name.
This is a **text contract only**: it is not evidence that any Windows package
was built (no Windows target is available on the development host).

The ``app-windows`` icon-embedding contract is a still-valid source contract and
is kept: ``build.rs`` embeds ``assets/yacr-icon.ico`` through
``tauri_build::build()`` for a Windows target, and the bundle icon is a
byte-identical copy of that shared asset.
"""
from __future__ import annotations

import json
import os
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/package-windows-release.sh"
FINALIZE_SCRIPT = ROOT / "scripts/finalize-windows-release.sh"
CJK_FALLBACK = "NotoSansCJKsc-Regular.otf"

# (label, exact fragment). Each must appear in the packaging script, and deleting
# every occurrence of it must be reported as a missing guarantee.
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("fail-closed shell options", "set -euo pipefail"),
    ("frontend dependency install", "npm ci"),
    ("frontend build", "npm run build"),
    ("React UI bundle location", "apps/desktop-ui/dist"),
    ("vendor plugin bundle", "plugins/tch.js"),
    ("release build invocation", "cargo build --release --locked"),
    ("embedded frontend protocol", "tauri/custom-protocol"),
    ("windows host package", "-p app-windows"),
    ("tauri host executable", "yacr-windows.exe"),
    ("windows triple validation", "is not a Windows target triple"),
    ("static CRT linkage", "+crt-static"),
    ("PE import inspection", "check-pe-imports.py"),
    ("x86-64 machine check", "0x8664"),
    ("embedded icon check", "--has-icon"),
    ("font catalogue assembly", "fetch-fonts.sh"),
    ("font manifest", "fonts/fonts.json"),
    ("release output root", "target/${TARGET}/release"),
    ("staging directory", 'DIST="${RELEASE}/dist"'),
    ("root-level GUI staging", 'cp "$GUI_PATH" "$STAGE/$GUI"'),
    ("root-level CLI staging", 'cp "$CLI_PATH" "$STAGE/$CLI"'),
    ("root-level icon check", '--has-icon "$STAGE/$GUI"'),
    ("stage-only signing handoff", "SKIP_ZIP"),
    ("shared finalize step", "finalize-windows-release.sh"),
    ("package manifest", "PACKAGE.txt"),
    ("signing note placeholder", "@@YACR_SIGNING_NOTE@@"),
)

# The zip/checksum fragments moved into the finalize script; CI runs it after the
# self-signed Authenticode step so the archive it writes is the one that ships.
FINALIZE_REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    ("fail-closed shell options", "set -euo pipefail"),
    ("zip writing", "zipfile.ZipFile"),
    ("checksum file", ".zip.sha256"),
    ("sha256 digest", "sha256"),
    ("signed mode", "--signed"),
    ("unsigned mode", "--unsigned"),
    ("package manifest", "PACKAGE.txt"),
    ("self-signed disclosure", "SELF-SIGNED"),
    ("shipped public certificate", "yacr-dev-codesign.cer"),
    ("root-level staged GUI", '[ -f "$STAGE/yacr-windows.exe" ]'),
)

FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("retired NOT IMPLEMENTED stub", re.compile(r"NOT\s+IMPLEMENTED", re.IGNORECASE)),
    (
        "retired GUI binary name (the host builds yacr-windows.exe)",
        re.compile(r"(?<![\w-])yacr\.exe"),
    ),
    (
        "retired bin/ package layout (the executables sit in the package root)",
        re.compile(r"bin[/\\\"'\s]+(?:yacr-windows|cad-cli-tools)\.exe"),
    ),
)

# The finalize script writes the zip from the same root layout; a `bin/` path
# creeping back in there must also be reported.
FINALIZE_FORBIDDEN_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    (
        "retired bin/ package layout (the executables sit in the package root)",
        re.compile(r"bin[/\\\"'\s]+(?:yacr-windows|cad-cli-tools)\.exe"),
    ),
)


def check_pipeline(text: str) -> list[str]:
    """Return every guarantee the packaging script text is missing."""
    problems: list[str] = []
    for label, fragment in REQUIRED_FRAGMENTS:
        if fragment not in text:
            problems.append(f"missing {label}: {fragment!r}")
    for label, pattern in FORBIDDEN_PATTERNS:
        if pattern.search(text):
            problems.append(f"must not contain {label}: {pattern.pattern!r}")
    return problems


def check_finalize(text: str) -> list[str]:
    """Return every guarantee the finalize script text is missing."""
    problems: list[str] = []
    for label, fragment in FINALIZE_REQUIRED_FRAGMENTS:
        if fragment not in text:
            problems.append(f"missing {label}: {fragment!r}")
    for label, pattern in FINALIZE_FORBIDDEN_PATTERNS:
        if pattern.search(text):
            problems.append(f"must not contain {label}: {pattern.pattern!r}")
    return problems


class PackageWindowsReleasePipelineContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.script = SCRIPT.read_text(encoding="utf-8")

    def test_script_implements_the_live_pipeline(self) -> None:
        self.assertEqual(check_pipeline(self.script), [])

    def test_retired_not_implemented_stub_is_reported(self) -> None:
        problems = check_pipeline(
            self.script + '\necho "NOT IMPLEMENTED" >&2\nexit 1\n'
        )
        self.assertTrue(
            any("NOT IMPLEMENTED" in problem for problem in problems), problems
        )

    def test_each_required_fragment_is_required(self) -> None:
        for label, fragment in REQUIRED_FRAGMENTS:
            with self.subTest(fragment=label):
                mutated = self.script.replace(fragment, "")
                problems = check_pipeline(mutated)
                self.assertTrue(
                    any(fragment in problem for problem in problems),
                    f"removing {label} was not reported: {problems}",
                )

    def test_old_yacr_exe_name_is_reported(self) -> None:
        mutated = self.script.replace("yacr-windows.exe", "yacr.exe")
        problems = check_pipeline(mutated)
        self.assertTrue(
            any("retired GUI binary name" in problem for problem in problems), problems
        )

    def test_retired_bin_layout_is_reported(self) -> None:
        # Moving the executables back under bin/ must fail the contract: the
        # released package keeps them at the extracted directory root.
        mutated = self.script.replace(
            'cp "$GUI_PATH" "$STAGE/$GUI"',
            'cp "$GUI_PATH" "$STAGE/bin/yacr-windows.exe"',
        )
        problems = check_pipeline(mutated)
        self.assertTrue(
            any("retired bin/ package layout" in problem for problem in problems),
            problems,
        )
        self.assertTrue(
            any("root-level GUI staging" in problem for problem in problems), problems
        )
        finalize_problems = check_finalize(
            FINALIZE_SCRIPT.read_text(encoding="utf-8").replace(
                'stage / "yacr-windows.exe"', 'stage / "bin" / "yacr-windows.exe"'
            )
        )
        self.assertTrue(
            any("retired bin/ package layout" in problem for problem in finalize_problems),
            finalize_problems,
        )

    def test_finalize_script_implements_the_zip_step(self) -> None:
        self.assertTrue(
            FINALIZE_SCRIPT.is_file(), f"missing {FINALIZE_SCRIPT.relative_to(ROOT)}"
        )
        self.assertEqual(
            check_finalize(FINALIZE_SCRIPT.read_text(encoding="utf-8")), []
        )

    def test_finalize_script_is_executable(self) -> None:
        self.assertTrue(
            os.access(FINALIZE_SCRIPT, os.X_OK),
            "scripts/finalize-windows-release.sh must be executable",
        )

    def test_each_finalize_fragment_is_required(self) -> None:
        text = FINALIZE_SCRIPT.read_text(encoding="utf-8")
        for label, fragment in FINALIZE_REQUIRED_FRAGMENTS:
            with self.subTest(fragment=label):
                problems = check_finalize(text.replace(fragment, ""))
                self.assertTrue(
                    any(fragment in problem for problem in problems),
                    f"removing {label} was not reported: {problems}",
                )

    def test_staged_fonts_expect_the_committed_cjk_fallback(self) -> None:
        # Both branches stage the committed fonts/ package: WITH_FONTS=1 merges
        # the fetched catalogue into it via fetch-fonts.sh, WITH_FONTS=0 copies
        # it offline. The Noto CJK fallback must be part of that package.
        entries = json.loads(
            (ROOT / "fonts" / "fonts.json").read_text(encoding="utf-8")
        )
        self.assertIn(CJK_FALLBACK, {entry.get("file") for entry in entries})
        self.assertTrue((ROOT / "fonts" / CJK_FALLBACK).is_file())
        self.assertIn("fetch-fonts.sh", self.script)
        self.assertIn('cp -R "$ROOT"/fonts/. "$STAGE/fonts/"', self.script)
        self.assertIn('[ -s "$STAGE/fonts/fonts.json" ]', self.script)

    def test_window_binary_build_script_embeds_the_shared_icon(self) -> None:
        # Still-valid source contract on the Windows host: `tauri-build` embeds
        # the PE icon from the shared asset for a Windows target, gated on the
        # *target* os so other targets never run the resource compiler, and the
        # bundle icon is a byte-identical copy of that shared asset so the two
        # entry points cannot drift apart.
        build_rs = (ROOT / "apps" / "app-windows" / "build.rs").read_text(encoding="utf-8")
        cargo = (ROOT / "apps" / "app-windows" / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn("assets/yacr-icon.ico", build_rs)
        self.assertIn("tauri_build::build()", build_rs)
        self.assertIn("CARGO_CFG_TARGET_OS", build_rs)
        self.assertIn("tauri-build", cargo)
        # The executable the pipeline stages is the package's declared bin.
        self.assertIn('name = "yacr-windows"', cargo)

        shared = (ROOT / "assets" / "yacr-icon.ico").read_bytes()
        bundled = (ROOT / "apps" / "app-windows" / "icons" / "icon.ico").read_bytes()
        self.assertEqual(
            bundled,
            shared,
            "apps/app-windows/icons/icon.ico must stay a copy of assets/yacr-icon.ico",
        )


if __name__ == "__main__":
    unittest.main()
