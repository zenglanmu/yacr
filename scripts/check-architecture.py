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
  Slint/toolkit ban, so a `slint`/`slint-build`/`iced` dependency cannot hide in
  a build script or a test target. Dependency renames
  (`x = { package = "slint-build", .. }`) and **workspace-inherited aliases**
  (`x.workspace = true`, defined in the selected root's
  `[workspace.dependencies]`) are both resolved to the real package name.
"""
from __future__ import annotations

import pathlib
import sys
import tomllib

DEFAULT_ROOT = pathlib.Path(__file__).resolve().parents[1]

# Cargo tables. The runtime graph intentionally excludes build/dev deps so the
# layering/require/forbid rules keep their original meaning.
RUNTIME_TABLES = ("dependencies",)
ALL_TABLES = ("dependencies", "build-dependencies", "dev-dependencies")
# Toolkits that must not appear anywhere in the workspace after the Slint
# removal (P3b).
TOOLKIT_PACKAGES = {"slint", "slint-build", "iced"}


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


def _dependency_names(section: dict, workspace_dependencies: dict) -> set[str]:
    """Package names declared in one dependency table.

    Resolves an explicit rename (`x = { package = "slint-build" }`) and an
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
    workspace_dependencies = load_workspace_dependencies(root)

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
        "cad-host", "slint", "slint-build", "wgpu",
    }
    for dependency in sorted(forbidden_engine_dependencies):
        forbid_dependency("cad-engine", dependency)
    forbid_dependency("cad-app", "cad-engine")

    # Shared UI layer boundary: the platform-independent UI logic (`cad-ui-core`)
    # sits above the UI-free application/host contracts it consumes — it depends on
    # `cad-host` (the owner-thread session wraps `HostCore`) — and below every
    # concrete host/presenter. It must stay toolkit- and GPU-free: no Slint, no
    # wgpu, no Tauri/GTK/portal, and no platform bindings. (`cad-ui-slint` was
    # deleted in P3b, so `slint`/`slint-build` are no longer workspace crates.)
    require_dependency("cad-ui-core", "cad-app")
    require_dependency("cad-ui-core", "cad-host")
    forbidden_ui_core_dependencies = {
        "slint", "slint-build", "wgpu",
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
    # `app-windows` is deliberately NOT required here: it is a NOT IMPLEMENTED
    # stub. Its dependency edge lands with the Windows host work package.
    require_dependency("cad-desktop-shell", "cad-ui-core")
    require_dependency("app-linux", "cad-desktop-shell")
    forbidden_desktop_shell_dependencies = {
        "gtk", "gdk", "gdkx11", "ashpd", "wgpu", "cad-render-wgpu", "cad-engine",
        "slint", "slint-build", "iced", "acadrust",
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
        # Every package is banned from Slint/iced, including build and dev
        # dependency tables (runtime graph alone would miss `slint-build`) and
        # workspace-inherited aliases.
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
    assert "patch" not in tomllib.loads((root / "Cargo.toml").read_text()), "Cargo patch forbidden"
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
