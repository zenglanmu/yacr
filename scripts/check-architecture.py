#!/usr/bin/env python3
"""Validate the workspace DAG and core integration boundaries (Python 3.11+).

Usage:
    python3 scripts/check-architecture.py [workspace-root]

`workspace-root` defaults to the repository the script lives in. It exists so the
guard contract test (`scripts/test-ui-migration-guards.py`) can point the check at
a temporary fixture workspace and mutate it without touching the real repo.

Two dependency views are kept deliberately separate:

* the **runtime graph** (ordinary `[dependencies]`, including target-specific
  ordinary tables) — used for `require_dependency` / `forbid_dependency`, cycle
  detection and the layer rules. Its semantics are unchanged.
* the **declared toolkit set** (ordinary + `build-dependencies` +
  `dev-dependencies`, each incl. target-specific tables) — used only by the
  toolkit ban, so an `iced` dependency cannot hide in a build script or a test
  target. Dependency renames
  (`x = { package = "iced", .. }`) and **workspace-inherited aliases**
  (`x.workspace = true`, defined in the selected root's
  `[workspace.dependencies]`) are both resolved to the real package name.
"""
from __future__ import annotations

import hashlib

import pathlib
import sys
import tomllib

DEFAULT_ROOT = pathlib.Path(__file__).resolve().parents[1]

# Cargo tables. The runtime graph intentionally excludes build/dev deps so the
# layering/require/forbid rules keep their original meaning.
RUNTIME_TABLES = ("dependencies",)
ALL_TABLES = ("dependencies", "build-dependencies", "dev-dependencies")
# Toolkits that must not appear anywhere in the workspace.
TOOLKIT_PACKAGES = {"iced"}


def load_workspace_dependencies(root: pathlib.Path) -> dict:
    """Root `[workspace.dependencies]` (the aliases members inherit)."""
    workspace = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]
    return workspace.get("dependencies", {})


def _workspace_alias_package(alias: str, workspace_dependencies: dict) -> str:
    """The real package name behind a `[workspace.dependencies]` alias entry."""
    entry = workspace_dependencies.get(alias)
    if isinstance(entry, dict):
        return entry.get("package", alias)
    return alias


# The only dependencies a `[patch.crates-io]` entry may name, each with the
# ledger that must record it (ADR-0031 for acadrust, ADR-0035 for rvt).
PATCH_LEDGERS = {
    "acadrust": "docs/acadrust-patches.md",
    "rvt": "docs/rvt-patches.md",
}
# Dependencies whose patch must be a **fixed in-repo source copy** (not a git
# ref), and the only `path` value accepted. The vendored manifest must keep the
# pinned version and Cargo.lock must resolve the package from that path: a
# version drift would otherwise make Cargo ignore the patch ("patch was not
# used") while the build happily linked the stock crate.
PATCH_LOCAL_PATHS = {"rvt": "vendor/rvt"}
# The only files inside the vendored copy that may differ from upstream. Every
# other file is pinned by `vendor/rvt/SHA256SUMS`, so patching a second upstream
# file (or dropping one in) fails the gate instead of passing silently.
PATCH_PATCHED_FILES = {"rvt": {"src/ifc/gltf.rs"}}


def _pinned_version(workspace_dependencies: dict, package: str) -> str | None:
    """The pinned workspace `package` version string, if it is a version dep."""
    entry = workspace_dependencies.get(package)
    if isinstance(entry, str):
        return entry
    if isinstance(entry, dict):
        version = entry.get("version")
        return version if isinstance(version, str) else None
    return None


