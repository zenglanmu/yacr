#!/usr/bin/env python3
"""Mutation contracts for the Cloudflare Pages deploy job (no deploy claim).

Mirrors scripts/test-linux-workflow.py: it proves that the `web-deploy` job
keeps its release trigger (workflow_dispatch / v* tag), its real deploy command
and the verified-artifact download, so a future edit cannot turn it into an
empty success. It never contacts Cloudflare.
"""
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


class WebDeployWorkflowContracts(unittest.TestCase):
    def check_text(self, text):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            path = pathlib.Path(directory) / "test.yml"
            path.write_text(text)
            return workflow.structural_pass([path])[1]

    def test_deploy_job_is_required_and_release_triggered(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        self.assertIn("web-deploy", workflow.REQUIRED_JOBS)
        block = workflow.job_block(text, "web-deploy")
        self.assertIsNotNone(block)
        self.assertIn("startsWith(github.ref, 'refs/tags/v')", block)
        self.assertEqual(self.check_text(text), [])

    def test_removing_release_trigger_fails(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        mutated = text.replace(
            "github.event_name == 'workflow_dispatch' || startsWith(github.ref, 'refs/tags/v')",
            "false",
        )
        self.assertTrue(self.check_text(mutated))

    def test_deploy_cannot_be_replaced_with_empty_success(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        mutated = text.replace(
            "          scripts/deploy-cloudflare-pages.sh\n",
            "          true\n",
        )
        self.assertTrue(self.check_text(mutated))

    def test_missing_verified_artifact_download_fails(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        mutated = text.replace("actions/download-artifact@v4", "actions/checkout@v4")
        self.assertTrue(self.check_text(mutated))


if __name__ == "__main__":
    unittest.main()

