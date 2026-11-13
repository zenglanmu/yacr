#!/usr/bin/env python3
"""Cross-carrier OS file-association gate.

``packaging/mime/formats.json`` is the **single authority** for the set of file
types Yacr claims at the OS level (AGENTS.md iron rule 9: a capability that every
carrier exposes needs one carrier-independent contract plus a test that fails when
a single carrier drifts). The carriers do not read the JSON at build time -- a
``.desktop`` file, a shared-mime-info XML, an AppStream metainfo entry, a Tauri
Android ``bundle.fileAssociations`` list, a *committed* generated
``AndroidManifest.xml`` and a ``CFBundleDocumentTypes`` heredoc inside a shell
script cannot -- so every one of them is a hand transcription that silently rots.
This gate re-reads all of them on every run and compares them with the authority.

What it checks:

* **canonical sanity** -- extensions are unique, lowercase and dot-free; every
  ``mimeTypes[].extensions`` member is a canonical extension; every canonical
  extension has at least one MIME type; MIME types are not duplicated;
  ``sharedMimeInfo`` types are a subset of the advertised MIME types; the
  Windows object-code type ``application/x-coff`` is never declared (the
  Geomview coloured-OFF variant is ``application/x-geomview-coff``; see the
  ``$comment`` prose in the authority, which explains that choice and is not a
  declaration).
* **in-app pickers** -- the three Rust ``OPEN_EXTENSIONS`` arrays
  (``app-linux`` / ``app-windows`` / ``app-macos`` ``src/platform.rs``, the
  ``const`` definition only, never the ``#[cfg(test)]`` uses) and the frontend
  ``WEB_OPEN_EXTENSIONS`` (``apps/desktop-ui/src/host/web-adapter.ts``) must hold
  the canonical extension set. The Rust arrays are display order, so they must
  match the canonical order exactly; the web array is compared as a set.
* **Linux** -- the ``.desktop`` ``MimeType=`` set equals the canonical MIME-type
  set and keeps its trailing ``;``; the Flatpak manifest installs the
  shared-mime-info XML at ``/app/share/mime/packages/<app-id>.xml`` and ships it
  as a source; the XML defines exactly the ``sharedMimeInfo`` types with exactly
  their declared globs and never redefines a type shared-mime-info already ships
  (``image/vnd.dwg``, ``image/vnd.dxf``, ``model/obj``, ``model/stl``); the
  AppStream metainfo ``<mediatype>`` list covers every canonical MIME type.
* **Android** -- ``bundle.fileAssociations`` has, for every canonical MIME type,
  exactly one association carrying that type and exactly that type's canonical
  extensions, every association declares the lowercase ``view`` intent action,
  one ``application/octet-stream`` association covers the whole canonical set,
  the union of all extensions is the canonical set, and the **committed**
  generated ``AndroidManifest.xml`` really contains the matching
  ``android:pathPattern`` / ``android:mimeType`` entries (a stale generated block
  is the failure this catches -- regenerating it is a separate step).
* **macOS** -- the ``CFBundleDocumentTypes`` block written by
  ``scripts/package-macos-release.sh`` covers every canonical extension.
* **Windows** -- there is no association to assert: the release artifact is a zip
  with no installer, so ``docs/windows-app.md`` must say explicitly that registry
  file associations are **NOT IMPLEMENTED** and that they can only be wired by an
  installer bundler. The gate asserts that honest boundary instead of inventing a
  carrier.

Exit codes: ``0`` every carrier agrees (``OK: 0 mismatches``), ``1`` at least one
carrier drifted (every mismatch is printed), ``2`` an input could not be read or
parsed -- a shape error is *never* reported as a pass (the anchors this gate
parses moved, so re-derive the gate, do not widen it by accident).

Stdlib only, no network, no writes. Paths resolve from ``__file__``, not the
current directory.
"""
from __future__ import annotations

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

FORMATS = ROOT / "packaging" / "mime" / "formats.json"
DESKTOP = ROOT / "packaging" / "flatpak" / "io.github.zenglanmu.yacr.desktop"
FLATPAK = ROOT / "packaging" / "flatpak" / "io.github.zenglanmu.yacr.yml"
MIME_XML = ROOT / "packaging" / "flatpak" / "io.github.zenglanmu.yacr.mime.xml"
METAINFO = ROOT / "packaging" / "flatpak" / "io.github.zenglanmu.yacr.metainfo.xml"

