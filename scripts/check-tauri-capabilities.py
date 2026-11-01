#!/usr/bin/env python3
"""Static Tauri v2 **capability** contracts for every shipped React/Tauri host.

Tauri v2 denies *every* core-plugin command (including `plugin:event|listen`)
unless a capability file grants it for the calling window. Application-defined
commands (`cad_*`) are **not** ACL-gated, so the only capability the chrome needs
is the push-only `cad_event` stream: `core:event:allow-listen` plus
`core:event:allow-unlisten` (and nothing else — not `emit`, not `core:default`).

What this file pins:

  (a) every host has at least one `capabilities/**/*.json` file. A host with no
      capability directory makes `tauri-build` generate an empty capability set,
      which is exactly how the runtime banner
      `Command plugin:event|listen not allowed by ACL` appears;
  (b) each capability file is a **single** capability object (an `identifier` plus
      `permissions`). Tauri also accepts a list/`{"capabilities": [...]}` file, but
      this check supports only the single-object form and says so instead of
      silently ignoring a list;
  (c) for every window label a host actually creates — extracted from the host's
      `WINDOW_LABEL` / `MAIN_WINDOW_LABEL` constant and, for Android, also from
      `tauri.conf.json`'s `app.windows[*].label` — at least one matching
      capability (matched with `fnmatch` globs over its `windows` entries) grants
      both event permissions, or a documented superset (`core:event:default`,
      `core:default`);
  (d) `app.security.capabilities` is absent or `[]`. A non-empty list means the
      build reads capabilities from the config instead of the files, so the files
      above could be ignored;
  (e) the shared chrome (`apps/desktop-ui/src/**/*.ts{,x}`) imports only
      `@tauri-apps/api/core` (app commands, no ACL) and
      `@tauri-apps/api/event` (needs the two permissions above). Any other core
      module fails: its permission must be added here and to the capability
      files.

It does **NOT** build, link, run or screenshot any host, and it is not a
substitute for a real window: runtime ACL behaviour, the generated
`gen/schemas/capabilities.json`, and the actual event delivery are NOT VERIFIED
here. Standard library only, no network, deterministic.

A capability's `windows` field only matches **window** labels; a window's child
webviews inherit the window's capabilities, which is why `probe-window` covers
the `probe-webview` child.
"""
from __future__ import annotations

import fnmatch
import json
import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
UI_SRC = ROOT / "apps" / "desktop-ui" / "src"

EVENT_DESKTOP_LABEL_RE = re.compile(
    r'pub const WINDOW_LABEL: &str = "([^"]+)";'
)
EVENT_ANDROID_LABEL_RE = re.compile(
    r'pub const MAIN_WINDOW_LABEL: &str = "([^"]+)";'
)
API_IMPORT_RE = re.compile(r"""from\s+['"]@tauri-apps/api/([^'"]+)['"]""")

# The two permissions the chrome's `listen()` call needs, plus the two supersets
# Tauri documents (`core:event:default` also allows emit; `core:default` is every
# core plugin). Any of these granted together satisfies the contract.
REQUIRED_PERMISSIONS = ("core:event:allow-listen", "core:event:allow-unlisten")
SUPERSET_PERMISSIONS = ("core:event:default", "core:default")

ALLOWED_API_MODULES = frozenset({"core", "event"})


@dataclass(frozen=True)
class Host:
    """One Tauri host and where its created window labels are declared."""

    name: str
    root: str
    label_source: str
    label_pattern: re.Pattern[str]
    labels_from_conf: bool = False


HOSTS: tuple[Host, ...] = (
    Host("app-linux", "apps/app-linux", "src/host/options.rs", EVENT_DESKTOP_LABEL_RE),
    Host("app-windows", "apps/app-windows", "src/host/mod.rs", EVENT_DESKTOP_LABEL_RE),
    Host("app-macos", "apps/app-macos", "src/host/mod.rs", EVENT_DESKTOP_LABEL_RE),
    Host(
        "app-android",
        "apps/app-android/src-tauri",
        "src/host.rs",
        EVENT_ANDROID_LABEL_RE,
        labels_from_conf=True,
    ),
)


