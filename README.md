# yacr — Cross-platform open-source CAD

[English](README.md) | [中文](README.zh-CN.md)

yacr is a cross-platform open-source CAD project. A shared Rust core handles DWG/DXF documents,
geometry, commands and rendering; React + Tauri provides the UI, and wgpu draws the scene. The same
document and rendering pipeline is intended for Linux (native Wayland or X11/`dom-canvas`), Windows,
macOS, Android and the web.

**Website:** <https://yacr.pages.dev/> · **Web app:** <https://yacr-examples.pages.dev/>

yacr is under active development, and **preview builds are available now** — Linux Flatpak, Windows
zip, macOS tarball, Android APK, and a browser build. It opens DWG/DXF, views 2D/3D drawings with
wgpu, measures, and has an implementation path for 223 of 981 commands (50 complete, 173 partial);
unsupported options are rejected by name instead of silently succeeding. Linux/Wayland is the primary
development target with the strongest runtime evidence (bounded local acceptance on GNOME/Mutter).
The Android APK has been sideloaded and run by the maintainer; the Windows and macOS hosts are real
Tauri hosts whose runtime/visual acceptance is still pending; the browser build runs the wasm
session and is exercised with headless/software rendering. It is **not yet a production CAD
system**: DWG compatibility, real-drawing, cross-vendor, golden-image and performance acceptance are
not complete, so keep your own copy of anything you open.

**Author / contact:** zenglanmu@126.com · **License:** AGPL-3.0 (`LICENSE`)

## Architecture

Data flow (UI and CLI share it):

```
UI / CLI → Application & permissions → Command / DrawingTransaction → DB transaction → ChangeSet
         → query / history / dependency invalidation → Representation → SceneDelta → Renderer → RenderHost
```

Drawing data and business logic operate **only** on the database, transactions and commands;
rendering is a **derived** result of the database. `cad-db` is the single source of truth; the UI and
the renderer are derived from it. The same core is reused by the desktop host and the headless CLI,
so behavior stays testable in isolation. Crate layering (bottom-up; enforced by
`scripts/check-architecture.py`):

```
cad-domain                                   domain model: typed IDs, f64 geometry, units, tolerances
  ↑
cad-db ─ cad-dependencies ─ cad-history      database/transactions, dependency invalidation, undo/recovery
  ↑
cad-geometry ─ cad-kernel-adapter ─ cad-representation   geometry, ACIS/SAT isolation, display representation
  ↑
cad-spatial ─ cad-scene ─ cad-render-wgpu    spatial index, scene cache, wgpu renderer
  ↑
cad-proxy ─ cad-import-acadrust ─ cad-resources   proxy graphics, DWG/DXF import, resource resolution
  ↑
cad-measure ─ cad-query ─ cad-app            measurement, paged queries, application/session/commands
  ↑
cad-engine ─ cad-platform ─ cad-diagnostics ─ cad-cli-tools   render session, platform contracts, diagnostics, CLI
  ↑
cad-host                                     platform-independent host orchestration (no UI toolkit)
  ↑
cad-ui-core                                  wire protocol, logical layout, CAD session, shell, i18n
  ↑
apps/app-linux / apps/app-windows / apps/app-macos (React + Tauri desktop hosts)
apps/app-android (Tauri WebView + native-below CAD session)
apps/app-web (browser carrier, wasm dom-canvas session)
```

**Boundaries that must hold** (see `docs/architecture.md`, `docs/core-invariants.md`):

- **renderer-only wgpu** — `cad-render-wgpu` is the only crate that depends on wgpu.
- **importer-only acadrust** — `cad-import-acadrust` is the only crate that depends on acadrust
  (unmodified **0.6.8**); **Cargo patches are forbidden**.
- `cad-host` depends on **no** UI toolkit; platform I/O lives only in `apps/app-*` behind the
  `cad-platform` contracts (`PlatformFiles` / `ConfigStore` / `SurfaceSource` / `FontLoader` / …,
  event-style, no futures, no `Send`).
- `cad-ui-core` carries the platform-independent UI logic (wire protocol, logical layout, CAD
  session, host admission/error state, i18n catalog) and must stay UI-toolkit- and GPU-neutral.
