#!/usr/bin/env python3
"""Static contracts for the Tauri 2 **Android host** (`apps/app-android`).

The host is a Tauri 2 WebView carrier with a **native `native-below`** CAD surface:
the shared React chrome runs in the Android WebView and the native Rust
`CadSession` owns the document, presenting through a `SurfaceView` layered beneath
the WebView (ADR 0027; the wasm `dom-canvas` session does not run on Android).
Nothing in this file builds an APK or runs a device;
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
  * the native side is a **native-below** carrier: the Activity owns a `SurfaceView`
    under the transparent WebView and bridges its lifecycle to the native wgpu
    renderer through the frozen JNI symbols. The shared command set is registered by
    the native host (the command registry, not this file, owns its exact membership);
    the two shell-only reports stay present because the chrome still sends them;
  * the SAF document contract: `DocumentPicker.kt` launches `ACTION_OPEN_DOCUMENT` /
    `ACTION_CREATE_DOCUMENT` and does every read/write through `ContentResolver`,
    while `MainActivity` exposes the five `document*` instance-JNI entry points Rust
    calls (plus `onNewIntent` for external "Open with"). `bundle.fileAssociations`
    declares `.dwg` so tauri-build can inject the VIEW intent filters;
  * the refusal contract: marker text, marker first, exit code `6` for `webgl2`;
  * the edge-to-edge gesture-bar inset: the Activity reads `WindowInsets` and
    publishes `--yacr-safe-bottom` on the page, and `mobile.css` is the consumer of
    that exact property name (the two halves of one contract, in two languages);
  * the generated Gradle project is committed, pinned to the documented NDK, packages
    the repository `fonts/` assets, and its build outputs are ignored;
  * the NDK linker floor: `ndk`'s `nativewindow` feature is off while minSdk is 24,
    because it emits `-lnativewindow` and the NDK only ships that stub from API 26.

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

# The shell-only reports `apps/desktop-ui/src/host/invoke.ts` still sends to Tauri on
# this carrier. They stay registered; the full CAD command set is generated from the
# shared registry and its exact membership is owned there, not pinned in this file.
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

    # Iron rule 8 used to forbid every `cad-*` crate here because the wasm session was
    # the only authority. The native-below carrier changes that: the native renderer
    # now owns the document, so `cad-*` dependencies are expected. This file does not
    # pin their names or placement (that is the Rust lane's/registry's contract); it
    # only keeps the Android-only `tauri` scoping above so host/wasm checks still
    # compile the documented empty shell.
    if "custom-protocol" not in manifest.get("features", {}):
        problems.append("src-tauri/Cargo.toml: the `custom-protocol` feature must stay declared (the CLI needs it)")

    # NDK linker floor: `ndk-sys`'s `nativewindow` feature emits an unconditional
    # `-lnativewindow`, and the NDK only ships that stub from API 26 while this
    # host's minSdk is 24. The ANativeWindow entry points the host actually uses
    # (`from_surface`, `ptr`/`width`/`height`, Clone/Drop) are exported by
    # `libandroid.so` at API 24, so the feature stays off unless the floor rises.
    ndk = android_table.get("dependencies", {}).get("ndk")
    ndk_features = ndk.get("features", []) if isinstance(ndk, dict) else []
    min_sdk = 24
    if CONF.is_file():
        min_sdk = (
            json.loads(read(CONF))
            .get("bundle", {})
            .get("android", {})
            .get("minSdkVersion", 24)
        )
    if "nativewindow" in ndk_features and min_sdk < 26:
        problems.append(
            "src-tauri/Cargo.toml: the ndk `nativewindow` feature links -lnativewindow, "
            f"whose NDK stub starts at API 26 (minSdk is {min_sdk}); drop the feature "
            "or raise minSdk to 26"
        )
    return problems


def check_config() -> list[str]:
    problems: list[str] = []
    if not CONF.is_file():
        return [f"missing {CONF.relative_to(ROOT)}"]
    conf = json.loads(read(CONF))
    if conf.get("identifier") != "io.github.zenglanmu.yacr":
        problems.append("tauri.conf.json: identifier must be 'io.github.zenglanmu.yacr'")
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
    for fragment in ("default-src 'self'", "script-src 'self' 'wasm-unsafe-eval' blob:", "worker-src 'self'",
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


def has_native_cad_dependency() -> bool:
    """Whether the Android crate now carries the shared native CAD crates."""
    manifest = tomllib.loads(read(CARGO))
    tables = [manifest.get("dependencies", {})]
    tables.extend(table.get("dependencies", {}) for table in manifest.get("target", {}).values())
    return any(name.startswith("cad-") for deps in tables for name in deps)


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
    native_carrier = has_native_cad_dependency()
    registered = re.findall(r"#\[tauri::command\]\s*pub fn (\w+)", host)
    if not native_carrier:
        # Before the shared native crates land, the wasm-backed carrier must keep its
        # two shell-only reports (the exact old contract). Once the native renderer
        # owns the document, the shared registry owns the command set and these reports
        # are no longer the whole story.
        for command in SHELL_ONLY_COMMANDS:
            if command not in registered:
                problems.append(
                    f"src/host.rs: the shell-only report '{command}' must stay a #[tauri::command]"
                )
    handler = re.search(r"generate_handler!\[([^\]]*)\]", host)
    if handler is None:
        problems.append("src/host.rs: the invoke handler must be generated")
        names = ()
    else:
        names = tuple(name.strip() for name in handler.group(1).split(",") if name.strip())
        if not names:
            problems.append("src/host.rs: generate_handler! must register the carrier's command set")
        if not native_carrier:
            for command in SHELL_ONLY_COMMANDS:
                if command not in names:
                    problems.append(f"src/host.rs: generate_handler! must keep registering '{command}'")
        elif "register_commands" in host:
            # Native carrier: the shared registry handler IS registered, by name
            # through `cad_desktop_shell::commands::register_commands()`, and the
            # local `generate_handler!` deliberately holds only this crate's two
            # shell reports (a `generate_handler!` list cannot splice another
            # closure in). Both reports must still be present in that local list.
            for command in SHELL_ONLY_COMMANDS:
                if command not in names:
                    problems.append(
                        f"src/host.rs: the local handler must keep registering '{command}'"
                    )
        elif set(names).issubset(SHELL_ONLY_COMMANDS):
            problems.append(
                "src/host.rs: the native carrier depends on cad-* but registers only the "
                f"shell reports (got {names}); register the shared command set"
            )
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
    activity = GEN_ANDROID / "app" / "src" / "main" / "java" / "io" / "github" / "zenglanmu" / "yacr" / "MainActivity.kt"
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


def check_native_carrier() -> list[str]:
    """The Kotlin half of the native-below carrier (P5).

    Pixels come from the native wgpu renderer on a `SurfaceView` **behind** the
    transparent WebView. The four `native*` symbols are the frozen Kotlin <-> Rust
    contract; the surface lifecycle and asset extraction are the only native
    responsibilities the Activity owns. Static wiring only: no APK is built and no
    device/emulator is run here.
    """
    problems: list[str] = []
    app = GEN_ANDROID / "app"
    activity = app / "src" / "main" / "java" / "io" / "github" / "zenglanmu" / "yacr" / "MainActivity.kt"
    if not activity.is_file():
        return [f"missing {activity.relative_to(ROOT)}"]
    kotlin = read(activity)

    for signature in (
        "external fun nativeSurfaceCreated(surface: Surface, width: Int, height: Int): Long",
        "external fun nativeSurfaceResized(handle: Long, width: Int, height: Int)",
        "external fun nativeSurfaceDestroyed(handle: Long)",
        "external fun nativeAssetsReady(fontsDir: String, imagesDir: String)",
    ):
        if signature not in kotlin:
            problems.append(f"MainActivity.kt: frozen JNI symbol missing: `{signature}`")

    for fragment in (
        "SurfaceView(this)",
        "setZOrderOnTop(false)",
        "setZOrderMediaOverlay(false)",
        "holder.addCallback(",
        "fun surfaceCreated(holder: SurfaceHolder)",
        "fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int)",
        "fun surfaceDestroyed(holder: SurfaceHolder)",
        "addOnAttachStateChangeListener",
        "setBackgroundColor(Color.TRANSPARENT)",
        "ASSET_EXTRACT_MARKER",
        "copyFlatFontAssets",
        "nativeAssetsReady(fontsDir.absolutePath, imagesDir.absolutePath)",
        "runOnUiThread",
    ):
        if fragment not in kotlin:
            problems.append(f"MainActivity.kt: native carrier wiring missing '{fragment}'")
    # The extraction marker is versioned: bumping it is how an existing install
    # re-extracts the assets after the bundled font package gains a file (v2 adds
    # the Noto Sans CJK SC fallback). The old unversioned marker must not return.
    if '".yacr-assets-v2-complete"' not in kotlin:
        problems.append(
            "MainActivity.kt: ASSET_EXTRACT_MARKER must be the versioned "
            "'.yacr-assets-v2-complete' so existing installs re-extract the new fonts"
        )
    if ".yacr-assets-complete" in kotlin:
        problems.append(
            "MainActivity.kt: the unversioned '.yacr-assets-complete' marker must stay retired"
        )
    if "setZOrderOnTop(true)" in kotlin:
        problems.append("MainActivity.kt: an on-top SurfaceView would cover the React chrome")
    # Wry calls onWebViewCreate before setContentView(webview), so the surface must be
    # inserted after attach, at index 0 of the WebView's parent, not from the hook.
    if not re.search(r"parent\.addView\(\s*surface,\s*0,", kotlin, re.S):
        problems.append("MainActivity.kt: insert the SurfaceView at index 0 of the WebView parent")

    gradle = read(app / "build.gradle.kts")
    if not re.search(r'ndkVersion\s*=\s*"29\.0\.13846066"', gradle):
        problems.append('build.gradle.kts: ndkVersion must be the documented "29.0.13846066"')
    source_set = re.search(r'assets\.srcDir\("([^"]*fonts)"\)', gradle)
    if source_set is None:
        problems.append("build.gradle.kts: the repository fonts/ package must be an assets srcDir")
    else:
        fonts_dir = (app / source_set.group(1)).resolve()
        if not (fonts_dir / "fonts.json").is_file():
            problems.append(
                f"build.gradle.kts: assets srcDir '{source_set.group(1)}' does not resolve to the "
                f"repository fonts/ package (tried {fonts_dir})"
            )

    for theme_name in ("values", "values-night"):
        theme = read(app / "src" / "main" / "res" / theme_name / "themes.xml")
        if "@android:color/transparent" not in theme:
            problems.append(
                f"res/{theme_name}/themes.xml: windowBackground must be transparent over the native surface"
            )
    theme = read(app / "src" / "main" / "res" / "values" / "themes.xml")
    if "unsupported on Android" not in theme:
        problems.append(
            "res/values/themes.xml: document that WebView-level transparency is used because "
            "Tauri's window `transparent` is unsupported on Android"
        )

    manifest = read(app / "src" / "main" / "AndroidManifest.xml")
    config_changes = re.search(r'android:configChanges="([^"]*)"', manifest)
    if config_changes is None:
        problems.append("AndroidManifest.xml: configChanges must stay declared (rotation keeps the surface alive)")
    else:
        declared = set(config_changes.group(1).split("|"))
        for flag in ("orientation", "screenSize", "smallestScreenSize", "uiMode"):
            if flag not in declared:
                problems.append(f"AndroidManifest.xml: configChanges must keep '{flag}'")
    return problems


def check_document_picker_wiring() -> list[str]:
    """The Kotlin half of the SAF document contract (open + save/export).

    The native host drives Kotlin through instance JNI on `MainActivity`:
    `documentLaunchOpen()`, `documentLaunchSave(defaultName)`,
    `documentReadUri(uri)`, `documentWriteUri(uri, bytes)` and
    `documentSnapshot()`. `DocumentPicker.kt` must own the picker/read plumbing so
    the Activity keeps holding no document state (iron rule 8), and the picker must
    be registered through `registerForActivityResult` — never `onActivityResult`.
    What is pinned here is only the wiring: no APK is built and no device is run.

    `bundle.fileAssociations` is pinned as well: tauri-build turns each entry into
    the activity's VIEW intent filters at build time, so the committed
    `AndroidManifest.xml` deliberately has no hand-edited association block.
    """
    problems: list[str] = []
    java_dir = GEN_ANDROID / "app" / "src" / "main" / "java" / "io" / "github" / "zenglanmu" / "yacr"
    activity_path = java_dir / "MainActivity.kt"
    picker_path = java_dir / "DocumentPicker.kt"
    if not activity_path.is_file():
        return [f"missing {activity_path.relative_to(ROOT)}"]
    if not picker_path.is_file():
        return [
            f"missing {picker_path.relative_to(ROOT)} "
            "(the SAF picker/read slot must not be folded into MainActivity)"
        ]
    activity = read(activity_path)
    picker = read(picker_path)

    # Picker plumbing lives in DocumentPicker.kt.
    for fragment in (
        "Intent.ACTION_OPEN_DOCUMENT",
        "Intent.ACTION_CREATE_DOCUMENT",
        "Intent.CATEGORY_OPENABLE",
        "contentResolver.openInputStream(",
        "contentResolver.openOutputStream(",
        '"wt"',
        "takePersistableUriPermission(",
        "FLAG_GRANT_READ_URI_PERMISSION",
        "FLAG_GRANT_WRITE_URI_PERMISSION",
        "FLAG_GRANT_PERSISTABLE_URI_PERMISSION",
        "registerForActivityResult(",
        "ActivityResultContracts.StartActivityForResult()",
        "ActivityResultLauncher<Intent>",
        "Intent.EXTRA_TITLE",
        '"application/octet-stream"',
        '"*/*"',
        "OpenableColumns.DISPLAY_NAME",
        "generation",
    ):
        if fragment not in picker:
            problems.append(f"DocumentPicker.kt: SAF wiring missing '{fragment}'")
    if "Intent.ACTION_GET_CONTENT" in picker:
        problems.append(
            "DocumentPicker.kt: use ACTION_OPEN_DOCUMENT (SAF grant), never ACTION_GET_CONTENT"
        )
    if "override fun onActivityResult" in activity or "override fun onActivityResult" in picker:
        problems.append(
            "MainActivity.kt: picker results must arrive through registerForActivityResult, "
            "not an onActivityResult override"
        )
    # 64 KiB read buffer: the whole document is streamed, never a truncated read.
    if not re.search(r"64 \* 1024", picker):
        problems.append("DocumentPicker.kt: document reads must use a 64 KiB buffer")
    # The state codes are the cross-language transport contract (NONE..FAILED).
    for name, value in (
        ("STATE_NONE", 0),
        ("STATE_LOADING", 1),
        ("STATE_OK", 2),
        ("STATE_CANCELLED", 3),
        ("STATE_FAILED", 4),
    ):
        if not re.search(rf"const val {name} = {value}\b", picker):
            problems.append(f"DocumentPicker.kt: `const val {name} = {value}` must stay declared")
    # Terminal results are consumed exactly once; a poll must not replay them.
    if "slot = EMPTY" not in picker:
        problems.append("DocumentPicker.kt: terminal snapshot results must be consumed (slot = EMPTY)")

    # The five JNI entry points are on the Activity (that is what Rust calls).
    for signature in (
        "fun documentLaunchOpen()",
        "fun documentLaunchSave(defaultName: String)",
        "fun documentReadUri(uri: String)",
        "fun documentWriteUri(uri: String, bytes: ByteArray)",
        "fun documentSnapshot(): Array<Any?>",
    ):
        if signature not in activity:
            problems.append(f"MainActivity.kt: Rust-facing document entry point missing: `{signature}`")
    for delegation in (
        "documentPicker.documentLaunchOpen()",
        "documentPicker.documentLaunchSave(defaultName)",
        "documentPicker.documentReadUri(uri)",
        "documentPicker.documentWriteUri(uri, bytes)",
        "documentPicker.documentSnapshot()",
    ):
        if delegation not in activity:
            problems.append(f"MainActivity.kt: the document entry points must delegate to DocumentPicker ({delegation})")
    # External "Open with": onCreate and onNewIntent both arm the same read slot.
    if "override fun onNewIntent(intent: Intent)" not in activity:
        problems.append("MainActivity.kt: external 'Open with' must be handled in onNewIntent")
    if "setIntent(intent)" not in activity:
        problems.append("MainActivity.kt: onNewIntent must setIntent(intent) so the carrier sees the document")
    if activity.count("handleIncomingIntent(intent)") < 2:
        problems.append(
            "MainActivity.kt: both onCreate and onNewIntent must arm the incoming-document read"
        )

    # fileAssociations -> tauri-build injects the VIEW filters into the manifest.
    conf = json.loads(read(CONF))
    associations = conf.get("bundle", {}).get("fileAssociations") or []
    if not associations:
        problems.append("tauri.conf.json: bundle.fileAssociations must declare the .dwg association")
    else:
        dwg = [
            association
            for association in associations
            if "dwg" in [str(ext).lstrip(".") for ext in association.get("ext") or []]
        ]
        if not dwg:
            problems.append("tauri.conf.json: bundle.fileAssociations must include ext 'dwg'")
        for association in dwg:
            actions = [str(action).lower() for action in association.get("androidIntentActionFilters") or []]
            if "view" not in actions:
                problems.append(
                    "tauri.conf.json: each .dwg association must filter on ACTION_VIEW "
                    "(`androidIntentActionFilters: [\"view\"]`)"
                )
        mime_types = {association.get("mimeType") for association in dwg}
        for expected in (
            "image/vnd.dwg",
            "application/acad",
            "application/dwg",
            "application/x-dwg",
            "application/autocad_dwg",
            # Providers commonly report .dwg as octet-stream; without this entry the
            # "Open with" filter would never match those files.
            "application/octet-stream",
        ):
            if expected not in mime_types:
                problems.append(
                    f"tauri.conf.json: the .dwg file associations must keep mimeType '{expected}'"
                )

    # Release R8 obfuscation: the five Java methods are called from Rust by exact
    # name/descriptor, so they must be pinned. This is not theoretical: a release
    # build without the keep block silently removed `documentLaunchOpen` from
    # classes.dex (R8 saw no Java caller) and every picker call would fail at
    # runtime with NoSuchMethodError.
    proguard = read(GEN_ANDROID / "app" / "proguard-rules.pro")
    if "-keepclassmembers class io.github.zenglanmu.yacr.MainActivity" not in proguard:
        problems.append(
            "gen/android/app/proguard-rules.pro: the JNI document methods need a "
            "-keepclassmembers class io.github.zenglanmu.yacr.MainActivity block"
        )
    for descriptor in (
        "void documentLaunchOpen();",
        "void documentLaunchSave(java.lang.String);",
        "void documentReadUri(java.lang.String);",
        "void documentWriteUri(java.lang.String, byte[]);",
        "java.lang.Object[] documentSnapshot();",
    ):
        if descriptor not in proguard:
            problems.append(
                f"proguard-rules.pro: JNI method must be kept verbatim: `{descriptor}`"
            )
    return problems


def check_generated_project_and_ignores() -> list[str]:
    problems: list[str] = []
    gradle = GEN_ANDROID / "app" / "build.gradle.kts"
    if not gradle.is_file():
        problems.append("src-tauri/gen/android: the generated Gradle project must be committed")
    else:
        text = read(gradle)
        if "io.github.zenglanmu.yacr" not in text:
            problems.append("gen/android/app/build.gradle.kts: namespace must be io.github.zenglanmu.yacr")
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
        check_native_carrier,
        check_document_picker_wiring,
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
    print("android-host OK: src-tauri layout, conf, Android-only tauri, shell reports, refusal contract, "
          "gesture-bar inset publisher, native SurfaceView/JNI carrier, SAF document picker/read slot, "
          "dwg file associations, pinned NDK + fonts assets")
    return 0


if __name__ == "__main__":
    sys.exit(main())
