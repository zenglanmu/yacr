# yacr — Rust CAD System (in development)

[English](README.md) | [中文](README.zh-CN.md)

> **The project is still in an early development stage and has no shippable product yet.**
> yacr is a CAD system written from scratch in Rust, targeting DWG/DXF drawings on desktop,
> Android and the browser. Current capabilities are backed by **contract tests, synthetic
> samples and actual runs on some hosts**; there is no real-device, real-GPU or real-drawing
> visual acceptance, and no validated DWG compatibility. Do not treat it as a usable or
> production CAD application.
>
> **Scope change**: the user annotation feature was removed as a whole on 2026-10-05, and the
> project now focuses on **viewing + measurement** (measurement F06 and DXF annotative scaling
> are retained).

## What yacr is

Drawing data and business logic only operate on the database, transactions and commands;
rendering is a derived result of the database. One shared core (`cad-*` crates: domain model,
database/transactions, geometry, semantic representation, scene, wgpu renderer, Slint UI) is
reused by every platform host, so behavior stays consistent across platforms, can be tested in
isolation, and is usable from a headless CLI and for plotting.

Foundation: DWG/DXF parsing uses **unmodified acadrust 0.6.3**, drawing uses **wgpu**, the UI
uses **Slint**, and the core has no platform dependencies (it compiles to `wasm32`). The
database (`cad-db`) is the single source of truth; UI and rendering are derived from it. See
`docs/architecture.md` for the full boundaries.

## What it can do today

The capabilities below exist in code and are covered by contract/synthetic tests, with some
hosts already run for real; **fidelity varies** — many entities are only approximate or still
have gaps. Per-item status is in `docs/compatibility.md` and `docs/dxf-entity-coverage.md`.

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
- **Fonts**: release packages ship a font directory; missing fonts fall back to a default
  outline face.
- **Plot**: model/layout raster PNG (the CLI also has a pure-CPU SVG/PDF vector path); no
  vector print style tables (CTB).
- **Large-drawing protection**: hard scene batch/vertex budgets; exceeding them **fails
  explicitly** instead of OOM.
- **Proxy entities**: custom entities from vendors such as Tianzheng and TSSD are shown only
  from the public **proxy graphics cache records**; no cached geometry means no display.
- **Cross-platform reuse**: one core + Slint UI, built for Linux/Windows/macOS desktop,
  Android and Web.

## Platforms and run status

**Compiling is not run acceptance.** How far each target has actually run (evidence in
`docs/validation*.md`):

