#!/usr/bin/env python3
"""Build the landing-page changelog from `docs/release-notes/*.md`.

Standard library only, no network. The landing page is a plain static site with
no build step, so this script is the only "build" its changelog has: it turns the
committed release notes into `apps/landing/changelog.json`, which `main.js`
fetches at runtime.

The output is deterministic (no timestamps) so `--check` can run in CI and fail
when a release note was added without regenerating the committed JSON.

Run:
    python3 scripts/build-landing-changelog.py             # write the JSON
    python3 scripts/build-landing-changelog.py --check     # fail if stale
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
NOTES_DIR = ROOT / "docs" / "release-notes"
METAINFO = ROOT / "packaging" / "flatpak" / "io.github.zenglanmu.yacr.metainfo.xml"
OUTPUT = ROOT / "apps" / "landing" / "changelog.json"
RELEASE_URL = "https://github.com/zenglanmu/yacr/releases/tag/{version}"

VERSION_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
LINK_RE = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
TAG_RE = re.compile(r"<[^>]+>")
FENCE_RE = re.compile(r"^\s*```")
RELEASED_RE = re.compile(r"\bReleased:\s*(\d{4}-\d{2}-\d{2})\b")
CJK_PUNCTUATION = "，。；：、（）【】“”‘’！？《》·"


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def version_key(version: str) -> tuple[int, int, int]:
    match = VERSION_RE.match(version)
    if match is None:
        fail(f"release note version {version!r} is not vMAJOR.MINOR.PATCH")
    return tuple(int(part) for part in match.groups())  # type: ignore[return-value]


def load_release_dates(metainfo: pathlib.Path) -> dict[str, str]:
    """Map `0.1.6` -> `2026-11-09` from the AppStream release list."""
    if not metainfo.is_file():
        return {}
    text = metainfo.read_text(encoding="utf-8")
    dates: dict[str, str] = {}
    for match in re.finditer(
        r'<release\s+version="([^"]+)"(?:\s+date="([^"]+)")?', text
    ):
        version, date = match.group(1), match.group(2)
        if date:
            dates[version] = date
    return dates


def strip_inline_markdown(text: str) -> str:
    """Reduce one line of release-note Markdown to readable plain text."""
    text = LINK_RE.sub(r"\1", text)
    text = TAG_RE.sub("", text)
    text = text.replace("**", "").replace("__", "")
    text = text.replace("`", "")
    return " ".join(text.split())


def join_wrapped(previous: str, line: str) -> str:
    """Join a wrapped Markdown line without inserting a spurious CJK space."""
    previous = previous.rstrip()
    line = line.strip()
    if not previous or not line:
        return previous or line
    left, right = previous[-1], line[0]
    if left in CJK_PUNCTUATION or right in CJK_PUNCTUATION:
        return previous + line
    if left.isascii() and right.isascii():
        return f"{previous} {line}"
    if not left.isascii() and not right.isascii():
        return previous + line
    return f"{previous} {line}"


def find_section(lines: list[str], prefix: str) -> tuple[int, int] | None:
    """Return (start, end) of a `## <prefix>` block, end exclusive."""
    start = None
    for index, line in enumerate(lines):
        if start is None:
            if re.match(rf"^##\s+{re.escape(prefix)}", line):
                start = index + 1
        elif re.match(r"^##\s+", line):
            return start, index
    if start is None:
        return None
    return start, len(lines)


def parse_changes(lines: list[str], changed_heading: str) -> list[dict[str, object]]:
    """Parse the `## 本版变化` block into heading/item sections.

    Release notes use `### heading` subsections; older notes (v0.1.4) put bullets
    directly under `##`, so they fall back to a single section.
    """
    block = find_section(lines, "本版变化")
    if block is None:
        return []
    start, end = block
    sections: list[dict[str, object]] = []
    current_heading: str | None = None
    current_items: list[str] = []
    in_fence = False

    def flush() -> None:
        if current_items:
            sections.append(
                {
                    "heading": current_heading or changed_heading,
                    "items": list(current_items),
                }
            )
            current_items.clear()

    for line in lines[start:end]:
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        heading = re.match(r"^###\s+(.*\S)\s*$", line)
        if heading:
            flush()
            current_heading = strip_inline_markdown(heading.group(1))
            continue
        bullet = re.match(r"^\s*[-*]\s+(.*\S)\s*$", line)
        if bullet:
            current_items.append(strip_inline_markdown(bullet.group(1)))
            continue
        if current_items and line.strip() and not line.lstrip().startswith("#"):
            # Continuation of the previous bullet (wrapped Markdown line).
            current_items[-1] = join_wrapped(
                current_items[-1], strip_inline_markdown(line)
            )

    flush()
    return sections


def release_date(
    lines: list[str], version: str, dates: dict[str, str]
) -> str | None:
    """Release date from the note itself, else the AppStream release list.

    `docs/release-notes/*.md` may carry `发布日期 / Released: YYYY-MM-DD`; AppStream
    metadata covers the versions that shipped a package bump. Older notes without
    either stay date-less rather than guessing.
    """
    for line in lines:
        match = RELEASED_RE.search(line)
        if match:
            return match.group(1)
    return dates.get(version.lstrip("v"))


def parse_release(path: pathlib.Path, dates: dict[str, str]) -> dict[str, object]:
    version = path.stem
    key = version_key(version)
    lines = path.read_text(encoding="utf-8").splitlines()
    title = version
    for line in lines:
        match = re.match(r"^#\s+(.*\S)\s*$", line)
        if match:
            title = strip_inline_markdown(match.group(1)) or version
            break
    changed_heading = "本版变化 / What changed"
    for line in lines:
        match = re.match(r"^##\s+(本版变化.*)$", line)
        if match:
            changed_heading = strip_inline_markdown(match.group(1))
            break
    return {
        "version": version,
        "title": title,
        "date": release_date(lines, version, dates),
        "url": RELEASE_URL.format(version=version),
        "sections": parse_changes(lines, changed_heading),
        "_key": key,
    }


def build(notes_dir: pathlib.Path, metainfo: pathlib.Path) -> dict[str, object]:
    if not notes_dir.is_dir():
        fail(f"release notes directory not found: {notes_dir}")
    dates = load_release_dates(metainfo)
    releases = []
    for path in notes_dir.glob("v*.md"):
        if VERSION_RE.match(path.stem) is None:
            continue
        release = parse_release(path, dates)
        if not release["sections"]:
            fail(f"{path.name} has no `## 本版变化` bullets to publish")
        releases.append(release)
    if not releases:
        fail(f"no vMAJOR.MINOR.PATCH.md release notes in {notes_dir}")
    releases.sort(key=lambda release: release["_key"], reverse=True)  # type: ignore[index]
    for release in releases:
        del release["_key"]
    return {"generatedFrom": "docs/release-notes", "releases": releases}


def render(data: dict[str, object]) -> str:
    return json.dumps(data, ensure_ascii=False, indent=2) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--notes-dir", type=pathlib.Path, default=NOTES_DIR)
    parser.add_argument("--metainfo", type=pathlib.Path, default=METAINFO)
    parser.add_argument("--output", type=pathlib.Path, default=OUTPUT)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed changelog is up to date instead of writing it",
    )
    args = parser.parse_args(argv)

    text = render(build(args.notes_dir, args.metainfo))
    if args.check:
        if not args.output.is_file():
            fail(f"missing {args.output}; run scripts/build-landing-changelog.py")
        if args.output.read_text(encoding="utf-8") != text:
            fail(
                f"{args.output.relative_to(ROOT)} is stale; run "
                "scripts/build-landing-changelog.py"
            )
        print(f"OK: {args.output.relative_to(ROOT)} is up to date")
        return 0

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text, encoding="utf-8")
    print(f"wrote {args.output.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
