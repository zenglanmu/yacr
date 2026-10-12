#!/usr/bin/env python3
"""Synthetic archive contracts for the Android APK font verifier."""
from __future__ import annotations

import importlib.util
import json
import pathlib
import tempfile
import unittest
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
MODULE_PATH = ROOT / "scripts/check-android-apk-fonts.py"
SPEC = importlib.util.spec_from_file_location("check_android_apk_fonts", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class AndroidApkFontContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.fonts = self.root / "fonts"
        self.fonts.mkdir()
        self.osifont = self.root / "repo-osifont.ttf"
        self.osifont.write_bytes(b"licensed osifont fixture")
        self.files = {
            "simplex.shx": b"synthetic SHX content",
            "osifont.ttf": self.osifont.read_bytes(),
        }
        self.write_manifest()
        for name, content in self.files.items():
            (self.fonts / name).write_bytes(content)
        self.apk = self.root / "test.apk"
        self.write_apk()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_manifest(self, entries: list[dict[str, object]] | None = None) -> None:
        if entries is None:
            entries = [
                {"file": "simplex.shx", "name": ["simplex"], "type": "shx"},
                {"file": "osifont.ttf", "name": ["osifont"], "type": "mesh"},
            ]
        (self.fonts / "fonts.json").write_text(
            json.dumps(entries, ensure_ascii=False), encoding="utf-8"
        )

    def write_apk(
        self,
        *,
        include_manifest: bool = True,
        files: dict[str, bytes] | None = None,
    ) -> None:
        packaged = self.files if files is None else files
        with zipfile.ZipFile(self.apk, "w") as archive:
            if include_manifest:
                archive.write(self.fonts / "fonts.json", "assets/fonts/fonts.json")
            for name, content in packaged.items():
                archive.writestr(f"assets/fonts/{name}", content)

    def test_valid_package_checks_dynamic_manifest_and_repository_osifont(self) -> None:
        count, total, _ = CHECKER.validate(self.apk, self.fonts, self.osifont)
        self.assertEqual(count, 2)
        self.assertEqual(total, sum(map(len, self.files.values())))

    def test_missing_manifest_is_rejected(self) -> None:
        self.write_apk(include_manifest=False)
        with self.assertRaisesRegex(CHECKER.FontPackageError, "missing assets/fonts/fonts.json"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)

    def test_missing_font_is_rejected(self) -> None:
        self.write_apk(files={"osifont.ttf": self.files["osifont.ttf"]})
        with self.assertRaisesRegex(CHECKER.FontPackageError, "missing font asset assets/fonts/simplex.shx"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)

    def test_empty_font_is_rejected(self) -> None:
        self.write_apk(files={**self.files, "simplex.shx": b""})
        with self.assertRaisesRegex(CHECKER.FontPackageError, "font asset is empty"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)

    def test_hash_mismatch_is_rejected(self) -> None:
        self.write_apk(files={**self.files, "simplex.shx": b"different bytes"})
        with self.assertRaisesRegex(CHECKER.FontPackageError, "differs from prepared asset"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)

    def test_manifest_mismatch_is_rejected(self) -> None:
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("assets/fonts/fonts.json", b"[]")
            for name, content in self.files.items():
                archive.writestr(f"assets/fonts/{name}", content)
        with self.assertRaisesRegex(CHECKER.FontPackageError, "differs from the prepared manifest"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)

    def test_duplicate_catalogue_file_is_rejected(self) -> None:
        self.write_manifest(
            [
                {"file": "simplex.shx", "name": ["simplex"], "type": "shx"},
                {"file": "simplex.shx", "name": ["duplicate"], "type": "shx"},
                {"file": "osifont.ttf", "name": ["osifont"], "type": "mesh"},
            ]
        )
        self.write_apk()
        with self.assertRaisesRegex(CHECKER.FontPackageError, "duplicate font file"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)

    def test_catalogue_path_escape_is_rejected(self) -> None:
        self.write_manifest(
            [
                {"file": "../escape.shx", "name": ["escape"], "type": "shx"},
                {"file": "osifont.ttf", "name": ["osifont"], "type": "mesh"},
            ]
        )
        self.write_apk()
        with self.assertRaisesRegex(CHECKER.FontPackageError, "unsafe font path"):
            CHECKER.validate(self.apk, self.fonts, self.osifont)


if __name__ == "__main__":
    unittest.main()