- `cad-engine` is a UI-agnostic render-session coordinator; `cad-app` does not depend on it, and it
  does not depend on `cad-ui-core` / `cad-host` or any UI toolkit.
- GPU/kernel/UI-toolkit/platform types never enter `cad-domain` / `cad-db`; no whole-frame readback;
  a single presenter per drawable.

**Desktop host composition** (Linux, `apps/app-linux`, binary `yacr`): a **native GTK/wgpu CAD
partition** (Rust + renderer-only wgpu) sits inside a GTK `GtkOverlay`; a Tauri 2 `WebView` renders
the React (Vite) chrome as a transparent sibling. The two are **co-visible**. The host supports
**X11/XWayland** (the CAD area gets its own native X child window) and **native Wayland** (an
app-owned `wl_surface` + `wl_subsurface` via a private C shim, `apps/app-linux/native/`). The backend
is decided explicitly before GTK starts (`apps/app-linux/src/backend.rs`); **there is no silent
fallback**. See ADR 0010 and ADR 0011.

## Highlights

- **One core, many consumers.** A shared `cad-*` core plus the platform-independent `cad-ui-core`
  layer is reused by the desktop host and the headless CLI, which share the same application command
  path and the same database.
- **Database as truth, rendering as a derived result.** Edits go through transactions and a
  `ChangeSet`; dependency invalidation rebuilds only the affected display representation instead of
  re-parsing the drawing.
- **Strict dependency isolation by construction** (see Boundaries above), enforced in CI.
- **Honest capability modeling.** Unsupported or unverified abilities are explicitly modeled
  (`Unsupported` / `Partial` / diagnostics); placeholders and fake data never count as done, and a
  failure is never turned into an empty success.
- **Single i18n source.** All user-visible strings come from `crates/cad-ui-core/i18n/{zh-CN,en}.json`
  (key sets must match); CJK literals are rejected in code.
- **Desktop host with real native rendering.** React chrome over a native GTK/wgpu CAD partition on
  both X11/XWayland and native Wayland, with an explicit, non-fallback backend decision.
- **Capability set** (shared by the core, the desktop GUI and the headless CLI; coverage differs per
  surface and is backed by contract/synthetic tests). **Fidelity varies**: many entities are only
  approximate or still have gaps; per-item status is in `docs/compatibility.md` and
  `docs/dxf-entity-coverage.md`.
  - **Open & parse** local DWG / DXF (ASCII and binary) through one semantic/database conversion,
    with asynchronous import, progress and cancellation.
  - **View**: model space and supported layout switching; layer hide/show, search and restore;
    entity selection, highlight and basic properties.
  - **Navigate**: pan, zoom, fit, reset; **2D and 3D observation** (orbit, standard views,
    orthographic/perspective).
  - **Measure**: distance, polyline length, angle and polygon area, with traceable
    unit/precision/geometry source.
  - **Entity display**: all drawable DXF entity types (LINE/POLYLINE/CIRCLE/ARC/ELLIPSE/SPLINE/
    INSERT/TEXT/MTEXT/HATCH/DIMENSION/LEADER/MULTILEADER/MLINE/TABLE/…). TEXT/MTEXT is shaped into
    line segments with real fonts (TTF/OTF/WOFF and SHX); HATCH supports multi-loop solid fill and
    gradients; DIMENSION synthesizes display geometry for
    linear/aligned/radius/diameter/angular/ordinate/arc-length (some subclasses are approximate).
  - **Raster images**: end-to-end texture display for RASTERIMAGE (relative-path resolution +
    PNG/JPEG decode + GPU texture, per-key dedup, UV orientation, clip polygons).
  - **ACIS solids (subset)**: 3DSOLID / BODY / REGION / SURFACE parsed into a neutral B-rep and
    tessellated only for planes (with holes) / spheres / cylinders / tori / cones; everything else is
    explicitly unsupported.
  - **Proxy entities**: custom vendor entities (e.g. Tianzheng, TSSD) are shown only from public
    **proxy graphics cache records**; no cached geometry means no display.
  - **Plot**: model/layout raster PNG (the CLI also has a pure-CPU SVG/PDF vector path); no vector
    print-style tables (CTB).
  - **Large-drawing protection**: hard scene batch/vertex budgets; exceeding them **fails explicitly**
    instead of OOM.
  - **Fonts**: two sources. The committed `fonts/` package ships `Noto Sans CJK SC`
    (OFL-1.1, the default face) and `osifont` (GPL-3 + font exception); the optional full
    `mlightcad/cad-data` catalogue is downloaded at packaging time by `scripts/fetch-fonts.sh`
    (not committed). Native hosts, the Android APK and both Web bundles stage the assembled package
    (`WITH_FONTS=1` by default; `WITH_FONTS=0`/`FONTS_DIR` are the offline paths), and the Web page
    still lazy-loads only the faces an open drawing needs from its same-origin `fonts/`. Every host
    reports the real shaping-font load: the read-only Font Manager (`FONTS`, backed by `cad_fonts`)
    lists loaded/failed/unresolved faces; there is no font-write command.

