#!/usr/bin/env python3
"""Reject oversized tracked code files (physical-line ceiling).

Usage:
    python3 scripts/check-file-size.py [--root DIR] [--limit N]

Code files above the limit must be split rather than allowed to grow without
bound. The in-scope extensions are ``.rs``, ``.js``, ``.jsx``, ``.mjs``,
``.cjs``, ``.ts``, ``.tsx`` and ``.py``. Markdown and every other extension are
explicitly out of scope (long docs are intentional logs/backlogs).

Discovery and counting:

  * only **tracked** files count (``git ls-files``), so build output and vendored
    trees are excluded by construction. ``target``/``node_modules``/``dist``/
    ``vendor`` path segments are skipped as well, as defence in depth;
  * lines are ``len(text.splitlines())`` of the file read as UTF-8. A file that
    cannot be decoded as UTF-8 is a hard failure reported with its path, never
    skipped silently;
  * a file with exactly ``limit`` lines passes; ``limit + 1`` fails.

``--root DIR`` selects the tree to scan (default: the repository the script lives
in) and ``--limit N`` overrides the default 1000-line ceiling. When the selected
root has no ``.git`` entry the script **falls back to walking the directory** —
this is what makes ``scripts/test-check-file-size.py`` able to point the gate at
a temporary fixture tree without a Git checkout.

Exit code is non-zero on any offender or unreadable file. Stdlib only, and the
output is deterministic (offenders sorted by descending size, then path).
"""
from __future__ import annotations

import pathlib
import subprocess
import sys

DEFAULT_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_LIMIT = 1000

# File extensions in scope. Doc `.md` (and everything else) is intentionally out.
EXTENSIONS = (".rs", ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".py")

# Directory segments never worth measuring, even if they show up as tracked
# (they normally do not). Defence in depth for a mis-tracked build/vendor tree.
SKIP_DIRS = frozenset({"node_modules", "dist", "target", "vendor"})


def has_skipped_part(rel: pathlib.Path) -> bool:
    return any(part in SKIP_DIRS for part in rel.parts)


def _git_ls_files(root: pathlib.Path) -> list[pathlib.Path] | None:
    """Tracked files relative to ``root``, or ``None`` if Git is unusable."""
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return None
    return [root / line for line in result.stdout.splitlines() if line.strip()]


def _walk(root: pathlib.Path) -> list[pathlib.Path]:
    return [path for path in root.rglob("*") if path.is_file()]


def code_file_paths(root: pathlib.Path) -> list[pathlib.Path]:
    """Tracked (or, without ``.git``, walked) in-scope code files under ``root``."""
    files = _git_ls_files(root) if (root / ".git").exists() else None
    if files is None:
        files = _walk(root)
    return sorted(
        path
        for path in files
        if path.suffix in EXTENSIONS and not has_skipped_part(path.relative_to(root))
    )


def check(root: pathlib.Path, limit: int) -> int:
    code_files = code_file_paths(root)
    offenders: list[tuple[str, int]] = []
    errors: list[str] = []
    for path in code_files:
        rel = str(path.relative_to(root))
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError) as error:
            errors.append(f"{rel}: cannot be read as UTF-8: {error}")
            continue
        lines = len(text.splitlines())
        if lines > limit:
            offenders.append((rel, lines))

    if offenders or errors:
        offenders.sort(key=lambda item: (-item[1], item[0]))
        for rel, lines in offenders:
            print(f"{rel}: {lines} lines (limit {limit})")
        for message in sorted(errors):
            print(message)
        summary = (
            f"file-size FAILED: {len(offenders)} file(s) exceed the limit ({limit} lines)"
        )
        if errors:
            summary += f", {len(errors)} file(s) not decodable as UTF-8"
        print(summary)
        return 1

    print(f"file-size OK: {len(code_files)} code file(s), limit {limit} lines")
    return 0


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    root = DEFAULT_ROOT
    limit = DEFAULT_LIMIT
    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg == "--limit" and index + 1 < len(argv):
            try:
                limit = int(argv[index + 1])
            except ValueError:
                print(
                    f"file-size FAILED: --limit expects an integer, "
                    f"got {argv[index + 1]!r}",
                    file=sys.stderr,
                )
                return 2
            index += 2
        elif arg == "--root" and index + 1 < len(argv):
            root = pathlib.Path(argv[index + 1])
            index += 2
        else:
            print(f"file-size FAILED: unknown argument {arg!r}", file=sys.stderr)
            return 2
    return check(root.resolve(), limit)


if __name__ == "__main__":
    sys.exit(main())
