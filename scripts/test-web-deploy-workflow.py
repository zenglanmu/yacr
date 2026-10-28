#!/usr/bin/env python3
"""Mutation contracts for the browser pipeline: build -> smoke -> publish.

The wasm CAD session (ADR 0024) is built by ``scripts/build-web.sh``, driven in a
browser by ``scripts/web-smoke.cjs`` and — only on a ``v*`` tag or a manual
dispatch — published by ``scripts/deploy-cloudflare-pages.sh``. These tests pin
the workflow jobs and the script-level contracts, so a silent regression back to
a retired stub, an unguarded publish, a smoke that never asserts the session, or
a bundler change that stops shipping an artifact fails here.

Nothing in this file contacts Cloudflare or launches a browser: it is a
text/contract check. The real smoke runs in CI (`web-smoke`) under software
rendering; its log says so.
"""
from __future__ import annotations

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


class WebPipelineWorkflowContracts(unittest.TestCase):
    def check_text(self, text):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            path = pathlib.Path(directory) / "test.yml"
            path.write_text(text)
            return workflow.structural_pass([path])[2]

    def block(self, job):
        block = workflow.job_block(BUILD.read_text(), job)
        self.assertIsNotNone(block, f"job '{job}' is not declared")
        return block

    def test_web_jobs_are_required_not_retired(self):
        for job in ("web-build", "web-smoke", "web-deploy"):
            with self.subTest(job=job):
                self.assertIn(job, workflow.REQUIRED_JOBS)
                self.assertNotIn(job, workflow.RETIRED_JOBS)
        self.assertEqual(self.check_text(BUILD.read_text()), [])

    def test_web_build_installs_the_pinned_cli_and_uploads_the_bundle(self):
        block = self.block("web-build")
        self.assertIn("bash scripts/build-web.sh", block)
        self.assertIn("wasm32-unknown-unknown", block)
        # The prebuilt CLI is fetched with a pinned SHA-256 and checked against the
        # workspace pin before the bundle is built — never "whatever is on PATH".
        self.assertIn("sha256sum -c -", block)
        self.assertIn("scripts/lib/wasm-toolchain.sh", block)
        self.assertIn("apps/app-web/dist", block)
        self.assertIn("web-dist", block)
        # The same job builds the shared chrome bundle web-deploy publishes.
        self.assertIn("npm ci", block)
        self.assertIn("bash scripts/build-web-ui.sh", block)
        self.assertIn("apps/app-web/dist-ui", block)
        self.assertIn("web-ui-dist", block)
        self.assertNotIn("if: false", block)

    def test_web_smoke_drives_the_uploaded_bundle(self):
        block = self.block("web-smoke")
        self.assertIn("needs: web-build", block)
        self.assertIn("actions/download-artifact", block)
        self.assertIn("scripts/serve-web.py", block)
        self.assertIn("scripts/web-smoke.cjs", block)
        self.assertIn("playwright", block)
        self.assertNotIn("if: false", block)
        self.assertNotIn("continue-on-error", block)

    def test_web_deploy_is_tag_or_dispatch_gated(self):
        block = self.block("web-deploy")
        self.assertIn("needs: web-build", block)
        self.assertIn("refs/tags/v", block)
        self.assertIn("workflow_dispatch", block)
        self.assertIn("scripts/deploy-cloudflare-pages.sh", block)
        self.assertIn("secrets.CLOUDFLARE_API_TOKEN", block)
        self.assertIn('SKIP_BUILD: "1"', block)
        # The published artifact is the shared chrome bundle, validated as `ui`.
        self.assertIn("web-ui-dist", block)
        self.assertIn("apps/app-web/dist-ui", block)
        self.assertIn("BUNDLE_KIND: ui", block)
        self.assertNotIn("if: false", block)

    def test_deploying_on_a_routine_push_fails_the_gate_contract(self):
        text = BUILD.read_text()
        mutated = text.replace("refs/tags/v", "refs/heads/main", 1)
        self.assertNotEqual(mutated, text)
        errors = self.check_text(mutated)
        self.assertTrue(
            any("web-deploy" in error for error in errors),
            f"an ungated deploy was not reported: {errors}",
        )

    def test_dropping_the_bundle_build_fails(self):
        text = BUILD.read_text()
        mutated = text.replace("run: bash scripts/build-web.sh", "run: true", 1)
        self.assertNotEqual(mutated, text)
        errors = self.check_text(mutated)
        self.assertTrue(any("build-web.sh" in error for error in errors), errors)

    def test_dropping_the_artifact_upload_fails(self):
        text = BUILD.read_text()
        mutated = text.replace("          name: web-dist\n", "", 1)
        self.assertNotEqual(mutated, text)
        errors = self.check_text(mutated)
        self.assertTrue(any("web-dist" in error for error in errors), errors)