RUST_PICKERS = {
    "linux": ROOT / "apps" / "app-linux" / "src" / "platform.rs",
    "windows": ROOT / "apps" / "app-windows" / "src" / "platform.rs",
    "macos": ROOT / "apps" / "app-macos" / "src" / "platform.rs",
}
WEB_PICKER = ROOT / "apps" / "desktop-ui" / "src" / "host" / "web-adapter.ts"

ANDROID_CONF = ROOT / "apps" / "app-android" / "src-tauri" / "tauri.conf.json"
ANDROID_MANIFEST = (
    ROOT
    / "apps"
    / "app-android"
    / "src-tauri"
    / "gen"
    / "android"
    / "app"
    / "src"
    / "main"
    / "AndroidManifest.xml"
)

MACOS_SCRIPT = ROOT / "scripts" / "package-macos-release.sh"
WINDOWS_DOC = ROOT / "docs" / "windows-app.md"

MIME_XML_INSTALL_TARGET = "/app/share/mime/packages/io.github.zenglanmu.yacr.xml"
MIME_XML_NAME = "io.github.zenglanmu.yacr.mime.xml"

# shared-mime-info >= 2.0 already owns these types. Redefining one would override
# the host database for every application, so the Flatpak XML must never do it.
SHIPPED_BY_SHARED_MIME_INFO = ("image/vnd.dwg", "image/vnd.dxf", "model/obj", "model/stl")

# Android object code, NOT the Geomview coloured-OFF variant. Never canonical.
FORBIDDEN_CANONICAL_TYPE = "application/x-coff"
FORBIDDEN_CANONICAL_JSON = json.dumps(FORBIDDEN_CANONICAL_TYPE)

RUST_PICKER_RE = re.compile(
    r"^[ \t]*(?:pub\s+)?const OPEN_EXTENSIONS: &\[&str\] = &\[(.*?)\];",
    re.S | re.M,
)
WEB_PICKER_RE = re.compile(
    r"export const WEB_OPEN_EXTENSIONS: readonly string\[\] = Object\.freeze\(\[(.*?)\]\);",
    re.S,
)
DESKTOP_MIME_RE = re.compile(r"^MimeType=(.*)$", re.M)
MIME_ATTR_RE = re.compile(r'android:mimeType="([^"]*)"')
PATH_PATTERN_RE = re.compile(r'android:pathPattern="([^"]*)"')
PLIST_HEREDOC = 'cat > "${STAGE}/Yacr.app/Contents/Info.plist" <<EOF'
PLIST_EXTENSIONS_ARRAY_RE = re.compile(
    r"<key>CFBundleTypeExtensions</key>\s*<array>(.*?)</array>", re.S
)


class ShapeError(Exception):
    """An input does not have the structure this gate parses."""


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        raise ShapeError("%s: cannot be read (%s)" % (path, exc)) from exc
    except UnicodeDecodeError as exc:
        raise ShapeError("%s: cannot be decoded as UTF-8 (%s)" % (path, exc)) from exc


def read_json(path: Path) -> tuple[object, str]:
    text = read_text(path)
    try:
        return json.loads(text), text
    except json.JSONDecodeError as exc:
        raise ShapeError("%s: invalid JSON (%s)" % (path, exc)) from exc


def read_xml(path: Path) -> ET.Element:
    try:
        return ET.parse(path).getroot()
    except OSError as exc:
        raise ShapeError("%s: cannot be read (%s)" % (path, exc)) from exc
    except ET.ParseError as exc:
        raise ShapeError("%s: invalid XML (%s)" % (path, exc)) from exc


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


class Canonical:
    """The parsed authority plus the raw text (for JSON-literal checks)."""

    def __init__(self, extensions: list[str], mime_types: list[dict],
                 shared_mime_info: list[dict], android_extra: list[str],
                 text: str) -> None:
        self.extensions = extensions
        self.mime_types = mime_types
        self.shared_mime_info = shared_mime_info
        self.android_extra = android_extra
        self.text = text

    @property
    def types(self) -> list[str]:
        return [entry["type"] for entry in self.mime_types]

    def extensions_for(self, mime_type: str) -> list[str]:
        for entry in self.mime_types:
            if entry["type"] == mime_type:
                return entry["extensions"]
        return []


