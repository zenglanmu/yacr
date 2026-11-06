#!/usr/bin/env python3
"""Static + mutation contracts for scripts/fetch-fonts.sh.

The font packer runs on Linux, Windows and macOS CI. It previously read the
UTF-8 ``fonts.json`` catalogue with ``pathlib.Path.read_text()``, which uses the
platform default encoding (cp1252 on Windows) and aborted the Windows release
with ``UnicodeDecodeError`` on a non-ASCII font name. This guard rail asserts the
catalogue is always read and written as UTF-8, that a failed download still fails
the build, and that the committed default face is merged.

Each guarantee is deleted in turn and the checker must report it, so the contract
cannot silently degrade into an empty success. The cache test runs the real script
against a `file://` catalogue, so it needs no network. This checks script
behaviour, not that the public catalogue was downloaded.
"""
from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/fetch-fonts.sh"
COMMITTED_FONTS = ROOT / "fonts"
COMMITTED_CATALOG = COMMITTED_FONTS / "fonts.json"

# The one bundled missing-font fallback every host must receive, with every
# alias `cad-resources::FontCatalog` resolves. `fetch-fonts.sh` merges the
# committed catalogue by file name, so a same-named remote entry cannot drop it.
CJK_FALLBACK = "NotoSansCJKsc-Regular.otf"
CJK_FALLBACK_NAMES = (
    "Noto Sans CJK SC",
    "NotoSansCJKsc",
    "NotoSansCJKsc-Regular",
    "Noto Sans CJK",
    "Noto Sans SC",
    "noto sans cjk sc",
)

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
    (
        "merge by file name (a remote same-name entry cannot duplicate or drop "
        "the committed CJK fallback)",
        'all(str(e.get("file")) != str(x.get("file")) for x in entries)',
    ),
    ("shared download cache", "FONT_CACHE_DIR"),
    ("cache/face writes are atomic", "os.replace(temporary, path)"),
    ("cache reuse is reported", "font cache:"),
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

    def test_committed_catalog_lists_the_cjk_fallback(self) -> None:
        entries = json.loads(COMMITTED_CATALOG.read_text(encoding="utf-8"))
        self.assertTrue(entries, "committed fonts.json must not be empty")
        self.assertEqual(
            entries[0].get("file"),
            "osifont.ttf",
            "osifont must stay the first (default) committed face",
        )
        matches = [entry for entry in entries if entry.get("file") == CJK_FALLBACK]
        self.assertEqual(
            len(matches), 1, f"committed fonts.json must list {CJK_FALLBACK} exactly once"
        )
        entry = matches[0]
        self.assertEqual(entry.get("type"), "mesh")
        for name in CJK_FALLBACK_NAMES:
            self.assertIn(name, entry.get("name") or [], f"missing alias {name!r}")
        self.assertTrue(
            (COMMITTED_FONTS / CJK_FALLBACK).is_file(),
            f"committed catalogue entry has no file: fonts/{CJK_FALLBACK}",
        )

    def test_merge_keeps_the_committed_fallback_on_a_filename_collision(self) -> None:
        # Mirror the inline merge: drop committed entries whose file name is
        # already in the destination catalogue, then extend it. A same-named
        # remote entry must still leave exactly one resolvable entry, so the
        # fallback cannot disappear from the staged catalogue.
        committed = json.loads(COMMITTED_CATALOG.read_text(encoding="utf-8"))
        remote = [{"file": CJK_FALLBACK, "name": ["remote"], "type": "mesh"}]
        merged = remote + [
            entry
            for entry in committed
            if all(
                str(entry.get("file")) != str(existing.get("file"))
                for existing in remote
            )
        ]
        files = [entry.get("file") for entry in merged]
        self.assertEqual(files.count(CJK_FALLBACK), 1)
        self.assertIn("osifont.ttf", files)
        self.assertTrue((COMMITTED_FONTS / CJK_FALLBACK).is_file())


def run_fetch(
    script: pathlib.Path,
    dest: pathlib.Path,
    remote: pathlib.Path,
    cache: pathlib.Path,
    force: bool = False,
) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.update(
        {
            "FONT_BASE_URL": remote.as_uri(),
            "FONT_FALLBACK_BASE_URL": (remote.parent / "missing-source").as_uri(),
            "FONT_RETRIES": "1",
            "FONT_CACHE_DIR": str(cache),
            "FONTS": "",
            "FORCE": "1" if force else "",
        }
    )
    return subprocess.run(
        ["bash", str(script), str(dest)],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
    )


@unittest.skipUnless(shutil.which("bash"), "bash is required to run fetch-fonts.sh")
class FetchFontsCacheBehaviour(unittest.TestCase):
    """The shared cache really avoids a second download (file:// source)."""

    def test_faces_are_downloaded_once_and_reused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            remote = root / "remote"
            remote.mkdir()
            name = "cache-probe.ttf"
            (remote / "fonts.json").write_text(
                json.dumps([{"file": name, "name": ["cache-probe"], "type": "mesh"}]),
                encoding="utf-8",
            )
            (remote / name).write_bytes(b"PROBE-FACE")
            cache = root / "cache"

            first = run_fetch(SCRIPT, root / "dest1", remote, cache)
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertTrue((cache / name).is_file(), "downloaded face must enter the cache")
            self.assertEqual((root / "dest1" / name).read_bytes(), b"PROBE-FACE")
            self.assertIn("1 downloaded", first.stdout)

            # The source is gone; only the cache can satisfy the next package.
            (remote / name).unlink()
            second = run_fetch(SCRIPT, root / "dest2", remote, cache)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual((root / "dest2" / name).read_bytes(), b"PROBE-FACE")
            self.assertIn("1 reused", second.stdout)

            # FORCE must bypass the cache and fail loudly when the source is gone.
            forced = run_fetch(SCRIPT, root / "dest3", remote, cache, force=True)
            self.assertNotEqual(forced.returncode, 0, "FORCE must bypass the cache")


if __name__ == "__main__":
    unittest.main()
