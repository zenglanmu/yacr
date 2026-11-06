#!/usr/bin/env python3
"""Contract check: both local Web bundles stage the full CAD font catalogue by default.

The browser carrier still installs fonts **lazily**: after a document opens, the
page asks ``cad_fonts`` what the drawing needs (``requested`` / ``textNeeded``),
fetches only those catalogue faces from the same-origin ``fonts/``, installs them
with ``set_font_package`` and re-runs the install on the live session with
``apply_font_package``. What changed is the sourcing: a local bundle
(``scripts/build-web.sh`` -> ``apps/app-web/dist`` and ``scripts/build-web-ui.sh``
-> ``apps/app-web/dist-ui``) now carries that package under its own ``fonts/`` by
default — the full mlightcad catalogue merged with the committed ``fonts/``
(``WITH_FONTS=1``), with ``WITH_FONTS=0`` (committed package only, offline) and
``FONTS_DIR=<dir>`` (reuse a prepared package) as explicit alternatives. The
deployed site is unchanged: ``scripts/deploy-cloudflare-pages.sh`` still runs
``fetch-fonts.sh`` as the authoritative merge/verification step at the site root
(it skips files that already exist).

That split is easy to regress silently: a build script that stages fonts
**before** the Vite build has them deleted with ``outDir``; a bootstrap/main that
fetches before ``ready``/``start`` would block the session on the network; a
bootstrap/main that drops ``apply_font_package`` would leave the open document
unshaped with only an ``unsupported-resources: fonts`` diagnostic. This check
pins all three, the harness bundle whitelist, and the deploy package.

Standard library only; run directly:

    python3 scripts/test-web-font-package.py
"""
from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
FONT_CATALOG = ROOT / "fonts" / "fonts.json"
NOTO = "NotoSansCJKsc-Regular.otf"
BUILD_WEB_UI = ROOT / "scripts" / "build-web-ui.sh"
BUILD_WEB = ROOT / "scripts" / "build-web.sh"
BUILD_CANVAS = ROOT / "scripts" / "build-canvas-wasm.sh"
DEPLOY = ROOT / "scripts" / "deploy-cloudflare-pages.sh"
MAIN_JS = ROOT / "apps" / "app-web" / "web" / "main.js"
WORKER_JS = ROOT / "apps" / "app-web" / "web" / "worker.js"