def load_canonical() -> Canonical:
    document, text = read_json(FORMATS)
    if not isinstance(document, dict):
        raise ShapeError("%s: top level must be an object" % FORMATS)
    extensions = document.get("extensions")
    mime_types = document.get("mimeTypes")
    shared = document.get("sharedMimeInfo")
    android_extra = document.get("androidExtraMimeTypes", [])
    if not isinstance(extensions, list) or not all(isinstance(item, str) for item in extensions):
        raise ShapeError("%s: `extensions` must be a list of strings" % FORMATS)
    if not isinstance(mime_types, list):
        raise ShapeError("%s: `mimeTypes` must be a list" % FORMATS)
    if not isinstance(shared, list):
        raise ShapeError("%s: `sharedMimeInfo` must be a list" % FORMATS)
    if not isinstance(android_extra, list) or not all(
        isinstance(item, str) for item in android_extra
    ):
        raise ShapeError("%s: `androidExtraMimeTypes` must be a list of strings" % FORMATS)
    for index, entry in enumerate(mime_types):
        if not isinstance(entry, dict) or not isinstance(entry.get("type"), str):
            raise ShapeError("%s: mimeTypes[%d].type must be a string" % (FORMATS, index))
        if not isinstance(entry.get("extensions"), list) or not all(
            isinstance(item, str) for item in entry["extensions"]
        ):
            raise ShapeError(
                "%s: mimeTypes[%d].extensions must be a list of strings" % (FORMATS, index)
            )
    for index, entry in enumerate(shared):
        if not isinstance(entry, dict) or not isinstance(entry.get("type"), str):
            raise ShapeError("%s: sharedMimeInfo[%d].type must be a string" % (FORMATS, index))
        if not isinstance(entry.get("globs"), list) or not all(
            isinstance(item, str) for item in entry["globs"]
        ):
            raise ShapeError(
                "%s: sharedMimeInfo[%d].globs must be a list of strings" % (FORMATS, index)
            )
    return Canonical(extensions, mime_types, shared, android_extra, text)


# ---------------------------------------------------------------------------
# canonical sanity
# ---------------------------------------------------------------------------
def check_canonical(canonical: Canonical) -> list[str]:
    problems: list[str] = []
    known = set()

    for extension in canonical.extensions:
        if not extension:
            problems.append("[canonical] empty extension in `extensions`")
            continue
        if extension in known:
            problems.append("[canonical] duplicate extension %r" % extension)
        known.add(extension)
        if extension != extension.lower():
            problems.append("[canonical] extension %r must be lowercase" % extension)
        if extension.startswith("."):
            problems.append("[canonical] extension %r must not start with a dot" % extension)

    types: list[str] = []
    covered: set[str] = set()
    for entry in canonical.mime_types:
        mime_type = entry["type"]
        if mime_type in types:
            problems.append("[canonical] duplicate mime type %r" % mime_type)
        types.append(mime_type)
        if not entry["extensions"]:
            problems.append("[canonical] mime type %r lists no extensions" % mime_type)
        for extension in entry["extensions"]:
            covered.add(extension)
            if extension not in known:
                problems.append(
                    "[canonical] mime type %r references %r, which is not in `extensions`"
                    % (mime_type, extension)
                )
    for extension in canonical.extensions:
        if extension not in covered:
            problems.append(
                "[canonical] extension %r has no mime type in `mimeTypes`" % extension
            )

    type_set = set(types)
    for entry in canonical.shared_mime_info:
        if entry["type"] not in type_set:
            problems.append(
                "[canonical] sharedMimeInfo type %r is not advertised in `mimeTypes`"
                % entry["type"]
            )
        for glob in entry["globs"]:
            match = re.fullmatch(r"\*\.([^.]+)", glob)
            if match is None:
                problems.append(
                    "[canonical] sharedMimeInfo type %r has a non-glob pattern %r"
                    % (entry["type"], glob)
                )
            elif match.group(1).lower() not in known:
                problems.append(
                    "[canonical] sharedMimeInfo type %r globs an undeclared extension %r"
                    % (entry["type"], glob)
                )

    # `application/x-coff` is Windows object code in shared-mime-info >= 2.5. The
    # `$comment` prose names it to explain the exclusion, so only the JSON string
    # literal (a declaration) and the parsed structures are rejected.
    if FORBIDDEN_CANONICAL_JSON in canonical.text:
        problems.append("[canonical] %r must never be declared" % FORBIDDEN_CANONICAL_TYPE)
    for entry in canonical.shared_mime_info:
        aliases = entry.get("aliases", [])
        if entry["type"] == FORBIDDEN_CANONICAL_TYPE or FORBIDDEN_CANONICAL_TYPE in aliases:
            problems.append(
                "[canonical] sharedMimeInfo must not declare/alias %r"
                % FORBIDDEN_CANONICAL_TYPE
            )
    return problems


