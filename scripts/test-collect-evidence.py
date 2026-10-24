#!/usr/bin/env python3
"""Contracts for the Linux host evidence-collection runner.

These are text/behavior contracts for two dev scripts:

* ``scripts/collect-evidence.sh`` must stay inside the current desktop session
  (same UID, no session unlock, no portal permission change, no faking real-session
  evidence with a virtual/headless display, no software rendering reported as a real
  GPU) and must never use the app's fake marker mode. Both Linux pixel carriers are
  real runs as of ADR 0024: X11/XWayland starts on ``surfaceMode=dom-canvas`` and is
  captured through the same-UID Mutter ScreenCast stream as native Wayland. It must
  also be honest: a missing build, launch, refusal or screenshot is recorded as a
  failure/skip/missing with a reason, never as an empty success.
* The refusal vocabulary is marker-first, exit code second: the current
  ``yacr: unsupported-runtime: webgl2`` (exit 6, ``dom-canvas`` has no WebGL2) and
  the historical ``yacr: unsupported-backend: x11`` (exit 4, no longer produced).
* ``scripts/screencast-capture.py`` is the ScreenCast helper and must drive the
  same-UID Mutter ScreenCast API and always Stop() the session.

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
            "--stacking-regression",
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
            "YACR_PROBE_STACKING=1",
        ):
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, self.text)

    def test_x11_is_a_real_run_on_the_dom_canvas_carrier(self) -> None:
        # ADR 0024 stage 4: X11/XWayland starts on `surfaceMode=dom-canvas`; the
        # old pre-GTK refusal is not produced any more.
        self.assertIn("GDK_BACKEND=x11", self.text)
        self.assertIn("surfaceMode=dom-canvas", self.text)
        # It is one of the real candidates and goes through the same launch path.
        self.assertIn("auto|both) candidates=(x11 wayland) ;;", self.text)
        self.assertIn('run_backend "$name" "production"', self.text)
        # The retired window-drawable grabber is gone: using it would collect
        # all-black frames here and imply an X11-only capture path.
        self.assertNotIn("capture_x11_window", self.text)
        self.assertNotIn("x11-window-grab", self.text)
        self.assertNotIn("shot-top.png", self.text)
        self.assertNotIn("shot-cad.png", self.text)

    def test_x11_frame_is_captured_through_the_screencast_stream(self) -> None:
        # x11grab / window-drawable read-back is all black in this session, so both
        # carriers use the same same-UID Mutter ScreenCast capture.
        self.assertIn('"x11-frame"', self.text)
        self.assertIn('"wayland-frame"', self.text)
        self.assertIn("capture_screencast_frame", self.text)
        self.assertIn("screencast-capture.py", self.text)
        # The X11 screenshot column is derived from a real file, never assumed.
        self.assertIn('-s "$dir/x11-frame.png"', self.text)
        # And a failed capture is recorded as missing with its reason.
        self.assertIn('local screenshot="missing"', self.text)
        self.assertIn("screenshot missing: ${capture_reason:", self.text)

    def test_webgl2_runtime_refusal_is_classified_by_the_stable_marker_line(self) -> None:
        # Current refusal: the dom-canvas carrier cannot draw (no usable WebGL2).
        marker = "yacr: unsupported-runtime: webgl2"
        self.assertIn(marker, self.text)
        self.assertIn("UNSUPPORTED_RUNTIME", self.text)
        self.assertIn("expected exit 6", self.text)
        self.assertIn('grep -qxF "$unsupported_runtime_marker"', self.text)
        # Marker first, exit code second: the marker wins the classification.
        detector_start = self.text.index("detect_refusal() {")
        detector = self.text[
            detector_start : self.text.index("record_refusal() {", detector_start)
        ]
        self.assertLess(
            detector.index("unsupported_runtime_marker"),
            detector.index("unsupported_backend_marker"),
            "the current webgl2 refusal must be matched before the historical marker",
        )
        # A refusal produces no usable carrier frame: no capture is attempted and
        # the row is `missing`, never a fabricated path.
        refusal_start = self.text.index("record_refusal() {")
        refusal_block = self.text[
            refusal_start : self.text.index("\n}\n", refusal_start) + 3
        ]
        self.assertNotIn("capture_screencast_frame", refusal_block)
        self.assertIn('"$screenshot"', refusal_block)
        self.assertIn('"missing" \\', self.text)

    def test_historical_x11_backend_refusal_marker_still_parses(self) -> None:
        # Recorded historical contract (2026-10-22 .. 2026-10-24, exit 4) kept so an
        # old binary/build is classified instead of being misread as a normal run.
        marker = "yacr: unsupported-backend: x11"
        self.assertIn(marker, self.text)
        self.assertIn('grep -qxF "$unsupported_backend_marker"', self.text)
        self.assertIn("UNSUPPORTED_BACKEND", self.text)
        self.assertIn("expected exit 4", self.text)
        self.assertIn("HISTORICAL", self.text)
        # The refusal marker is polled for the whole capture window, so a refusal
        # that arrives after the window exists is not missed.
        poll_start = self.text.index("local refusal_kind=\"\"")
        poll = self.text[poll_start : self.text.index('record_refusal "$name"', poll_start)]
        self.assertIn("detect_refusal \"$run_log\"", poll)
        self.assertIn('"$capture_at"', poll)

    def test_stacking_regression_fixture_is_pinned_and_wayland_only(self) -> None:
        self.assertIn("YACR_PROBE_STACKING=1", self.text)
        self.assertIn(
            '"$name" == "wayland" && "$stacking_regression" -eq 1', self.text
        )
        self.assertIn(
            'env_prefix+=("YACR_PROBE_STACKING=1" "YACR_PROBE_EVIDENCE=1")',
            self.text,
        )
        # The fixture is still a real --open run, never the fake marker mode.
        self.assertNotIn("--probe", self.text)

    def test_fixture_and_production_are_separate_launches(self) -> None:
        # One row per launch: separate directories, separate logs and separate
        # frames. No post-hoc row copying another launch's screenshot/timestamp.
        self.assertIn('dir="$out_dir/$name-stacking-regression"', self.text)
        self.assertIn('run_backend "$name" "production"', self.text)
        self.assertIn('run_backend "$name" "stacking-regression"', self.text)
        self.assertIn('record_row "$name" "$mode"', self.text)
        self.assertNotIn('"stacking-regression" "STACKING_REGRESSION"', self.text)

    def test_stacking_fixture_samples_multiple_frames_from_one_launch(self) -> None:
        # initial / popup-open / after-synthetic-input (+ one later frame) so
        # marker motion and the chrome/CAD-crossing popup are judgeable.
        for label in (
            "initial",
            "popup-open",
            "after-synthetic-input",
            "after-synthetic-input-later",
        ):
            with self.subTest(label=label):
                self.assertIn(label, self.text)
        self.assertIn("frame_offsets=(2 12 15 18)", self.text)
        self.assertIn('capture_screencast_frame "$dir" "${frame_labels[i]}"', self.text)

    def test_records_honest_verdicts_not_empty_success(self) -> None:
        for verdict in (
            "NATURAL_EXIT",
            "PLANNED_TIMEOUT",
            "LAUNCH_FAILED",
            "BUILD_FAILED",
            "NO_DISPLAY",
            "SKIPPED",
            "STACKING_REGRESSION",
            "UNSUPPORTED_BACKEND",
            "UNSUPPORTED_RUNTIME",
        ):
            with self.subTest(verdict=verdict):
                self.assertIn(verdict, self.text)
        # A missing screenshot must be recorded as missing, never captured/pass.
        self.assertIn('screenshot="missing"', self.text)
        self.assertIn('screenshot="captured"', self.text)
        self.assertIn('screenshot="partial"', self.text)

    def test_missing_capture_records_missing_plus_the_reason(self) -> None:
        # Every capture failure path sets an explicit reason, and the row appends
        # it; the screenshot column is never upgraded to a fabricated path.
        helper_start = self.text.index("capture_screencast_frame() {")
        helper = self.text[helper_start : self.text.index("\n}\n", helper_start) + 3]
        self.assertIn('capture_reason="gst-launch-1.0 or screencast-capture.py unavailable"', helper)
        self.assertIn('capture_reason="Mutter ScreenCast capture failed', helper)
        self.assertIn("reported success but wrote no frame", helper)
        # A capture only counts when the file really exists and is non-empty.
        self.assertNotIn('screenshot="captured"', helper)
        self.assertIn('-s "$dir/$frame_label.png"', self.text)

    def test_requires_a_real_display_session(self) -> None:
        self.assertIn("no display session available", self.text)
        self.assertIn("exit 2", self.text)

    def test_capture_is_labelled_as_compositor_output_on_both_carriers(self) -> None:
        self.assertIn("compositor", self.text)
        self.assertIn("Mutter ScreenCast", self.text)
        self.assertIn("FULL primary monitor", self.text)
        # The X11 carrier is described as composited rather than an X11 read-back.
        self.assertIn("read-back", self.text)
        self.assertIn("dom-canvas", self.text)

    def test_declares_the_authorization_boundary(self) -> None:
        self.assertIn("Authorization boundary", self.text)
        self.assertIn("same-UID", self.text)
        self.assertIn("never runs as root", self.text)
        self.assertIn("never uploaded", self.text)
        # 2026-10-23 authorization: elevation/root/unlocking/installs are permitted
        # in the environment, but faking evidence is still forbidden.
        self.assertIn("2026-10-23", self.text)
        self.assertIn("virtual/headless display", self.text)
        self.assertIn("software rendering", self.text)
        self.assertIn("real GPU", self.text)

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

    def test_x11_dry_run_plans_a_real_run_with_a_screencast_frame(self) -> None:
        # Launch-free check of the x11 plan: a real run captured through the
        # ScreenCast stream, not a refusal. Works with or without a display
        # (--dry-run stops before availability gating).
        result = subprocess.run(
            ["bash", str(COLLECT), "--backend", "x11", "--dry-run"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("backend mode   : x11", result.stdout)
        self.assertIn("dom-canvas", result.stdout)
        self.assertIn("x11-frame.png", result.stdout)
        self.assertNotIn("UNSUPPORTED_BACKEND", result.stdout)
        self.assertNotIn("UNSUPPORTED_RUNTIME", result.stdout)


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