def read(path: pathlib.Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        return f"__READ_ERROR__: {error}"


def catalog_files() -> list[str]:
    try:
        entries = json.loads(FONT_CATALOG.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return []
    if not isinstance(entries, list):
        return []
    return [
        entry["file"]
        for entry in entries
        if isinstance(entry, dict) and isinstance(entry.get("file"), str) and entry["file"]
    ]


def require_tokens(label: str, text: str, tokens: tuple[str, ...]) -> list[str]:
    return [f"{label}: missing {token!r}" for token in tokens if token not in text]


def check_catalog(files: list[str]) -> list[str]:
    problems: list[str] = []
    if not FONT_CATALOG.is_file():
        problems.append(f"{FONT_CATALOG.relative_to(ROOT)}: missing")
        return problems
    try:
        json.loads(FONT_CATALOG.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        problems.append(f"{FONT_CATALOG.relative_to(ROOT)}: cannot parse: {error}")
        return problems
    if not files:
        problems.append(f"{FONT_CATALOG.relative_to(ROOT)}: names no files")
    if NOTO not in files:
        problems.append(f"{FONT_CATALOG.relative_to(ROOT)}: no entry for {NOTO}")
    for name in files:
        if not (ROOT / "fonts" / name).is_file():
            problems.append(f"fonts/{name}: committed catalog file is missing")
    return problems


def check_build_web_ui() -> list[str]:
    """The chrome bundle stages the package with the shared helper, after the build."""
    label = "scripts/build-web-ui.sh"
    text = read(BUILD_WEB_UI)
    problems = require_tokens(
        label,
        text,
        (
            "scripts/lib/font-package.sh",
            "yacr_stage_fonts",
            "WITH_FONTS",
            "FONTS_DIR",
            'yacr_stage_fonts "$OUT_DIR/fonts"',
            "fonts/fonts.json",
        ),
    )
    for forbidden in ("YACR_WEB_FONTS", "no font package staged"):
        if forbidden in text:
            problems.append(
                f"{label}: still carries the opt-in/no-font contract ({forbidden!r})"
            )
    stage = text.find('yacr_stage_fonts "$OUT_DIR/fonts"')
    build = text.find("npm run build:web")
    if stage < 0:
        problems.append(f"{label}: no yacr_stage_fonts call for $OUT_DIR/fonts")
    elif build >= 0 and stage < build:
        problems.append(f"{label}: stages fonts before the Vite build (outDir is emptied)")
    return problems


def check_build_web() -> list[str]:
    """The harness bundle stages the same package and whitelists its files."""
    label = "scripts/build-web.sh"
    text = read(BUILD_WEB)
    problems = require_tokens(
        label,
        text,
        (
            "scripts/lib/font-package.sh",
            "yacr_stage_fonts",
            "WITH_FONTS",
            "FONTS_DIR",
            'yacr_stage_fonts "$OUT_DIR/fonts"',
            "fonts/fonts.json",
            "yacr_font_package_faces",
            "ensureFontPackage",
        ),
    )
    # The exact-file-count guard became a whitelist: the six shell/pkg artifacts
    # plus anything under `fonts/`. Any other path must fail the build.
    problems.extend(
        require_tokens(
            label,
            text,
            (
                'find "$OUT_DIR" -type f -print0',
                'rel="${path#"$OUT_DIR"/}"',
                "fonts/*",
                "unexpected file in $OUT_DIR",
            ),
        )
    )
    if "expected+=(" in text:
        problems.append(
            f"{label}: appends extra expected files; the bundle is the six artifacts plus fonts/"
        )
    return problems


def check_bootstrap() -> list[str]:
    """`ready` resolves from init; the package is lazy, after `open_bytes`."""
    label = "scripts/build-canvas-wasm.sh"
    text = read(BUILD_CANVAS)
    problems = require_tokens(
        label,
        text,
        (
            "ensureFontPackage",
            "set_font_package",
            "apply_font_package",
            "__YACR_FONT_BASE_URL__",
            "__YACR_FONT_PACKAGE__",
            "__YACR_FONT_PACKAGE_ERROR__",
            "session.open_bytes = async",
        ),
    )
    if "const ready = wasm.then" in text:
        problems.append(f"{label}: ready is gated on the package")
    open_at = text.find("await mod.open_bytes(")
    ensure_at = text.find("await ensureFontPackage()", open_at if open_at >= 0 else 0)
    if open_at < 0:
        problems.append(f"{label}: bootstrap does not wrap mod.open_bytes")
    elif ensure_at < 0:
        problems.append(f"{label}: wrapped open_bytes does not await ensureFontPackage()")
    # No pre-open fetch: the old `../fonts/fonts.json` URL must be gone.
    if "../fonts/fonts.json" in text:
        problems.append(f"{label}: bootstrap still preloads ../fonts/fonts.json")
    # The bootstrap must not hand a second give-away of the ready contract.
    if "const ready = wasm.then" in text:
        problems.append(f"{label}: ready is gated on the package")
    return problems


def check_main_js() -> list[str]:
    """The harness main-thread carrier behaves the same: lazy, after the open."""
    label = "apps/app-web/web/main.js"
    text = read(MAIN_JS)
    problems = require_tokens(
        label,
        text,
        (
            "ensureFontPackage",
            "set_font_package",
            "apply_font_package",
            "__YACR_FONT_BASE_URL__",
            "__YACR_FONT_PACKAGE_ERROR__",
        ),
    )
    if "await loadFontPackage()" in text:
        problems.append(f"{label}: still preloads the package in startMainThread")
    open_at = text.find("await open_bytes(")
    ensure_at = text.find("await ensureFontPackage()", open_at if open_at >= 0 else 0)
    if open_at < 0:
        problems.append(f"{label}: no open_bytes call to attach the lazy load to")
    elif ensure_at < 0:
        problems.append(f"{label}: openFile does not await ensureFontPackage() after the open")
    start_fn = text.find("async function startMainThread()")
    start_end = text.find("async function startIsolated()", start_fn if start_fn >= 0 else 0)
    if start_fn >= 0 and start_end > start_fn:
        if "await ensureFontPackage" in text[start_fn:start_end]:
            problems.append(f"{label}: startMainThread loads the font package before an open")
    if "../fonts/fonts.json" in text:
        problems.append(f"{label}: still fetches ../fonts/fonts.json")
    return problems


def check_worker_js() -> list[str]:
    text = read(WORKER_JS)
    problems = require_tokens(
        "apps/app-web/web/worker.js",
        text,
        ("FONT_PACKAGE_REFUSAL", "unsupported-runtime: fonts"),
    )
    for forbidden in ("ensureFontPackage", "set_font_package", "apply_font_package"):
        if forbidden in text:
            problems.append(
                f"apps/app-web/web/worker.js: must not load a font package ({forbidden!r})"
            )
    return problems


def check_deploy() -> list[str]:
    """The site root gets the full CDN package, assembled before the upload."""
    label = "scripts/deploy-cloudflare-pages.sh"
    text = read(DEPLOY)
    problems = require_tokens(
        label,
        text,
        (
            "scripts/fetch-fonts.sh",
            "SKIP_FONTS",
            '"$FONT_DEST/fonts.json"',
            NOTO,
            "BUNDLE_KIND",
        ),
    )
    fetch_at = text.find("scripts/fetch-fonts.sh")
    upload_at = text.find("pages deploy")
    if fetch_at < 0:
        problems.append(f"{label}: never assembles the font package")
    elif upload_at >= 0 and fetch_at > upload_at:
        problems.append(f"{label}: assembles fonts after the upload")
    # Both bundle kinds must get `fonts/`; the assembly must not be inside a kind
    # branch.
    kind_branch = text.find('case "$BUNDLE_KIND" in')
    if kind_branch >= 0 and fetch_at > kind_branch:
        problems.append(f"{label}: font assembly is inside a BUNDLE_KIND branch")
    return problems


def main() -> int:
    files = catalog_files()
    problems: list[str] = []
    for check in (
        lambda: check_catalog(files),
        check_build_web_ui,
        check_build_web,
        check_bootstrap,
        check_main_js,
        check_worker_js,
        check_deploy,
    ):
        try:
            problems.extend(check())
        except Exception as error:  # noqa: BLE001 - report, never crash into a pass
            problems.append(f"{check.__name__}: {error}")
    if problems:
        print("web-font-package: FAILED", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(
        "web-font-package OK: "
        f"{len(files)} committed catalog file(s); both local bundles stage the "
        "package after the build (WITH_FONTS=1 full catalogue, WITH_FONTS=0 "
        "committed, FONTS_DIR reuse); deploy still assembles/merges the site "
        "package; bootstrap/main install lazily after open (set_font_package + "
        "apply_font_package); worker refuses by name"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
