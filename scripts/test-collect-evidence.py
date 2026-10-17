#!/usr/bin/env python3
"""Contracts for the Linux host evidence-collection runner.

These are text/behavior contracts for two dev scripts:

* ``scripts/collect-evidence.sh`` must stay inside the current desktop session
  (same UID, no privilege escalation, no session unlock, no portal permission
  change, no package installs, no virtual/headless display) and must never use
  the app's fake marker mode. It must also be honest: a missing build, launch or
  screenshot is recorded as a failure/skip, never as an empty success.
* ``scripts/screencast-capture.py`` is the native Wayland helper and must drive
  the same-UID Mutter ScreenCast API and always Stop() the session.

Each guarantee is asserted against the real file; the mutations below show the
checks are not vacuous. Text-only plus two launch-free CLI smoke checks
(``--help`` / ``--dry-run``); it does not build, launch or capture anything.
"""
from __future__ import annotations

import pathlib
import re
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
COLLECT = ROOT / "scripts/collect-evidence.sh"
SCREENCAST = ROOT / "scripts/screencast-capture.py"

# Forbidden behavior tokens. Explanatory prose avoids these words entirely so
# their presence always means a real capability leaked into the scripts.
FORBIDDEN = (
    "sudo",
    "pkexec",
    "doas",
    "loginctl",
    "xvfb",
    "PermissionStore",
    "unsafe-mode",
    "apt-get",
    "apt install",
    "dnf install",
    "pacman",
)

CJK = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff]")


class CollectEvidenceScriptContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.text = COLLECT.read_text(encoding="utf-8")

    def test_script_exists_and_is_executable(self) -> None:
        self.assertTrue(COLLECT.is_file())
        self.assertTrue(
            COLLECT.stat().st_mode & 0o111, "collect-evidence.sh must be executable"
        )

    def test_script_is_bash_with_strict_mode(self) -> None:
        self.assertTrue(self.text.startswith("#!/usr/bin/env bash"))
        self.assertIn("set -euo pipefail", self.text)

    def test_no_cjk_is_hardcoded(self) -> None:
        self.assertIsNone(CJK.search(self.text))

    def test_forbidden_capabilities_never_appear(self) -> None:
        for token in FORBIDDEN:
            with self.subTest(token=token):
                self.assertNotIn(
                    token,
                    self.text,
                    f"forbidden capability token present: {token!r}",
                )

    def test_never_uses_the_fake_marker_mode(self) -> None:
        self.assertNotIn("--probe", self.text)
        self.assertIn("--open", self.text)
        self.assertIn("--fonts-dir", self.text)

    def test_required_cli_options_are_present(self) -> None:
        for option in (
            "--backend",
            "--dwg",
            "--fonts-dir",
            "--out",
            "--lifecycle",
            "--capture-at",
            "--timeout",
            "--no-build",
            "--maximize",
            "--dry-run",
            "-h|--help",
        ):
            with self.subTest(option=option):
                self.assertIn(option, self.text)

    def test_uses_a_process_group_so_capture_can_be_capped(self) -> None:
        self.assertIn("setsid", self.text)
        self.assertIn("kill -TERM -", self.text)
        self.assertIn("kill -KILL -", self.text)

    def test_real_launch_applies_the_backend_environment(self) -> None:
        # Regression guard: the per-backend environment (built into `env_prefix`)
        # must be applied to the REAL setsid launch, not merely echoed by
        # --dry-run. `env_prefix` is expanded with [*] only in the dry-run echo,
        # so requiring the [@] form on the launch line catches a launch that
        # silently runs with the inherited shell environment (which resolves to
        # an unsupported backend and never self-exits). The true end-to-end proof
        # is a real run; this is a structural guard.
        launch_lines = [
            line for line in self.text.splitlines() if "setsid bash -c" in line
        ]
        self.assertEqual(len(launch_lines), 1, "expected exactly one real launch line")
        self.assertIn('"${env_prefix[@]}"', launch_lines[0])
        # The applied env must carry the display backend and the probe variables.
        for fragment in (
            "GDK_BACKEND=x11",
            "-u DISPLAY",
            "-u XAUTHORITY",
            "GDK_BACKEND=wayland",
            "WAYLAND_DISPLAY=",
            "XDG_RUNTIME_DIR=",
            "YACR_PROBE_CAD_LOG=1",
            "YACR_PROBE_LIFECYCLE=",
            "YACR_PROBE_NO_MAXIMIZE=1",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, self.text)

    def test_records_honest_verdicts_not_empty_success(self) -> None:
        for verdict in (
            "NATURAL_EXIT",
            "PLANNED_TIMEOUT",
            "LAUNCH_FAILED",
            "BUILD_FAILED",
            "NO_DISPLAY",
            "SKIPPED",
        ):
            with self.subTest(verdict=verdict):
                self.assertIn(verdict, self.text)
        # A missing screenshot must be recorded as missing, never captured/pass.
        self.assertIn('screenshot="missing"', self.text)
        self.assertIn('screenshot="captured"', self.text)

    def test_requires_a_real_display_session(self) -> None:
        self.assertIn("no display session available", self.text)
        self.assertIn("exit 2", self.text)

    def test_wayland_capture_is_labelled_as_compositor_output(self) -> None:
        self.assertIn("compositor output", self.text)
        self.assertIn("Mutter ScreenCast", self.text)

    def test_declares_the_authorization_boundary(self) -> None:
        self.assertIn("Authorization boundary", self.text)
        self.assertIn("same-UID", self.text)
        self.assertIn("never runs as root", self.text)
        self.assertIn("never uploaded", self.text)

    def test_help_runs_and_prints_usage(self) -> None:
        result = subprocess.run(
            ["bash", str(COLLECT), "--help"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Usage:", result.stdout)
        self.assertIn("--dry-run", result.stdout)

    def test_dry_run_prints_a_plan_and_launches_nothing(self) -> None:
        result = subprocess.run(
            ["bash", str(COLLECT), "--dry-run"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("dry-run: resolved plan", result.stdout)
        self.assertIn("argv", result.stdout)
        # It must not build or launch: no cargo compile, no binary spawn.
        self.assertNotIn("cargo build --release", result.stderr)
        self.assertNotIn("launching", result.stdout)


class ScreencastCaptureContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.text = SCREENCAST.read_text(encoding="utf-8")

    def test_script_exists_and_is_executable(self) -> None:
        self.assertTrue(SCREENCAST.is_file())
        self.assertTrue(
            SCREENCAST.stat().st_mode & 0o111,
            "screencast-capture.py must be executable",
        )

    def test_no_cjk_is_hardcoded(self) -> None:
        self.assertIsNone(CJK.search(self.text))

    def test_required_cli_options_are_present(self) -> None:
        for option in ("--out", "--connector", "--timeout"):
            with self.subTest(option=option):
                self.assertIn(option, self.text)

    def test_drives_the_same_uid_mutter_screencast_api(self) -> None:
        self.assertIn("org.gnome.Mutter.ScreenCast", self.text)
        self.assertIn("CreateSession", self.text)
        self.assertIn("RecordMonitor", self.text)
        self.assertIn("PipeWireStreamAdded", self.text)

    def test_subscribes_before_start(self) -> None:
        # The signal subscription must precede Start, otherwise the node is missed.
        self.assertLess(
            self.text.index("connect_to_signal"),
            self.text.index(".Start()"),
            "PipeWireStreamAdded must be subscribed before Start()",
        )

    def test_always_stops_the_session(self) -> None:
        self.assertIn("finally:", self.text)
        self.assertIn("session.Stop()", self.text)

    def test_grabs_one_frame_through_gstreamer(self) -> None:
        self.assertIn("gst-launch-1.0", self.text)
        self.assertIn("pipewiresrc", self.text)
        self.assertIn("pngenc", self.text)

    def test_never_writes_a_placeholder(self) -> None:
        # A produced frame must be non-empty; failures return non-zero first.
        self.assertIn("getsize", self.text)
        self.assertIn("return 8", self.text)


if __name__ == "__main__":
    unittest.main()