def window_labels(host: Host, problems: list[str]) -> set[str]:
    """Labels from the host constant plus (Android) `app.windows[*].label`."""
    source = ROOT / host.root / host.label_source
    labels: set[str] = set()
    try:
        text = source.read_text(encoding="utf-8")
    except OSError as exc:
        problems.append(f"{host.name}: cannot read {host.label_source} ({exc})")
    else:
        for match in host.label_pattern.finditer(text):
            labels.add(match.group(1))
        if not labels:
            problems.append(
                f"{host.name}: no window label constant in {host.label_source}"
            )

    if not host.labels_from_conf:
        return labels

    conf = ROOT / host.root / "tauri.conf.json"
    try:
        data = json.loads(conf.read_text(encoding="utf-8"))
    except OSError as exc:
        problems.append(f"{host.name}: cannot read tauri.conf.json ({exc})")
        return labels
    except json.JSONDecodeError as exc:
        problems.append(f"{host.name}: tauri.conf.json is not JSON ({exc})")
        return labels
    windows = ((data.get("app") or {}).get("windows")) or []
    if not isinstance(windows, list):
        problems.append(f"{host.name}: tauri.conf.json app.windows is not a list")
        return labels
    for index, entry in enumerate(windows):
        if not isinstance(entry, dict):
            problems.append(
                f"{host.name}: tauri.conf.json app.windows[{index}] is not an object"
            )
            continue
        # Tauri's `WindowConfig` defaults a missing label to `main`.
        label = entry.get("label", "main")
        if not isinstance(label, str) or not label:
            problems.append(
                f"{host.name}: tauri.conf.json app.windows[{index}].label is not a "
                "non-empty string"
            )
            continue
        labels.add(label)
    return labels


def permission_ids(entry: object) -> set[str]:
    """Permission identifiers of one capability entry (string or extended form)."""
    if isinstance(entry, str):
        return {entry}
    if isinstance(entry, dict):
        identifier = entry.get("identifier")
        if isinstance(identifier, str):
            return {identifier}
    return set()


def capability_problems(path: pathlib.Path, problems: list[str]) -> dict | None:
    """Validate the single-object capability shape; return it when valid."""
    rel = path.relative_to(ROOT)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        problems.append(f"{rel}: cannot read ({exc})")
        return None
    except json.JSONDecodeError as exc:
        problems.append(f"{rel}: is not JSON ({exc})")
        return None

    if isinstance(data, list):
        problems.append(
            f"{rel}: Tauri list-form capability files are not supported by this "
            "check; use one single-object capability per file"
        )
        return None
    if isinstance(data, dict) and "identifier" not in data and isinstance(
        data.get("capabilities"), list
    ):
        problems.append(
            f"{rel}: Tauri `{{\"capabilities\": [...]}}` files are not supported by "
            "this check; use one single-object capability per file"
        )
        return None
    if not isinstance(data, dict):
        problems.append(f"{rel}: capability must be a single JSON object")
        return None

    identifier = data.get("identifier")
    if not isinstance(identifier, str) or not identifier:
        problems.append(f"{rel}: missing string 'identifier'")
    permissions = data.get("permissions")
    if not isinstance(permissions, list):
        problems.append(f"{rel}: missing list 'permissions'")
    windows = data.get("windows", [])
    if not isinstance(windows, list) or not all(
        isinstance(pattern, str) for pattern in windows
    ):
        problems.append(
            f"{rel}: 'windows' must be a list of label glob patterns (got "
            f"{type(windows).__name__})"
        )
    return data


def matching_capabilities(
    host: Host, label: str, capabilities: list[tuple[pathlib.Path, dict]]
) -> list[tuple[pathlib.Path, dict]]:
    matched: list[tuple[pathlib.Path, dict]] = []
    for path, data in capabilities:
        patterns = data.get("windows")
        if not isinstance(patterns, list):
            continue
        if any(
            isinstance(pattern, str) and fnmatch.fnmatchcase(label, pattern)
            for pattern in patterns
        ):
            matched.append((path, data))
    return matched