# ---------------------------------------------------------------------------
# in-app pickers
# ---------------------------------------------------------------------------
def rust_picker_extensions(label: str, path: Path) -> list[str]:
    text = read_text(path)
    matches = RUST_PICKER_RE.findall(text)
    if len(matches) != 1:
        raise ShapeError(
            "%s: expected exactly one `const OPEN_EXTENSIONS: &[&str]` array, found %d"
            % (path, len(matches))
        )
    return re.findall(r'"([^"]*)"', matches[0])


def web_picker_extensions(path: Path) -> list[str]:
    text = read_text(path)
    match = WEB_PICKER_RE.search(text)
    if match is None:
        raise ShapeError(
            "%s: cannot find `export const WEB_OPEN_EXTENSIONS: readonly string[]`" % path
        )
    return [
        value[1:] if value.startswith(".") else value
        for value in re.findall(r"'([^']*)'", match.group(1))
    ]


def describe_set_drift(declared: list[str], expected: list[str]) -> str:
    declared_set, expected_set = set(declared), set(expected)
    parts = []
    missing = [item for item in expected if item not in declared_set]
    extra = [item for item in declared if item not in expected_set]
    if missing:
        parts.append("missing %s" % missing)
    if extra:
        parts.append("extra %s" % extra)
    duplicates = sorted({item for item in declared if declared.count(item) > 1})
    if duplicates:
        parts.append("duplicated %s" % duplicates)
    return "; ".join(parts)


def check_pickers(canonical: Canonical) -> list[str]:
    problems: list[str] = []
    for label, path in RUST_PICKERS.items():
        declared = rust_picker_extensions(label, path)
        if declared == canonical.extensions:
            continue
        drift = describe_set_drift(declared, canonical.extensions)
        if set(declared) == set(canonical.extensions) and not drift:
            problems.append(
                "[picker:%s] %s OPEN_EXTENSIONS must keep the canonical display order "
                "(declared %s, expected %s)"
                % (label, rel(path), declared, canonical.extensions)
            )
        else:
            problems.append(
                "[picker:%s] %s OPEN_EXTENSIONS drifts from packaging/mime/formats.json: %s"
                % (label, rel(path), drift)
            )

    declared = web_picker_extensions(WEB_PICKER)
    if set(declared) != set(canonical.extensions):
        problems.append(
            "[picker:web] %s WEB_OPEN_EXTENSIONS drifts from "
            "packaging/mime/formats.json: %s"
            % (rel(WEB_PICKER), describe_set_drift(declared, canonical.extensions))
        )
    return problems


