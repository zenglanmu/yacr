# yacr — Rust CAD System (in development)

[English](README.md) | [中文](README.zh-CN.md)

> **The project is still in an early development stage and has no shippable product yet.**
> yacr is a CAD system written from scratch in Rust, targeting DWG/DXF drawings. The desktop
> UI was migrated from Slint to a **React + Tauri native host** (Linux first, 2026-10-16); the
> Web/Android UIs and the Windows/macOS desktop hosts are currently **explicit NOT IMPLEMENTED
> placeholders**. Current capabilities are backed by **contract tests, synthetic samples and
> some host runs**; there is no real-device, real-GPU or real-drawing visual acceptance, and no
> validated DWG compatibility. Do not treat it as a usable or production CAD application.
>
> **Scope changes**: user annotations were removed on 2026-10-05 (viewing + measurement only);
> on 2026-10-16 the desktop UI moved from Slint to React/Tauri and `cad-ui-slint` was deleted.

## What yacr is

Drawing data and business logic only operate on the database, transactions and commands;
rendering is a derived result of the database. One shared core (`cad-*` crates: domain model,
database/transactions, geometry, semantic representation, scene, wgpu renderer) plus the
platform-independent UI layer (`cad-ui-core`: wire protocol, layout, CAD session, shell state,
i18n) is reused by the host, so behavior stays testable in isolation and usable from a headless
CLI and for plotting.

Foundation: DWG/DXF parsing uses **unmodified acadrust 0.6.3**, drawing uses **wgpu**
(renderer-only: `cad-render-wgpu` is the only crate that depends on wgpu), and the desktop UI
is a **React (Vite) chrome in a Tauri 2 WebView** composited over a **native GTK/wgpu CAD
partition** (`apps/app-linux`, binary `yacr`). The database (`cad-db`) is the single source of
truth; UI and rendering are derived from it. See `docs/architecture.md` for the full boundaries
and [`docs/adr/0010-react-linux-host-replaces-slint.md`](docs/adr/0010-react-linux-host-replaces-slint.md)
for the migration decision.

## What it can do today

The capabilities below describe the **shared core / headless-CLI capability set** (covered by
contract/synthetic tests, and historically run on the now-retired Slint hosts). They are **not**
the current GUI scope: the current **React/Tauri desktop host wires only OPEN and FIT plus native
navigation**, with every other control explicitly disabled, and the Windows/macOS/Android/Web
hosts are **NOT IMPLEMENTED** placeholders. **Fidelity varies** — many entities are only
approximate or still have gaps. Per-item status is in `docs/compatibility.md` and
`docs/dxf-entity-coverage.md`.

- **Open & parse**: local DWG / DXF (ASCII and binary), sharing one semantic/database
  conversion; asynchronous import with progress and cancellation.
- **View**: model space and supported layout switching; layer hide/show, search and restore;
  entity selection, highlight and basic properties.
- **Navigate**: pan, zoom, fit drawing, reset view, with consistent mouse and touch behavior;
  **2D and 3D observation** (orbit, standard views, orthographic/perspective).
- **Measure**: distance, polyline length, angle and polygon area, with traceable
  unit/precision/geometry source.
- **Entity display**: every drawable DXF entity type (LINE/POLYLINE/CIRCLE/ARC/ELLIPSE/SPLINE/
  INSERT/TEXT/MTEXT/HATCH/DIMENSION/LEADER/MULTILEADER/MLINE/TABLE/…). TEXT/MTEXT is shaped
  into line segments with real fonts (TTF/OTF/WOFF and SHX); HATCH supports multi-loop solid
  fill and gradients; DIMENSION synthesizes display geometry for
  linear/aligned/radius/diameter/angular/ordinate/arc-length (some subclasses are approximate).
- **Raster images**: end-to-end texture display for RASTERIMAGE (relative-path resolution +
  PNG/JPEG decoding + GPU texture, per-key dedup, UV orientation correction, clip polygons).
- **ACIS solids (subset)**: 3DSOLID / BODY / REGION / SURFACE are parsed into a neutral B-rep
  and tessellated only for planes (with holes) / spheres / cylinders / tori / cones; everything
  else is explicitly marked unsupported.
- **Fonts**: a bundled/configured font directory is resolved at runtime; missing fonts fall back
  to a default outline face. (A local Flatpak bundle can be built with `scripts/package-linux-release.sh`
  — see `docs/flatpak.md`; it is **not** published to Flathub and the CI release job stays disabled,
  so there is still no distributed package. This describes available resources/configuration, not a
  distributable package.)
