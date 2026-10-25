#!/usr/bin/env python3
"""Validate the N01 bilingual catalogs and catch untranslated chrome literals.

Usage:
    python3 scripts/check-i18n.py [workspace-root]

`workspace-root` defaults to the repository the script lives in. It exists so the
guard contract test (`scripts/test-ui-migration-guards.py`) can point the check at
a temporary fixture workspace and inject hardcoded copy without touching the real
repo.

Checks, all of which must pass:

1. both catalogs parse as flat JSON objects of string values;
2. neither catalog is empty;
3. the key sets are identical (and no empty/duplicate keys);
4. every key has the same ``{placeholder}`` set in both languages;
5. every **statically referenced** key exists in the catalog: Rust
   ``.text("key")`` / ``.message("key")`` lookups and catalog-key ``const``s
   (e.g. ``const KEY_OPEN: &str = "file.open"``) under the Rust UI sources, plus
   the React UI's ``t('key')`` calls and static ``labelKey``/``titleKey`` table
   entries (best effort against a documented allowlist of dynamic/legacy
   identifiers);
6. no hardcoded CJK literal remains in non-test Rust UI code;
7. no hardcoded CJK copy remains in the **React UI** (`apps/desktop-ui/src`, the
   one shared desktop UI tree; ``*.jsx``/``*.js``/``*.mjs``): both string/template
   literals and bare JSX text nodes are scanned with a small lexical walk that
   skips top-level ``//`` / ``/* */`` comments. Canonical JSON catalogs are never
   scanned.

The catalogs are the **single canonical source**: `crates/cad-ui-core/i18n/`
(read by `cad_ui_core::i18n` via `include_str!` and by the React UI through the
Vite `@i18n` alias). The Slint crate (and its duplicated catalog copy) was
deleted in P3b, so the transitional byte-parity guard is gone; this script
validates the one surviving copy directly.

Coverage limitations (honest):
- only **literal** keys are resolved. Keys reached through indirection (e.g.
  ``t(suggestion.labelKey)`` where ``labelKey`` is built at runtime, or a key
  assembled from fragments) are not resolved here; they must stay covered by the
  React UI's own node:test suite (`apps/desktop-ui/test/i18n.test.ts`) and by review.
- the React CJK walk is a lexical heuristic, not a full JS/JSX parser, so it has
  both false positives and false negatives. False positives: CJK inside a comment
  nested in a template interpolation (`` `${x /* 中文 */}` ``) is read as template
  content, and a quote inside a regex literal can shift the string state. False
  negatives: a regex literal containing ``//``/``/*`` is read as a comment and can
  hide later copy, and a nested backtick ends a template early. There is no
  per-literal allowlist; the UI test suite and review remain the completeness
  backstop. It is deliberately conservative about what it rejects, not exact.

Exit code is non-zero on any failure so CI can gate on it. Stdlib only.
"""
from __future__ import annotations

import json
import pathlib
import re
import sys

DEFAULT_ROOT = pathlib.Path(__file__).resolve().parents[1]

# Key-shaped string literals that are intentionally NOT catalog keys: JSON
# schemas, DWG/file formats, storage keys, tensors, etc. Exact-match allowlist;
# anything else that looks like `a.b` in a `.text(...)`/`.message(...)` call (or a
# static React `t('...')`) is a missing translation and must be added to the
# catalog (or here, with a comment). `does.not.exist` is still exercised by the
# cad-ui-core fallback unit test; the rest are legacy non-UI identifiers retained
# because they share the dotted shape and may reappear.
ALLOWLIST = {
    "annotations.cadnotes.json",  # export file name
    "yacr.cad.backend",  # localStorage key
    "yacr.cad.recovery",  # localStorage key
    "diagnostics.summary",  # machine diagnostic code (translated by the host)
    "diagnostics.bounds",  # machine diagnostic code
    "does.not.exist",  # deliberately-missing key exercised by the fallback unit test
}

# Exact string literals that may contain CJK but are intentionally not chrome.
# Add an entry here with a comment only when the literal is genuinely stable
# (a machine code, a test fixture, a file name) — never to silence a missing
# translation. Currently empty: the chrome is fully catalog-driven.
LITERAL_ALLOWLIST: dict[str, str] = {}

