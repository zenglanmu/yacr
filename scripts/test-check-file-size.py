#!/usr/bin/env python3
"""Behavior contracts for ``scripts/check-file-size.py``.

The gate is exercised against a **temporary fixture tree** passed with
``--root``, so the real repository is never mutated and no allowlist entry is
needed. The fixture tree has no ``.git``: this relies on the documented fallback
in ``check-file-size.py`` that walks the directory when the root has no ``.git``
entry (a temp directory cannot be enumerated with ``git ls-files``).

Covered: the exact-limit boundary (passes), one line over (fails), skipped
``node_modules``/``dist`` segments, an out-of-scope extension, the ``--limit``
override, and an undecodable file reported as a hard failure. Stdlib only.
"""
from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECK = ROOT / "scripts" / "check-file-size.py"


def write_lines(path: pathlib.Path, count: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(f"line {index}\n" for index in range(count)), encoding="utf-8")


class FileSizeGateContracts(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = pathlib.Path(self._tmp.name)

    def run_check(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(CHECK), "--root", str(self.root), *args],
            capture_output=True,
            text=True,
            timeout=60,
        )

    def test_docstring_documents_the_no_git_walk_fallback(self) -> None:
        # The self-test depends on this fallback; keep it documented.
        text = CHECK.read_text(encoding="utf-8")
        self.assertIn("falls back to walking the directory", text)

    def test_default_limit_reports_exactly_the_offenders(self) -> None:
        write_lines(self.root / "at_limit.rs", 1000)
        write_lines(self.root / "over.rs", 1001)
        write_lines(self.root / "node_modules" / "skip.rs", 1500)
        write_lines(self.root / "dist" / "skip.js", 1200)
        write_lines(self.root / "vendor" / "skip.ts", 1200)
        write_lines(self.root / "README.md", 2000)  # out of scope

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(
            result.stdout,
            "over.rs: 1001 lines (limit 1000)\n"
            "file-size FAILED: 1 file(s) exceed the limit (1000 lines)\n",
        )

    def test_offenders_are_sorted_by_descending_size_then_path(self) -> None:
        write_lines(self.root / "small.rs", 1002)
        write_lines(self.root / "wide.rs", 1500)
        write_lines(self.root / "also.rs", 1002)

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(
            result.stdout,
            "wide.rs: 1500 lines (limit 1000)\n"
            "also.rs: 1002 lines (limit 1000)\n"
            "small.rs: 1002 lines (limit 1000)\n"
            "file-size FAILED: 3 file(s) exceed the limit (1000 lines)\n",
        )

    def test_limit_override_with_exact_boundary(self) -> None:
        write_lines(self.root / "exact.ts", 5)
        write_lines(self.root / "over.tsx", 6)

        result = self.run_check("--limit", "5")

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(
            result.stdout,
            "over.tsx: 6 lines (limit 5)\n"
            "file-size FAILED: 1 file(s) exceed the limit (5 lines)\n",
        )

    def test_success_line_and_exit_zero(self) -> None:
        write_lines(self.root / "ok.mjs", 3)

        result = self.run_check()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "file-size OK: 1 code file(s), limit 1000 lines\n")

    def test_undecodable_file_is_a_hard_failure(self) -> None:
        path = self.root / "broken.rs"
        path.write_bytes(b"\xff\xfe not valid utf-8")

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("broken.rs", result.stdout)
        self.assertIn("cannot be read as UTF-8", result.stdout)


if __name__ == "__main__":
    unittest.main()
