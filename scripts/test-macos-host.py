#!/usr/bin/env python3
"""Static wiring contracts for the native **macOS host** (`apps/app-macos`).

The host is a real Tauri 2 desktop app: a transparent WKWebView (the shared
React chrome) composited **above** an in-process child `NSView` that a wgpu
**Metal** presenter draws into (ADR 0023 composition direction, same model as
`app-windows`). Nothing here builds, links, runs or screenshots it: the only
available acceptance is `cargo check -p app-macos --target
aarch64-apple-darwin --all-targets` and the GitHub `macos-check` compile gate.
Runtime, Metal-GPU, DPI, z-order and packaging behaviour is **NOT VERIFIED**.

What this file pins is the wiring that keeps such a compile gate honest:

  (a) `tauri` is reachable **only** for a macOS target, so host/wasm check gates
      keep compiling the documented empty shell instead of a host that cannot
      exist there;
  (b) the binary stays `yacr-macos` (referenced by the `cad-platform`
      font/image fixtures, and distinct from the Linux `yacr` artifact path);
  (c) no `cad-*` crate is listed for a non-macOS target, and the
      target-independent `[dependencies]` table stays the UI-framework-free
      `cad-ui-core` only (the macOS seam lives in the macOS table);
  (d) `build.rs` gates `tauri_build::build()` on the **target** os, so a Linux
      host check and the wasm `--lib` gate never generate a Tauri context;
  (e) `tauri.conf.json` points `frontendDist` at the one shared React bundle
      (`../desktop-ui/dist`) and keeps the wasm-capable CSP, with `windows: []`
      (the window is built in code) and `bundle.active: false` (packaging stays
      retired);
  (f) `.github/workflows/build.yml`'s `macos-check` builds the frontend
      **before** `cargo check -p app-macos`, because `tauri::generate_context!`
      embeds `frontendDist` at compile time — exactly like `linux-app`.

Every rule is a pure function over file text so the built-in self-tests can
mutate a copy and prove the check really fails (a guard that cannot fail is not
a guard). Standard library only; this runs in the `core-quality` job on Linux,
where no macOS SDK exists.
"""
from __future__ import annotations

import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
APP_DIR = ROOT / "apps" / "app-macos"
MANIFEST = APP_DIR / "Cargo.toml"
BUILD_RS = APP_DIR / "build.rs"
CONF = APP_DIR / "tauri.conf.json"
ICON = APP_DIR / "icons" / "icon.png"
WORKFLOW = ROOT / ".github" / "workflows" / "build.yml"

BIN_NAME = "yacr-macos"
MACOS_TARGET = 'cfg(target_os = "macos")'
FRONTEND_DIST = "../desktop-ui/dist"

# The macOS-only dependency set `apps/app-macos/Cargo.toml` must keep: the Tauri
# runtime, the renderer surface (built from raw `AppKitWindowHandle`s), the shared
# host/shell layers, and the Objective-C bindings for the child `NSView` plus the
# in-process `NSOpenPanel`.
MACOS_DEPENDENCIES = frozenset(
    {
        "tauri",
        "cad-render-wgpu",
        "raw-window-handle",
        "cad-desktop-shell",
        "cad-app",
        "cad-engine",
        "cad-host",
        "cad-platform",
        "cad-resources",
        "cad-db",
        "cad-domain",
        "serde_json",
        "objc2",
        "objc2-foundation",
        "objc2-app-kit",
    }
)
# The one target-independent dependency: launch options, wire protocol and i18n
# lookup must be testable on every host, so this table may not grow a platform dep.
PLAIN_DEPENDENCIES = frozenset({"cad-ui-core"})
# Objective-C binding crates are version-pinned to the family that
# `tauri`/`wry`/`tao` already put in `Cargo.lock`; a floating range would pull a
# second binding set into the metadata graph.
OBJC2_CRATES = ("objc2", "objc2-foundation", "objc2-app-kit")