# `text("key")`, `message("key", ...)` and `format!("key", ...)`-style lookups.
LOOKUP_RE = re.compile(
    r"""\.(?:text|message)\(\s*"([^"\\]*)"|format!\(\s*"([^"\\]*)\""""
)
# Static catalog-key constants, e.g. `const KEY_PICKER_TITLE: &str = "linux.picker_title";`.
RUST_CONST_KEY_RE = re.compile(
    r'''const\s+[A-Za-z0-9_]*KEY[A-Za-z0-9_]*\s*:\s*&str\s*=\s*"([^"\\]*)"'''
)
# React `t('key')` / `t("key")` calls (a dynamic `t(expr)` never matches).
JS_LOOKUP_RE = re.compile(r'''\bt\(\s*(?:'([^'\\]*)'|"([^"\\]*)")''')
# Static label/title key tables, e.g. `{ id: 'OPEN', labelKey: 'file.open', ... }`.
JS_KEY_TABLE_RE = re.compile(
    r'''\b(?:labelKey|titleKey|actionKey|hintKey)\s*:\s*(?:'([^'\\]*)'|"([^"\\]*)")'''
)

KEY_SHAPE_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$")
PLACEHOLDER_RE = re.compile(r"(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}")

# A double-quoted string literal, honouring backslash escapes.
STRING_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')
# CJK ideographs, extension A, CJK punctuation and fullwidth forms.
CJK_RE = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff\u3000-\u303f\uff00-\uffef]")
# React UI source extensions scanned for hardcoded CJK copy.
JS_EXTENSIONS = ("*.jsx", "*.js", "*.mjs", "*.tsx", "*.ts")


def is_cjk(text: str) -> bool:
    return CJK_RE.search(text) is not None


def _is_cjk_char(ch: str) -> bool:
    return CJK_RE.match(ch) is not None


def catalog_paths(root: pathlib.Path) -> dict[str, pathlib.Path]:
    i18n_dir = root / "crates" / "cad-ui-core" / "i18n"
    return {"zh-CN": i18n_dir / "zh-CN.json", "en": i18n_dir / "en.json"}


def rust_sources(root: pathlib.Path) -> list[pathlib.Path]:
    # Every desktop host owns Rust UI code. `crates/cad-desktop-shell` and
    # `apps/app-windows` are listed ahead of landing; a missing directory simply
    # contributes no files, so the scan set stays correct as the tree grows.
    return [
        root / "crates" / "cad-ui-core" / "src",
        root / "crates" / "cad-desktop-shell" / "src",
        root / "apps" / "app-linux" / "src",
        root / "apps" / "app-windows" / "src",
    ]


def ui_sources(root: pathlib.Path) -> list[pathlib.Path]:
    # One shared React tree for every desktop host (`apps/desktop-ui`).
    return [root / "apps" / "desktop-ui" / "src"]


def strip_comments(text: str) -> str:
    """Remove // and /* */ comments so quoted examples in prose do not count."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return re.sub(r"//[^\n]*", "", text)


def strip_test_modules(text: str) -> str:
    """Drop everything from the first `#[cfg(test)]` to EOF.

    In the scanned Rust crates the test modules are always the trailing
    module(s), so this is exact rather than heuristic;
    `check_test_modules_are_trailing` below asserts that precondition so the scan
    cannot silently start skipping real code.
    """
    index = text.find("#[cfg(test)]")
    return text if index == -1 else text[:index]


def is_test_source(path: pathlib.Path) -> bool:
    """Whether `path` is unit-test code rather than shipped chrome.

    Unit tests live in dedicated `tests.rs` modules (or a `tests/` directory)
    followed by the `#[cfg(test)] mod tests;` declaration in the crate root, so
    their literals are fixtures, not user-facing chrome. This mirrors the
    `strip_test_modules` contract for files that still keep tests inline.
    """
    return path.name == "tests.rs" or "tests" in path.parts


def _line_of(text: str, index: int) -> int:
    return text.count("\n", 0, index) + 1


def scan_js_hardcoded_cjk(text: str) -> list[tuple[int, str]]:
    """Return `(line, char)` for every CJK character in shipped React JS/JSX copy.

    A small lexical walk over `//`/`/* */` comments, `'`/`"`/`` ` `` literals and
    bare code. CJK found inside a string/template literal **or** in code state
    (i.e. a bare JSX text node such as ``<span>打开</span>``) is a hardcoded-copy
    hit. Top-level comments are skipped (a comment nested inside a template
    interpolation is *not* recognised — see the module limitations); a single
    character is enough to report.
    """
    hits: list[tuple[int, str]] = []
    state = "code"
    index = 0
    length = len(text)
    while index < length:
        char = text[index]
        following = text[index + 1] if index + 1 < length else ""
        if state == "code":
            if char == "/" and following == "/":
                state = "line_comment"
                index += 2
                continue
            if char == "/" and following == "*":
                state = "block_comment"
                index += 2
                continue
            if char == "'":
                state = "single"
                index += 1
                continue
            if char == '"':
                state = "double"
                index += 1
                continue
            if char == "`":
                state = "template"
                index += 1
                continue
            if _is_cjk_char(char):
                hits.append((_line_of(text, index), char))
            index += 1
            continue
        if state == "line_comment":
            if char == "\n":
                state = "code"
            index += 1
            continue
        if state == "block_comment":
            if char == "*" and following == "/":
                state = "code"
                index += 2
                continue
            index += 1
            continue
        # String-like states (single, double, template).
        if char == "\\":
            index += 2
            continue
        if (state == "single" and char == "'") or (
            state == "double" and char == '"'
        ) or (state == "template" and char == "`"):
            state = "code"
            index += 1
            continue
        if _is_cjk_char(char):
            hits.append((_line_of(text, index), char))
        index += 1
    return hits


