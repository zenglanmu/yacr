#!/usr/bin/env python3
"""Validate the N01 bilingual catalogs (spec v2.0 §3.5, audit U10).

Checks, all of which must pass:

1. both catalogs parse as flat JSON objects of string values;
2. neither catalog is empty;
3. the key sets are identical (and no empty/duplicate keys);
4. every key has the same ``{placeholder}`` set in both languages;
5. every key referenced from Rust UI code exists in the catalog (best effort
   against a documented allowlist of dynamic/legacy identifiers).

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

# `text("key")`, `message("key", ...)` and `format!("key", ...)`-style lookups.
LOOKUP_RE = re.compile(
    r"""\.(?:text|message)\(\s*"([^"\\]*)"|format!\(\s*"([^"\\]*)\""""
)
KEY_SHAPE_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$")
PLACEHOLDER_RE = re.compile(r"(?<!\\)\{([A-Za-z_][A-Za-z0-9_]*)\}")


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


def main() -> int:
    problems: list[str] = []
    catalogs = {name: load_catalog(name, path, problems) for name, path in CATALOGS.items()}
    if any(catalogs.values()):
        check_parity(catalogs, problems)
        check_rust_references(catalogs, problems)
    if problems:
        print("i18n check FAILED:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    total = len(next(iter(catalogs.values()), {}))
    print(
        f"i18n OK: {total} keys, {len(CATALOGS)} catalogs, "
        "keys/placeholders/Rust references consistent"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
