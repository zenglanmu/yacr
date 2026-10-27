#!/usr/bin/env python3
"""Command-name documentation gate.

The docs must never invent a command name or an alias: the single authority is
``crates/cad-ui-core/src/session/commands.rs`` (``COMMAND_TABLE``,
``MEASUREMENT_SELECTORS``, ``ZOOM_STEP_SELECTORS``, ``ARGUMENT_SELECTORS``,
``COMMAND_ALIASES``). This gate re-parses those tables on every run and checks
the places where humans transcribe them by hand:

* **§2** (``docs/shortcuts.md``) -- the ``canonical`` column must be an
  *admitted* selector, and every backticked token in the alias cell must be a
  real ``COMMAND_ALIASES`` row that maps back to **that** canonical.
* **§2b** (``docs/shortcuts.md``) -- the alias table (both the four-column
  ``别名 | canonical`` pairs and the ``a`/`b`->`target` `` sentence) must equal
  ``COMMAND_ALIASES`` **exactly**: no missing alias, no extra alias, same
  mapping.
* **§3.7** (``docs/autocad-command-coverage.md``) -- the first column must be an
  admitted selector.
* **both docs** -- any token the prose declares with ``别名 `x` `` (fenced code
  blocks are stripped first, so the gate never reports on its own source) must
  be a real alias.

*Admitted* = ``COMMAND_TABLE`` + ``MEASUREMENT_SELECTORS`` +
``ZOOM_STEP_SELECTORS``. Aliases are admitted *spellings*, not canonical
selectors, so they are deliberately excluded from that set (99 canonical
selectors vs. 150 typable spellings on 2026-10-27; do not mix the two).

Exit codes: ``0`` consistent (``OK: 0 mismatches``), ``1`` at least one
mismatch, ``2`` the inputs could not be read or parsed (missing selector table
or missing doc section) -- a shape error is never reported as a pass.

Stdlib only, no network, no writes. Paths default to this repository (resolved
from ``__file__``, *not* the current directory) and can be overridden with
``--doc`` / ``--coverage`` / ``--commands``.

Behaviour is ported from the audit snippet that used to live in
``docs/autocad-command-coverage.md`` §5b (2026-10-27). That snippet was a
pasted heredoc that nothing executed; this script is its single runnable copy.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_COMMANDS = ROOT / "crates" / "cad-ui-core" / "src" / "session" / "commands.rs"
DEFAULT_DOC = ROOT / "docs" / "shortcuts.md"
DEFAULT_COVERAGE = ROOT / "docs" / "autocad-command-coverage.md"

COMMAND_TABLE_RE = r'pub const COMMAND_TABLE: &\[\(&str, CommandId\)\] = &\[(.*?)\n\];'
MEASUREMENT_RE = (
    r'pub const MEASUREMENT_SELECTORS: &\[\(&str, MeasurementToolKind\)\] = &\[(.*?)\n\];'
)
ZOOM_RE = r'pub const ZOOM_STEP_SELECTORS: &\[\(&str, f64\)\] =\s*&\[(.*?)\];'
ARGUMENTS_RE = r'pub const ARGUMENT_SELECTORS: &\[&str\] = &\[(.*?)\n\];'
ALIASES_RE = r'pub const COMMAND_ALIASES: &\[\(&str, &str\)\] = &\[(.*?)\n\];'

# Section boundaries inside the two documents.
SEC2_HEADING = r'\n## 2\.'
SEC2_STOP = r'\n## 2b\.'
SEC2B_HEADING = r'\n## 2b\.'
SEC2B_STOP = r'\n## 3\.'
SEC37_HEADING = r'\n### 3\.7'
SEC37_STOP = r'\n---'

FENCE = '```'


class ShapeError(Exception):
    """The inputs do not have the structure this gate parses."""


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding='utf-8')
    except OSError as exc:
        raise ShapeError('%s: cannot be read (%s)' % (path, exc)) from exc
    except UnicodeDecodeError as exc:
        raise ShapeError('%s: cannot be decoded as UTF-8 (%s)' % (path, exc)) from exc


def const_body(pattern: str, source: str, path: Path, name: str) -> str:
    match = re.search(pattern, source, re.S)
    if match is None:
        raise ShapeError('%s: cannot find `pub const %s`' % (path, name))
    return match.group(1)


def section(text: str, heading: str, stop: str, path: Path, label: str) -> str:
    """Text after ``heading`` up to ``stop`` (the audit snippet's ``split(...)[1]``)."""
    match = re.search(heading, text)
    if match is None:
        raise ShapeError('%s: cannot find the %s section (expected heading %r)'
                         % (path, label, heading))
    rest = text[match.end():]
    end = re.search(stop, rest)
    if end is None:
        raise ShapeError('%s: cannot find the end of the %s section (expected %r)'
                         % (path, label, stop))
    return rest[:end.start()]


def cells_of(line: str) -> list[str]:
    return [cell.strip() for cell in line.strip().strip('|').split('|')]


class Report:
    """Everything the run observed, plus every mismatch it found."""

    def __init__(self) -> None:
        self.mismatches: list[str] = []
        self.table = 0
        self.measurement = 0
        self.zoom = 0
        self.arguments = 0
        self.aliases = 0
        self.admitted = 0
        self.section2_rows = 0
        self.section2b_aliases = 0
        self.section37_rows = 0
        self.alias_mentions = 0


def check(commands_text: str, doc_text: str, coverage_text: str,
          commands_path: Path, doc_path: Path, coverage_path: Path) -> Report:
    report = Report()

    table = re.findall(r'\("([^"]+)"',
                       const_body(COMMAND_TABLE_RE, commands_text, commands_path, 'COMMAND_TABLE'))
    measurement = re.findall(
        r'\("([^"]+)"',
        const_body(MEASUREMENT_RE, commands_text, commands_path, 'MEASUREMENT_SELECTORS'))
    zoom = re.findall(r'\("([^"]+)"',
                      const_body(ZOOM_RE, commands_text, commands_path, 'ZOOM_STEP_SELECTORS'))
    # ARGUMENT_SELECTORS holds constant identifiers, not string literals; the
    # count is reported only (it is not an admitted-name check).
    arguments = re.findall(r'^\s{4}([A-Z_]+_SELECTOR),$',
                           const_body(ARGUMENTS_RE, commands_text, commands_path,
                                      'ARGUMENT_SELECTORS'),
                           re.M)
    alias = dict(re.findall(r'\("([^"]+)",\s*"([^"]+)"\)',
                            const_body(ALIASES_RE, commands_text, commands_path,
                                       'COMMAND_ALIASES')))

    admitted = set(table) | set(measurement) | set(zoom)
    report.table = len(table)
    report.measurement = len(measurement)
    report.zoom = len(zoom)
    report.arguments = len(arguments)
    report.aliases = len(alias)
    report.admitted = len(admitted)

    bad = report.mismatches

    # All three sections are located up front, §2b first, so a document that
    # lost a section fails with a message naming *that* section instead of
    # "§2 has no end". The checks themselves then run in audit order below.
    sec2b = section(doc_text, SEC2B_HEADING, SEC2B_STOP, doc_path, '§2b')
    sec2 = section(doc_text, SEC2_HEADING, SEC2_STOP, doc_path, '§2')
    sec37 = section(coverage_text, SEC37_HEADING, SEC37_STOP, coverage_path, '§3.7')

    # --- §2: canonical column + alias cells ---------------------------------
    for line in sec2.splitlines():
        if not line.startswith('| ') or line.startswith('|---'):
            continue
        cells = cells_of(line)
        if len(cells) != 4 or cells[0] == 'canonical':
            continue
        report.section2_rows += 1
        name = cells[0].strip('` ')
        if name not in admitted:
            bad.append('§2 canonical not admitted: %r' % name)
        cell = cells[3]
        if cell in ('—', '-', ''):
            continue
        for token in re.findall(r'`([^`]+)`', cell):
            if token not in alias:
                bad.append('§2 %s: alias %r is not in COMMAND_ALIASES' % (name, token))
            elif alias[token] != name:
                bad.append('§2 %s: alias %r maps to %r' % (name, token, alias[token]))

    # --- §2b: the alias table must be exactly COMMAND_ALIASES ---------------
    documented: dict[str, str] = {}
    for line in sec2b.splitlines():
        if not line.startswith('| ') or line.startswith('|---'):
            continue
        cells = cells_of(line)
        if len(cells) != 4:
            continue
        for a, c in ((cells[0], cells[1]), (cells[2], cells[3])):
            a, c = a.strip('` '), c.strip('` ')
            if a and a not in ('别名', 'canonical'):
                documented[a] = c
    for match in re.finditer(r'((?:`[^`]+`/?)+)→(`[^`]+`)', sec2b):
        target = match.group(2).strip('`')
        for a in re.findall(r'`([^`]+)`', match.group(1)):
            documented[a] = target
    report.section2b_aliases = len(documented)

    for a in sorted(set(alias) - set(documented)):
        bad.append('§2b missing alias %r' % a)
    for a in sorted(set(documented) - set(alias)):
        bad.append('§2b extra alias %r' % a)
    for a in sorted(set(alias) & set(documented)):
        if alias[a] != documented[a]:
            bad.append('§2b %s: %r != %r' % (a, documented[a], alias[a]))

    # --- §3.7 (the audit doc): its first column must be admitted names ------
    for line in sec37.splitlines():
        if not line.startswith('| ') or line.startswith('|---'):
            continue
        cells = cells_of(line)
        if len(cells) != 3 or cells[0] in ('选择子', 'AutoCAD 名'):
            continue
        report.section37_rows += 1
        name = cells[0].strip('` ')
        if name not in admitted:
            bad.append('§3.7 not admitted: %r' % name)

    # --- every token the docs call an alias must be a real alias row --------
    # Fenced blocks are stripped first: the prose quote of this gate's own
    # rules names example aliases, and scanning them would report on itself.
    for text in (doc_text, coverage_text):
        stripped = re.sub(FENCE + '.*?' + FENCE, '', text, flags=re.S)
        for match in re.finditer(r'别名\s*`([^`]+)`', stripped):
            report.alias_mentions += 1
            if match.group(1) not in alias:
                bad.append('alias mention %r is not in COMMAND_ALIASES' % match.group(1))

    return report


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description='Reject command names/aliases the docs invented (see scripts/check-command-docs.py).',
    )
    parser.add_argument('--commands', type=Path, default=DEFAULT_COMMANDS,
                        help='authority table source (default: %(default)s)')
    parser.add_argument('--doc', type=Path, default=DEFAULT_DOC,
                        help='shortcuts doc to check (default: %(default)s)')
    parser.add_argument('--coverage', type=Path, default=DEFAULT_COVERAGE,
                        help='audit doc holding §3.7 (default: %(default)s)')
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        report = check(
            read_text(args.commands),
            read_text(args.doc),
            read_text(args.coverage),
            args.commands,
            args.doc,
            args.coverage,
        )
    except ShapeError as exc:
        print('command-docs FAILED: %s' % exc)
        return 2

    print('tables: %d/%d/%d/%d selectors, %d aliases, %d canonical selectors (aliases excluded)'
          % (report.table, report.measurement, report.zoom, report.arguments,
             report.aliases, report.admitted))
    print('§2 rows: %d, §2b aliases: %d, §3.7 rows: %d, alias mentions: %d'
          % (report.section2_rows, report.section2b_aliases, report.section37_rows,
             report.alias_mentions))

    if report.mismatches:
        print('\nFAIL: %d mismatch(es)' % len(report.mismatches))
        for line in report.mismatches:
            print('  - ' + line)
        return 1
    print('OK: 0 mismatches')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