# ---------------------------------------------------------------------------
# Linux: .desktop, Flatpak manifest, shared-mime-info XML, AppStream metainfo
# ---------------------------------------------------------------------------
def check_linux(canonical: Canonical) -> list[str]:
    problems: list[str] = []

    text = read_text(DESKTOP)
    match = DESKTOP_MIME_RE.search(text)
    if match is None:
        raise ShapeError("%s: no `MimeType=` line" % DESKTOP)
    raw = match.group(1).strip()
    if not raw.endswith(";"):
        problems.append("[linux] %s: `MimeType=` must end with ';'" % rel(DESKTOP))
    declared = [item for item in raw.split(";") if item]
    if set(declared) != set(canonical.types) or len(declared) != len(set(declared)):
        problems.append(
            "[linux] %s: `MimeType=` must equal the canonical mime-type set: %s"
            % (rel(DESKTOP), describe_set_drift(declared, canonical.types))
        )

    manifest = read_text(FLATPAK)
    install_re = re.compile(
        r"install\s+-\S+\s+%s\s+%s"
        % (re.escape(MIME_XML_NAME), re.escape(MIME_XML_INSTALL_TARGET))
    )
    if install_re.search(manifest) is None:
        problems.append(
            "[linux] %s: must install %s at %s"
            % (rel(FLATPAK), MIME_XML_NAME, MIME_XML_INSTALL_TARGET)
        )
    source_re = re.compile(r"^[ \t]*path:[ \t]*%s[ \t]*$" % re.escape(MIME_XML_NAME), re.M)
    if source_re.search(manifest) is None:
        problems.append(
            "[linux] %s: must list %s under `sources:`" % (rel(FLATPAK), MIME_XML_NAME)
        )

    root = read_xml(MIME_XML)
    defined: dict[str, list[str]] = {}
    aliases: set[str] = set()
    for element in root:
        if local_name(element.tag) != "mime-type":
            continue
        mime_type = element.get("type")
        if not mime_type:
            raise ShapeError("%s: a <mime-type> has no `type` attribute" % MIME_XML)
        if mime_type in defined:
            problems.append("[linux] %s: duplicate <mime-type %r>" % (rel(MIME_XML), mime_type))
        defined[mime_type] = [
            child.get("pattern", "")
            for child in element
            if local_name(child.tag) == "glob"
        ]
        aliases.update(
            child.get("type", "")
            for child in element
            if local_name(child.tag) == "alias"
        )
    wanted = {entry["type"]: entry["globs"] for entry in canonical.shared_mime_info}
    for mime_type, globs in wanted.items():
        if mime_type not in defined:
            problems.append(
                "[linux] %s: missing <mime-type %r> (declared in sharedMimeInfo)"
                % (rel(MIME_XML), mime_type)
            )
            continue
        absent = [glob for glob in globs if glob not in defined[mime_type]]
        unexpected = [glob for glob in defined[mime_type] if glob not in globs]
        if absent:
            problems.append(
                "[linux] %s: <mime-type %r> is missing globs %s"
                % (rel(MIME_XML), mime_type, absent)
            )
        if unexpected:
            problems.append(
                "[linux] %s: <mime-type %r> declares globs not in sharedMimeInfo: %s"
                % (rel(MIME_XML), mime_type, unexpected)
            )
    for mime_type in defined:
        if mime_type not in wanted:
            problems.append(
                "[linux] %s: defines %r, which is not in sharedMimeInfo"
                % (rel(MIME_XML), mime_type)
            )
    for mime_type in SHIPPED_BY_SHARED_MIME_INFO:
        if mime_type in defined or mime_type in aliases:
            problems.append(
                "[linux] %s: must not redefine %r (shared-mime-info already ships it)"
                % (rel(MIME_XML), mime_type)
            )

    root = read_xml(METAINFO)
    provided = {
        (element.text or "").strip()
        for element in root.iter()
        if local_name(element.tag) == "mediatype"
    }
    absent = [mime_type for mime_type in canonical.types if mime_type not in provided]
    if absent:
        problems.append(
            "[linux] %s: <provides>/<mediatype> misses %s"
            % (rel(METAINFO), absent)
        )
    return problems


