# tch plugin

Tianzheng (`TCH_*`) decoder plugin for yacr. It runs **inside the WebView** and is
loaded by the host as a module script; it never touches `cad-db`, the filesystem
or Tauri IPC.

## Contract (ABI v4)

```ts
export const abiVersion: 4;
export function decode(records: ArrayBuffer): ArrayBuffer;
```

The built module also registers itself on
`globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion, decode }`; the host injects
`plugins/tch.js` with `<script type="module">` and validates that global.

`records` is the binary frame described in `docs/vendor-plugin.md` §4. The result
is the ABI v4 frame: entity groups + primitives (lines/segments/arcs/circles/
text/block inserts), with `applied` equal to the primitive count.

Current coverage: base-6 `TCH_WALL` (straight walls; arc/profile walls are
reported unsupported), base-6 `TCH_COLUMN`, `TCH_TEXT`/`TCH_MTEXT`, and
`TCH_OPENING`. Openings cut the wall body, emit hole jamb lines, and reference
the opening's real 2D symbol block by **source handle** (the host resolves it and
expands the imported block). Other `TCH_*` classes are counted in
`vendor.tch.unsupported` diagnostics; they never increase `applied`. Dimensions,
stairs, axis labels and the opening's threshold lines are not decoded yet.

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

The host only assumes the build command above and the ABI v4 contract; it does
not import plugin source.

## Status

- records transfer: wired (v4)
- frame validation: wired
- TCH decoding: base-6 walls, columns, text/MTEXT, openings (cuts + 2D symbols)
- geometry output: ABI v4 primitives (display-only derived representation)