## Build from source

Toolchain **1.99.0** (`rust-toolchain.toml`) with a pinned `Cargo.lock`; if `cargo` is not on your
`PATH`, add `$HOME/.cargo/bin`.

### Linux desktop host (primary development target)

System prerequisites (the scripts do not install them):

```bash
sudo apt-get install -y pkgconf libgtk-3-dev libwebkit2gtk-4.1-dev libwayland-dev \
  libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
pkg-config --exists webkit2gtk-4.1 gtk+-3.0 wayland-client   # should succeed
```

Recommended launcher (does not force `GDK_BACKEND`; builds the Vite frontend and runs with
`tauri/custom-protocol`):

```bash
bash scripts/run-react-linux.sh --open /absolute/drawing.dwg
```

Direct run (build the frontend first):

```bash
(cd apps/desktop-ui && npm ci && npm run build)
cargo run --release --locked -p app-linux --bin yacr \
  --features tauri/custom-protocol -- --open /absolute/drawing.dwg
```

Native Wayland (real, not XWayland) and the explicit X11 alternative:

```bash
env -u DISPLAY -u XAUTHORITY GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 ./target/release/yacr --open /absolute/drawing.dwg
# explicit X11/XWayland: GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open ...
```

Host flags: `--open <path>`, `--fonts-dir <path>`, `--images-dir <path>`, `--probe`. Config lives in
`~/.config/yacr/` (migrated from `~/.config/yacr-react/`, **no automatic transfer**). The host needs a
real display; **headless SSH is not supported**. Full options, backend policy and limits are in
`docs/react-linux-app.md`.

### Headless CLI

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

On success stdout contains only a JSON result document; on failure you get a non-zero exit code plus
a structured error. See `docs/cli.md`.

### Local Flatpak bundle (Linux)

```bash
bash scripts/package-linux-release.sh
# → target/release/dist/yacr-<version>-linux-<arch>.flatpak  (+ .sha256)
flatpak install --user target/release/dist/yacr-<version>-linux-<arch>.flatpak
flatpak run io.github.zenglanmu.yacr --open /absolute/drawing.dwg
```

This builds a single-file Flatpak bundle (`bin/yacr` plus a font package, runtime
`org.gnome.Platform//51`). By default (`WITH_FONTS=1`) the payload fonts are assembled on the host by
`scripts/fetch-fonts.sh` — the full `mlightcad/cad-data` catalogue merged with the committed
`fonts/` (`osifont` and `Noto Sans CJK SC`) — so drawings resolve more than the two shipped faces.
`WITH_FONTS=0` stages only the committed package and `FONTS_DIR=<dir>` reuses an already assembled
directory; both need no network during packaging. It is **not** a distribution channel: nothing
is published to Flathub; the CI `linux-release` job is event-gated (`v*` tag or manual dispatch)
and uploads a workflow artifact. See `docs/flatpak.md`.

### Code signing

Release artifacts are signed with repository-held credentials and never claim public trust:

