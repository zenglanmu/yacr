# yacr — Rust CAD system (in development)

[English](README.md) | [中文](README.zh-CN.md)

> **Early development; there is no shippable product yet.** yacr is a CAD system written from
> scratch in Rust that targets DWG/DXF drawings. The desktop UI is a **React + Tauri native host**
> (Linux first; migrated from Slint on 2026-10-16). The Windows/macOS desktop hosts and the
> Android/Web UIs are **explicit NOT IMPLEMENTED placeholders**, not runnable products. Current
> capabilities are backed by **contract/synthetic tests plus bounded real-GPU host runs on synthetic
> fixtures**; there is **no** validated DWG compatibility, real-drawing, cross-vendor, golden-image
> or performance acceptance. Do not treat it as a usable or production CAD application.
>
> **Scope changes:** user annotations were removed on 2026-10-05 (viewing + measurement only); on
> 2026-10-16 the desktop UI moved from Slint to React/Tauri and `cad-ui-slint` was deleted.

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
apps/app-linux (React + Tauri)   apps/app-{windows,macos,android,web} (NOT IMPLEMENTED placeholders)
```

**Boundaries that must hold** (see `docs/architecture.md`, `docs/core-invariants.md`):

- **renderer-only wgpu** — `cad-render-wgpu` is the only crate that depends on wgpu.
- **importer-only acadrust** — `cad-import-acadrust` is the only crate that depends on acadrust
  (unmodified **0.6.3**); **Cargo patches are forbidden**.
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
- **Capability set** (the shared core / headless-CLI set, covered by contract/synthetic tests — the
  current React/Tauri GUI wires only OPEN, FIT and native navigation). **Fidelity varies**: many
  entities are only approximate or still have gaps; per-item status is in `docs/compatibility.md`
  and `docs/dxf-entity-coverage.md`.
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

## Build from source

Toolchain **1.99.0** (`rust-toolchain.toml`) with a pinned `Cargo.lock`; if `cargo` is not on your
`PATH`, add `$HOME/.cargo/bin`.

### Linux desktop host (primary, the only runnable GUI)

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
(cd apps/app-linux/ui && npm ci && npm run build)
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
flatpak run dev.yacr.app --open /absolute/drawing.dwg
```

This builds a single-file Flatpak bundle (`bin/yacr` + committed `fonts/`, runtime
`org.gnome.Platform//51`). It is **not** a distribution channel: nothing is published to Flathub and
the CI `linux-release` job stays hard-disabled. See `docs/flatpak.md`.

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
(cd apps/app-linux/ui && npm ci && npm test && npm run build)
```

Daily development only verifies the changed scope (that crate's compile/tests/fmt/clippy); the full
gates above are for explicit requests and pre-release acceptance. `NOT RUN` is always stated as
`NOT RUN` — old evidence is never presented as a current result.

### Other platforms

Windows/macOS/Android/Web are **NOT IMPLEMENTED** this round; their build/package/deploy scripts fail
non-zero (retired) and the CI release jobs are hard-disabled. See `docs/build.md` and `docs/ci.md` —
do not treat the retired scripts as working.

## Current progress

**Compiling is not run acceptance.** How far each target has actually run (historical Slint-host
evidence in `docs/validation*.md` does **not** transfer to the React/Tauri host):

| Platform | Host | Actual status |
|---|---|---|
| **Linux desktop (primary, current)** | `apps/app-linux` (`yacr`) | React/Tauri chrome + native GTK/wgpu CAD partition. **Only OPEN and FIT plus native navigation are wired**; every other control is explicitly disabled with a reason. Bounded local acceptance has run: React-migration P6 gate **ACCEPT-WITH-FIXES** (hardware-Vulkan real-window smoke on a synthetic fixture), and native-Wayland W3 **ACCEPT-WITH-LIMITATIONS** (GNOME/Mutter + Quadro P620, `scale=1`) — see `docs/validation-react-linux-migration-2026-10-16.md` and `docs/validation-native-wayland-2026-10-16.md`. **Bounded local-Linux acceptance only** — not a full CAD product / cross-platform / vendor / golden / performance approval. |
| **Windows desktop** | `apps/app-windows` (`yacr-windows`) | **NOT IMPLEMENTED** failure stub (no Tauri dependency; prints a diagnostic and exits non-zero). CI only compiles it (`windows-check`); no runnable GUI. |
| **macOS desktop** | `apps/app-macos` (`yacr-macos`) | **NOT IMPLEMENTED** failure stub. CI only compiles it (`macos-check`); no runnable GUI. |
| **Android** | `apps/app-android` | **NOT IMPLEMENTED** placeholder (retired UI; the native entry aborts with a diagnostic). |
| **Web** | `apps/app-web` | **NOT IMPLEMENTED** placeholder (retired UI; the wasm entry returns an error). |
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
decisions: `docs/adr/` (ADR 0010 = React host replaces Slint, ADR 0011 = native Wayland subsurface,
ADR 0012 = scene spatial index and interaction); headless rendering: `docs/headless-render.md`; CI
layers: `docs/ci.md`; CLI: `docs/cli.md`; fonts: `docs/fonts.md`; app icon: `docs/app-icon.md`.
Retired (historical, kept with notices): `docs/linux-app.md`, `docs/windows-app.md`,
`docs/macos-app.md`, `docs/verify-ui.md`.

**Capabilities & topics** — entity coverage: `docs/dxf-entity-coverage.md`; proxy support:
`docs/proxy-support.md`; curve geometry: `docs/curve-geometry.md`; ACIS: `docs/kernel-acis.md`; 3D
observation: `docs/view-3d.md`; draw order: `docs/render-order.md`; render backends:
`docs/render-backends.md`; measurement: `docs/measure.md`; layouts: `docs/layouts.md`; plot:
`docs/plot.md`; dynamic blocks: `docs/dynamic-blocks.md`; annotative scaling:
`docs/annotative-scaling.md`; command line & keymap: `docs/shortcuts.md`; drawing/editing subset:
`docs/drawing-edit.md`; performance and budgets: `docs/performance.md`; DWG test flow:
`docs/testing-dwg.md`.

## License and third parties

Released under **AGPL-3.0** (`LICENSE`). Sources and licenses of dependencies and bundled resources
(acadrust, wgpu/Tauri/GTK, QCAD samples, osifont, mlightcad fonts, etc.) are documented in
`THIRD_PARTY_NOTICES.md` and `fonts/SOURCE.md`.