- **Plot**: model/layout raster PNG (the CLI also has a pure-CPU SVG/PDF vector path); no
  vector print style tables (CTB).
- **Large-drawing protection**: hard scene batch/vertex budgets; exceeding them **fails
  explicitly** instead of OOM.
- **Proxy entities**: custom entities from vendors such as Tianzheng and TSSD are shown only
  from the public **proxy graphics cache records**; no cached geometry means no display.
- **Cross-platform reuse**: one core + the `cad-ui-core` shared UI layer; the desktop host is
  built for Linux only today, and the Windows/macOS/Android/Web hosts are explicit NOT
  IMPLEMENTED placeholders (see below).

## Platforms and run status

**Compiling is not run acceptance.** How far each target has actually run (evidence in
`docs/validation*.md`, which is historical for the retired Slint host):

| Platform | Host | Actual status |
|---|---|---|
| **Linux desktop (primary, current)** | `apps/app-linux` (`yacr`) | React/Tauri chrome + native GTK/wgpu CAD partition. **Only OPEN and FIT + native navigation are wired**; every other control is explicitly disabled with a reason. P6 final verification **executed (bounded)**; Gate P6 attempt2 **ACCEPT-WITH-FIXES** (doc qualification applied) and the observer found **no material visual defect** in captured frames — see [`docs/validation-react-linux-migration-2026-10-16.md`](docs/validation-react-linux-migration-2026-10-16.md). **Bounded local-Linux migration acceptance only** — not a full CAD product / cross-platform / vendor / golden / performance approval. |
| **Windows desktop** | `apps/app-windows` (`yacr-windows`) | **NOT IMPLEMENTED** failure stub (no Tauri dependency, prints a diagnostic, exits non-zero). CI compiles it (`windows-check`) but there is no runnable GUI product. |
| **macOS desktop** | `apps/app-macos` (`yacr-macos`) | **NOT IMPLEMENTED** failure stub. CI compiles it (`macos-check`); no runnable GUI product. |
| **Android** | `apps/app-android` | **NOT IMPLEMENTED** placeholder (retired UI; the native entry aborts with a diagnostic). |
| **Web** | `apps/app-web` | **NOT IMPLEMENTED** placeholder (retired UI; wasm entry points return an error). |
| **Headless CLI** | `cad-cli-tools` | Kept: opens/plots headless through the shared application path. |

> Historical Slint-host evidence (offscreen/software-Vulkan/emulator/browser runs) is retained in
> `docs/validation*.md` **as history** and does **not** transfer to the React/Tauri host.

## Quick start

Toolchain 1.99.0 (`rust-toolchain.toml`), with `Cargo.lock` pinned; if `cargo` is not on your
`PATH`, add `$HOME/.cargo/bin`.

### Linux desktop (primary host)

```bash
sudo apt-get install -y pkgconf libgtk-3-dev libwebkit2gtk-4.1-dev libwayland-dev \
  libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
# Recommended launcher: does NOT force GDK_BACKEND (Auto prefers native Wayland; explicit
# x11/wayland preserved), builds the Vite frontend, runs with custom-protocol
bash scripts/run-react-linux.sh --open /absolute/drawing.dwg
```

Direct run (build the frontend first with `npm run build`). Native Wayland (real, not XWayland):

```bash
env -u DISPLAY -u XAUTHORITY GDK_BACKEND=wayland WAYLAND_DISPLAY=wayland-0 \
    XDG_RUNTIME_DIR=/run/user/1000 ./target/release/yacr --open /absolute/drawing.dwg
# explicit X11/XWayland alternative: GDK_BACKEND=x11 DISPLAY=:0 ./target/release/yacr --open ...
```

Config lives in `~/.config/yacr/` (migrated from `~/.config/yacr-react/`, **no automatic
transfer**). Backend policy, the native Wayland subsurface and full options/limits are in
`docs/react-linux-app.md` and `docs/adr/0011-native-wayland-subsurface.md`.

### Headless CLI

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

On success stdout contains only a JSON result document; on failure you get a non-zero exit code
plus a structured error. Operations and options are in `docs/cli.md`.

### Other platforms

Windows/macOS/Android/Web are **NOT IMPLEMENTED** this round; their build/package/deploy scripts
fail non-zero (retired) and the CI release jobs are hard-disabled. See `docs/build.md` and
`docs/ci.md` for the honest current state — do not treat the retired scripts as working.

## Current status and boundaries (honest)

- **There is no validated DWG compatibility, entity or platform.** `fixtures/manifest` holds
  synthetic fixtures plus one open-source QCAD `flange` sample (`Partial`); build evidence is
  not compatibility.