def grants_event(matched: list[tuple[pathlib.Path, dict]]) -> bool:
    """True when one matching capability grants both permissions/supersets."""
    for _path, data in matched:
        granted: set[str] = set()
        for entry in data.get("permissions") or []:
            granted |= permission_ids(entry)
        if set(REQUIRED_PERMISSIONS) <= granted or granted & set(SUPERSET_PERMISSIONS):
            return True
    return False


def security_config_problems(host: Host, problems: list[str]) -> None:
    conf = ROOT / host.root / "tauri.conf.json"
    try:
        data = json.loads(conf.read_text(encoding="utf-8"))
    except OSError as exc:
        problems.append(f"{host.name}: cannot read tauri.conf.json ({exc})")
        return
    except json.JSONDecodeError as exc:
        problems.append(f"{host.name}: tauri.conf.json is not JSON ({exc})")
        return
    security = ((data.get("app") or {}).get("security")) or {}
    declared = security.get("capabilities") if isinstance(security, dict) else None
    if declared is None:
        return
    if isinstance(declared, list) and not declared:
        return
    problems.append(
        f"{host.name}: app.security.capabilities is not absent/[] — capabilities "
        "from capability files may be ignored"
    )


def host_problems(host: Host, problems: list[str]) -> None:
    root = ROOT / host.root
    capability_dir = root / "capabilities"
    if not capability_dir.is_dir():
        problems.append(
            f"{host.name}: no capabilities/ directory — tauri-build generates an "
            "empty capability set and denies plugin:event|listen"
        )
        return
    paths = sorted(path for path in capability_dir.glob("**/*.json") if path.is_file())
    if not paths:
        problems.append(
            f"{host.name}: capabilities/ has no *.json — tauri-build generates an "
            "empty capability set and denies plugin:event|listen"
        )
        return

    capabilities: list[tuple[pathlib.Path, dict]] = []
    for path in paths:
        data = capability_problems(path, problems)
        if data is not None:
            capabilities.append((path, data))

    labels = window_labels(host, problems)
    for label in sorted(labels):
        matched = matching_capabilities(host, label, capabilities)
        if not matched:
            problems.append(
                f"{host.name}: window {label!r} matches no capability 'windows' "
                "glob — its webview has no IPC access at all"
            )
            continue
        if not grants_event(matched):
            granted = ", ".join(str(path.relative_to(ROOT)) for path, _ in matched)
            problems.append(
                f"{host.name}: window {label!r} matches {granted} but no matching "
                "capability grants both "
                f"{REQUIRED_PERMISSIONS[0]!r} and {REQUIRED_PERMISSIONS[1]!r} "
                "(or a superset)"
            )

    security_config_problems(host, problems)


def api_import_problems(problems: list[str]) -> None:
    if not UI_SRC.is_dir():
        problems.append(f"shared chrome source missing: {UI_SRC.relative_to(ROOT)}")
        return
    files = sorted(
        path
        for path in UI_SRC.rglob("*")
        if path.is_file() and path.suffix in (".ts", ".tsx")
    )
    for path in files:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as exc:
            problems.append(f"{path.relative_to(ROOT)}: cannot read ({exc})")
            continue
        for module in API_IMPORT_RE.findall(text):
            if module in ALLOWED_API_MODULES:
                continue
            problems.append(
                f"{path.relative_to(ROOT)}: imports '@tauri-apps/api/{module}': "
                "add a capability permission and update "
                "scripts/check-tauri-capabilities.py"
            )


def main() -> int:
    problems: list[str] = []
    for host in HOSTS:
        host_problems(host, problems)
    api_import_problems(problems)

    if problems:
        for problem in problems:
            print(problem, file=sys.stderr)
        return 1

    host_list = ", ".join(host.name for host in HOSTS)
    print(
        f"OK: {len(HOSTS)} Tauri hosts ({host_list}) ship capability files that grant "
        "the event listen/unlisten pair for every created window label; "
        "app.security.capabilities is absent/[]; the shared chrome imports only "
        "@tauri-apps/api/{core,event} (static text check only; runtime ACL NOT VERIFIED)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