def read(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8")


def workspace_version() -> str:
    workspace = tomllib.loads(read(ROOT / "Cargo.toml"))
    return workspace["workspace"]["package"]["version"]


def dependency_names(table: dict) -> set[str]:
    """Package names in one dependency table (rename- and alias-aware)."""
    names: set[str] = set()
    for key, value in table.items():
        if isinstance(value, dict) and isinstance(value.get("package"), str):
            names.add(value["package"])
        else:
            names.add(key)
    return names


def manifest_problems(text: str) -> list[str]:
    """Checks (a)-(c): target gating, the binary name and the CAD dependency set."""
    problems: list[str] = []
    try:
        manifest = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        return [f"Cargo.toml: invalid TOML: {error}"]

    if manifest.get("package", {}).get("name") != "app-macos":
        problems.append("Cargo.toml: package.name must be 'app-macos'")

    # (b) the binary name is a cross-crate contract (cad-platform fixtures) and a
    # per-target artifact-path guard (the Linux host is `yacr`).
    bins = manifest.get("bin") or []
    names = [entry.get("name") for entry in bins if isinstance(entry, dict)]
    if BIN_NAME not in names:
        problems.append(f"Cargo.toml: [[bin]] name = \"{BIN_NAME}\" is required (got {names})")
    for entry in bins:
        if isinstance(entry, dict) and entry.get("name") == BIN_NAME:
            if entry.get("path") != "src/main.rs":
                problems.append(
                    f"Cargo.toml: the '{BIN_NAME}' binary must keep path = \"src/main.rs\""
                )

    # (a) `tauri` may only be a macOS-target dependency; anywhere else it would
    # make the non-macOS (host/wasm) checks compile a Tauri app.
    macos_table = manifest.get("target", {}).get(MACOS_TARGET, {}).get("dependencies", {})
    if "tauri" not in macos_table:
        problems.append(
            f"Cargo.toml: `tauri` must live under [target.'{MACOS_TARGET}'.dependencies]"
        )
    if "tauri" in manifest.get("dependencies", {}):
        problems.append("Cargo.toml: `tauri` must not be an unconditional dependency")
    if "tauri" in manifest.get("build-dependencies", {}):
        problems.append("Cargo.toml: `tauri` must not be a build-dependency (only `tauri-build`)")
    for key in manifest.get("target", {}):
        if key == MACOS_TARGET:
            continue
        table = manifest["target"][key]
        # (c) a non-macOS target table must not pull a CAD crate or Tauri.
        for name in sorted(dependency_names(table.get("dependencies", {}))):
            if name.startswith("cad-") or name == "tauri":
                problems.append(
                    f"Cargo.toml: non-macOS target '{key}' must not depend on '{name}'"
                )
        # (a) ... and `tauri` may not appear there either.
        if "tauri" in table.get("dependencies", {}):
            problems.append(f"Cargo.toml: `tauri` must stay macOS-only (found under '{key}')")

    if "tauri-build" not in manifest.get("build-dependencies", {}):
        problems.append("Cargo.toml: `tauri-build` must be a build-dependency")

    # (c) the target-independent table stays UI-framework-free.
    plain = dependency_names(manifest.get("dependencies", {}))
    if plain != PLAIN_DEPENDENCIES:
        problems.append(
            "Cargo.toml: [dependencies] must be exactly "
            f"{sorted(PLAIN_DEPENDENCIES)} (got {sorted(plain)})"
        )

    # The macOS table must keep the contract set: a dropped layer silently turns
    # a compile error into a missing seam at runtime.
    macos_names = dependency_names(macos_table)
    for missing in sorted(MACOS_DEPENDENCIES - macos_names):
        problems.append(
            f"Cargo.toml: the macOS target table is missing the required dependency '{missing}'"
        )
    for extra in sorted(name for name in macos_names if name.startswith("cad-")):
        if extra not in MACOS_DEPENDENCIES:
            problems.append(
                f"Cargo.toml: unexpected CAD dependency '{extra}' in the macOS target table"
            )
    for crate in OBJC2_CRATES:
        entry = macos_table.get(crate)
        if not isinstance(entry, dict) or not isinstance(entry.get("version"), str):
            problems.append(
                f"Cargo.toml: '{crate}' must be pinned to an exact version string "
                "(matching the objc2 family already in Cargo.lock)"
            )
    return problems


def build_rs_problems(text: str) -> list[str]:
    """Check (d): the Tauri context is generated for a macOS target only."""
    problems: list[str] = []
    call = "tauri_build::build()"
    calls = text.count(call)
    if calls == 0:
        return ["build.rs: must call tauri_build::build() for a macOS target"]
    if calls > 1:
        problems.append("build.rs: tauri_build::build() must be called exactly once")
    gate = text.find("CARGO_CFG_TARGET_OS")
    if gate == -1:
        problems.append("build.rs: the call must be gated on CARGO_CFG_TARGET_OS")
    elif gate > text.find(call):
        problems.append("build.rs: tauri_build::build() must sit inside the target-os gate")
    if '"macos"' not in text:
        problems.append('build.rs: the gate must compare the target os with "macos"')
    if re.search(r"^\s*tauri_build::build\(\)", text, re.MULTILINE) and gate == -1:
        problems.append("build.rs: an unconditional tauri_build::build() breaks non-macOS gates")
    return problems


def conf_problems(text: str, version: str) -> list[str]:
    """Check (e): the shared React bundle, the wasm CSP, no window, no packaging."""
    problems: list[str] = []
    try:
        conf = json.loads(text)
    except json.JSONDecodeError as error:
        return [f"tauri.conf.json: invalid JSON: {error}"]

    if conf.get("identifier") != "dev.yacr.app":
        problems.append("tauri.conf.json: identifier must be 'dev.yacr.app'")
    if conf.get("productName") != "Yacr":
        problems.append("tauri.conf.json: productName must be 'Yacr'")
    if conf.get("version") != version:
        problems.append(f"tauri.conf.json: version must match the workspace ({version})")

    build = conf.get("build", {})
    if build.get("frontendDist") != FRONTEND_DIST:
        problems.append(
            f"tauri.conf.json: build.frontendDist must be '{FRONTEND_DIST}' "
            "(the one shared React bundle, not a per-host copy)"
        )
    if not str(build.get("devUrl", "")).startswith("http://localhost"):
        problems.append("tauri.conf.json: build.devUrl must name the local Vite dev server")

    app = conf.get("app", {})
    if app.get("windows") != []:
        problems.append(
            "tauri.conf.json: app.windows must stay empty — the host builds the window "
            "and the child webview in code (src/host/init.rs)"
        )
    csp = str(app.get("security", {}).get("csp", ""))
    for fragment in (
        "default-src 'self'",
        "script-src 'self' 'wasm-unsafe-eval'",
        "style-src 'self' 'unsafe-inline'",
        "connect-src 'self' ipc: http://ipc.localhost",
    ):
        if fragment not in csp:
            problems.append(
                f"tauri.conf.json: CSP must keep '{fragment}' (chrome + IPC; "
                "`wasm-unsafe-eval` is required by the wasm CAD session bundles)"
            )

    bundle = conf.get("bundle", {})
    if bundle.get("active") is not False:
        problems.append(
            "tauri.conf.json: bundle.active must stay false — macOS packaging is "
            "NOT IMPLEMENTED (scripts/package-macos-release.sh fails non-zero)"
        )
    icons = bundle.get("icon") or []
    if "icons/icon.png" not in icons:
        problems.append("tauri.conf.json: bundle.icon must name 'icons/icon.png'")
    return problems


def job_block(text: str, key: str) -> str | None:
    """Return one top-level job's text (2-space key) up to the next job."""
    match = re.search(rf"^  {re.escape(key)}\s*:\s*$", text, re.MULTILINE)
    if match is None:
        return None
    rest = text[match.end():]
    if rest.startswith("\n"):
        rest = rest[1:]
    end = len(rest)
    for following in re.finditer(r"^  [A-Za-z0-9_-]+\s*:", rest, re.MULTILINE):
        end = following.start()
        break
    return rest[:end]


def workflow_problems(text: str) -> list[str]:
    """Check (f): `macos-check` builds the embedded frontend before the compile."""
    block = job_block(text, "macos-check")
    if block is None:
        return ["build.yml: the required 'macos-check' job is missing"]
    problems: list[str] = []

    compile_at = block.find("cargo check -p app-macos --all-targets --locked")
    if compile_at == -1:
        return ["build.yml: macos-check must run `cargo check -p app-macos --all-targets --locked`"]

    # `tauri::generate_context!` embeds frontendDist, so the bundle must exist
    # before the Rust host is type-checked; a compile-first order is a hard failure
    # on a clean runner (missing-dist panic), not a silent pass.
    for fragment, label in (
        ("actions/setup-node@v4", "Node setup"),
        ("apps/desktop-ui", "the apps/desktop-ui working directory"),
        ("npm ci", "a reproducible install"),
        ("npm test", "the frontend test run"),
        ("npm run build", "the Vite bundle build"),
    ):
        index = block.find(fragment)
        if index == -1:
            problems.append(f"build.yml: macos-check is missing {label} ('{fragment}')")
        elif index > compile_at:
            problems.append(
                f"build.yml: macos-check must build the frontend ('{fragment}') "
                "before `cargo check -p app-macos` (generate_context! embeds it)"
            )
    # The compile gate must stay a real check, never an empty success or a run.
    for forbidden, label in (
        ("--release", "a release build"),
        ("cargo test", "executed tests (this job is compile-only)"),
        ("cargo run", "a host run"),
    ):
        if forbidden in block:
            problems.append(f"build.yml: macos-check must not perform {label} ('{forbidden}')")
    if "macos-check.log" not in block:
        problems.append("build.yml: macos-check must tee its log to 'macos-check.log'")
    return problems


def real_file_problems() -> list[str]:
    problems: list[str] = []
    for path in (MANIFEST, BUILD_RS, CONF):
        if not path.is_file():
            problems.append(f"missing {path.relative_to(ROOT)}")
    if problems:
        return problems
    if not ICON.is_file():
        problems.append(f"missing {ICON.relative_to(ROOT)} (bundle.icon source)")

    problems.extend(manifest_problems(read(MANIFEST)))
    problems.extend(build_rs_problems(read(BUILD_RS)))
    problems.extend(conf_problems(read(CONF), workspace_version()))
    if WORKFLOW.is_file():
        problems.extend(workflow_problems(read(WORKFLOW)))
    else:
        problems.append(f"missing {WORKFLOW.relative_to(ROOT)}")
    return problems


def self_test_problems() -> list[str]:
    """Mutate each contract in turn; every mutation must be reported."""
    problems: list[str] = []
    manifest = read(MANIFEST)
    build_rs = read(BUILD_RS)
    conf = read(CONF)
    workflow = read(WORKFLOW)
    version = workspace_version()

    mutations: tuple[tuple[str, list[str]], ...] = (
        (
            "tauri made an unconditional dependency",
            manifest_problems(
                manifest.replace("[dependencies]\ncad-ui-core", "[dependencies]\ntauri.workspace = true\ncad-ui-core")
            ),
        ),
        (
            "tauri moved into a non-macOS target table",
            manifest_problems(
                manifest.replace(
                    "[target.'cfg(target_os = \"macos\")'.dependencies]",
                    "[target.'cfg(target_os = \"linux\")'.dependencies]\ntauri.workspace = true\n\n"
                    "[target.'cfg(target_os = \"macos\")'.dependencies]",
                )
            ),
        ),
        (
            "a CAD crate added to a non-macOS target table",
            manifest_problems(
                manifest.replace(
                    "[dependencies]\ncad-ui-core",
                    "[target.'cfg(target_os = \"windows\")'.dependencies]\ncad-app.workspace = true\n\n"
                    "[dependencies]\ncad-ui-core",
                )
            ),
        ),
        (
            "binary renamed",
            manifest_problems(manifest.replace(f'name = "{BIN_NAME}"', 'name = "yacr"')),
        ),
        (
            "build.rs gate removed",
            build_rs_problems(
                build_rs.replace(
                    'if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {',
                    "if true {",
                )
            ),
        ),
        (
            "build.rs gated on the wrong os",
            build_rs_problems(build_rs.replace('== Ok("macos")', '== Ok("linux")')),
        ),
        (
            "frontendDist pointed at a per-host copy",
            conf_problems(conf.replace(FRONTEND_DIST, "../dist"), version),
        ),
        (
            "wasm CSP dropped",
            conf_problems(conf.replace(" 'wasm-unsafe-eval'", ""), version),
        ),
        (
            "packaging turned back on",
            conf_problems(conf.replace('"active": false', '"active": true'), version),
        ),
        (
            "frontend build removed from macos-check",
            workflow_problems(
                re.sub(r"      - name: Test and build the React frontend[\s\S]*?npm run build\n", "", workflow)
            ),
        ),
        (
            "frontend built after the compile",
            workflow_problems(
                workflow.replace(
                    "      - name: Test and build the React frontend (apps/desktop-ui)",
                    "      - name: macOS host debug compilation (arm64)\n        shell: bash\n"
                    "        run: cargo check -p app-macos --all-targets --locked 2>&1 | tee macos-check.log\n"
                    "      - name: Test and build the React frontend (apps/desktop-ui)",
                )
            ),
        ),
    )
    for label, found in mutations:
        if not found:
            problems.append(f"self-test: mutation not detected ({label})")
    return problems


def main() -> int:
    problems = real_file_problems()
    problems.extend(self_test_problems())
    if problems:
        print("macos-host: FAILED", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(
        "macos-host OK: macOS-only tauri target table, `yacr-macos` binary, no CAD dep "
        "outside the macOS table, target-gated tauri-build, shared frontendDist + wasm CSP, "
        "no packaging, macos-check builds the frontend before the compile (self-tests pass)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