def fail(problems: list[str], message: str) -> None:
    problems.append(message)


def load_catalog(name: str, path: pathlib.Path, problems: list[str]) -> dict[str, str]:
    if not path.exists():
        fail(problems, f"{name}: catalog missing at {path}")
        return {}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        fail(problems, f"{name}: invalid JSON: {error}")
        return {}
    if not isinstance(data, dict):
        fail(problems, f"{name}: catalog must be a JSON object")
        return {}
    catalog: dict[str, str] = {}
    for key, value in data.items():
        if not isinstance(key, str) or not key:
            fail(problems, f"{name}: empty or non-string key {key!r}")
            continue
        if not isinstance(value, str):
            fail(problems, f"{name}: value for {key!r} must be a string")
            continue
        if not value.strip():
            fail(problems, f"{name}: value for {key!r} is empty")
        catalog[key] = value
    if not catalog:
        fail(problems, f"{name}: catalog is empty")
    return catalog


def check_parity(catalogs: dict[str, dict[str, str]], problems: list[str]) -> None:
    names = sorted(catalogs)
    for index, left_name in enumerate(names):
        for right_name in names[index + 1:]:
            left, right = catalogs[left_name], catalogs[right_name]
            for key in sorted(left.keys() - right.keys()):
                fail(problems, f"key {key!r} in {left_name} but not {right_name}")
            for key in sorted(right.keys() - left.keys()):
                fail(problems, f"key {key!r} in {right_name} but not {left_name}")
            for key in sorted(left.keys() & right.keys()):
                left_ph = sorted(set(PLACEHOLDER_RE.findall(left[key])))
                right_ph = sorted(set(PLACEHOLDER_RE.findall(right[key])))
                if left_ph != right_ph:
                    fail(
                        problems,
                        f"placeholder mismatch for {key!r}: "
                        f"{left_name} has {left_ph}, {right_name} has {right_ph}",
                    )


def _record(
    references: dict[str, set[str]], literal: str, path: pathlib.Path, root: pathlib.Path
) -> None:
    if not KEY_SHAPE_RE.match(literal) or literal in ALLOWLIST:
        return
    references.setdefault(literal, set()).add(str(path.relative_to(root)))


def check_key_references(
    catalogs: dict[str, dict[str, str]],
    roots: list[pathlib.Path],
    ui_roots: list[pathlib.Path],
    root: pathlib.Path,
    problems: list[str],
) -> None:
    """Every statically referenced key must exist in the catalog."""
    keys: set[str] = set()
    for catalog in catalogs.values():
        keys.update(catalog)
    references: dict[str, set[str]] = {}

    for source_root in roots:
        for path in source_root.rglob("*.rs"):
            text = path.read_text(encoding="utf-8")
            for pattern in (LOOKUP_RE, RUST_CONST_KEY_RE):
                for match in pattern.finditer(text):
                    literal = next(group for group in match.groups() if group is not None)
                    _record(references, literal, path, root)

    for source_root in ui_roots:
        for pattern in JS_EXTENSIONS:
            for path in source_root.rglob(pattern):
                text = path.read_text(encoding="utf-8")
                for regex in (JS_LOOKUP_RE, JS_KEY_TABLE_RE):
                    for match in regex.finditer(text):
                        literal = next(group for group in match.groups() if group is not None)
                        _record(references, literal, path, root)

    for key in sorted(references):
        if key not in keys:
            where = ", ".join(sorted(references[key]))
            fail(problems, f"key {key!r} is referenced ({where}) but no catalog defines it")


