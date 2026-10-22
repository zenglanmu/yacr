#!/usr/bin/env python3
"""Contract tests for the migration guards (authored; run in CI/P6).

Entrypoint:  python3 scripts/test-ui-migration-guards.py

These are **negative + positive controls** for the two guards that enforce the
Slint→React migration invariants. Each control builds a throwaway fixture
workspace under a temporary directory and runs the guard as a subprocess with an
explicit root, so the real repository is never modified:

* `scripts/check-architecture.py` — the Slint/toolkit ban must catch a
  `slint`/`slint-build`/`iced` dependency no matter which table declares it
  (runtime, `build-dependencies`, `dev-dependencies`, target-specific), including
  Cargo dependency renames (`x = { package = "slint-build" }`) and
  workspace-inherited aliases (`x.workspace = true` resolving through the root
  `[workspace.dependencies]`), while still accepting a legitimate non-toolkit
  build dependency and a benign inherited alias.
* `scripts/check-i18n.py` — hardcoded CJK copy in React JSX/JS source (bare JSX
  text, string literal, template literal, attribute) must be rejected, while a
  `t('key')` lookup plus a CJK comment and CJK catalog values are accepted; the
  scan must not be disabled by a checkout whose ancestor path contains `test`.

Stdlib only. The tests shell out to the guards; they do not import them.
"""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
ARCHITECTURE_GUARD = ROOT / "scripts" / "check-architecture.py"
I18N_GUARD = ROOT / "scripts" / "check-i18n.py"


def run_guard(guard: pathlib.Path, workspace_root: pathlib.Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(guard), str(workspace_root)],
        capture_output=True,
        text=True,
    )


# --------------------------------------------------------------------------- #
# check-architecture.py controls
# --------------------------------------------------------------------------- #

# Minimal member set that satisfies every require/forbid rule of the guard.
ARCH_MEMBERS: dict[str, str] = {
    "crates/cad-domain": (
        '[package]\nname = "cad-domain"\nversion = "0.1.0"\n'
    ),
    "crates/cad-db": (
        '[package]\nname = "cad-db"\nversion = "0.1.0"\n\n'
        '[dependencies]\ncad-domain = { path = "../cad-domain" }\n'
    ),
    "crates/cad-app": (
        '[package]\nname = "cad-app"\nversion = "0.1.0"\n'
    ),
    "crates/cad-host": (
        '[package]\nname = "cad-host"\nversion = "0.1.0"\n'
    ),
    "crates/cad-platform": (
        '[package]\nname = "cad-platform"\nversion = "0.1.0"\n'
    ),
    "crates/cad-engine": (
        '[package]\nname = "cad-engine"\nversion = "0.1.0"\n\n'
        '[dependencies]\ncad-app = { path = "../cad-app" }\n'
        'cad-render-wgpu = { path = "../cad-render-wgpu" }\n'
    ),
    "crates/cad-render-wgpu": (
        '[package]\nname = "cad-render-wgpu"\nversion = "0.1.0"\n\n'
        '[dependencies]\nwgpu = "=30.0.1"\n'
    ),
    "crates/cad-import-acadrust": (
        '[package]\nname = "cad-import-acadrust"\nversion = "0.1.0"\n\n'
        '[dependencies]\nacadrust = "0.6.3"\n'
    ),
    "crates/cad-ui-core": (
        '[package]\nname = "cad-ui-core"\nversion = "0.1.0"\n\n'
        '[dependencies]\ncad-app = { path = "../cad-app" }\n'
        'cad-host = { path = "../cad-host" }\n'
    ),
    "crates/cad-desktop-shell": (
        '[package]\nname = "cad-desktop-shell"\nversion = "0.1.0"\n\n'
        '[dependencies]\ncad-ui-core = { path = "../cad-ui-core" }\n'
    ),
    "apps/app-linux": (
        '[package]\nname = "app-linux"\nversion = "0.1.0"\n\n'
        '[dependencies]\ncad-desktop-shell = { path = "../../crates/cad-desktop-shell" }\n'
    ),
}


