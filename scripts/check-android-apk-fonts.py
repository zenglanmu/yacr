#!/usr/bin/env python3
"""Verify that an Android APK contains the exact prepared font package.

The catalogue is intentionally dynamic: the prepared ``fonts.json`` is the
authority for which font assets must be present. This checker does not assume a
catalogue size or a particular upstream revision.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
import zipfile


APK_FONT_PREFIX = "assets/fonts/"
MANIFEST_NAME = f"{APK_FONT_PREFIX}fonts.json"
OSIFONT_NAME = "osifont.ttf"


class FontPackageError(ValueError):
    """Raised when prepared font assets and the APK do not match safely."""


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _font_files(manifest_bytes: bytes) -> list[str]:
    try:
        entries = json.loads(manifest_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise FontPackageError(f"invalid fonts.json: {exc}") from exc
    if not isinstance(entries, list):
        raise FontPackageError("fonts.json must be a JSON array")

    result: list[str] = []
    seen: set[str] = set()
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or not isinstance(entry.get("file"), str):
            raise FontPackageError(f"fonts.json entry {index} must contain a string 'file'")
        name = entry["file"]
        # fetch-fonts currently provides flat basenames. Reject separators,
        # absolute paths, dot components, and NULs rather than allowing an
        # untrusted/upstream catalogue to escape the font asset namespace.
        if (
            not name
            or name in {".", ".."}
            or "/" in name
            or "\\" in name
            or "\x00" in name
            or pathlib.PurePath(name).is_absolute()
        ):
            raise FontPackageError(f"unsafe font path in fonts.json entry {index}: {name!r}")
        if name in seen:
            raise FontPackageError(f"duplicate font file in fonts.json: {name!r}")
        seen.add(name)
        result.append(name)

    if OSIFONT_NAME not in seen:
        raise FontPackageError(f"fonts.json is missing merged repository font {OSIFONT_NAME!r}")
    return result


def validate(
    apk_path: pathlib.Path,
    fonts_dir: pathlib.Path,
    osifont_source: pathlib.Path,
) -> tuple[int, int, str]:
    """Validate package contents; return (font count, bytes, manifest SHA-256)."""
    manifest_path = fonts_dir / "fonts.json"
    if not manifest_path.is_file():
        raise FontPackageError(f"prepared font manifest is missing: {manifest_path}")
    manifest_bytes = manifest_path.read_bytes()
    files = _font_files(manifest_bytes)

    prepared: dict[str, bytes] = {}
    total_bytes = 0
    for name in files:
        path = fonts_dir / name
        # Names are constrained to basenames above; additionally reject symlink
        # assets so validation is against files actually prepared in this dir.
        if path.is_symlink() or not path.is_file():
            raise FontPackageError(f"prepared font is missing or not a regular file: {path}")
        content = path.read_bytes()
        if not content:
            raise FontPackageError(f"prepared font is empty: {name}")
        prepared[name] = content
        total_bytes += len(content)

    reference_osifont = osifont_source.read_bytes()
    if not reference_osifont or _sha256(prepared[OSIFONT_NAME]) != _sha256(reference_osifont):
        raise FontPackageError(
            f"prepared {OSIFONT_NAME} does not match the repository's licensed font asset"
        )

    try:
        with zipfile.ZipFile(apk_path) as apk:
            infos = apk.infolist()
            by_name: dict[str, zipfile.ZipInfo] = {}
            for info in infos:
                if info.filename in by_name and (
                    info.filename == MANIFEST_NAME
                    or info.filename.startswith(APK_FONT_PREFIX)
                    and pathlib.PurePosixPath(info.filename).name in prepared
                ):
                    raise FontPackageError(f"duplicate APK font entry: {info.filename!r}")
                by_name.setdefault(info.filename, info)

            manifest_info = by_name.get(MANIFEST_NAME)
            if manifest_info is None:
                raise FontPackageError(f"APK is missing {MANIFEST_NAME}")
            packaged_manifest = apk.read(manifest_info)
            if packaged_manifest != manifest_bytes:
                raise FontPackageError("APK fonts.json differs from the prepared manifest")

            for name, content in prepared.items():
                archive_name = APK_FONT_PREFIX + name
                info = by_name.get(archive_name)
                if info is None:
                    raise FontPackageError(f"APK is missing font asset {archive_name}")
                if info.file_size == 0:
                    raise FontPackageError(f"APK font asset is empty: {archive_name}")
                packaged = apk.read(info)
                if not packaged:
                    raise FontPackageError(f"APK font asset is empty: {archive_name}")
                if _sha256(packaged) != _sha256(content):
                    raise FontPackageError(f"APK font differs from prepared asset: {archive_name}")
    except zipfile.BadZipFile as exc:
        raise FontPackageError(f"invalid APK/ZIP: {exc}") from exc

    return len(files), total_bytes, _sha256(manifest_bytes)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=pathlib.Path, help="release APK to inspect")
    parser.add_argument(
        "--fonts-dir",
        type=pathlib.Path,
        default=pathlib.Path("apps/app-android/assets/fonts"),
        help="prepared font directory (default: apps/app-android/assets/fonts)",
    )
    parser.add_argument(
        "--osifont-source",
        type=pathlib.Path,
        default=pathlib.Path("fonts/osifont.ttf"),
        help="committed licensed osifont.ttf used by fetch-fonts (default: fonts/osifont.ttf)",
    )
    args = parser.parse_args(argv)
    try:
        count, total_bytes, manifest_hash = validate(
            args.apk, args.fonts_dir, args.osifont_source
        )
    except (FontPackageError, OSError, zipfile.BadZipFile) as exc:
        print(f"Android APK font verification failed: {exc}", file=sys.stderr)
        return 1
    print(
        f"Android APK fonts verified: {count} manifest assets, {total_bytes} bytes; "
        f"fonts.json sha256={manifest_hash}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
