#!/usr/bin/env python3
"""Static + mutation contracts for the web WebGPU -> WebGL2 fallback.

Regression guarded here: `cad-ui-slint::web::select_backend` previously gated a
forced `WebGpu` request on `webgpu_api_present()` (does `navigator.gpu` exist?)
only. A browser can expose `navigator.gpu` and even satisfy the JS host's
`requestAdapter()` pre-probe with a SwiftShader software adapter behind
`--enable-unsafe-webgpu`, while wgpu's `BROWSER_WEBGPU` backend still returns no
adapter. `select_backend` then returned `Ok`, Slint selected a renderer, but the
CAD bridge never attached: the page hung at "initializing renderer" with
``lifecycle=Detached`` / ``adapter=None`` and no error and no fallback.

The fix requires the *same* real `webgpu_available()` probe the renderer uses, so
a failure returns `GpuFailure` and `app-web::start_with_preference` takes its
existing non-Auto fallback to WebGL2. This checks the source text only; it is not
evidence that a browser was run (see docs/validation-web.md §12).
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
WEB_RS = ROOT / "crates/cad-ui-slint/src/web.rs"
BROWSER_RS = ROOT / "apps/app-web/src/browser.rs"

# (human label, fragment that must appear).
REQUIRED_FRAGMENTS: tuple[tuple[str, str], ...] = (
    (
        "forced WebGPU uses the real wgpu probe",
        "BackendPreference::WebGpu && !webgpu_available().await",
    ),
    (
        "software WebGPU adapter is rejected",
        "matches!(info.device_type, wgpu::DeviceType::Cpu)",
    ),
    (
        "non-Auto failure falls back to WebGL2",
        "select_backend(cad_ui_slint::web::BackendPreference::WebGl2).await?",
    ),
)

# Fragment that would re-introduce the silent hang: API presence alone deciding
# the forced WebGPU path.
FORBIDDEN_FRAGMENT = "BackendPreference::WebGpu && !webgpu_api_present()"


def check_sources(web_text: str, browser_text: str) -> list[str]:
    """Return one problem string per broken fallback guarantee."""
    problems = [
        f"missing {label}: {fragment!r}"
        for label, fragment in REQUIRED_FRAGMENTS
        if fragment not in web_text and fragment not in browser_text
    ]
    if FORBIDDEN_FRAGMENT in web_text:
        problems.append(
            "forced WebGPU must not select on navigator.gpu presence alone: "
            f"{FORBIDDEN_FRAGMENT!r} is present"
        )
    return problems


class WebBackendFallbackContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.web = WEB_RS.read_text(encoding="utf-8")
        self.browser = BROWSER_RS.read_text(encoding="utf-8")

    def test_real_sources_satisfy_every_guarantee(self) -> None:
        self.assertEqual(check_sources(self.web, self.browser), [])

    def test_each_guarantee_is_required(self) -> None:
        for label, fragment in REQUIRED_FRAGMENTS:
            with self.subTest(guarantee=label):
                # The fragment lives in one of the two sources; drop it from both
                # so the mutation always reaches the copy that contains it.
                mutated_web = self.web.replace(fragment, "")
                mutated_browser = self.browser.replace(fragment, "")
                self.assertNotEqual(
                    check_sources(mutated_web, mutated_browser),
                    [],
                    f"mutation not detected: {label}",
                )

    def test_api_presence_alone_is_rejected(self) -> None:
        # Reinstate the exact pre-fix guard and the checker must catch it.
        mutated = self.web.replace(
            "BackendPreference::WebGpu && !webgpu_available().await",
            FORBIDDEN_FRAGMENT,
        )
        problems = check_sources(mutated, self.browser)
        self.assertTrue(problems)
        self.assertTrue(any("presence alone" in problem for problem in problems))

    def test_missing_fallback_is_rejected(self) -> None:
        mutated = self.browser.replace(
            "select_backend(cad_ui_slint::web::BackendPreference::WebGl2).await?", ""
        )
        problems = check_sources(self.web, mutated)
        self.assertTrue(problems)
        self.assertTrue(any("falls back to WebGL2" in problem for problem in problems))


if __name__ == "__main__":
    unittest.main()
