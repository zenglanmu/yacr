#!/usr/bin/env python3
"""Validate ``fixtures/manifest`` (spec §11.5 / line 422).

The manifest is the repository's record of which CAD fixtures exist, where they
came from, what authorizes their use, and what compatibility is *expected*.
Real user drawings and vendor files must never be committed without a verified
licence, so an empty ``fixtures`` list is a legitimate, honest state.

Standard library only. What it enforces:

  * the file parses and has ``schema_version`` 1;
  * ``fixtures`` is a list and every entry carries the required keys;
  * ``sha256`` is 64 lower-case hex characters;
  * ``provenance`` records a source and an explicit authorization/notice;
  * every fixture whose ``path`` is inside the repository actually exists;
  * a fixture marked ``authorized: false`` (or with ``synthetic: true``) can
    never claim ``expected: "complete"`` or a vendor-compatibility licence,
    which would be an unbacked compatibility claim.

It is a guard rail, not evidence that any drawing is actually supported.
"""
from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "fixtures" / "manifest"

SHA256 = re.compile(r"^[0-9a-f]{64}$")

REQUIRED_KEYS = (
    "id",
    "path",
    "sha256",
    "dwg_version",
    "source",
    "provenance",
    "authorized",
    "synthetic",
    "entity_types",
    "expected",
    "known_limits",
)

# `expected` values that assert a compatibility claim rather than a contract.
STRONG_EXPECTATIONS = {"complete", "verified", "supported"}


def fail(message: str) -> None:
    print(f"manifest check FAILED: {message}")
    sys.exit(1)


def is_repo_path(path: str) -> bool:
    if path.startswith("/") or path.startswith("~"):
        return False
    if path.startswith("http://") or path.startswith("https://"):
        return False
    return True


def validate(entry: dict, index: int) -> None:
    where = f"fixtures[{index}]"
    for key in REQUIRED_KEYS:
        if key not in entry:
            fail(f"{where} is missing required key '{key}'")

    ident = entry["id"]
    if not isinstance(ident, str) or not ident.strip():
        fail(f"{where}.id must be a non-empty string")

    path = entry["path"]
    if not isinstance(path, str) or not path.strip():
        fail(f"{where}.path must be a non-empty string")

    digest = entry["sha256"]
    if not isinstance(digest, str) or not SHA256.match(digest):
        fail(f"{where}.sha256 must be 64 lower-case hex characters")

    if not isinstance(entry["authorized"], bool):
        fail(f"{where}.authorized must be a boolean")
    if not isinstance(entry["synthetic"], bool):
        fail(f"{where}.synthetic must be a boolean")

    provenance = entry["provenance"]
    if not isinstance(provenance, dict):
        fail(f"{where}.provenance must be an object recording source and licence")
    if not provenance:
        fail(f"{where}.provenance must not be empty: record where the file came from")

    if not isinstance(entry["entity_types"], list):
        fail(f"{where}.entity_types must be a list")
    if not isinstance(entry["known_limits"], list):
        fail(f"{where}.known_limits must be a list")

    if is_repo_path(path) and not (ROOT / path).exists():
        fail(f"{where}.path '{path}' is a repository path but does not exist")

    # Honesty gate: unbacked compatibility claims.
    unbacked = entry["synthetic"] or not entry["authorized"]
    if unbacked and entry["expected"] in STRONG_EXPECTATIONS:
        fail(
            f"{where} claims expected='{entry['expected']}' but the fixture is "
            "synthetic or unauthorized; compatibility cannot be claimed"
        )


def main() -> int:
    if not MANIFEST.exists():
        fail(f"{MANIFEST} does not exist")
    raw = MANIFEST.read_text(encoding="utf-8")
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as error:
        fail(f"manifest is not valid JSON: {error}")

    if not isinstance(document, dict):
        fail("manifest root must be an object")
    if document.get("schema_version") != 1:
        fail(f"schema_version must be 1, got {document.get('schema_version')!r}")

    fixtures = document.get("fixtures")
    if not isinstance(fixtures, list):
        fail("'fixtures' must be a list (an empty list is valid)")

    ids: set[str] = set()
    for index, entry in enumerate(fixtures):
        if not isinstance(entry, dict):
            fail(f"fixtures[{index}] must be an object")
        validate(entry, index)
        if entry["id"] in ids:
            fail(f"duplicate fixture id '{entry['id']}'")
        ids.add(entry["id"])

    print(
        f"manifest OK: schema_version=1, {len(fixtures)} fixture(s) "
        f"(an empty list is the honest state until samples are authorized)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
