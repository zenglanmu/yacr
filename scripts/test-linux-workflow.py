#!/usr/bin/env python3
"""Mutation contracts for the primary Linux workflow (no runner/GPU claim)."""
import importlib.util
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("workflow", ROOT / "scripts/check-workflows.py")
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)


class LinuxWorkflowContracts(unittest.TestCase):
    def check_text(self, text):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            path = pathlib.Path(directory) / "test.yml"
            path.write_text(text)
            return workflow.structural_pass([path])[1]

    def test_real_linux_job_is_required_and_valid(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        self.assertIn("linux-app", workflow.REQUIRED_JOBS)
        self.assertEqual(self.check_text(text), [])

    def test_primary_job_cannot_skip_or_ignore_failure(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        for directive in ("    if: false\n", "    continue-on-error: true\n"):
            self.assertTrue(self.check_text(text.replace("  linux-app:\n", "  linux-app:\n" + directive)))

    def test_host_compilation_cannot_be_replaced_with_empty_success(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        self.assertTrue(self.check_text(text.replace("cargo check -p app-linux --all-targets --locked", "true")))

    def test_default_linux_gate_does_not_render_or_build_release(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        block = workflow.job_block(text, "linux-app")
        self.assertIsNotNone(block)
        for forbidden in ("check-linux-app.sh", "--release", "target/release/", "cargo test", "--headless"):
            self.assertNotIn(forbidden, block)

    def test_core_gate_compiles_without_running_the_full_suite(self):
        text = (ROOT / ".github/workflows/core.yml").read_text()
        self.assertEqual(self.check_text(text), [])
        block = workflow.job_block(text, "core-quality")
        self.assertIsNotNone(block)
        self.assertNotIn("cargo test", block)
        self.assertNotIn("--release", block)


if __name__ == "__main__":
    unittest.main()