- **Windows / Android / macOS** — repository-held **self-signed** material: Authenticode for the
  Windows `.exe` files (PFX + exported `.cer`), APK signing for Android (keystore), and ad-hoc
  `codesign` for the macOS bundle. These signatures are **not** publicly trusted: SmartScreen and
  Gatekeeper still warn. Windows falls back to a `-unsigned` artifact when its PFX is absent; the
  Android release job **fails closed** without a keystore (it only publishes a signed APK); the
  macOS tarball is always ad-hoc signed but never notarized.
- **Flatpak (Linux)** — optional OSTree/GPG commit signing via the `FLATPAK_GPG_*`
  secrets/variables; a signed bundle ships with a matching `*.flatpak.gpg`.

Real Windows/Android/macOS signing requests are **NOT RUN** until those credentials exist, and no
public CA / notarization is involved. The runbook, required secrets and evidence boundaries are in
`docs/code-signing.md`.

### Full gates (CI-equivalent; run before release)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-workflows.py
python3 scripts/check-i18n.py
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
cargo test --workspace --locked -- --test-threads=1   # GPU/CLI cases run serially
(cd apps/desktop-ui && npm ci && npm test && npm run build)
```

Daily development only verifies the changed scope (that crate's compile/tests/fmt/clippy); the full
gates above are for explicit requests and pre-release acceptance. `NOT RUN` is always stated as
`NOT RUN` — old evidence is never presented as a current result.

### Packaging and running per platform (local)

Every entry point is real and fails loudly when a prerequisite is missing — no partial artifact, no
silent fallback. Frontend builds share the same prerequisites: the `plugins/tch` submodule
(`git submodule update --init --recursive`; a missing `dist/plugins/tch.js` is a hard failure),
Node/npm, and the pinned Rust toolchain. Fonts come from `scripts/fetch-fonts.sh`: `WITH_FONTS=1`
(default) downloads the full mlightcad catalogue, `WITH_FONTS=0` stages the committed package offline,
and `FONTS_DIR=<dir>` reuses a prepared directory. Downloads are cached per file under
`${XDG_CACHE_HOME:-$HOME/.cache}/yacr/fonts` (`FONT_CACHE_DIR` overrides it; an explicit empty value
disables the cache; `FORCE=1` refreshes it), so repeated packaging does not download the same faces
again. **A produced package is not runtime, GPU, device or pixel acceptance.**

| Platform | Command (repo root) | Artifact |
|---|---|---|
| Linux (Flatpak) | `bash scripts/package-linux-release.sh` | `target/release/dist/yacr-<version>-linux-<arch>.flatpak` + `.sha256` |
| Windows (zip) | `bash scripts/package-windows-release.sh` | `target/<target>/release/dist/yacr-<version>-windows-x86_64.zip` + `.sha256` |
| macOS (tar.gz) | `bash scripts/package-macos-release.sh` | `target/macos/<arch>/dist/yacr-<version>-macos-<arch>.tar.gz` + `.sha256` |
| Android (APK) | `bash scripts/build-android.sh --release --target aarch64` | `apps/app-android/src-tauri/gen/android/app/build/outputs/apk/universal/release/*.apk` |
| Web (bundle) | `bash scripts/build-web.sh` / `bash scripts/build-web-ui.sh` | `apps/app-web/dist/` / `apps/app-web/dist-ui/` |

Linux/Wayland is the primary development target; the Flatpak commands, install and run steps are in
[Local Flatpak bundle (Linux)](#local-flatpak-bundle-linux) above.

**Windows zip.** Runs on Windows with plain `cargo` (native MSVC, static CRT), and locally on Linux
through MinGW-w64 or `cargo xwin`:

```bash
bash scripts/package-windows-release.sh                                  # Windows (MSVC)
TARGET=x86_64-pc-windows-gnu bash scripts/package-windows-release.sh     # Linux + MinGW-w64
CARGO_XWIN=1 TARGET=x86_64-pc-windows-msvc bash scripts/package-windows-release.sh  # Linux + xwin
```

The local zip is **UNSIGNED** (`WITH_FONTS=1` fetches the full font catalogue, `0` stages the committed
`fonts/` offline). CI signs the staged executables instead: `SKIP_ZIP=1` → `signtool` →
`bash scripts/finalize-windows-release.sh --signed <stage> <dist>`. Run
`bin/yacr-windows.exe --open <dwg>`; without WebView2 it prints a diagnostic and exits 5.

**macOS tar.gz.** Must run on macOS — Mach-O needs Apple's SDK/linker and cannot be cross-built from
Linux:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin   # universal build (default)
bash scripts/package-macos-release.sh                        # universal
MACOS_ARCH=arm64 bash scripts/package-macos-release.sh       # or x86_64
```

The bundle is **ad-hoc signed** by default (`MACOS_SIGN_IDENTITY`, default `-`) and **not notarized**;
on another Mac unquarantine it (`xattr -dr com.apple.quarantine Yacr.app`) or right-click → Open.
Run with `open ./Yacr.app --args --open /absolute/drawing.dwg`.

**Android APK.** Requires `ANDROID_HOME`, `NDK_HOME` (`ndk/29.0.13846066`) and JDK 17 in `JAVA_HOME`:

```bash
bash scripts/build-android.sh --release --target aarch64   # real devices; release APK is UNSIGNED
bash scripts/build-android.sh --debug   --target x86_64    # local emulator; debug-signed
adb install -r apps/app-android/src-tauri/gen/android/app/build/outputs/apk/universal/debug/*.apk
```

A local release APK must be signed (`apksigner`) before it can be installed; CI is the only place that
signs one.

The script runs `scripts/build-android-ui.sh` first (`YACR_RUNTIME=android`) and then Gradle. That step
also assembles the APK font package into the generated `apps/app-android/font-assets/` (full mlightcad
catalogue merged with the committed faces by default; `WITH_FONTS=0` keeps the committed package,
`FONTS_DIR=<dir>` reuses a prepared one), which Gradle exposes as assets. The wasm `dom-canvas` session
is **not** instantiated in this `native-below` carrier.

**Web bundle.** Two separate, non-overlapping artifacts:

```bash
bash scripts/build-web.sh       # standalone vanilla harness → apps/app-web/dist
bash scripts/build-web-ui.sh    # shared React chrome → apps/app-web/dist-ui
python3 scripts/serve-web.py --directory apps/app-web/dist-ui --port 8090
# open http://127.0.0.1:8090/ ; file:// cannot instantiate wasm
```

Both bundles stage the font package into `fonts/` by default: `WITH_FONTS=1` assembles the full
mlightcad catalogue with `scripts/fetch-fonts.sh` (network), `WITH_FONTS=0` keeps the committed package
(offline) and `FONTS_DIR=<dir>` reuses a prepared one. The page still resolves fonts lazily, after a
document opens, from that same-origin `fonts/`. Wasm builds additionally need the
`wasm32-unknown-unknown` target and the pinned `wasm-bindgen 0.2.129`.

### Other platforms

The Windows and macOS desktop hosts are implemented but **compile-only** (CI compile gates, no runtime
acceptance); the Web carrier has bounded local evidence, while the Android native `native-below`
carrier has **compile-level evidence only** (no device/emulator run). Packaging/release scripts are
real entry points; the CI release jobs are event-gated (`v*` tag or manual dispatch) and upload
workflow artifacts (optionally signed when credentials exist). See `docs/build.md`, `docs/ci.md`,
`docs/windows-app.md`, `docs/macos-app.md` and `docs/android-host.md`; packaging/signing is not
runtime/GPU acceptance.

## Current progress

**Compiling is not run acceptance.** How far each target has actually run (historical evidence in
`docs/validation*.md` does **not** transfer to the current React/Tauri host):

| Platform | Host | Actual status |
|---|---|---|
| **Linux desktop (primary, current)** | `apps/app-linux` (`yacr`) | React/Tauri chrome + native GTK/wgpu CAD partition. The command surface covers open/close, navigation, selection, layers, measurement and a 223/981-command implementation path; per-item gaps are in `docs/compatibility.md`. Bounded local acceptance has run: React-migration P6 gate **ACCEPT-WITH-FIXES** (hardware-Vulkan real-window smoke on a synthetic fixture) and native-Wayland W3 **ACCEPT-WITH-LIMITATIONS** (GNOME/Mutter + Quadro P620, `scale=1`) — see `docs/validation-react-linux-migration-2026-10-16.md`, `docs/validation-native-wayland-2026-10-16.md` and the real screenshot on the website. **Bounded local-Linux acceptance only** — not a full CAD product / cross-platform / vendor / golden / performance approval. |
| **Windows desktop** | `apps/app-windows` (`yacr-windows`) | **Real Tauri/WebView2 host, `native-below`**: transparent WebView over a Win32 child HWND. CI compiles it (`windows-check`) and packages a release zip (`windows-release`, event-gated; self-signed Authenticode or `-unsigned`). Visual/runtime/real-Windows acceptance is **NOT VERIFIED** (Wine only covers the missing-WebView2 refusal contract, exit 5); packaging is not runtime evidence. |
| **macOS desktop** | `apps/app-macos` (`yacr-macos`) | **Real Tauri 2 host, compile-only**: transparent WKWebView above a native CAD child `NSView` presented by wgpu/Metal. CI compiles it (`macos-check`) and packages an ad-hoc signed, non-notarized universal tarball (`macos-release`, event-gated). Window/Metal/z-order/DPI/dialog/pixel behaviour is **NOT VERIFIED**; packaging is not runtime evidence. |
| **Android** | `apps/app-android` | **Native `native-below` carrier**: Tauri WebView runs the shared React chrome, a native Rust `CadSession` owns `cad-db`, and wgpu (Vulkan) presents to a `SurfaceView` below the WebView. SAF open and DXF export (SAVE) are wired; DWG save is explicitly deferred. The maintainer has sideloaded and run the release APK (2026-11-08, not a CI acceptance); device/real-GPU/pixel acceptance is still **NOT VERIFIED**. See `docs/android-host.md`. |
| **Web** | `apps/app-web` | **Browser carrier (`dom-canvas`)**: wasm `CadSession` + DOM canvas; OPEN is a page upload, SAVE/PLOT are browser downloads (DXF/SVG/PDF, no DWG write-back). Both bundles stage the full font package into `fonts/` by default (`WITH_FONTS=0`/`FONTS_DIR` offline); after a document opens, the page still fetches only the needed faces from that same-origin `fonts/`; `cad_fonts`/`FONTS` reports the real load. Only bounded local evidence (headless Chromium, **software rendering**) — no real-GPU, pixel or device acceptance. |
| **Headless CLI** | `cad-cli-tools` | Kept: opens and plots headless through the shared application path. |

Recent work (2026-10-17/18) hardened the renderer and picking path under real use: a scene spatial
index for CPU picking (ADR 0012, S4; per-chunk R-tree in `cad-spatial`, with a measured trade-off
documented honestly), frame-pacing / incremental-commit fixes, per-frame CPU plan and encode-skip
optimizations, GPU frame-time measurement, block-level frustum culling, and device-limit
negotiation. The current local Flatpak bundle and the auto evidence-collection script
(`scripts/collect-evidence.sh`) date from the same period. Per-round evidence is in `docs/handoff.md`
and `docs/validation*.md`.

### Honest boundaries

- **No validated DWG compatibility, entity, or platform.** `fixtures/manifest` holds synthetic
  fixtures plus one open-source QCAD `flange` sample (`Partial`); build evidence is not compatibility.
- **Evidence boundaries**: static gates and contract/synthetic tests, plus bounded executable
  evidence (frontend tests, `--locked` compile/tests, hardware-Vulkan real-window smoke on a synthetic
  fixture, and a bounded native-Wayland run). Real-GPU pixel matrices, real devices, exact visual
  goldens, performance and real-drawing acceptance are **not covered** and must not be generalized.
  Software-rendered results are labelled as such and are never used as real-GPU conclusions.
- **Explicitly unsupported / unimplemented** (no success is faked):
  - Vendor opcode records of proxy entities (public cache records only); no cached geometry → no display.
  - External content: PDF/DWF/DGN underlays, OLE2 embedded objects, CoordinationModel / Navisworks
    NWD are not loaded.
  - Full ACIS geometry kernel (3DSOLID/REGION/BODY/SURFACE are only a subset tessellation);
    point-cloud point data (bounding box only).
  - Draw order (`draw_order`) is not plumbed through (= upload order); WIPEOUT mask fill is not
    rendered; line width is explicitly not drawn.
  - Images: unsupported codecs (TIFF/CCITT/EPS), brightness/contrast/fade, and outside/mask clipping.
  - Plot is raster PNG only (the CLI also has SVG/PDF); no CTB/HPGL; drawings are never written back
    and original entities are never modified.
  - User annotations were removed as a whole; the enhanced mode contains measurement only.
- Per-item status is in `docs/compatibility.md` and `docs/dxf-entity-coverage.md`; **unverified is
  stated as unverified** and never reported as complete.

## Documentation

**Overview** — requirements: `CAD_IMPLEMENTATION_SPEC.md` (v2.0); round-by-round handoff:
`docs/handoff.md`; architecture and invariants: `docs/architecture.md`, `docs/core-invariants.md`;
compatibility matrix: `docs/compatibility.md`; validation evidence: `docs/validation.md`,
`docs/validation-dwg.md`.

**Build & platforms** — per-platform build: `docs/build.md`; Linux desktop host:
`docs/react-linux-app.md`; Linux Flatpak (local bundle, not published): `docs/flatpak.md`;
decisions: `docs/adr/` (ADR 0010 = React/Tauri desktop host, ADR 0011 = native Wayland subsurface,
ADR 0012 = scene spatial index and interaction); headless rendering: `docs/headless-render.md`; CI
layers: `docs/ci.md`; CLI: `docs/cli.md`; fonts: `docs/fonts.md` and
`docs/font-host-loading.md`; app icon: `docs/app-icon.md`.
Retired (historical, kept with notices): `docs/linux-app.md`, `docs/windows-app.md`,
`docs/verify-ui.md`. Current macOS host: `docs/macos-app.md`.

**Capabilities & topics** — entity coverage: `docs/dxf-entity-coverage.md`; proxy support:
`docs/proxy-support.md`; curve geometry: `docs/curve-geometry.md`; ACIS: `docs/kernel-acis.md`; 3D
observation: `docs/view-3d.md`; draw order: `docs/render-order.md`; render backends:
`docs/render-backends.md`; measurement: `docs/measure.md`; layouts: `docs/layouts.md`; plot:
`docs/plot.md`; dynamic blocks: `docs/dynamic-blocks.md`; annotative scaling:
`docs/annotative-scaling.md`; command line & keymap: `docs/shortcuts.md`; drawing/editing subset:
`docs/drawing-edit.md`; performance and budgets: `docs/performance.md`; DWG test flow:
`docs/testing-dwg.md`.

## Credits and references

yacr is an independent implementation. These open-source projects informed it or supply tested
inputs.

- **acadrust** — <https://github.com/hakanaktt/acadrust>: pure-Rust DWG/DXF reading, writing and
  inspection. Pinned at `0.6.8`, used unmodified and only through `cad-import-acadrust`.
- **QCAD** — <https://qcad.org>: mature open-source 2D CAD. Its open-source `flange` example is used
  as an import/render fixture (see `fixtures/dxf/qcad-flange/SOURCE.md` and
  `THIRD_PARTY_NOTICES.md`).
- **OpenCADStudio** — <https://www.opencadstudio.com/>: reference for user-visible feature scope and
  UI interaction. Feature-spec reference only; no source code was taken from it.
- **MiniCAD** — lightweight open-source 2D/3D CAD used as a feature and implementation reference;
  no source code was copied from it.

## License and third parties

Released under **AGPL-3.0** (`LICENSE`). Sources and licenses of dependencies and bundled resources
(acadrust, wgpu/Tauri/GTK, QCAD samples, osifont, Noto Sans CJK SC (OFL-1.1), mlightcad fonts,
etc.) are documented in `THIRD_PARTY_NOTICES.md` and `fonts/SOURCE.md`.
