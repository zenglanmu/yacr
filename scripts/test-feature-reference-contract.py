"""Synthetic documentation contracts for feature-only reference boundaries."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class FeatureReferenceContractTests(unittest.TestCase):
    def test_spec_requires_independent_implementation(self):
        spec = (ROOT / "CAD_IMPLEMENTATION_SPEC.md").read_text()
        section = spec.split("## 10. ", 1)[1].split("## 11. ", 1)[0]
        self.assertIn("docs/ui-requirements/00-INDEX.md", section)
        self.assertIn("不授权抽取或移植 OpenCADStudio 源码", section)
        self.assertIn("不再作为源码迁移任务或交付门禁", section)
        self.assertNotIn("源 commit/文件/函数 → 目标模块", section)

    def test_ui_reference_does_not_expand_product_scope(self):
        index = (ROOT / "docs/ui-requirements/00-INDEX.md").read_text()
        self.assertIn("不是源码、算法、shader 或依赖采用来源", index)
        self.assertIn("931 个参考条目不是全部必须交付", index)
        self.assertIn("为唯一需求权威", index)

    def test_historical_mapping_is_not_an_active_migration_plan(self):
        mapping = (ROOT / "docs/migration-map.md").read_text()
        self.assertIn("以下记录保留此前源码调查事实", mapping)
        self.assertIn("不作为后续实现任务或交付门禁", mapping)


if __name__ == "__main__":
    unittest.main()