# ---------------------------------------------------------------------------
# Android: bundle.fileAssociations + the committed generated manifest
# ---------------------------------------------------------------------------
def check_android(canonical: Canonical) -> list[str]:
    problems: list[str] = []
    document, _ = read_json(ANDROID_CONF)
    if not isinstance(document, dict):
        raise ShapeError("%s: top level must be an object" % ANDROID_CONF)
    bundle = document.get("bundle")
    if not isinstance(bundle, dict):
        raise ShapeError("%s: missing `bundle` object" % ANDROID_CONF)
    associations = bundle.get("fileAssociations")
    if not isinstance(associations, list):
        raise ShapeError("%s: missing `bundle.fileAssociations` list" % ANDROID_CONF)

    by_type: dict[str, list[str]] = {}
    ordered: list[dict] = []
    for index, association in enumerate(associations):
        if not isinstance(association, dict):
            raise ShapeError("%s: fileAssociations[%d] must be an object" % (ANDROID_CONF, index))
        mime_type = association.get("mimeType")
        extensions = association.get("ext")
        if not isinstance(mime_type, str) or not isinstance(extensions, list) or not all(
            isinstance(item, str) for item in extensions
        ):
            raise ShapeError(
                "%s: fileAssociations[%d] needs a string `mimeType` and an `ext` list "
                "of strings" % (ANDROID_CONF, index)
            )
        ordered.append(association)
        actions = association.get("androidIntentActionFilters")
        if not isinstance(actions, list) or "view" not in actions:
            problems.append(
                "[android] %s: fileAssociations[%d] (%s) needs "
                "`androidIntentActionFilters: ['view']`"
                % (rel(ANDROID_CONF), index, mime_type)
            )
        if mime_type in by_type:
            problems.append(
                "[android] %s: mimeType %r is declared twice"
                % (rel(ANDROID_CONF), mime_type)
            )
        by_type[mime_type] = extensions

    for mime_type in canonical.types:
        expected = canonical.extensions_for(mime_type)
        if mime_type not in by_type:
            problems.append(
                "[android] %s: missing fileAssociation for %r"
                % (rel(ANDROID_CONF), mime_type)
            )
            continue
        declared = by_type[mime_type]
        if set(declared) != set(expected) or len(declared) != len(set(declared)):
            problems.append(
                "[android] %s: %r must list exactly %s: %s"
                % (
                    rel(ANDROID_CONF),
                    mime_type,
                    expected,
                    describe_set_drift(declared, expected),
                )
            )

    allowed = set(canonical.types) | set(canonical.android_extra)
    for mime_type in by_type:
        if mime_type not in allowed:
            problems.append(
                "[android] %s: stale mimeType %r (not canonical, not in "
                "`androidExtraMimeTypes`)" % (rel(ANDROID_CONF), mime_type)
            )

    union = {extension for extensions in by_type.values() for extension in extensions}
    if union != set(canonical.extensions):
        problems.append(
            "[android] %s: the union of association extensions must be the canonical "
            "set: %s"
            % (rel(ANDROID_CONF), describe_set_drift(sorted(union), canonical.extensions))
        )

    for extra in canonical.android_extra:
        if extra not in by_type:
            problems.append(
                "[android] %s: missing the %r association (declared in "
                "`androidExtraMimeTypes`)" % (rel(ANDROID_CONF), extra)
            )
            continue
        declared = by_type[extra]
        if set(declared) != set(canonical.extensions) or len(declared) != len(
            set(canonical.extensions)
        ):
            problems.append(
                "[android] %s: %r must cover all canonical extensions: %s"
                % (
                    rel(ANDROID_CONF),
                    extra,
                    describe_set_drift(declared, canonical.extensions),
                )
            )

    # The generated manifest is committed: tauri-build only rewrites it during a
    # real Android build, so a stale block ships silently. Compare it with the
    # conf, not with a hand-written expectation.
    manifest = read_text(ANDROID_MANIFEST)
    patterns = set(PATH_PATTERN_RE.findall(manifest))
    for extension in canonical.extensions:
        expected = ".*\\\\." + extension
        if expected not in patterns:
            problems.append(
                "[android] %s: missing `android:pathPattern=%r` (stale generated "
                "file-association block?)" % (rel(ANDROID_MANIFEST), expected)
            )
    manifest_types = set(MIME_ATTR_RE.findall(manifest))
    conf_types = set(by_type)
    for mime_type in sorted(conf_types - manifest_types):
        problems.append(
            "[android] %s: missing `android:mimeType=%r` from "
            "bundle.fileAssociations" % (rel(ANDROID_MANIFEST), mime_type)
        )
    for mime_type in sorted(manifest_types - conf_types):
        problems.append(
            "[android] %s: `android:mimeType=%r` is stale (not in "
            "bundle.fileAssociations)" % (rel(ANDROID_MANIFEST), mime_type)
        )
    return problems


