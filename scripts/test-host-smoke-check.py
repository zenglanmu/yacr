#!/usr/bin/env python3
"""Contract tests for `scripts/host-smoke-check.py`.

Standard library only; runs in the `core-quality` CI job (no display, no GPU).
Each case is a synthetic host log: the checker must accept a healthy settled run
and reject every failure this gate exists for.

Run directly: `python3 scripts/test-host-smoke-check.py`
"""

from __future__ import annotations

import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("host_smoke_check", ROOT / "scripts/host-smoke-check.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

GEOMETRY_MATCH = (
    "[probe] geometry top=1012x689 container=960x600 container_xy=(26,60) "
    "dom=960x489 dom_xy=(0,52) drawing_area=960x489 origin=(26,112) scale=1 "
    "dom_physical=960x489 native_surface=960x489 margins top=52 bottom=59 "
    "start=0 end=0 revalidated=false"
)
GEOMETRY_MISMATCH = GEOMETRY_MATCH.replace(
    "dom_physical=960x489 native_surface=960x489",
    "dom_physical=3851x1645 native_surface=3852x1644",
)
GEOMETRY_UNCONFIGURED = GEOMETRY_MATCH.replace(
    "dom_physical=960x489 native_surface=960x489",
    "dom_physical=960x489 native_surface=0x0",
)
FRAME = (
    "[probe] cad-frame presented view_current=3 view_rendered=3 "
    "complete=current_scene device_epoch=1 texture_rev=1 size=960x489 "
    "frame_id=42 frame_us=2000"
)


def log(*lines: str) -> str:
    return "\n".join(lines) + "\n"


class HostSmokeCheckContracts(unittest.TestCase):
    def test_a_settled_run_passes(self) -> None:
        ok, summary, findings = checker.check(
            log(
                "[probe] cad session created",
                "[probe] cad render session bound (generation 0)",
                GEOMETRY_UNCONFIGURED,  # pre-presenter line: skipped
                GEOMETRY_MATCH,
                FRAME,
            )
        )
        self.assertTrue(ok, findings)
        self.assertIn("frames=1", summary)
        self.assertIn("settled=1", summary)

    def test_a_transient_mismatch_then_settle_passes(self) -> None:
        transient = GEOMETRY_MATCH.replace("native_surface=960x489", "native_surface=960x470")
        ok, _summary, findings = checker.check(log(transient, GEOMETRY_MATCH, FRAME))
        self.assertTrue(ok, findings)

    def test_the_reported_pixel_mismatch_fails(self) -> None:
        ok, _summary, findings = checker.check(log(GEOMETRY_MISMATCH, FRAME))
        self.assertFalse(ok)
        self.assertTrue(any("settled size mismatch" in item for item in findings), findings)

    def test_a_present_refusal_fails(self) -> None:
        refusal = (
            "[probe] cad present failed: present_texture failed: native texture 3851x1645 "
            "does not match the 3852x1644 surface; refusing to stretch it"
        )
        ok, _summary, findings = checker.check(log(GEOMETRY_MISMATCH, refusal, FRAME))
        self.assertFalse(ok)
        self.assertTrue(any("refusing to stretch" in item for item in findings), findings)

    def test_no_presented_frame_fails(self) -> None:
        ok, _summary, findings = checker.check(log(GEOMETRY_MATCH))
        self.assertFalse(ok)
        self.assertTrue(any("no CAD frame was presented" in item for item in findings), findings)

    def test_a_frame_that_never_completes_fails(self) -> None:
        partial = FRAME.replace("complete=current_scene", "complete=partial")
        ok, _summary, findings = checker.check(log(GEOMETRY_MATCH, partial))
        self.assertFalse(ok)
        self.assertTrue(any("complete=current_scene" in item for item in findings), findings)

    def test_a_frame_size_that_disagrees_with_the_surface_fails(self) -> None:
        stale = FRAME.replace("size=960x489", "size=960x470")
        ok, _summary, findings = checker.check(log(GEOMETRY_MATCH, stale))
        self.assertFalse(ok)
        self.assertTrue(any("last presented texture size" in item for item in findings), findings)

    def test_no_geometry_fails(self) -> None:
        ok, _summary, findings = checker.check(log(FRAME))
        self.assertFalse(ok)
        self.assertTrue(any("no '[probe] geometry' line" in item for item in findings), findings)

    def test_only_unconfigured_geometry_fails(self) -> None:
        ok, _summary, findings = checker.check(log(GEOMETRY_UNCONFIGURED, FRAME))
        self.assertFalse(ok)
        self.assertTrue(any("never configured" in item for item in findings), findings)

    def test_cli_exit_codes_and_summary(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            good = pathlib.Path(directory) / "good.log"
            bad = pathlib.Path(directory) / "bad.log"
            good.write_text(log(GEOMETRY_MATCH, FRAME), encoding="utf-8")
            bad.write_text(log(GEOMETRY_MISMATCH, FRAME), encoding="utf-8")
            passed = subprocess.run(
                [sys.executable, str(ROOT / "scripts/host-smoke-check.py"), str(good)],
                capture_output=True,
                text=True,
            )
            failed = subprocess.run(
                [sys.executable, str(ROOT / "scripts/host-smoke-check.py"), str(bad)],
                capture_output=True,
                text=True,
            )
        self.assertEqual(passed.returncode, 0, passed.stdout + passed.stderr)
        self.assertIn("PASS", passed.stdout)
        self.assertEqual(failed.returncode, 1, failed.stdout + failed.stderr)
        self.assertIn("FAIL", failed.stdout)


if __name__ == "__main__":
    unittest.main()
