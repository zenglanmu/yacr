#!/usr/bin/env python3
"""Contract test for the static yacr landing page and its deployment path.

Standard library only. It deliberately does not run any network call, browser or
Cloudflare command: it checks the committed page/manifest shape and the
fail-closed fragments of the two deployment scripts and the manual workflow.

Run: python3 scripts/test-landing-site.py
"""
from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LANDING = ROOT / "apps" / "landing"
DEPLOY = ROOT / "scripts" / "deploy-landing-pages.sh"
UPLOAD = ROOT / "scripts" / "upload-r2-downloads.sh"
WORKFLOW = ROOT / ".github" / "workflows" / "site.yml"
BUILD_CHANGELOG = ROOT / "scripts" / "build-landing-changelog.py"
CHANGELOG = LANDING / "changelog.json"
HEADERS = LANDING / "_headers"

RELEASE = "v0.1.10"
GITHUB_BASE = "https://github.com/zenglanmu/yacr/releases/download/v0.1.10/"
CDN_BASE = "https://dl-yacr.snakeheartgo.top/releases/v0.1.10"
KNOWN_ASSETS = {
    "linux": ("yacr-0.1.10-linux-x86_64.flatpak", 111603792),
    "windows": ("yacr-0.1.10-windows-x86_64.zip", 105630803),
    "macos": ("yacr-0.1.10-macos-universal.tar.gz", 150206682),
    "android": ("app-universal-release.apk", 112248289),
}

# The page shows one genuine product screenshot; the asset must exist, be a PNG
# and keep the declared pixel size (the <img width/height> derive from it).
SCREENSHOT = LANDING / "assets" / "screenshots" / "desktop-flange-2026-11-08.png"
SCREENSHOT_SIZE = (1206, 834)


def check_screenshot(errors: list[str]) -> None:
    if not SCREENSHOT.is_file():
        fail(f"missing screenshot asset: {SCREENSHOT.relative_to(ROOT)}", errors)
        return
    data = SCREENSHOT.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        fail(f"{SCREENSHOT.name} is not a PNG", errors)
        return
    if len(data) < 24:
        fail(f"{SCREENSHOT.name} is truncated", errors)
        return
    width = int.from_bytes(data[16:20], "big")
    height = int.from_bytes(data[20:24], "big")
    if (width, height) != SCREENSHOT_SIZE:
        fail(
            f"{SCREENSHOT.name} is {width}x{height}; page/contract expect "
            f"{SCREENSHOT_SIZE[0]}x{SCREENSHOT_SIZE[1]}",
            errors,
        )


def fail(message: str, errors: list[str]) -> None:
    errors.append(message)


def non_empty(path: pathlib.Path, errors: list[str]) -> str | None:
    if not path.is_file():
        fail(f"missing file: {path.relative_to(ROOT)}", errors)
        return None
    text = path.read_text(encoding="utf-8")
    if not text.strip():
        fail(f"empty file: {path.relative_to(ROOT)}", errors)
        return None
    return text


def check_manifest(text: str, errors: list[str]) -> None:
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        fail(f"downloads.json is not valid JSON: {exc}", errors)
        return
    if not isinstance(data, dict):
        fail("downloads.json must be a JSON object", errors)
        return

    if data.get("release") != RELEASE:
        fail(f"downloads.json release must be {RELEASE!r}", errors)
    if data.get("githubBase") != GITHUB_BASE:
        fail(f"downloads.json githubBase must be {GITHUB_BASE!r}", errors)
    if data.get("cdnBase") != CDN_BASE:
        fail(
            f"downloads.json cdnBase must be {CDN_BASE!r} (the cached custom "
            "download domain)",
            errors,
        )

    cdn_base = data.get("cdnBase")
    if cdn_base is not None and not (
        isinstance(cdn_base, str) and cdn_base.startswith("https://")
    ):
        fail("downloads.json cdnBase must be null or an https:// URL", errors)

    assets = data.get("assets")
    if not isinstance(assets, dict):
        fail("downloads.json assets must be an object", errors)
        return
    for label, (filename, size) in KNOWN_ASSETS.items():
        entry = assets.get(label)
        if not isinstance(entry, dict):
            fail(f"downloads.json assets.{label} is missing", errors)
            continue
        if entry.get("file") != filename:
            fail(
                f"downloads.json assets.{label}.file must be {filename!r}",
                errors,
            )
        if entry.get("size") != size:
            fail(f"downloads.json assets.{label}.size must be {size}", errors)
        sha256 = entry.get("sha256")
        # The committed fallback manifest may omit hashes; when the R2 mirror
        # rewrites it (`upload-r2-downloads.sh`) every asset carries one.
        if sha256 is not None and (
            not isinstance(sha256, str)
            or re.fullmatch(r"[0-9a-fA-F]{64}", sha256) is None
        ):
            fail(f"downloads.json assets.{label}.sha256 is not a SHA-256 digest", errors)