def check_patch_policy(
    root: pathlib.Path, manifest: dict, workspace_dependencies: dict
) -> None:
    """Allow only tracked, pinned `[patch.crates-io]` entries from a fixed list.

    The relaxed rule (ADR-0031, extended by ADR-0035) is an exception, not
    general vendoring:

    * the only patch table may be `[patch.crates-io]`, and every key in it must
      be one of `PATCH_LEDGERS`;
    * that dependency's ledger must exist and must name the workspace's
      resolved version, so a patch cannot exist without a ledger row.

    No patch at all is the normal state; the current exception is `rvt`.
    """
    patch = manifest.get("patch")
    if patch is None:
        return
    assert set(patch) == {"crates-io"}, (
        "Cargo patch is allowed only as [patch.crates-io] for a dependency in "
        f"{sorted(PATCH_LEDGERS)} (found {sorted(patch)})"
    )
    entries = patch["crates-io"]
    assert isinstance(entries, dict) and entries, "[patch.crates-io] must not be empty"
    other = sorted(set(entries) - set(PATCH_LEDGERS))
    assert not other, (
        "Cargo patch is allowed only for "
        f"{sorted(PATCH_LEDGERS)}; found {other} in [patch.crates-io]"
    )
    for package, ledger_name in sorted(PATCH_LEDGERS.items()):
        if package not in entries:
            continue
        ledger = root / ledger_name
        assert ledger.is_file(), (
            f"a {package} [patch.crates-io] entry requires {ledger_name} "
            "(the tracked patch ledger)"
        )
        version = _pinned_version(workspace_dependencies, package)
        assert version, (
            f"cannot resolve the workspace {package} version; a {package} patch "
            "needs a pinned version string for the ledger"
        )
        assert version in ledger.read_text(encoding="utf-8"), (
            f"{ledger_name} must record the patched {package} version "
            f"{version!r}"
        )
        local = PATCH_LOCAL_PATHS.get(package)
        if local is None:
            continue
        entry = entries[package]
        assert isinstance(entry, dict) and set(entry) == {"path"}, (
            f"[patch.crates-io] {package} must be a fixed in-repo path "
            f'({{ path = "{local}" }}); found {entry!r}'
        )
        assert entry["path"] == local, (
            f"[patch.crates-io] {package} must point at {local!r}; "
            f"found {entry['path']!r}"
        )
        manifest = root / local / "Cargo.toml"
        assert manifest.is_file(), (
            f"[patch.crates-io] {package} points at {local!r}, which has no Cargo.toml"
        )
        vendored = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        assert vendored.get("name") == package, (
            f"{local}/Cargo.toml must declare package {package!r}; "
            f"found {vendored.get('name')!r}"
        )
        bare_version = version.lstrip("=^~ ")
        assert vendored.get("version") == bare_version, (
            f"{local}/Cargo.toml must keep the pinned version {bare_version!r} so the "
            f"patch matches the workspace dependency; found {vendored.get('version')!r}"
        )
        locked = [
            package_entry
            for package_entry in tomllib.loads(
                (root / "Cargo.lock").read_text(encoding="utf-8")
            ).get("package", [])
            if package_entry.get("name") == package
        ]
        assert len(locked) == 1, (
            f"Cargo.lock must contain exactly one {package!r} package; found {len(locked)}"
        )
        assert "source" not in locked[0], (
            f"Cargo.lock still resolves {package} from a source, so the in-repo patch is "
            "not in effect; update Cargo.lock after changing the pin"
        )
        patched = PATCH_PATCHED_FILES.get(package, set())
        sums = root / local / "SHA256SUMS"
        assert sums.is_file(), (
            f"{local}/SHA256SUMS must record every vendored file except the patched "
            f"{sorted(patched)}, so an unrecorded edit fails closed"
        )
        declared: dict[str, str] = {}
        for line in sums.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            digest, separator, relative = line.partition("  ")
            assert separator and len(digest) == 64, (
                f"malformed {local}/SHA256SUMS line: {line!r}"
            )
            declared[relative] = digest
        for relative, digest in declared.items():
            file = root / local / relative
            assert file.is_file(), f"{local}/SHA256SUMS records a missing file {relative!r}"
            assert hashlib.sha256(file.read_bytes()).hexdigest() == digest, (
                f"{local}/{relative} differs from the recorded upstream copy; only "
                f"{sorted(patched)} may be patched (ADR-0035)"
            )
        present = {
            file.relative_to(root / local).as_posix()
            for file in (root / local).rglob("*")
            if file.is_file()
        }
        unrecorded = sorted(present - set(declared) - patched - {sums.name})
        assert not unrecorded, (
            f"{local} holds file(s) not recorded in SHA256SUMS: {unrecorded}; record the "
            "upstream file or add it to the patch's allowed set (ADR-0035)"
        )