| Platform | Host | Actual status |
|---|---|---|
| **Linux desktop (primary)** | `apps/app-linux` (`yacr-linux`) | Default gate is debug compile + static checks; release GUI packaging and **offscreen rendering (software Vulkan / lavapipe)** have been run. Real window system and real-GPU pixel acceptance **NOT RUN**. |
| **Windows desktop** | `apps/app-windows` (`yacr.exe`) | Reuses the shared desktop implementation; CI compiles natively with MSVC on `windows-latest` and packages a font-bundled zip; reproduced locally via GNU cross / MSVC plus a Wine argument-parsing smoke. Real Windows window/file-dialog/GPU **NOT RUN**. |
| **macOS desktop** | `apps/app-macos` (`yacr-macos`) | Reuses the shared desktop implementation; CI builds on `macos-latest` and packages a universal (arm64+x86_64) `Yacr.app`. A Linux host cannot produce Mach-O, so real Mac / Metal GPU **NOT RUN**. |
| **Android** | `apps/app-android` | The x86_64 release APK was installed, launched and rendered on a headless **emulator** (KVM + SwiftShader); canvas pan/fit verified by pixel diff. **Real device** and SAF file picking **NOT RUN**. |
| **Web** | `apps/app-web` (`web-dist/`) | The wasm artifact runs in headless Chromium on WebGL2 and was re-verified on a real desktop browser locally (Playwright, real GPU); CI `web-deploy` has actually published to Cloudflare Pages (example <https://yacr-examples.pages.dev>). Real WebGPU hardware adapters and the browser matrix **NOT RUN**. |

## Quick start

Toolchain 1.99.0 (`rust-toolchain.toml`), with `Cargo.lock` pinned; if `cargo` is not on your
`PATH`, add `$HOME/.cargo/bin`.

### Linux desktop (primary host)

```bash
sudo apt-get install -y pkgconf libfontconfig-dev libfreetype-dev mesa-vulkan-drivers
cargo build -p app-linux --bin yacr-linux --release --locked
./target/release/yacr-linux                                  # launch (needs a desktop session)
./target/release/yacr-linux --open /absolute/drawing.dwg      # open a drawing
./target/release/yacr-linux --open /absolute/drawing.dxf --locale en
bash scripts/check-linux-app.sh                              # windowless offscreen run check (lavapipe)
```

`--headless --output <new-dir>` plots offscreen through the same host/controller/Slint bridge;
`--gpu auto|high|low` selects an adapter preference (dual-GPU defaults to the discrete GPU).
Full options and limits are in `docs/linux-app.md`.

### Headless CLI

```bash
cargo build -p cad-cli-tools --release --locked
./target/release/cad-cli-tools render /absolute/drawing.dwg --png /tmp/opencode/out.png
./target/release/cad-cli-tools --help
```

On success stdout contains only a JSON result document; on failure you get a non-zero exit code
plus a structured error. Operations and options are in `docs/cli.md`.

### Other platforms

- Android (`cargo-apk`, not a Gradle project): `docs/build.md`, `docs/validation-android.md`.
- Web (wasm + minimal JS host): `scripts/build-web.sh` produces `web-dist/`, served locally by
  `scripts/serve-web.py`; see `docs/build.md`.
- Windows / macOS packaging (CI `windows-release` / `macos-release`): `docs/windows-app.md`,
  `docs/macos-app.md`.

### Release packages

CI (`.github/workflows/build.yml`), triggered by `workflow_dispatch` or a `v*` tag, produces
font-bundled release packages for Linux (a `dev.yacr.app` Flatpak bundle), Windows, macOS and
Android and uploads them as artifacts (per-platform packaging scripts are in `docs/build.md`).
The outputs are CI artifacts; **no official distribution channel exists yet**.

## Current status and boundaries (honest)

- **There is no validated DWG compatibility, entity or platform.** `fixtures/manifest` holds
  synthetic fixtures plus one open-source QCAD `flange` sample (`Partial`); build evidence is
  not compatibility.
- **Evidence boundaries**: static gates and contract/synthetic tests, software Vulkan
  (lavapipe) offscreen rendering, a headless emulator, and headless/real browsers. Real-GPU
  pixel matrices, real devices and real-drawing visual acceptance are **all NOT RUN** and must
  not be generalized from the above.
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
- Linux host: `docs/linux-app.md`; Windows host: `docs/windows-app.md`; macOS host: `docs/macos-app.md`
- Linux Flatpak packaging: `docs/flatpak.md`
- Headless rendering: `docs/headless-render.md`; CI layers: `docs/ci.md`
- CLI: `docs/cli.md`; fonts: `docs/fonts.md`; app icon: `docs/app-icon.md`

**Capabilities & topics**

- Entity coverage: `docs/dxf-entity-coverage.md`; proxy support: `docs/proxy-support.md`
- Curve geometry: `docs/curve-geometry.md`; ACIS: `docs/kernel-acis.md`
- 3D observation: `docs/view-3d.md`; draw order: `docs/render-order.md`; render backends: `docs/render-backends.md`
- Measurement: `docs/measure.md`; layouts: `docs/layouts.md`; plot: `docs/plot.md`
- Dynamic blocks: `docs/dynamic-blocks.md`; annotative scaling: `docs/annotative-scaling.md`
- Performance and budgets: `docs/performance.md`
- DWG test flow: `docs/testing-dwg.md`; headless UI debugging: `docs/verify-ui.md`

**Reference & decisions**

- OpenCADStudio functional-spec reference (not source): `docs/ui-requirements/00-INDEX.md`; historical survey: `docs/migration-map.md`
- Decision records: `docs/adr/`

## License and third parties

This repository is released under **AGPL-3.0** (`LICENSE`). Sources and licenses of
dependencies and bundled resources (acadrust, Slint/wgpu, QCAD samples, osifont, mlightcad
fonts, etc.) are documented in `THIRD_PARTY_NOTICES.md` and `fonts/SOURCE.md`.