def check_changelog(text: str, release: str | None, errors: list[str]) -> None:
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        fail(f"changelog.json is not valid JSON: {exc}", errors)
        return
    if not isinstance(data, dict):
        fail("changelog.json must be a JSON object", errors)
        return
    if data.get("generatedFrom") != "docs/release-notes":
        fail("changelog.json generatedFrom must be 'docs/release-notes'", errors)
    releases = data.get("releases")
    if not isinstance(releases, list) or not releases:
        fail("changelog.json releases must be a non-empty array", errors)
        return

    keys: list[tuple[int, int, int]] = []
    versions: list[str] = []
    for index, entry in enumerate(releases):
        where = f"changelog.json releases[{index}]"
        if not isinstance(entry, dict):
            fail(f"{where} must be an object", errors)
            continue
        version = entry.get("version")
        if not isinstance(version, str) or re.fullmatch(r"v\d+\.\d+\.\d+", version) is None:
            fail(f"{where}.version must be vMAJOR.MINOR.PATCH", errors)
        else:
            versions.append(version)
            keys.append(tuple(int(part) for part in version[1:].split(".")))  # type: ignore[arg-type]
        if not isinstance(entry.get("title"), str) or not entry["title"].strip():
            fail(f"{where}.title must be a non-empty string", errors)
        url = entry.get("url")
        if not isinstance(url, str) or not url.startswith(
            "https://github.com/zenglanmu/yacr/releases/tag/"
        ):
            fail(f"{where}.url must be a GitHub release tag URL", errors)
        date = entry.get("date")
        if date is not None and (
            not isinstance(date, str) or re.fullmatch(r"\d{4}-\d{2}-\d{2}", date) is None
        ):
            fail(f"{where}.date must be null or YYYY-MM-DD", errors)
        sections = entry.get("sections")
        if not isinstance(sections, list) or not sections:
            fail(f"{where}.sections must be a non-empty array", errors)
            continue
        for section in sections:
            if not isinstance(section, dict):
                fail(f"{where}.sections entries must be objects", errors)
                continue
            if not isinstance(section.get("heading"), str) or not section["heading"].strip():
                fail(f"{where}.sections[].heading must be a non-empty string", errors)
            items = section.get("items")
            if not isinstance(items, list) or not items:
                fail(f"{where}.sections[].items must be a non-empty array", errors)
                continue
            if any(not isinstance(item, str) or not item.strip() for item in items):
                fail(f"{where}.sections[].items must be non-empty strings", errors)

    if len(versions) != len(set(versions)):
        fail("changelog.json releases contain duplicate versions", errors)
    if keys and keys != sorted(keys, reverse=True):
        fail("changelog.json releases must be newest-first", errors)
    if release is not None and versions and versions[0] != release:
        fail(
            f"changelog.json latest release {versions[0]!r} must match "
            f"downloads.json release {release!r}",
            errors,
        )