- **Evidence boundaries**: static gates and contract/synthetic tests, and the P6 executable evidence
  (frontend tests, `--locked` compile/tests, and a hardware-Vulkan real-window smoke on the synthetic
  fixture — `docs/validation-react-linux-migration-2026-10-16.md`; Gate P6 attempt2 **ACCEPT-WITH-FIXES**,
  bounded local-Linux acceptance). Historical Slint-host runs remain historical. Real-GPU pixel matrices,
  real devices, exact visual golden, performance and real-drawing acceptance are **not covered** and must
  not be generalized.
- **Explicitly unsupported / unimplemented** (no success is faked):
  - Vendor opcode records of proxy entities (public cache records only); no cached geometry
    means no display.
  - External content: PDF/DWF/DGN underlays, OLE2 embedded objects, CoordinationModel /
    Navisworks NWD are not loaded.
  - Full ACIS geometry kernel (3DSOLID/REGION/BODY/SURFACE are only a subset tessellation);
    point-cloud point data (bounding box only).
  - Draw order (`draw_order`) is not plumbed through (= upload order); WIPEOUT mask fill is not
    rendered; line width is explicitly not drawn.
  - Images: unsupported codecs (TIFF/CCITT/EPS), brightness/contrast/fade, and outside/mask
    clipping.
  - Plot is raster PNG only (the CLI also has SVG/PDF); no CTB/HPGL; drawings are never written
    back and original entities are never modified.
  - User annotations were removed as a whole; the enhanced mode contains measurement only.
- Unsupported items are listed per item in `docs/compatibility.md`, and per-entity status is in
  `docs/dxf-entity-coverage.md`; **unverified is stated as unverified** and never reported as
  complete.

## Documentation index

**Overview**

- Authoritative requirements: `CAD_IMPLEMENTATION_SPEC.md` (v2.0)
- Round-by-round handoff entry: `docs/handoff.md`
- Architecture boundaries and invariants: `docs/architecture.md`, `docs/core-invariants.md`
- Compatibility and capability matrix: `docs/compatibility.md`
- Validation and run evidence: `docs/validation.md`, `docs/validation-dwg.md`

**Build & platforms**

- Per-platform build: `docs/build.md`
- Linux desktop host (current): `docs/react-linux-app.md`
- Linux Flatpak packaging (local bundle, not published to Flathub): `docs/flatpak.md`,
  `docs/validation-flatpak-linux-2026-10-17.md`
- Authored/technical decisions: `docs/adr/` (ADR 0010 = React host replaces Slint)
- Headless rendering: `docs/headless-render.md`; CI layers: `docs/ci.md`
- CLI: `docs/cli.md`; fonts: `docs/fonts.md`; app icon: `docs/app-icon.md`
- Retired (historical, kept with notices): `docs/linux-app.md`, `docs/windows-app.md`,
  `docs/macos-app.md`, `docs/verify-ui.md`

**Capabilities & topics**

- Entity coverage: `docs/dxf-entity-coverage.md`; proxy support: `docs/proxy-support.md`
- Curve geometry: `docs/curve-geometry.md`; ACIS: `docs/kernel-acis.md`
- 3D observation: `docs/view-3d.md`; draw order: `docs/render-order.md`; render backends: `docs/render-backends.md`
- Measurement: `docs/measure.md`; layouts: `docs/layouts.md`; plot: `docs/plot.md`
- Dynamic blocks: `docs/dynamic-blocks.md`; annotative scaling: `docs/annotative-scaling.md`
- Command line & keymap reference: `docs/shortcuts.md`; drawing/editing subset: `docs/drawing-edit.md`
- Performance and budgets: `docs/performance.md`
- DWG test flow: `docs/testing-dwg.md`; headless UI debugging: `docs/verify-ui.md`

**Reference & decisions**

- OpenCADStudio historical functional/interaction reference (before the 2026-10-15 close-out; no code was ever copied): survey kept in `docs/migration-map.md`; UI design now follows AutoCAD desktop conventions (`docs/react-linux-app.md`)
- Decision records: `docs/adr/` — the desktop UI decision is **ADR 0010 (React host replaces Slint)**

## License and third parties

This repository is released under **AGPL-3.0** (`LICENSE`). Sources and licenses of
dependencies and bundled resources (acadrust, wgpu/Tauri/GTK, QCAD samples, osifont, mlightcad
fonts, etc.) are documented in `THIRD_PARTY_NOTICES.md` and `fonts/SOURCE.md`.
