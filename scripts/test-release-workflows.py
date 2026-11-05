#!/usr/bin/env python3
"""Contract tests for the re-enabled release packaging jobs.

The five platform release jobs (``linux-release``, ``windows-release``,
``macos-release``, ``android-apk``, ``android-release``) were hard-disabled
during the Slint removal. They are re-enabled (2026-11-01) for the current
React/Tauri hosts as **event-gated artifact builders**: a ``v*`` tag or a manual
``workflow_dispatch`` may run them; a routine push/PR may not. They upload
workflow artifacts and never publish, notarize or touch a store. With
credentials they may **sign** what they build (SignPath for the Windows
executables and the Android release APK, a repository GPG key for the Flatpak
OSTree commit); without them they upload an explicit ``*-unsigned`` fallback.

Publication is a **separate, manual** step: `publish-release.yml` (the only
write-scoped workflow) downloads those artifacts from a named build run and
attaches them to the tag's public GitHub Release. It never builds or signs
anything.

These are text/mutation contracts: no runner, package toolchain or signature is
exercised here.
"""
from __future__ import annotations

import importlib.util
import pathlib
import re
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("workflow", ROOT / "scripts" / "check-workflows.py")
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)

BUILD = ROOT / ".github/workflows/build.yml"
PUBLISH = ROOT / ".github/workflows/publish-release.yml"

PUBLISH_ARTIFACTS = (
    "yacr-linux-release",
    "yacr-windows-release",
    "yacr-windows-release-unsigned",
    "yacr-macos-release",
    "android-apk-debug",
    "android-release",
    "android-release-unsigned",
)

# The only repository secrets a release job may read: the SignPath API token and
# the Flatpak signing key material. Everything else belongs to another workflow.
ALLOWED_RELEASE_SECRETS = frozenset(
    {
        "SIGNPATH_API_TOKEN",
        "FLATPAK_GPG_PRIVATE_KEY",
        "FLATPAK_GPG_PASSPHRASE",
    }
)

RELEASE_JOBS = {
    "linux-release": "scripts/package-linux-release.sh",
    "windows-release": "scripts/package-windows-release.sh",
    "macos-release": "scripts/package-macos-release.sh",
    "android-apk": "scripts/build-android.sh",
    "android-release": "scripts/build-android.sh",
}


class ReleaseWorkflowContracts(unittest.TestCase):
    def check_text(self, text):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            path = pathlib.Path(directory) / "test.yml"
            path.write_text(text)
            return workflow.structural_pass([path])[2]

    def block(self, job):
        block = workflow.job_block(BUILD.read_text(), job)
        self.assertIsNotNone(block, f"job '{job}' is not declared")
        return block

    def test_no_release_job_is_retired(self):
        self.assertEqual(workflow.RETIRED_JOBS, ())
        for job in RELEASE_JOBS:
            self.assertIn(job, workflow.REQUIRED_JOBS)
            self.assertNotIn(job, workflow.RETIRED_JOBS)

    def test_release_workflow_is_structurally_valid(self):
        self.assertEqual(self.check_text(BUILD.read_text()), [])

    def test_each_release_job_calls_its_entry_point_and_uploads_an_artifact(self):
        for job, script in RELEASE_JOBS.items():
            with self.subTest(job=job):
                block = self.block(job)
                self.assertIn(script, block)
                self.assertIn("actions/upload-artifact", block)
                self.assertNotIn("if: false", block)
                self.assertNotIn("continue-on-error", block)

    def test_each_release_job_is_tag_or_dispatch_gated(self):
        for job in RELEASE_JOBS:
            with self.subTest(job=job):
                block = self.block(job)
                self.assertIn("refs/tags/v", block)
                self.assertIn("workflow_dispatch", block)

    def test_release_jobs_do_not_publish(self):
        for job in RELEASE_JOBS:
            with self.subTest(job=job):
                block = self.block(job)
                self.assertNotIn("gh release", block)
                self.assertNotIn("wrangler", block)
                self.assertNotIn("flatpak build-commit-from", block)
                for secret in re.findall(r"secrets\.([A-Z0-9_]+)", block):
                    self.assertIn(
                        secret,
                        ALLOWED_RELEASE_SECRETS,
                        f"release job '{job}' reads an unexpected secret: {secret}",
                    )

    def test_windows_release_signs_with_signpath_and_verifies_locally(self):
        block = self.block("windows-release")
        self.assertIn("signpath/github-action-submit-signing-request@v3", block)
        self.assertIn(
            "github-artifact-id: ${{ steps.upload-windows-stage.outputs.artifact-id }}",
            block,
        )
        self.assertIn("output-artifact-directory: target/windows-signed", block)
        self.assertIn("Get-AuthenticodeSignature", block)
        self.assertIn("wait-for-completion-timeout-in-seconds: 3600", block)
        self.assertIn("finalize-windows-release.sh --signed", block)
        self.assertIn("finalize-windows-release.sh --unsigned", block)

    def test_android_release_signs_with_signpath_and_verifies_locally(self):
        block = self.block("android-release")
        self.assertIn("signpath/github-action-submit-signing-request@v3", block)
        self.assertIn(
            "github-artifact-id: ${{ steps.upload-android-unsigned.outputs.artifact-id }}",
            block,
        )
        self.assertIn("output-artifact-directory: target/android-signed", block)
        self.assertIn("apksigner", block)
        self.assertIn("app-universal-release.apk", block)
        self.assertIn("wait-for-completion-timeout-in-seconds: 3600", block)
        self.assertIn("zipalign", block)

    def test_signing_steps_gate_on_env_not_on_secrets(self):
        for job in ("windows-release", "android-release"):
            with self.subTest(job=job):
                block = self.block(job)
                self.assertIn("if: env.SIGNPATH_API_TOKEN != ''", block)
                self.assertIn("if: env.SIGNPATH_API_TOKEN == ''", block)
                # Job-level `if:` cannot read secrets and neither should any step
                # condition: expose the secret in `env:` and compare that.
                for line in block.splitlines():
                    if re.match(r"\s*if:", line):
                        self.assertNotIn(
                            "secrets.", line, f"step gate reads a secret: {line.strip()}"
                        )

    def test_linux_release_can_sign_the_flatpak_commit(self):
        block = self.block("linux-release")
        self.assertIn("FLATPAK_GPG_KEY_ID", block)
        self.assertIn("--gpg-sign", block)
        self.assertIn(".flatpak.gpg", block)
        self.assertIn("UNSIGNED", block)

    def test_android_jobs_build_the_intended_variant(self):
        self.assertIn("--debug", self.block("android-apk"))
        self.assertIn("--release", self.block("android-release"))
        for job in ("android-apk", "android-release"):
            with self.subTest(job=job):
                block = self.block(job)
                self.assertIn("ndk;29.0.13846066", block)
                self.assertIn("setup-java@v4", block)
                self.assertIn("java-version: \"17\"", block)
        self.assertIn("UNSIGNED", self.block("android-release"))

    def test_windows_and_macos_build_the_react_frontend_first(self):
        # The packaging scripts build desktop-ui/dist, which tauri::generate_context!
        # embeds; the jobs must at least carry Node setup for that step.
        for job in ("windows-release", "macos-release"):
            with self.subTest(job=job):
                self.assertIn("actions/setup-node@v4", self.block(job))

    def test_reverting_a_release_job_to_if_false_fails_the_gate(self):
        text = BUILD.read_text()
        block = self.block("linux-release")
        mutated = text.replace(block, block.replace(
            "    if: ${{ github.event_name == 'workflow_dispatch' || "
            "startsWith(github.ref, 'refs/tags/v') }}\n",
            "    if: false\n",
        ), 1)
        self.assertNotEqual(mutated, text)
        errors = self.check_text(mutated)
        self.assertTrue(any("linux-release" in error for error in errors), errors)

    def test_dropping_a_release_entry_point_fails_the_gate(self):
        text = BUILD.read_text()
        mutated = text.replace("bash scripts/package-linux-release.sh", "true", 1)
        self.assertNotEqual(mutated, text)
        errors = self.check_text(mutated)
        self.assertTrue(any("linux-release" in error for error in errors), errors)

    def test_removing_a_release_job_fails_the_gate(self):
        text = BUILD.read_text()
        block = self.block("windows-release")
        mutated = text.replace(block, "", 1)
        self.assertNotEqual(mutated, text)
        errors = self.check_text(mutated)
        self.assertTrue(any("windows-release" in error for error in errors), errors)


