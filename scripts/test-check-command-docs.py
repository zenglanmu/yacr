#!/usr/bin/env python3
"""Behaviour contracts for ``scripts/check-command-docs.py``.

The gate is exercised against **temporary copies** of the two documents, passed
with ``--doc`` / ``--coverage``; the Rust authority table is read from the real
repository because it is not what these tests mutate. The real
``docs/shortcuts.md`` and ``docs/autocad-command-coverage.md`` are never written
(one test asserts that by hashing them before and after the suite).

Covered: the real corpus passes; one fabricated canonical name **and** one
fabricated alias are both reported (exit 1); a dropped §2b row is reported as a
missing alias; §2b rejects an extra alias; §3.7 rejects a non-admitted first
column and an invented ``别名 `x` `` mention; a truncated document is a clear
shape failure (exit 2, no traceback) instead of a pass. Stdlib only.
"""
from __future__ import annotations

import hashlib
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
CHECK = ROOT / "scripts" / "check-command-docs.py"
DOC = ROOT / "docs" / "shortcuts.md"
COVERAGE = ROOT / "docs" / "autocad-command-coverage.md"

# Tokens that cannot collide with a real selector or alias row.
FAKE_CANONICAL = "ZZSELFTEST COMMAND"
FAKE_ALIAS = "ZZSELFTEST"


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def insert_after_table_header(text: str, heading: str, row: str) -> str:
    """Insert ``row`` after the first ``|---`` separator following ``heading``."""
    start = text.index(heading)
    lines = text[start:].splitlines(keepends=True)
    for index, line in enumerate(lines):
        if line.startswith("|---"):
            lines.insert(index + 1, row + "\n")
            break
    else:  # pragma: no cover - the corpus always has a table under §2
        raise AssertionError(f"no table separator after {heading!r}")
    return text[:start] + "".join(lines)


def section_bounds(text: str, heading: str, stop: str) -> tuple[int, int]:
    start = text.index(heading)
    end = text.index(stop, start)
    return start, end


class CommandDocsGateContracts(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = pathlib.Path(self._tmp.name)
        self.doc = self.root / "shortcuts.md"
        self.coverage = self.root / "autocad-command-coverage.md"
        self.doc.write_text(DOC.read_text(encoding="utf-8"), encoding="utf-8")
        self.coverage.write_text(COVERAGE.read_text(encoding="utf-8"), encoding="utf-8")

    # -- helpers ------------------------------------------------------------
    def run_check(self, *, doc: pathlib.Path | None = None,
                  coverage: pathlib.Path | None = None) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable,
                str(CHECK),
                "--doc",
                str(doc if doc is not None else self.doc),
                "--coverage",
                str(coverage if coverage is not None else self.coverage),
            ],
            capture_output=True,
            text=True,
            timeout=60,
            cwd=self.root,
        )

    def rewrite_doc(self, text: str) -> None:
        self.doc.write_text(text, encoding="utf-8")

    # -- contracts ----------------------------------------------------------
    def test_real_corpus_passes(self) -> None:
        before = (digest(DOC), digest(COVERAGE))

        result = subprocess.run(
            [sys.executable, str(CHECK)],
            capture_output=True,
            text=True,
            timeout=60,
            cwd=ROOT,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(result.stdout.endswith("OK: 0 mismatches\n"), result.stdout)
        self.assertIn("canonical selectors (aliases excluded)", result.stdout)
        self.assertIn("§2 rows: ", result.stdout)
        self.assertEqual(before, (digest(DOC), digest(COVERAGE)))

    def test_fabricated_canonical_and_alias_are_both_reported(self) -> None:
        row = f"| `{FAKE_CANONICAL}` | 自检注入 | 是 | `{FAKE_ALIAS}` |"
        self.rewrite_doc(insert_after_table_header(
            self.doc.read_text(encoding="utf-8"), "\n## 2.", row))

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("FAIL: 2 mismatch(es)", result.stdout)
        self.assertIn(f"§2 canonical not admitted: '{FAKE_CANONICAL}'", result.stdout)
        self.assertIn(
            f"§2 {FAKE_CANONICAL}: alias '{FAKE_ALIAS}' is not in COMMAND_ALIASES",
            result.stdout,
        )
        self.assertNotIn("OK: 0 mismatches", result.stdout)

    def test_section_2b_missing_alias_row_is_reported(self) -> None:
        text = self.doc.read_text(encoding="utf-8")
        start, end = section_bounds(text, "\n## 2b.", "\n## 3.")
        lines = text[start:end].splitlines(keepends=True)
        # Find the first four-column data row of §2b (not the heading, not |---).
        for index, line in enumerate(lines):
            if not line.startswith("| ") or line.startswith("|---"):
                continue
            cells = [cell.strip().strip("` ") for cell in line.strip().strip("|").split("|")]
            if len(cells) != 4 or cells[0] in ("别名", "canonical"):
                continue
            dropped = cells[0]
            del lines[index]
            break
        else:  # pragma: no cover - the corpus always has §2b rows
            raise AssertionError("§2b has no data row to drop")
        self.rewrite_doc(text[:start] + "".join(lines) + text[end:])

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"§2b missing alias '{dropped}'", result.stdout)

    def test_section_2b_extra_alias_is_reported(self) -> None:
        text = self.doc.read_text(encoding="utf-8")
        injected = f"`{FAKE_ALIAS}`/`{FAKE_ALIAS}X`→`LINE`"
        anchor = "\n## 2b."
        start = text.index(anchor) + len(anchor)
        self.rewrite_doc(text[:start] + "\n\n" + injected + "\n" + text[start:])

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"§2b extra alias '{FAKE_ALIAS}'", result.stdout)

    def test_section_3_7_column_and_alias_mention_are_reported(self) -> None:
        text = self.coverage.read_text(encoding="utf-8")
        start, end = section_bounds(text, "\n### 3.7", "\n---")
        section = text[start:end]
        self.assertIn("| `DIST` |", section)
        section = section.replace("| `DIST` |", f"| `{FAKE_CANONICAL}` |", 1)
        row_anchor = "\n| `AREA` |"
        # Prose, not a table cell: the mention must stay outside the table so
        # the row keeps its three columns.
        section = section.replace(
            row_anchor,
            f"\n> 自检注入：别名 `{FAKE_ALIAS}`\n{row_anchor.lstrip(chr(10))}",
            1,
        )
        self.coverage.write_text(text[:start] + section + text[end:], encoding="utf-8")

        result = self.run_check()

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"§3.7 not admitted: '{FAKE_CANONICAL}'", result.stdout)
        self.assertIn(
            f"alias mention '{FAKE_ALIAS}' is not in COMMAND_ALIASES", result.stdout
        )

    def test_truncated_document_is_a_shape_failure_not_a_pass(self) -> None:
        text = self.doc.read_text(encoding="utf-8")
        self.rewrite_doc(text[: text.index("\n## 2b.")])

        result = self.run_check()

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("command-docs FAILED", result.stdout)
        self.assertIn("cannot find the §2b section", result.stdout)
        self.assertNotIn("Traceback", result.stderr)
        self.assertNotIn("OK: 0 mismatches", result.stdout)

    def test_fenced_blocks_are_not_scanned_for_alias_mentions(self) -> None:
        # The docs quote their own rules inside fences; that must stay quiet.
        text = self.doc.read_text(encoding="utf-8")
        fenced = f"\n```text\n别名 `{FAKE_ALIAS}`\n```\n"
        self.rewrite_doc(text + fenced)

        result = self.run_check()

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK: 0 mismatches", result.stdout)


if __name__ == "__main__":
    unittest.main()