def _dependency_names(section: dict, workspace_dependencies: dict) -> set[str]:
    """Package names declared in one dependency table.

    Resolves an explicit rename (`x = { package = "iced" }`) and an
    inherited workspace alias (`x.workspace = true`, whose real package is named
    in the selected root's `[workspace.dependencies]`) to the actual package name.
    """
    names: set[str] = set()
    for key, value in section.items():
        if not isinstance(value, dict):
            names.add(key)
        elif "package" in value:
            names.add(value["package"])
        elif value.get("workspace") is True:
            names.add(_workspace_alias_package(key, workspace_dependencies))
        else:
            names.add(key)
    return names


def _declared(
    manifest: dict, tables: tuple[str, ...], workspace_dependencies: dict
) -> set[str]:
    """Union of the named dependency tables, ordinary and target-specific."""
    names: set[str] = set()
    for table in tables:
        names |= _dependency_names(manifest.get(table, {}), workspace_dependencies)
    for target in manifest.get("target", {}).values():
        for table in tables:
            names |= _dependency_names(target.get(table, {}), workspace_dependencies)
    return names


def runtime_dependencies(
    manifest: dict, workspace_dependencies: dict | None = None
) -> set[str]:
    """Ordinary + target-ordinary deps (the historical `dependencies()` view)."""
    return _declared(manifest, RUNTIME_TABLES, workspace_dependencies or {})


def declared_dependencies(
    manifest: dict, workspace_dependencies: dict | None = None
) -> set[str]:
    """Ordinary + build + dev deps (incl. target tables), rename/alias-aware."""
    return _declared(manifest, ALL_TABLES, workspace_dependencies or {})


def load_packages(root: pathlib.Path) -> dict:
    workspace = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]
    packages = {}
    for member in workspace["members"]:
        manifest = tomllib.loads((root / member / "Cargo.toml").read_text())
        packages[manifest["package"]["name"]] = manifest
    return packages


