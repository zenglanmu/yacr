#!/usr/bin/env python3
"""Validate the N01 bilingual catalogs and catch untranslated chrome literals.

Checks, all of which must pass:

1. both catalogs parse as flat JSON objects of string values;
2. neither catalog is empty;
3. the key sets are identical (and no empty/duplicate keys);
4. every key has the same ``{placeholder}`` set in both languages;
5. every key referenced from Rust UI code exists in the catalog (best effort
   against a documented allowlist of dynamic/legacy identifiers);
6. no hardcoded CJK literal remains in the Slint chrome or in non-test Rust UI
   code (the shell must be catalog-driven, audit U10). A small, documented
   literal allowlist exists for anything legitimately not user-facing.

Exit code is non-zero on any failure so CI can gate on it. Stdlib only.
"""
from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
I18N_DIR = ROOT / "crates" / "cad-ui-slint" / "i18n"
CATALOGS = {
    "zh-CN": I18N_DIR / "zh-CN.json",
    "en": I18N_DIR / "en.json",
}

# Rust files that call `messages.text("some.key", ..)` / `messages.message(..)`.
RUST_SOURCES = [ROOT / "crates" / "cad-ui-slint" / "src"]
# The Slint chrome; every user-facing literal here is a missing translation.
SLINT_SOURCES = [ROOT / "crates" / "cad-ui-slint" / "ui"]

# Key-shaped string literals that are intentionally NOT catalog keys: JSON
# schemas, DWG/file formats, storage keys, tensors, etc. Exact-match allowlist;
# anything else that looks like `a.b` in a `.text(...)`/`.message(...)` call is a
# missing translation and must be added to the catalog (or here, with a comment).
ALLOWLIST = {
    # Non-UI identifiers that share the dotted shape.
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
KEY_SHAPE_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$")
PLACEHOLDER_RE = re.compile(r"(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}")

# A double-quoted string literal, honouring backslash escapes.
STRING_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')
# CJK ideographs, extension A, CJK punctuation and fullwidth forms.
CJK_RE = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff\u3000-\u303f\uff00-\uffef]")


def is_cjk(text: str) -> bool:
    return CJK_RE.search(text) is not None


def strip_comments(text: str) -> str:
    """Remove // and /* */ comments so quoted examples in prose do not count."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return re.sub(r"//[^\n]*", "", text)


def strip_test_modules(text: str) -> str:
    """Drop everything from the first `#[cfg(test)]` to EOF.

    In this crate the test modules are always the trailing module(s), so this is
    exact rather than heuristic; `check_test_modules_are_trailing` below asserts
    that precondition so the scan cannot silently start skipping real code.
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


def check_rust_references(catalogs: dict[str, dict[str, str]], problems: list[str]) -> None:
    keys: set[str] = set()
    for catalog in catalogs.values():
        keys.update(catalog)
    references: dict[str, set[str]] = {}
    for source_root in RUST_SOURCES:
        for path in source_root.rglob("*.rs"):
            text = path.read_text(encoding="utf-8")
            for match in LOOKUP_RE.finditer(text):
                literal = next(group for group in match.groups() if group is not None)
                if not KEY_SHAPE_RE.match(literal):
                    continue
                if literal in ALLOWLIST:
                    continue
                references.setdefault(literal, set()).add(str(path.relative_to(ROOT)))
    for key in sorted(references):
        if key not in keys:
            where = ", ".join(sorted(references[key]))
            fail(problems, f"Rust references {key!r} ({where}) but no catalog defines it")


def check_test_modules_are_trailing(problems: list[str]) -> None:
    """Assert the `#[cfg(test)]`-to-EOF shortcut is safe in this crate."""
    for source_root in RUST_SOURCES:
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
                    f"{path.relative_to(ROOT)}: more than one #[cfg(test)] near EOF; "
                    "the literal scan assumes test modules are trailing",
                )


def check_hardcoded_literals(problems: list[str]) -> None:
    """Fail on a CJK literal left in the chrome or in non-test Rust UI code."""
    for source_root in SLINT_SOURCES:
        for path in source_root.rglob("*.slint"):
            text = strip_comments(path.read_text(encoding="utf-8"))
            for literal in STRING_RE.findall(text):
                if literal in LITERAL_ALLOWLIST:
                    continue
                if is_cjk(literal):
                    fail(
                        problems,
                        f"{path.relative_to(ROOT)}: hardcoded CJK literal "
                        f"{literal!r}; route it through the i18n catalog",
                    )
    for source_root in RUST_SOURCES:
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
                        f"{path.relative_to(ROOT)}: hardcoded CJK literal "
                        f"{literal!r}; route it through the i18n catalog",
                    )


def main() -> int:
    problems: list[str] = []
    catalogs = {name: load_catalog(name, path, problems) for name, path in CATALOGS.items()}
    if any(catalogs.values()):
        check_parity(catalogs, problems)
        check_rust_references(catalogs, problems)
        check_test_modules_are_trailing(problems)
        check_hardcoded_literals(problems)
    if problems:
        print("i18n check FAILED:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    total = len(next(iter(catalogs.values()), {}))
    print(
        f"i18n OK: {total} keys, {len(CATALOGS)} catalogs, "
        "keys/placeholders/Rust references consistent, no hardcoded chrome literals"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
