#!/usr/bin/env python3
"""Mutation contracts for the retired web build/deploy jobs (no deploy claim).

The browser UI was retired: ``scripts/build-web.sh`` and
``scripts/deploy-cloudflare-pages.sh`` now fail with ``NOT IMPLEMENTED``, and the
Cloudflare Pages deploy job is hard-disabled. This checker proves the disable is
real — an unconditional ``if: false`` plus a ``RETIRED`` marker, never a mutable
``vars.…`` switch and never a ``secrets.…`` publish path — so a future edit
cannot quietly re-enable a stub publish. It never contacts Cloudflare.
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

BUILD = ROOT / ".github/workflows/build.yml"


class RetiredWebJobsContracts(unittest.TestCase):
    def check_text(self, text):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            path = pathlib.Path(directory) / "test.yml"
            path.write_text(text)
            return workflow.structural_pass([path])[2]

    def test_web_build_and_deploy_jobs_are_retired_and_declared(self):
        text = BUILD.read_text()
        for job in ("web-build", "web-deploy", "web-smoke"):
            with self.subTest(job=job):
                self.assertIn(job, workflow.RETIRED_JOBS)
        self.assertEqual(self.check_text(text), [])

    def test_retired_jobs_are_hard_disabled_not_capability_gated(self):
        text = BUILD.read_text()
        for job in ("web-build", "web-deploy", "web-smoke"):
            with self.subTest(job=job):
                block = workflow.job_block(text, job)
                self.assertIsNotNone(block)
                self.assertRegex(block, r"(?m)^    if:\s*false\s*(?:#.*)?$")
                self.assertNotIn("vars.", block)
                self.assertIn("RETIRED", block)

    def test_removing_the_hard_disable_fails(self):
        text = BUILD.read_text()
        mutated = text.replace("    if: false\n", "    if: true\n", 1)
        self.assertNotEqual(mutated, text)
        self.assertTrue(self.check_text(mutated))

    def test_reintroducing_a_mutable_capability_switch_fails(self):
        text = BUILD.read_text()
        mutated = text.replace(
            "    if: false\n",
            "    if: ${{ vars.WEB_SMOKE_ENABLED == 'true' }}\n",
            1,
        )
        self.assertNotEqual(mutated, text)
        self.assertTrue(self.check_text(mutated))

    def test_retired_jobs_still_fail_nonzero_if_run(self):
        text = BUILD.read_text()
        # Every retired job's only step fails non-zero, so an accidental
        # re-enable can never pass as an empty success.
        for job in ("web-build", "web-deploy", "web-smoke"):
            with self.subTest(job=job):
                block = workflow.job_block(text, job)
                self.assertIsNotNone(block)
                self.assertIn("exit 1", block)
        mutated = text.replace("          exit 1\n", "          true\n", 1)
        self.assertNotEqual(mutated, text)
        self.assertTrue(self.check_text(mutated))

    def test_publish_command_and_credentials_are_gone_from_the_retired_deploy_job(self):
        text = BUILD.read_text()
        block = workflow.job_block(text, "web-deploy")
        self.assertIsNotNone(block)
        self.assertNotIn("scripts/deploy-cloudflare-pages.sh", block)
        self.assertNotIn("secrets.", block)
        # A stray credential reference in the disabled job must be caught.
        mutated = text.replace(
            "  web-deploy:\n",
            "  web-deploy:\n    env:\n      CLOUDFLARE_API_TOKEN: ${{ secrets.CLOUDFLARE_API_TOKEN }}\n",
        )
        self.assertNotEqual(mutated, text)
        self.assertTrue(self.check_text(mutated))

    def test_deploy_script_itself_fails_not_implemented(self):
        script = (ROOT / "scripts/deploy-cloudflare-pages.sh").read_text()
        self.assertIn("NOT IMPLEMENTED", script)
        self.assertRegex(script, r"(?m)^exit 1\s*$")
        self.assertNotIn("wrangler", script)


if __name__ == "__main__":
    unittest.main()
