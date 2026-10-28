#!/usr/bin/env python3
"""Static contracts for the Tauri 2 **Android host** (`apps/app-android`).

The host is an asset/WebView carrier: the shared React chrome runs in the Android
WebView and the wasm `dom-canvas` CAD session owns the document (ADR 0024,
`AGENTS.md` iron rule 8). Nothing in this file builds an APK or runs a device;
that is `scripts/build-android.sh` plus a real emulator/device run. What it pins
is the wiring that makes such a run honest:

  * the standard Tauri mobile layout (`src-tauri/`, `[lib]` crate-type, no `main`);
  * `tauri` reachable **only** for an Android target, so host/`wasm32` checks keep
    compiling the documented empty shell instead of a host that cannot exist there;
  * `tauri.conf.json`: identifier, product name, workspace version, the Android UI
    bundle as `frontendDist`, the `beforeBuildCommand` that builds it, the bundle
    Android floor, the wasm-capable CSP;
  * the Android UI bundle writes its **own** directory (`apps/app-android/dist`)
    and never the desktop `apps/desktop-ui/dist`;
  * `app-android/package.json` keeps the pinned CLI and the `tauri` script the
    generated Gradle `rust` plugin invokes (`npm run tauri android
    android-studio-script`);
  * the native side registers **exactly** the two shell-only canvas reports and no
    CAD command, and pulls in no `cad-*` crate (no second document model);
  * the refusal contract: marker text, marker first, exit code `6` for `webgl2`;
  * the edge-to-edge gesture-bar inset: the Activity reads `WindowInsets` and
    publishes `--yacr-safe-bottom` on the page, and `mobile.css` is the consumer of
    that exact property name (the two halves of one contract, in two languages);
  * the generated Gradle project is committed, and the build outputs are ignored.

Every check reports one problem string; `main` prints them and exits non-zero.
Standard library only: this runs in the `core-quality` job, not on a runner with
the Android SDK.
"""
from __future__ import annotations

import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
APP_DIR = ROOT / "apps" / "app-android"
SRC_TAURI = APP_DIR / "src-tauri"
CONF = SRC_TAURI / "tauri.conf.json"
CARGO = SRC_TAURI / "Cargo.toml"
LIB_RS = SRC_TAURI / "src" / "lib.rs"
HOST_RS = SRC_TAURI / "src" / "host.rs"
REFUSAL_RS = SRC_TAURI / "src" / "refusal.rs"
GEN_ANDROID = SRC_TAURI / "gen" / "android"
UI_PACKAGE = ROOT / "apps" / "desktop-ui" / "package.json"
UI_SCRIPT = ROOT / "scripts" / "build-android-ui.sh"

# The two shell-only reports `apps/desktop-ui/src/host/invoke.ts` sends to Tauri on
# this carrier; the native host must register these and nothing else.
SHELL_ONLY_COMMANDS = ("canvas_session_status", "canvas_runtime_failed")
MARKER = "yacr: unsupported-runtime: webgl2"