def check(root: pathlib.Path) -> int:
    packages = load_packages(root)
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    workspace_dependencies = manifest.get("workspace", {}).get("dependencies", {})
    check_patch_policy(root, manifest, workspace_dependencies)

    graph = {
        name: runtime_dependencies(manifest, workspace_dependencies) & packages.keys()
        for name, manifest in packages.items()
    }

    def require_dependency(package, dependency):
        assert package in packages, f"required package missing from workspace: {package}"
        assert dependency in graph[package], (
            f"{package} must depend on {dependency} (manifest dependency required)"
        )

    def forbid_dependency(package, dependency):
        assert package in packages, f"required package missing from workspace: {package}"
        assert dependency not in runtime_dependencies(
            packages[package], workspace_dependencies
        ), f"{package} must not depend on {dependency} (manifest dependency forbidden)"

    # Engine extraction boundaries: check declared package dependencies, not source
    # text, so violations have a stable, actionable manifest-level diagnostic.
    require_dependency("cad-engine", "cad-app")
    require_dependency("cad-engine", "cad-render-wgpu")
    forbidden_engine_dependencies = {
        "cad-host", "wgpu",
    }
    for dependency in sorted(forbidden_engine_dependencies):
        forbid_dependency("cad-engine", dependency)
    forbid_dependency("cad-app", "cad-engine")

    # Shared UI layer boundary: the platform-independent UI logic (`cad-ui-core`)
    # sits above the UI-free application/host contracts it consumes — it depends on
    # `cad-host` (the owner-thread session wraps `HostCore`) — and below every
    # concrete host/presenter. It must stay toolkit- and GPU-free: no UI
    # toolkit, no wgpu, no Tauri/GTK/portal, and no platform bindings.
    require_dependency("cad-ui-core", "cad-app")
    require_dependency("cad-ui-core", "cad-host")
    forbidden_ui_core_dependencies = {
        "wgpu",
        "tauri", "tauri-build", "gtk", "gdk", "gdkx11", "ashpd",
        "web-sys", "android-activity", "jni", "ndk",
    }
    for dependency in sorted(forbidden_ui_core_dependencies):
        forbid_dependency("cad-ui-core", dependency)

    # Desktop shell layer: `cad-desktop-shell` is the shared Tauri command/state
    # layer between `cad-ui-core` and the concrete `apps/app-*` hosts. It is the
    # **only** shared layer allowed to depend on `tauri`, and it must stay
    # toolkit-, portal- and GPU-free. `cad-app`/`cad-host`/`cad-platform` are
    # allowed (it names `cad_app` queue types); the renderer and engine are not
    # (telemetry stays app-side behind the `ShellTelemetry` seam).
    #
    # `app-windows` is the Windows desktop host (Tauri/WebView2 + native CAD
    # child HWND) and `app-macos` the macOS host (Tauri/WKWebView + native CAD
    # child NSView): like `app-linux`, both consume the shared command/state
    # layer, so the edge is required rather than merely allowed.
    #
    # `app-android` is the Android `native-below` carrier (P5): the native
    # `CadSession` owns `cad-db`, so its CAD commands (`cad_status`,
    # `cad_command`, the settings/layers reads, `probe_input`) are served by the
    # same shared registry — the edge is required, not optional. The JNI/NDK
    # bindings stay in that app (the `cad-*` platform-leak rule below).
    require_dependency("cad-desktop-shell", "cad-ui-core")
    require_dependency("app-linux", "cad-desktop-shell")
    require_dependency("app-windows", "cad-desktop-shell")
    require_dependency("app-macos", "cad-desktop-shell")
    require_dependency("app-android", "cad-desktop-shell")
    forbidden_desktop_shell_dependencies = {
        "gtk", "gdk", "gdkx11", "ashpd", "wgpu", "cad-render-wgpu", "cad-engine",
        "iced", "acadrust",
    }
    for dependency in sorted(forbidden_desktop_shell_dependencies):
        forbid_dependency("cad-desktop-shell", dependency)

    visited, active = set(), set()

    def visit(name):
        assert name not in active, f"dependency cycle at {name}"
        if name in visited:
            return
        active.add(name)
        for dependency in graph[name]:
            visit(dependency)
        active.remove(name)
        visited.add(name)

    for name, manifest in packages.items():
        visit(name)
        deps = runtime_dependencies(manifest, workspace_dependencies)
        if name != "cad-import-acadrust":
            assert "acadrust" not in deps, f"parser escaped importer: {name}"
        # Every package is banned from the toolkit, including build and dev
        # dependency tables (the runtime graph alone would miss a build-time
        # toolkit) and workspace-inherited aliases.
        declared = declared_dependencies(manifest, workspace_dependencies)
        toolkit = declared & TOOLKIT_PACKAGES
        assert not toolkit, f"UI/toolkit dependency leaked into {name}: {sorted(toolkit)}"
        if name != "cad-render-wgpu":
            assert "wgpu" not in deps, f"GPU dependency escaped renderer: {name}"
        if name.startswith("cad-") and name != "cad-render-wgpu":
            assert not deps & {"web-sys", "android-activity", "jni", "ndk"}, f"platform leaked into {name}"
        if name == "cad-domain":
            assert not graph[name], "domain depends on another CAD package"
        if name == "cad-db":
            assert graph[name] <= {"cad-domain"}, "database depends on a higher layer"
    # Cargo patch policy: only a tracked, pinned entry from `PATCH_LEDGERS`.
    # (The pre-check above has already validated it; no patch is the norm.)
    print(f"Architecture OK: {len(packages)} packages, acyclic and core boundaries intact")
    return 0


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    root = pathlib.Path(argv[0]).resolve() if argv else DEFAULT_ROOT
    try:
        return check(root)
    except AssertionError as error:
        print(f"architecture check FAILED: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
