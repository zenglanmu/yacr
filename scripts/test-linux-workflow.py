#!/usr/bin/env python3
"""Mutation contracts for the primary Linux workflow (no runner/GPU claim).

Mirrors the surviving real gates after the UI replacement: the primary
`linux-app` job is a React/Tauri native compile gate, so it must install the
GTK3/WebKitGTK4.1 prerequisites, run the React (Vite) test suite and build the
frontend before the Rust host compiles (``tauri::generate_context!`` needs
``ui/dist``). It must still never render, run Rust tests or build a release.
``core-quality`` must also run the React tests, the retained platform-neutral
node PNG contract and the UI-migration guards. ``android-check`` must compile the
retired ``app-android`` placeholder without the deleted Slint crate.
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


class LinuxWorkflowContracts(unittest.TestCase):
    def check_text(self, text):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            path = pathlib.Path(directory) / "test.yml"
            path.write_text(text)
            return workflow.structural_pass([path])[2]

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

    def test_linux_gate_requires_tauri_native_prereqs_and_frontend_build(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        # The real job satisfies every required fragment.
        self.assertEqual(self.check_text(text), [])
        # Dropping the WebKitGTK/Tauri prerequisite, the React test run or the
        # frontend build must be reported, not silently accepted.
        for fragment in ("libwebkit2gtk-4.1-dev", "apps/desktop-ui", "npm test", "npm run build"):
            with self.subTest(fragment=fragment):
                self.assertNotEqual(
                    self.check_text(text.replace(fragment, "")),
                    [],
                    f"mutation not detected: {fragment}",
                )

    def test_linux_gate_runs_react_tests_before_build_and_rust_compile(self):
        text = (ROOT / ".github/workflows/build.yml").read_text()
        block = workflow.job_block(text, "linux-app")
        self.assertIsNotNone(block)
        self.assertLess(
            block.index("npm test"),
            block.index("npm run build"),
            "the React tests must run before the frontend is built",
        )
        self.assertLess(
            block.index("npm run build"),
            block.index("cargo check -p app-linux --all-targets --locked"),
            "the frontend must be built before the Rust host that embeds ui/dist",
        )

    def test_core_gate_runs_react_tests_before_build(self):
        text = (ROOT / ".github/workflows/core.yml").read_text()
        block = workflow.job_block(text, "core-quality")
        self.assertIsNotNone(block)
        self.assertLess(
            block.index("npm test"),
            block.index("npm run build"),
            "core-quality must run the React tests before building the frontend",
        )

    def test_core_gate_requires_the_retained_node_and_migration_contracts(self):
        text = (ROOT / ".github/workflows/core.yml").read_text()
        block = workflow.job_block(text, "core-quality")
        self.assertIsNotNone(block)
        for fragment in (
            "npm test",
            "node --test scripts/test-web-pixels.mjs",
            "test-ui-migration-guards.py",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, block)
                self.assertNotEqual(
                    self.check_text(text.replace(fragment, "")),
                    [],
                    f"mutation not detected: {fragment}",
                )

    def test_core_gate_compiles_without_running_the_full_suite(self):
        text = (ROOT / ".github/workflows/core.yml").read_text()
        self.assertEqual(self.check_text(text), [])
        block = workflow.job_block(text, "core-quality")
        self.assertIsNotNone(block)
        self.assertNotIn("cargo test", block)
        self.assertNotIn("--release", block)

    def test_core_gate_must_check_all_host_workspace_members(self):
        text = (ROOT / ".github/workflows/core.yml").read_text()
        old_command = (
            "cargo check --workspace --exclude app-android --exclude app-web "
            "--all-targets --locked"
        )
        mutated = text.replace("cargo check --workspace --all-targets --locked", old_command)
        self.assertNotEqual(mutated, text)
        self.assertTrue(self.check_text(mutated))

    def test_android_compile_gate_drops_the_retired_slint_crate(self):
        text = (ROOT / ".github/workflows/core.yml").read_text()
        block = workflow.job_block(text, "android-check")
        self.assertIsNotNone(block)
        self.assertIn("cargo check --target aarch64-linux-android -p app-android --locked", block)
        self.assertNotIn("cad-ui-slint", block)
        # Re-introducing the deleted crate into the compile gate must be caught.
        mutated = text.replace(
            "cargo check --target aarch64-linux-android -p app-android --locked",
            "cargo check --target aarch64-linux-android -p cad-ui-slint -p app-android --locked",
        )
        self.assertTrue(self.check_text(mutated))


if __name__ == "__main__":
    unittest.main()