def read(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8")


def workspace_version() -> str:
    workspace = tomllib.loads(read(ROOT / "Cargo.toml"))
    return workspace["workspace"]["package"]["version"]


def check_workspace_member() -> list[str]:
    problems: list[str] = []
    members = tomllib.loads(read(ROOT / "Cargo.toml"))["workspace"]["members"]
    if "apps/app-android/src-tauri" not in members:
        problems.append("Cargo.toml: workspace member must be 'apps/app-android/src-tauri'")
    if "apps/app-android" in members:
        problems.append("Cargo.toml: the retired 'apps/app-android' member must be gone")
    return problems


def check_manifest() -> list[str]:
    problems: list[str] = []
    if not CARGO.is_file():
        return [f"missing {CARGO.relative_to(ROOT)}"]
    manifest = tomllib.loads(read(CARGO))
    if manifest.get("package", {}).get("name") != "app-android":
        problems.append("src-tauri/Cargo.toml: package.name must be 'app-android'")

    lib = manifest.get("lib", {})
    if lib.get("name") != "app_android":
        problems.append("src-tauri/Cargo.toml: [lib] name must be 'app_android' (WRY_ANDROID_LIBRARY)")
    crate_type = set(lib.get("crate-type", []))
    for required in ("staticlib", "cdylib", "rlib"):
        if required not in crate_type:
            problems.append(f"src-tauri/Cargo.toml: [lib] crate-type must include '{required}'")
    if "bin" in manifest or "[[bin]]" in read(CARGO):
        problems.append("src-tauri/Cargo.toml: an Android host has no binary; drop [[bin]]")

    # `tauri` must be Android-only: a host/wasm check of this member must not try to
    # compile a Tauri app, which is what keeps the workspace gates green.
    target_tables = manifest.get("target", {})
    android_table = target_tables.get('cfg(target_os = "android")', {})
    if "tauri" not in android_table.get("dependencies", {}):
        problems.append("src-tauri/Cargo.toml: `tauri` must live under [target.'cfg(target_os = \"android\")'.dependencies]")
    if "tauri" in manifest.get("dependencies", {}):
        problems.append("src-tauri/Cargo.toml: `tauri` must not be an unconditional dependency")
    if "tauri-build" not in manifest.get("build-dependencies", {}):
        problems.append("src-tauri/Cargo.toml: `tauri-build` must be a build-dependency")

    # Iron rule 8: the native carrier holds no document model.
    for section in ("dependencies",):
        for name in manifest.get(section, {}):
            if name.startswith("cad-"):
                problems.append(
                    f"src-tauri/Cargo.toml: the Android carrier must not depend on '{name}' "
                    "(the wasm session is the only CAD authority)"
                )
    if "custom-protocol" not in manifest.get("features", {}):
        problems.append("src-tauri/Cargo.toml: the `custom-protocol` feature must stay declared (the CLI needs it)")
    return problems


def check_config() -> list[str]:
    problems: list[str] = []
    if not CONF.is_file():
        return [f"missing {CONF.relative_to(ROOT)}"]
    conf = json.loads(read(CONF))
    if conf.get("identifier") != "dev.yacr.app":
        problems.append("tauri.conf.json: identifier must be 'dev.yacr.app'")
    if conf.get("productName") != "Yacr":
        problems.append("tauri.conf.json: productName must be 'Yacr'")
    version = workspace_version()
    if conf.get("version") != version:
        problems.append(f"tauri.conf.json: version must match the workspace ({version})")

    build = conf.get("build", {})
    if build.get("frontendDist") != "../dist":
        problems.append("tauri.conf.json: build.frontendDist must be '../dist' (the Android UI bundle)")
    before = str(build.get("beforeBuildCommand", ""))
    if "scripts/build-android-ui.sh" not in before:
        problems.append("tauri.conf.json: build.beforeBuildCommand must run scripts/build-android-ui.sh")

    windows = conf.get("app", {}).get("windows", [])
    if not windows:
        problems.append(
            "tauri.conf.json: app.windows must declare one window "
            "(tauri::app::setup builds mobile webviews from app.windows; an empty array shows nothing)"
        )

    csp = str(conf.get("app", {}).get("security", {}).get("csp", ""))
    for fragment in ("default-src 'self'", "script-src 'self' 'wasm-unsafe-eval'", "worker-src 'self'",
                     "connect-src 'self' ipc: http://ipc.localhost", "img-src 'self' data: blob:",
                     "style-src 'self' 'unsafe-inline'"):
        if fragment not in csp:
            problems.append(f"tauri.conf.json: CSP must keep '{fragment}' (wasm session + IPC)")

    bundle = conf.get("bundle", {})
    if bundle.get("active") is not True:
        problems.append("tauri.conf.json: bundle.active must be true")
    if bundle.get("android", {}).get("minSdkVersion") != 24:
        problems.append("tauri.conf.json: bundle.android.minSdkVersion must be 24")
    icons = bundle.get("icon") or []
    if not icons:
        problems.append("tauri.conf.json: bundle.icon must name the shared repo icon")
    for icon in icons:
        if not (SRC_TAURI / icon).is_file():
            problems.append(f"tauri.conf.json: icon '{icon}' does not exist under src-tauri/")
    return problems


def check_sources() -> list[str]:
    problems: list[str] = []
    for path in (LIB_RS, HOST_RS, REFUSAL_RS):
        if not path.is_file():
            problems.append(f"missing {path.relative_to(ROOT)}")
    if problems:
        return problems

    lib = read(LIB_RS)
    if 'cfg(target_os = "android")' not in lib:
        problems.append("src/lib.rs: the host module must be gated on cfg(target_os = \"android\")")

    host = read(HOST_RS)
    if "mobile_entry_point" not in host:
        problems.append(
            "src/host.rs: run() must carry #[cfg_attr(mobile, tauri::mobile_entry_point)] "
            "(the generated Kotlin glue calls the `start_app` symbol it emits)"
        )
    registered = re.findall(r"#\[tauri::command\]\s*pub fn (\w+)", host)
    if tuple(registered) != SHELL_ONLY_COMMANDS:
        problems.append(
            "src/host.rs: exactly the two shell-only reports must be #[tauri::command] "
            f"(expected {SHELL_ONLY_COMMANDS}, found {tuple(registered)})"
        )
    handler = re.search(r"generate_handler!\[([^\]]*)\]", host)
    if handler is None:
        problems.append("src/host.rs: the invoke handler must be generated")
    else:
        names = tuple(name.strip() for name in handler.group(1).split(",") if name.strip())
        if names != SHELL_ONLY_COMMANDS:
            problems.append(f"src/host.rs: generate_handler! must register exactly {SHELL_ONLY_COMMANDS} (got {names})")
    if "process::exit" not in host:
        problems.append("src/host.rs: the webgl2 refusal must end the process")

    refusal = read(REFUSAL_RS)
    if f'"{MARKER}"' not in refusal:
        problems.append(f"src/refusal.rs: the stable marker line must stay '{MARKER}'")
    if not re.search(r"WEBGL2_EXIT_CODE:\s*i32\s*=\s*6", refusal):
        problems.append("src/refusal.rs: the webgl2 refusal exit code must be 6")
    if "unsupported-runtime: " not in refusal:
        problems.append("src/refusal.rs: the marker prefix must stay 'yacr: unsupported-runtime: '")
    return problems


def check_ui_bundle_wiring() -> list[str]:
    problems: list[str] = []
    package = json.loads(read(UI_PACKAGE))
    scripts = package.get("scripts", {})
    android = str(scripts.get("build:android", ""))
    if "YACR_RUNTIME=android" not in android:
        problems.append("apps/desktop-ui/package.json: build:android must set YACR_RUNTIME=android")
    if "--outDir ../app-android/dist" not in android:
        problems.append("apps/desktop-ui/package.json: build:android must write ../app-android/dist")
    if "--emptyOutDir" not in android:
        problems.append("apps/desktop-ui/package.json: build:android must pass --emptyOutDir (no stale files)")
    desktop = str(scripts.get("build", ""))
    if "outDir" in desktop:
        problems.append("apps/desktop-ui/package.json: the desktop build must keep writing apps/desktop-ui/dist")

    apk_package = json.loads(read(APP_DIR / "package.json"))
    cli = apk_package.get("devDependencies", {}).get("@tauri-apps/cli")
    if cli is None:
        problems.append("apps/app-android/package.json: @tauri-apps/cli must be a devDependency")
    if "tauri" not in apk_package.get("scripts", {}):
        problems.append("apps/app-android/package.json: the 'tauri' script is required (Gradle calls `npm run tauri …`)")

    script = read(UI_SCRIPT)
    for fragment in ("apps/app-android/dist", "build-canvas-wasm.sh", "build:android", "app_web_bg.wasm"):
        if fragment not in script:
            problems.append(f"scripts/build-android-ui.sh: must keep '{fragment}'")
    return problems


def check_gesture_bar_inset() -> list[str]:
    """The edge-to-edge gesture pill is the host's fact, published as a CSS var.

    `enableEdgeToEdge()` draws the system gesture bar **over** the page, so the page
    must reserve room for it. `env(safe-area-inset-bottom)` inside the WebView does
    not report that height (it is the *display cutout* area), so the Activity reads
    `WindowInsets` and publishes `--yacr-safe-bottom`; `mobile.css` is the only
    consumer. Both halves are pinned here because either one alone silently returns
    the pill to the middle of the command field (measured 2026-10-28).
    """
    problems: list[str] = []
    activity = GEN_ANDROID / "app" / "src" / "main" / "java" / "dev" / "yacr" / "app" / "MainActivity.kt"
    if not activity.is_file():
        return [f"missing {activity.relative_to(ROOT)} (the host must publish the gesture-bar inset)"]
    kotlin = read(activity)
    if "enableEdgeToEdge()" not in kotlin:
        problems.append("MainActivity.kt: the host is edge-to-edge; keep enableEdgeToEdge() and the inset publisher")
    if "Type.navigationBars()" not in kotlin:
        problems.append("MainActivity.kt: the bottom inset must come from WindowInsets.Type.navigationBars()")
    if "'--yacr-safe-bottom'" not in kotlin:
        problems.append("MainActivity.kt: the inset must be published as the CSS property '--yacr-safe-bottom'")
    if "evaluateJavascript" not in kotlin:
        problems.append("MainActivity.kt: the inset must reach the page through the WebView's JavaScript bridge")
    css = read(ROOT / "apps" / "desktop-ui" / "src" / "mobile.css")
    if "var(--yacr-safe-bottom, 0px)" not in css:
        problems.append("apps/desktop-ui/src/mobile.css: .mobile-commandbar must consume var(--yacr-safe-bottom, 0px)")
    return problems


def check_generated_project_and_ignores() -> list[str]:
    problems: list[str] = []
    gradle = GEN_ANDROID / "app" / "build.gradle.kts"
    if not gradle.is_file():
        problems.append("src-tauri/gen/android: the generated Gradle project must be committed")
    else:
        text = read(gradle)
        if "dev.yacr.app" not in text:
            problems.append("gen/android/app/build.gradle.kts: namespace must be dev.yacr.app")
        if not re.search(r"minSdk\s*=\s*24", text):
            problems.append("gen/android/app/build.gradle.kts: minSdk must be 24")

    ignore_rules = read(ROOT / ".gitignore")
    for rule in ("apps/app-android/dist/", "apps/app-android/node_modules/"):
        if rule not in ignore_rules:
            problems.append(f".gitignore: must ignore '{rule}'")
    return problems


def main() -> int:
    problems: list[str] = []
    for check in (
        check_workspace_member,
        check_manifest,
        check_config,
        check_sources,
        check_ui_bundle_wiring,
        check_gesture_bar_inset,
        check_generated_project_and_ignores,
    ):
        try:
            problems.extend(check())
        except Exception as error:  # noqa: BLE001 - a broken file must be reported, not crash
            problems.append(f"{check.__name__}: {error}")
    if problems:
        print("android-host: FAILED", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print("android-host OK: src-tauri layout, conf, Android-only tauri, shell-only commands, refusal contract, "
          "gesture-bar inset publisher")
    return 0


if __name__ == "__main__":
    sys.exit(main())
