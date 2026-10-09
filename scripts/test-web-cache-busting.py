#!/usr/bin/env python3
"""Static + mutation contracts for web build-stamp cache busting.

Regression guarded here: `apps/app-web/web/main.js` imported the wasm-bindgen
glue and wasm with bare URLs (`./pkg/yacr.js`, `yacr_bg.wasm`). Because deploys
do not content-hash those file names, a browser could hold an old `pkg/yacr.js`
while fetching a newer `pkg/yacr_bg.wasm`; the new wasm then asks for a
`__wbg_*` import the stale glue does not define and the page dies with
``LinkError: import object field '__wbg_...' is not a Function`` (observed on the
production Pages deploy). `scripts/build-web.sh` now stamps index.html with the
wasm content hash and the host requests main.js, pkg/yacr.js and
pkg/yacr_bg.wasm under that one stamp, so the pair is always fetched together.

This checks the source text only; it is not evidence that a browser was run.
"""
from __future__ import annotations

import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent

# (file, human label, fragment that must appear).
REQUIRED_FRAGMENTS: tuple[tuple[pathlib.Path, str, str], ...] = (
    (
        ROOT / "apps/app-web/web/index.html",
        "build stamp meta placeholder",
        'name="yacr-build" content="__YACR_BUILD__"',
    ),
    (
        ROOT / "apps/app-web/web/index.html",
        "host entry is stamped",
        "./main.js?v=${build}",
    ),
    (
        ROOT / "apps/app-web/web/main.js",
        "build stamp is read",
        'meta[name="yacr-build"]',
    ),
    (
        ROOT / "apps/app-web/web/main.js",
        "glue is stamped",
        "./pkg/yacr.js?v=${buildStamp}",
    ),
    (
        ROOT / "apps/app-web/web/main.js",
        "wasm is stamped",
        "./pkg/yacr_bg.wasm?v=${buildStamp}",
    ),
    (
        ROOT / "scripts/build-web.sh",
        "stamp is the wasm content hash",
        'hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest()[:16]',
    ),
    (
        ROOT / "scripts/build-web.sh",
        "placeholder is replaced",
        'text.replace("__YACR_BUILD__", sys.argv[2])',
    ),
    (
        ROOT / "scripts/build-web.sh",
        "missing placeholder fails the build",
        "is missing the __YACR_BUILD__ placeholder",
    ),
)


def check_sources(texts: dict[pathlib.Path, str]) -> list[str]:
    """Return one problem string per missing cache-busting guarantee."""
    problems = []
    for path, label, fragment in REQUIRED_FRAGMENTS:
        if fragment not in texts[path]:
            problems.append(f"missing {label} in {path.name}: {fragment!r}")
    return problems


class WebCacheBustingContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.texts = {
            path: path.read_text(encoding="utf-8")
            for path, _, _ in REQUIRED_FRAGMENTS
        }

    def test_real_sources_satisfy_every_guarantee(self) -> None:
        self.assertEqual(check_sources(self.texts), [])

    def test_each_guarantee_is_required(self) -> None:
        for path, label, fragment in REQUIRED_FRAGMENTS:
            with self.subTest(guarantee=label):
                mutated = dict(self.texts)
                mutated[path] = mutated[path].replace(fragment, "")
                self.assertNotEqual(
                    check_sources(mutated), [], f"mutation not detected: {label}"
                )

    def test_bare_unversioned_glue_import_is_rejected(self) -> None:
        # Reinstating the pre-fix bare specifier must be caught.
        mutated = dict(self.texts)
        main = ROOT / "apps/app-web/web/main.js"
        mutated[main] = mutated[main].replace(
            "import(`./pkg/yacr.js?v=${buildStamp}`)", 'import("./pkg/yacr.js")'
        )
        problems = check_sources(mutated)
        self.assertTrue(any("glue is stamped" in p for p in problems))


if __name__ == "__main__":
    unittest.main()
