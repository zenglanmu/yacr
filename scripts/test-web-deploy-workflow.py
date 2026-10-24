#!/usr/bin/env python3
"""Mutation contracts for the web bundle / deploy entry points (no deploy claim).

The browser wasm session was revived (ADR 0024), so ``scripts/build-web.sh`` is a
real local bundle build again: it re-reads the ``wasm-bindgen`` pin, compiles
``app-web`` for ``wasm32-unknown-unknown`` (release, ``--locked``), emits the
``--target web`` glue and assembles ``apps/app-web/dist``. These tests pin that
contract (and the shared ``scripts/lib/wasm-toolchain.sh`` it calls) so a silent
regression back to a "NOT IMPLEMENTED" stub, a dropped pin check or a drifted
output directory fails.

Publishing is **still** not restored: ``scripts/deploy-cloudflare-pages.sh``
keeps failing with ``NOT IMPLEMENTED`` and the Cloudflare Pages deploy job stays
hard-disabled. This checker proves that disable is real — an unconditional
``if: false`` plus a ``RETIRED`` marker, never a mutable ``vars.…`` switch and
never a ``secrets.…`` publish path — so a future edit cannot quietly re-enable a
stub publish. It never contacts Cloudflare.
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
        # It must point at the local bundle instead of implying a publish path.
        self.assertIn(workflow.BUILD_WEB_OUT_DIR, script)

    def test_removing_the_retired_deploy_diagnostic_fails(self):
        script = (ROOT / "scripts/deploy-cloudflare-pages.sh").read_text()
        mutated = script.replace("NOT IMPLEMENTED", "")
        self.assertNotEqual(mutated, script)
        self.assertTrue(workflow.retired_script_errors("scripts/deploy-cloudflare-pages.sh", mutated))

    def test_deploy_script_must_exit_nonzero(self):
        script = (ROOT / "scripts/deploy-cloudflare-pages.sh").read_text()
        mutated = script.replace("exit 1", "exit 0")
        self.assertNotEqual(mutated, script)
        self.assertTrue(workflow.retired_script_errors("scripts/deploy-cloudflare-pages.sh", mutated))


class LocalBrowserBundleContracts(unittest.TestCase):
    """`scripts/build-web.sh` builds a real bundle; it is not a retired stub."""

    def build_web(self):
        return (ROOT / workflow.BUILD_WEB_SCRIPT).read_text()

    def toolchain(self):
        return (ROOT / workflow.WASM_TOOLCHAIN_SCRIPT).read_text()

    def test_build_web_satisfies_its_contract(self):
        self.assertEqual(workflow.build_web_errors(self.build_web()), [])

    def test_shared_toolchain_satisfies_its_contract(self):
        self.assertEqual(workflow.wasm_toolchain_errors(self.toolchain()), [])

    def test_entry_point_pass_accepts_the_real_scripts(self):
        ok, errors = workflow.entry_point_pass()
        self.assertEqual(errors, [])
        self.assertTrue(any(workflow.BUILD_WEB_SCRIPT in line for line in ok))

    def test_reintroducing_the_retired_stub_fails(self):
        text = self.build_web()
        mutated = (
            "#!/usr/bin/env bash\n"
            "# RETIRED: NOT IMPLEMENTED\n"
            'echo "NOT IMPLEMENTED" >&2\nexit 1\n'
        )
        self.assertNotEqual(mutated, text)
        errors = workflow.build_web_errors(mutated)
        self.assertTrue(any("NOT IMPLEMENTED" in error for error in errors))
        self.assertTrue(any("wasm32" in error or "yacr_build_app_web_wasm" in error for error in errors))

    def test_dropping_the_wasm_bindgen_pin_check_fails(self):
        text = self.toolchain()
        mutated = text.replace("--version", "--help")
        self.assertNotEqual(mutated, text)
        self.assertTrue(workflow.wasm_toolchain_errors(mutated))

    def test_dropping_the_wasm32_build_fails(self):
        text = self.toolchain()
        mutated = text.replace("wasm32-unknown-unknown", "x86_64-unknown-linux-gnu")
        self.assertNotEqual(mutated, text)
        self.assertTrue(workflow.wasm_toolchain_errors(mutated))

    def test_build_web_must_call_the_shared_toolchain(self):
        text = self.build_web()
        mutated = text.replace(workflow.WASM_TOOLCHAIN_SCRIPT, "scripts/build-canvas-wasm.sh")
        self.assertNotEqual(mutated, text)
        self.assertTrue(workflow.build_web_errors(mutated))

    def test_output_directory_must_match_the_documented_bundle(self):
        text = self.build_web()
        mutated = text.replace(
            f'readonly OUT_DIR="$REPO_ROOT/{workflow.BUILD_WEB_OUT_DIR}"',
            'readonly OUT_DIR="$REPO_ROOT/apps/app-web/web"',
        )
        self.assertNotEqual(mutated, text)
        errors = workflow.build_web_errors(mutated)
        self.assertTrue(any("documented bundle directory" in error for error in errors))

    def test_the_printed_directory_must_be_the_written_one(self):
        text = self.build_web()
        old = 'printf \'artifacts in %s\\n\' "$OUT_DIR"'
        new = 'printf \'artifacts in %s\\n\' "apps/app-web/dist"'
        self.assertIn(old, text)
        mutated = text.replace(old, new)
        self.assertNotEqual(mutated, text)
        errors = workflow.build_web_errors(mutated)
        self.assertTrue(any("does not print the directory it wrote" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