class PublishWorkflowContracts(unittest.TestCase):
    """The public-release publisher stays manual, write-scoped and build-free."""

    def text(self):
        self.assertTrue(PUBLISH.is_file(), f"missing {PUBLISH.relative_to(ROOT)}")
        return PUBLISH.read_text()

    def test_publisher_is_manual_only_and_write_scoped(self):
        text = self.text()
        self.assertIn("workflow_dispatch:", text)
        self.assertNotRegex(text, r"(?m)^  push:")
        self.assertNotRegex(text, r"(?m)^  pull_request:")
        self.assertIn("contents: write", text)
        # Downloading the run's artifacts needs the Actions scope explicitly:
        # listing a `permissions:` block zeroes every unlisted scope.
        self.assertIn("actions: read", text)

    def test_publisher_attaches_every_platform_artifact(self):
        text = self.text()
        for name in PUBLISH_ARTIFACTS:
            with self.subTest(artifact=name):
                self.assertIn(name, text)
        self.assertIn("gh release create", text)
        self.assertIn("--verify-tag", text)

    def test_publisher_accepts_signed_and_unsigned_fallbacks(self) -> None:
        text = self.text()
        for name in (
            "yacr-windows-release",
            "yacr-windows-release-unsigned",
            "android-release",
            "android-release-unsigned",
            "app-universal-release.apk",
            "app-universal-release-unsigned.apk",
            "*.apk.idsig",
            "*.apk.sha256",
            "*.flatpak.gpg",
        ):
            with self.subTest(name=name):
                self.assertIn(name, text)
        # Resolve the artifact name from the run's artifact list before
        # downloading, so a transient download error is not mistaken for an
        # absent signed artifact.
        self.assertIn("gh api", text)
        self.assertIn("artifact_names", text)
        self.assertIn("pick_artifact", text)
        self.assertNotIn("download_optional", text)

    def test_publisher_stages_outside_the_tracked_assets_directory(self):
        # The repository tracks its own `assets/` (icons). Staging there made
        # `gh release create assets/*` try to upload `assets/icons` as an asset.
        text = self.text()
        self.assertIn("release-assets", text)
        self.assertNotIn("mkdir -p assets", text)
        self.assertNotRegex(text, r"(?<![\w-])assets/\*")
        self.assertIn("release-assets/app-universal-debug.apk", text)
        self.assertIn("release-assets/*.tar.gz", text)

    def test_publisher_never_builds_or_signs(self):
        text = self.text()
        for forbidden in (
            "scripts/package-",
            "cargo build",
            "build-android.sh",
            "wrangler",
            "apksigner",
            "codesign",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, text)


if __name__ == "__main__":
    unittest.main()
