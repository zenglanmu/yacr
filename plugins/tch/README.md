# tch plugin

Tianzheng (`TCH_*`) decoder plugin for yacr. It runs **inside the WebView** and is
loaded by the host as a module script; it never touches `cad-db`, the filesystem
or Tauri IPC.

## Contract (ABI v1)

```ts
export const abiVersion: 1;
export function decode(records: ArrayBuffer): ArrayBuffer;
```

The built module also registers itself on
`globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion, decode }`; the host injects
`plugins/tch.js` with `<script type="module">` and validates that global.

`records` is the binary frame described in `docs/vendor-plugin.md` §4. The result
is the ABI v1 frame: entity groups + primitives (lines/segments/arcs/circles/
text/block inserts), with `applied` equal to the primitive count.

Current coverage (real `test-tangent-real.dwg`, 10,612 vendor records; 9,889
entities / 14,911 primitives):

| group | classes |
|---|---|
| structure | `TCH_WALL` (straight walls), `TCH_COLUMN`, `TCH_OPENING` (wall cuts, hole jambs, door/window 2D symbol blocks, threshold lines, band windows, composite groups) |
| text | `TCH_TEXT`, `TCH_MTEXT` |
| annotation | `TCH_ELEVATION`, `TCH_ARROW`, `TCH_MULTILEADER`, `TCH_DRAWINGINDEX`, `TCH_DRAWINGNAME`, `TCH_NORTHTHUMB` |
| dimension | `TCH_DIMENSION2`, `TCH_AXIS_LABEL`, `TCH_RADIUSDIM` |
| stair | `TCH_LINESTAIR`, `TCH_RECTSTAIR` |
| symbol / line work | `TCH_SYMB_SECTION`, `TCH_INDEXPOINTER`, `TCH_CORNER_WINDOW`, `TCH_BLOCK_INSERT`, `TCH_RUPTURE`, `TCH_MODI` |

`TCH_BLOCK_INSERT` and the door/window symbols emit a block reference by **source
handle** (kind 5); the host resolves it through
`DrawingDatabase::block_by_source_handle` and expands the imported block. All 40
distinct handles in the real fixture resolve to imported blocks that have
entities.

Still unsupported, and counted explicitly rather than drawn: `TCH_SPACE` (383),
`TCH_WALL` records that fail the verified header/arc layout (157), `TCH_OPENING`
records with a body version we do not know or with a null wall (70),
`TCH_DIMENSION` (35, the older class; MiniCAD has no decoder for it either),
`TCH_SHEET` (1), `TCH_ERRORMSG` (1). Records whose class body is valid but has no
drawable geometry land in `vendor.tch.no_display`, and the result frame carries a
coverage proof: `decoded + unsupported + no_display == record count` (a mismatch
emits `vendor.tch.unaccounted`).

The class text stream starts with a shared `TDBXDATAMAP_BLOCK_BEGIN_ … _END_`
block; `ClassText` skips it once per record so real labels are emitted instead of
the sentinel (verified: 0 sentinel text primitives in the real result frame).

## Build

From the repository root:

```bash
bash scripts/build-vendor-plugin.sh
```

This writes `plugins/tch/dist/index.js` and copies it to
`apps/desktop-ui/public/plugins/tch.js`, which every carrier's Vite build embeds
as `dist/plugins/tch.js`.

## Submodule

This directory is intended to be a **git submodule** so the plugin can be
versioned and released independently of the host:

```bash
git submodule add <plugin-repo-url> plugins/tch
```

The host only assumes the build command above and the ABI v1 contract; it does
not import plugin source.

## Status

- records transfer: wired (ABI v1)
- frame validation: wired
- TCH decoding: the 22 classes listed above; the rest are explicit diagnostics
- geometry output: ABI v1 primitives (display-only derived representation)
- runtime evidence: Linux `native-below` real-GPU run loaded the full scene
  (Quadro P620, Vulkan); Windows/macOS/Android/Web remain compile/contract only