# ---------------------------------------------------------------------------
# macOS: the CFBundleDocumentTypes heredoc
# ---------------------------------------------------------------------------
def macos_document_type_extensions(path: Path) -> list[str]:
    text = read_text(path)
    start = text.find(PLIST_HEREDOC)
    if start == -1:
        raise ShapeError("%s: cannot find the Info.plist heredoc" % path)
    end = text.find("\nEOF\n", start)
    if end == -1:
        raise ShapeError("%s: cannot find the end of the Info.plist heredoc" % path)
    block = text[start:end]
    if "<key>CFBundleDocumentTypes</key>" not in block:
        return []
    return [
        extension
        for array in PLIST_EXTENSIONS_ARRAY_RE.findall(block)
        for extension in re.findall(r"<string>([^<]+)</string>", array)
    ]


def check_macos(canonical: Canonical) -> list[str]:
    problems: list[str] = []
    declared = macos_document_type_extensions(MACOS_SCRIPT)
    for extension in canonical.extensions:
        if extension not in declared:
            problems.append(
                "[macos] %s: CFBundleDocumentTypes misses %r"
                % (rel(MACOS_SCRIPT), extension)
            )
    for extension in sorted(set(declared)):
        if declared.count(extension) > 1:
            problems.append(
                "[macos] %s: CFBundleDocumentTypes declares %r more than once"
                % (rel(MACOS_SCRIPT), extension)
            )
    return problems


# ---------------------------------------------------------------------------
# Windows: the honest boundary, not an association
# ---------------------------------------------------------------------------
def check_windows(canonical: Canonical) -> list[str]:
    text = read_text(WINDOWS_DOC)
    problems: list[str] = []
    marker = "NOT IMPLEMENTED"
    if marker not in text:
        return [
            "[windows] %s: must state that registry file associations are %s"
            % (rel(WINDOWS_DOC), marker)
        ]
    # A stray "NOT IMPLEMENTED" elsewhere in the document does not count: the
    # statement must be *about* registry file associations.
    about_associations = False
    for match in re.finditer(re.escape(marker), text):
        window = text[max(0, match.start() - 600): match.start() + 600]
        if re.search(r"注册表|registry", window) and re.search(
            r"文件关联|file association", window, re.I
        ):
            about_associations = True
            break
    if not about_associations:
        problems.append(
            "[windows] %s: the %s statement must be about registry file associations"
            % (rel(WINDOWS_DOC), marker)
        )
    if "no installer" not in text:
        problems.append(
            "[windows] %s: must state that the release has no installer (registry "
            "associations need one)" % rel(WINDOWS_DOC)
        )
    if rel(FORMATS) not in text:
        problems.append(
            "[windows] %s: must cross-reference %s" % (rel(WINDOWS_DOC), rel(FORMATS))
        )
    return problems


def main() -> int:
    try:
        canonical = load_canonical()
        problems: list[str] = []
        problems.extend(check_canonical(canonical))
        problems.extend(check_pickers(canonical))
        problems.extend(check_linux(canonical))
        problems.extend(check_android(canonical))
        problems.extend(check_macos(canonical))
        problems.extend(check_windows(canonical))
    except ShapeError as exc:
        print("file-associations FAILED: %s" % exc)
        return 2

    print(
        "authority: %s: %d extensions, %d mime types, %d shared-mime-info types"
        % (
            rel(FORMATS),
            len(canonical.extensions),
            len(canonical.types),
            len(canonical.shared_mime_info),
        )
    )
    print(
        "carriers: pickers (linux/windows/macos/web), linux (.desktop + flatpak "
        "manifest + mime xml + metainfo), android (conf + committed manifest), "
        "macos (Info.plist), windows (NOT IMPLEMENTED boundary)"
    )

    if problems:
        print("\nFAIL: %d mismatch(es)" % len(problems))
        for line in problems:
            print("  - " + line)
        return 1
    print("OK: 0 mismatches")
    return 0


if __name__ == "__main__":
    sys.exit(main())