class ArchitectureGuardContract(unittest.TestCase):
    """The toolkit ban rejects Slint in every dependency table (incl. renames)."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._tmp.name) / "workspace"
        self.root.mkdir()
        self.write_root()
        for member, text in ARCH_MEMBERS.items():
            directory = self.root / member
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "Cargo.toml").write_text(text, encoding="utf-8")

    def write_root(self, workspace_dependencies: str = "") -> None:
        """(Re)write the root manifest, optionally with `[workspace.dependencies]`."""
        members = ", ".join(f'"{member}"' for member in ARCH_MEMBERS)
        text = f'[workspace]\nresolver = "2"\nmembers = [{members}]\n'
        if workspace_dependencies:
            text += f'\n[workspace.dependencies]\n{workspace_dependencies}\n'
        (self.root / "Cargo.toml").write_text(text, encoding="utf-8")

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def append_to(self, member: str, extra: str) -> None:
        manifest = self.root / member / "Cargo.toml"
        manifest.write_text(manifest.read_text(encoding="utf-8") + extra, encoding="utf-8")

    def assert_rejected(self, completed: subprocess.CompletedProcess) -> None:
        self.assertNotEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("FAILED", completed.stderr)

    def test_baseline_workspace_is_accepted(self) -> None:
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("Architecture OK", completed.stdout)

    def test_legitimate_build_dependency_is_accepted(self) -> None:
        # Proves the widened collection does not over-reject ordinary build deps.
        self.append_to("crates/cad-app", '\n[build-dependencies]\ncc = "1"\n')
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_slint_build_in_build_dependencies_is_rejected(self) -> None:
        self.append_to(
            "crates/cad-app", '\n[build-dependencies]\nslint-build = "=1.18.1"\n'
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)
        self.assertIn("slint-build", completed.stderr)

    def test_slint_build_in_target_build_dependencies_is_rejected(self) -> None:
        self.append_to(
            "crates/cad-app",
            '\n[target.\'cfg(target_os = "linux")\'.build-dependencies]\n'
            'slint-build = "=1.18.1"\n',
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)

    def test_slint_in_dev_dependencies_is_rejected(self) -> None:
        self.append_to("crates/cad-app", '\n[dev-dependencies]\nslint = "=1.18.1"\n')
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)

    def test_slint_build_in_target_dev_dependencies_is_rejected(self) -> None:
        self.append_to(
            "crates/cad-app",
            '\n[target.\'cfg(target_os = "linux")\'.dev-dependencies]\n'
            'slint-build = "=1.18.1"\n',
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)

    def test_renamed_slint_build_dependency_is_rejected(self) -> None:
        self.append_to(
            "crates/cad-app",
            '\n[build-dependencies]\n'
            'ui_toolkit = { package = "slint-build", version = "=1.18.1" }\n',
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)
        self.assertIn("slint-build", completed.stderr)

    def test_iced_dependency_is_rejected(self) -> None:
        self.append_to(
            "crates/cad-app", '\n[target.\'cfg(target_os = "linux")\'.dependencies]\niced = "0.13"\n'
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)

    def test_inherited_slint_build_alias_in_build_dependencies_is_rejected(self) -> None:
        # `ui_toolkit.workspace = true` inherits the alias from the selected root;
        # the real package is `slint-build`, so the toolkit ban must resolve it.
        self.write_root(
            'ui_toolkit = { package = "slint-build", version = "=1.18.1" }\n'
        )
        self.append_to(
            "crates/cad-app",
            "\n[build-dependencies]\nui_toolkit.workspace = true\n",
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)
        self.assertIn("slint-build", completed.stderr)

    def test_inherited_slint_build_alias_in_target_dev_dependencies_is_rejected(self) -> None:
        self.write_root(
            'ui_toolkit = { package = "slint-build", version = "=1.18.1" }\n'
        )
        self.append_to(
            "crates/cad-app",
            '\n[target.\'cfg(target_os = "linux")\'.dev-dependencies]\n'
            "ui_toolkit.workspace = true\n",
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assert_rejected(completed)
        self.assertIn("slint-build", completed.stderr)

    def test_inherited_benign_alias_is_accepted(self) -> None:
        # An inherited alias that is not a toolkit must not be over-rejected: with
        # no `package` rename it resolves to its own key.
        self.write_root('cc_toolkit = "1"\n')
        self.append_to(
            "crates/cad-app",
            "\n[build-dependencies]\ncc_toolkit.workspace = true\n",
        )
        completed = run_guard(ARCHITECTURE_GUARD, self.root)
        self.assertEqual(completed.returncode, 0, completed.stderr)


# --------------------------------------------------------------------------- #
# check-i18n.py controls
# --------------------------------------------------------------------------- #

CATALOG_ZH = {"file.open": "打开图纸", "units.unknown": "图纸单位"}
CATALOG_EN = {"file.open": "Open Drawing", "units.unknown": "Drawing units"}


class I18nGuardContract(unittest.TestCase):
    """Hardcoded CJK React copy is rejected; catalog-driven copy is accepted."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._tmp.name) / "workspace"
        i18n_dir = self.root / "crates" / "cad-ui-core" / "i18n"
        i18n_dir.mkdir(parents=True)
        (i18n_dir / "zh-CN.json").write_text(
            json.dumps(CATALOG_ZH, ensure_ascii=False), encoding="utf-8"
        )
        (i18n_dir / "en.json").write_text(
            json.dumps(CATALOG_EN, ensure_ascii=False), encoding="utf-8"
        )
        self.ui_src = self.root / "apps" / "desktop-ui" / "src"
        self.ui_src.mkdir(parents=True)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def write_sources(self, files: dict[str, str]) -> None:
        for name in list(self.ui_src.rglob("*")):
            if name.is_file():
                name.unlink()
        for name, text in files.items():
            (self.ui_src / name).write_text(text, encoding="utf-8")

    def assert_rejected(self, completed: subprocess.CompletedProcess) -> None:
        self.assertNotEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertIn("hardcoded CJK", completed.stderr)

    def test_catalog_driven_source_with_cjk_comment_and_catalog_is_accepted(self) -> None:
        # CJK in the canonical JSON catalog and in a JS comment must be accepted;
        # the visible copy comes from `t('key')`.
        self.write_sources(
            {
                "App.jsx": (
                    "// 打开图纸 — catalog-driven only\n"
                    "export const label = t('file.open');\n"
                )
            }
        )
        completed = run_guard(I18N_GUARD, self.root)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("i18n OK", completed.stdout)

    def test_hardcoded_jsx_text_is_rejected(self) -> None:
        self.write_sources(
            {"Bad.jsx": "export const View = () => <span>打开图纸</span>;\n"}
        )
        self.assert_rejected(run_guard(I18N_GUARD, self.root))

    def test_hardcoded_string_literal_is_rejected(self) -> None:
        self.write_sources({"Bad.jsx": "export const title = '打开图纸';\n"})
        self.assert_rejected(run_guard(I18N_GUARD, self.root))

    def test_hardcoded_jsx_attribute_is_rejected(self) -> None:
        self.write_sources(
            {"Bad.jsx": 'export const View = () => <button title="关闭">x</button>;\n'}
        )
        self.assert_rejected(run_guard(I18N_GUARD, self.root))

    def test_hardcoded_template_literal_is_rejected(self) -> None:
        self.write_sources({"bad.mjs": "export const label = `打开图纸`;\n"})
        self.assert_rejected(run_guard(I18N_GUARD, self.root))

    def test_block_comment_cjk_is_ignored(self) -> None:
        # `/* ... */` (incl. the JSX `{/* */}` form) must not be treated as copy.
        self.write_sources(
            {"App.jsx": "/* 打开图纸 */\nexport const label = t('units.unknown');\n"}
        )
        completed = run_guard(I18N_GUARD, self.root)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_sources_under_a_test_named_ancestor_are_still_scanned(self) -> None:
        # Regression: `test` exclusion must be relative to the scanned source root,
        # never the absolute path — a checkout under `.../test/...` must still scan.
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp) / "test" / "workspace"
            i18n_dir = root / "crates" / "cad-ui-core" / "i18n"
            i18n_dir.mkdir(parents=True)
            (i18n_dir / "zh-CN.json").write_text(
                json.dumps(CATALOG_ZH, ensure_ascii=False), encoding="utf-8"
            )
            (i18n_dir / "en.json").write_text(
                json.dumps(CATALOG_EN, ensure_ascii=False), encoding="utf-8"
            )
            src = root / "apps" / "desktop-ui" / "src"
            src.mkdir(parents=True)
            (src / "Bad.jsx").write_text(
                "export const View = () => <span>打开图纸</span>;\n", encoding="utf-8"
            )
            completed = run_guard(I18N_GUARD, root)
            self.assert_rejected(completed)
            self.assertIn("Bad.jsx", completed.stderr)


if __name__ == "__main__":
    unittest.main()