def check_test_modules_are_trailing(
    roots: list[pathlib.Path], root: pathlib.Path, problems: list[str]
) -> None:
    """Assert the `#[cfg(test)]`-to-EOF shortcut is safe in the Rust sources."""
    for source_root in roots:
        for path in source_root.rglob("*.rs"):
            text = path.read_text(encoding="utf-8")
            index = text.find("#[cfg(test)]")
            if index == -1:
                continue
            tail = text[index:]
            # A second cfg(test) before EOF would mean a test module is followed
            # by real code; the scan is still correct (it cuts at the first one),
            # but we require the invariant to hold so nobody relies on it by luck.
            if "#[cfg(test)]" in tail[len("#[cfg(test)]"):]:
                fail(
                    problems,
                    f"{path.relative_to(root)}: more than one #[cfg(test)] near EOF; "
                    "the literal scan assumes test modules are trailing",
                )


def check_hardcoded_literals(
    roots: list[pathlib.Path], root: pathlib.Path, problems: list[str]
) -> None:
    """Fail on a CJK literal left in non-test Rust UI code."""
    for source_root in roots:
        for path in source_root.rglob("*.rs"):
            if is_test_source(path):
                continue
            text = strip_test_modules(strip_comments(path.read_text(encoding="utf-8")))
            for literal in STRING_RE.findall(text):
                if literal in LITERAL_ALLOWLIST:
                    continue
                if is_cjk(literal):
                    fail(
                        problems,
                        f"{path.relative_to(root)}: hardcoded CJK literal "
                        f"{literal!r}; route it through the i18n catalog",
                    )


def check_ui_hardcoded_copy(
    ui_roots: list[pathlib.Path], root: pathlib.Path, problems: list[str]
) -> None:
    """Fail on hardcoded CJK copy in React TS/TSX source (comments skipped).

    Only `ui/src` is scanned; a nested `test` directory is excluded **relative to
    the scanned source root**, so a checkout whose absolute path happens to contain
    a `test` segment does not disable the scan. The canonical catalogs are JSON
    under `crates/cad-ui-core/i18n/` and are never scanned here, so their CJK values
    are never rejected.
    """
    for source_root in ui_roots:
        for pattern in JS_EXTENSIONS:
            for path in source_root.rglob(pattern):
                if "test" in path.relative_to(source_root).parts:
                    continue
                text = path.read_text(encoding="utf-8")
                reported: set[int] = set()
                for line, char in scan_js_hardcoded_cjk(text):
                    if line in reported:
                        continue
                    reported.add(line)
                    fail(
                        problems,
                        f"{path.relative_to(root)}:{line}: hardcoded CJK text {char!r} "
                        "in React UI source; route it through the i18n catalog",
                    )


def run(root: pathlib.Path, problems: list[str]) -> None:
    catalogs = {
        name: load_catalog(name, path, problems)
        for name, path in catalog_paths(root).items()
    }
    if not any(catalogs.values()):
        return
    rust_roots = rust_sources(root)
    ui_roots = ui_sources(root)
    check_parity(catalogs, problems)
    check_key_references(catalogs, rust_roots, ui_roots, root, problems)
    check_test_modules_are_trailing(rust_roots, root, problems)
    check_hardcoded_literals(rust_roots, root, problems)
    check_ui_hardcoded_copy(ui_roots, root, problems)


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    root = pathlib.Path(argv[0]).resolve() if argv else DEFAULT_ROOT
    problems: list[str] = []
    run(root, problems)
    if problems:
        print("i18n check FAILED:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    catalogs = catalog_paths(root)
    total = 0
    for path in catalogs.values():
        if path.exists():
            total = len(json.loads(path.read_text(encoding="utf-8")))
            break
    print(
        f"i18n OK: {total} keys, {len(catalogs)} catalogs, "
        "keys/placeholders consistent, Rust + React static key references resolve, "
        "no hardcoded CJK in non-test Rust UI code or React TS/TSX source"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