class WebEntryPointScriptContracts(unittest.TestCase):
    """The three browser entry points are real; each has a fail-closed contract."""

    def read(self, rel):
        return (ROOT / rel).read_text()

    def test_all_entry_points_satisfy_their_contract(self):
        self.assertEqual(workflow.build_web_errors(self.read(workflow.BUILD_WEB_SCRIPT)), [])
        self.assertEqual(
            workflow.wasm_toolchain_errors(self.read(workflow.WASM_TOOLCHAIN_SCRIPT)), []
        )
        self.assertEqual(workflow.web_smoke_errors(self.read(workflow.WEB_SMOKE_SCRIPT)), [])
        self.assertEqual(workflow.deploy_script_errors(self.read(workflow.DEPLOY_SCRIPT)), [])

    def test_entry_point_pass_accepts_the_real_scripts(self):
        ok, errors = workflow.entry_point_pass()
        self.assertEqual(errors, [])
        joined = "\n".join(ok)
        for rel in (
            workflow.BUILD_WEB_SCRIPT,
            workflow.WASM_TOOLCHAIN_SCRIPT,
            workflow.WEB_SMOKE_SCRIPT,
            workflow.DEPLOY_SCRIPT,
        ):
            self.assertIn(rel, joined)

    def test_reintroducing_the_retired_build_stub_fails(self):
        mutated = (
            "#!/usr/bin/env bash\n"
            "# RETIRED: NOT IMPLEMENTED\n"
            'echo "NOT IMPLEMENTED" >&2\nexit 1\n'
        )
        self.assertTrue(workflow.build_web_errors(mutated))

    def test_smoke_that_does_not_assert_the_session_fails(self):
        text = self.read(workflow.WEB_SMOKE_SCRIPT)
        for fragment in ("start: ok", "#cad-status", "__YACR_CANVAS__"):
            with self.subTest(fragment=fragment):
                mutated = text.replace(fragment, "")
                self.assertNotEqual(mutated, text)
                self.assertTrue(workflow.web_smoke_errors(mutated))

    def test_smoke_that_cannot_fail_the_job_is_rejected(self):
        text = self.read(workflow.WEB_SMOKE_SCRIPT)
        mutated = text.replace("process.exitCode", "process.exit(0)")
        self.assertNotEqual(mutated, text)
        self.assertTrue(workflow.web_smoke_errors(mutated))

    def test_deploy_without_a_publish_command_fails(self):
        text = self.read(workflow.DEPLOY_SCRIPT)
        mutated = text.replace("wrangler", "noop")
        self.assertNotEqual(mutated, text)
        self.assertTrue(workflow.deploy_script_errors(mutated))

    def test_deploy_that_cannot_detect_a_partial_bundle_fails(self):
        text = self.read(workflow.DEPLOY_SCRIPT)
        for artifact in (
            "worker.js",
            "pkg/app_web.js",
            "pkg/app_web_bg.wasm",
            "canvas/app_web.js",
            "canvas/app_web_bg.wasm",
            "canvas/bootstrap.js",
        ):
            with self.subTest(artifact=artifact):
                mutated = text.replace(artifact, "")
                self.assertNotEqual(mutated, text)
                self.assertTrue(workflow.deploy_script_errors(mutated))

    def test_deploy_without_the_artifact_guard_fails(self):
        text = self.read(workflow.DEPLOY_SCRIPT)
        mutated = text.replace('[ -s "$DIST/$artifact" ]', "true")
        self.assertNotEqual(mutated, text)
        self.assertTrue(workflow.deploy_script_errors(mutated))

    def test_deploy_that_still_says_not_implemented_fails(self):
        text = self.read(workflow.DEPLOY_SCRIPT)
        mutated = text + "\n# NOT IMPLEMENTED\n"
        self.assertTrue(workflow.deploy_script_errors(mutated))


if __name__ == "__main__":
    unittest.main()
