"""Synthetic documentation contracts for the historical feature-reference boundary.

The OpenCADStudio functional-reference exercise is closed (2026-10-15 user decision):
the local `docs/ui-requirements/` reference set was removed and UI design now follows
AutoCAD desktop conventions. These contracts assert the new state instead of the old
one: the spec must not point back at the removed directory, the directory must stay
gone, and the historical survey must stay honest (consultation happened, no code was
copied, and it is not an active migration plan).
"""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class FeatureReferenceContractTests(unittest.TestCase):
    def test_spec_requires_independent_implementation(self):
        spec = (ROOT / "CAD_IMPLEMENTATION_SPEC.md").read_text()
        section = spec.split("## 10. ", 1)[1].split("## 11. ", 1)[0]
        self.assertIn("不授权抽取或移植 OpenCADStudio 源码", section)
        self.assertIn("不再作为源码迁移任务或交付门禁", section)
        self.assertNotIn("源 commit/文件/函数 → 目标模块", section)

    def test_removed_ui_reference_set_stays_removed(self):
        self.assertFalse((ROOT / "docs/ui-requirements").exists())
        # The spec may record the removal as history, but must not point at the
        # deleted reference set as a live source.
        spec = (ROOT / "CAD_IMPLEMENTATION_SPEC.md").read_text()
        self.assertNotIn("docs/ui-requirements/00-INDEX.md", spec)
        self.assertNotIn("](docs/ui-requirements", spec)

    def test_historical_mapping_is_not_an_active_migration_plan(self):
        mapping = (ROOT / "docs/migration-map.md").read_text()
        self.assertIn("以下记录保留此前源码调查事实", mapping)
        self.assertIn("不作为后续实现任务或交付门禁", mapping)


if __name__ == "__main__":
    unittest.main()
