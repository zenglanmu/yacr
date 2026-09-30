#!/usr/bin/env python3
"""Validate the workspace DAG and core integration boundaries (Python 3.11+)."""
import pathlib
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
packages = {}
for member in workspace["members"]:
    manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
    packages[manifest["package"]["name"]] = manifest

def dependencies(manifest):
    result = set(manifest.get("dependencies", {}))
    for target in manifest.get("target", {}).values():
        result.update(target.get("dependencies", {}))
    return result

graph = {name: dependencies(manifest) & packages.keys() for name, manifest in packages.items()}
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
    deps = dependencies(manifest)
    if name != "cad-import-acadrust":
        assert "acadrust" not in deps, f"parser escaped importer: {name}"
    if name not in {"cad-ui-slint", "app-android", "app-web"}:
        assert not deps & {"slint", "slint-build", "iced"}, f"UI leaked into {name}"
    if name != "cad-render-wgpu":
        assert "wgpu" not in deps, f"GPU dependency escaped renderer: {name}"
    if name.startswith("cad-") and name not in {"cad-ui-slint", "cad-render-wgpu"}:
        assert not deps & {"web-sys", "android-activity", "jni", "ndk"}, f"platform leaked into {name}"
    if name == "cad-domain":
        assert not graph[name], "domain depends on another CAD package"
    if name == "cad-db":
        assert graph[name] <= {"cad-domain"}, "database depends on a higher layer"
assert "patch" not in tomllib.loads((ROOT / "Cargo.toml").read_text()), "Cargo patch forbidden"
print(f"Architecture OK: {len(packages)} packages, acyclic and core boundaries intact")