def check_changelog_freshness(errors: list[str]) -> None:
    """The committed changelog must equal a fresh build from the release notes."""
    try:
        completed = subprocess.run(
            [sys.executable, str(BUILD_CHANGELOG), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as exc:
        fail(f"could not run scripts/build-landing-changelog.py: {exc}", errors)
        return
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        fail(
            "apps/landing/changelog.json is stale; run "
            f"scripts/build-landing-changelog.py ({detail})",
            errors,
        )


def check_headers(text: str, errors: list[str]) -> None:
    """`/downloads.json` must stay cross-origin readable by the desktop/web app.

    The in-app update check fetches this manifest from `tauri://localhost` /
    `http://tauri.localhost` / the web bundle origin, so the landing deployment
    has to send `Access-Control-Allow-Origin: *`. The rest of the site stays
    same-origin-only (`connect-src 'self'`).
    """
    blocks: dict[str, list[str]] = {}
    current: str | None = None
    for line in text.splitlines():
        if line and not line[0].isspace():
            current = line.strip()
            blocks[current] = []
        elif current is not None:
            blocks[current].append(line.strip())
    manifest = blocks.get("/downloads.json")
    if manifest is None:
        fail("apps/landing/_headers has no /downloads.json block", errors)
        return
    headers = [line for line in manifest if line]
    if "Access-Control-Allow-Origin: *" not in headers:
        fail(
            "apps/landing/_headers /downloads.json must send "
            "'Access-Control-Allow-Origin: *' for the in-app update check",
            errors,
        )
    if "Content-Type: application/json; charset=utf-8" not in headers:
        fail("apps/landing/_headers /downloads.json must declare JSON content type", errors)


def check_index(text: str, errors: list[str]) -> None:
    for fragment in (
        "yacr.pages.dev",
        "yacr-examples.pages.dev",
        "assets/screenshots/desktop-flange-2026-11-08.png",
        "REAL SCREENSHOT",
        "725K",
        "823",
        "981",
        "68",
        "755",
        "158",
        "Rust",
        "React",
        "Tauri",
        "AGPL",
        "Cloudflare",
    ):
        if fragment not in text:
            fail(f"apps/landing/index.html is missing {fragment!r}", errors)


def check_main_js(text: str, errors: list[str]) -> None:
    for fragment in ("downloads.json", "changelog.json", "cdnBase"):
        if fragment not in text:
            fail(f"apps/landing/main.js is missing {fragment!r}", errors)
    if "catch" not in text and "githubBase" not in text:
        fail(
            "apps/landing/main.js has no fallback path when the manifest fetch fails "
            "(expected `catch` or `githubBase`)",
            errors,
        )


def check_scripts(errors: list[str]) -> None:
    deploy = non_empty(DEPLOY, errors)
    if deploy is not None:
        for fragment in (
            "set -euo pipefail",
            "wrangler",
            "pages deploy",
            "yacr",
            "CLOUDFLARE_API_TOKEN",
            "CLOUDFLARE_ACCOUNT_ID",
            "node --check",
            "downloads.json",
            "build-landing-changelog.py",
        ):
            if fragment not in deploy:
                fail(f"scripts/deploy-landing-pages.sh is missing {fragment!r}", errors)

    upload = non_empty(UPLOAD, errors)
    if upload is not None:
        for fragment in (
            "set -euo pipefail",
            "domains/managed",
            "result.domain",
            "--remote",
            "sha256sum -c",
            "releases/",
            "downloads.json",
            "app-universal-release.apk",
            "R2_PUBLIC_BASE",
            "dl-yacr.snakeheartgo.top",
        ):
            if fragment not in upload:
                fail(f"scripts/upload-r2-downloads.sh is missing {fragment!r}", errors)


def check_workflow(errors: list[str]) -> None:
    text = non_empty(WORKFLOW, errors)
    if text is None:
        return
    for fragment in (
        "workflow_dispatch",
        "upload_downloads",
        "scripts/upload-r2-downloads.sh",
        "scripts/deploy-landing-pages.sh",
        "CLOUDFLARE_API_TOKEN",
        "secrets.CLOUDFLARE_ACCOUNT_ID",
        "R2_PUBLIC_BASE",
        "public_base",
        "concurrency",
        "timeout-minutes",
    ):
        if fragment not in text:
            fail(f".github/workflows/site.yml is missing {fragment!r}", errors)

    # Manual-only: a tag push can race `publish-release.yml` before the release
    # assets exist, so this workflow must not gain a `v*` tag trigger.
    if "refs/tags/v" in text:
        fail(
            ".github/workflows/site.yml must stay workflow_dispatch-only; a tag "
            "trigger can race publish-release.yml before assets exist",
            errors,
        )
    if re.search(r"continue-on-error\s*:\s*true\b", text, re.IGNORECASE):
        fail(".github/workflows/site.yml must not use continue-on-error: true", errors)
    if "gh release" in text:
        fail(
            ".github/workflows/site.yml must not publish a GitHub Release "
            "(publish-release.yml is the only entry point)",
            errors,
        )


def main() -> int:
    errors: list[str] = []
    texts: dict[str, str] = {}
    for name in (
        "index.html",
        "styles.css",
        "main.js",
        "downloads.json",
        "changelog.json",
    ):
        text = non_empty(LANDING / name, errors)
        if text is not None:
            texts[name] = text
    release: str | None = None
    if "downloads.json" in texts:
        check_manifest(texts["downloads.json"], errors)
        try:
            parsed = json.loads(texts["downloads.json"])
        except json.JSONDecodeError:
            parsed = None
        if isinstance(parsed, dict) and isinstance(parsed.get("release"), str):
            release = parsed["release"]
    if "changelog.json" in texts:
        check_changelog(texts["changelog.json"], release, errors)
    check_changelog_freshness(errors)
    if "index.html" in texts:
        check_index(texts["index.html"], errors)
    check_screenshot(errors)
    if "main.js" in texts:
        check_main_js(texts["main.js"], errors)
    headers = non_empty(HEADERS, errors)
    if headers is not None:
        check_headers(headers, errors)
    check_scripts(errors)
    check_workflow(errors)

    if errors:
        print(f"FAIL: {len(errors)} landing-site contract problem(s):", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print("OK: landing page, download manifest and deployment entry points satisfy the contract")
    return 0


if __name__ == "__main__":
    sys.exit(main())
