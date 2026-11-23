// The File ribbon's EXPORT split button (pure data, no React).
//
// Split out of `ribbon-model.ts` (which is at the file-size ceiling) exactly like
// `variants.ts`: a plain sibling module, not a barrel, so the model imports the
// one frozen item it places in the File panel.
//
// The parent `EXPORT` stays the plain row: the host opens its export picker and
// the chosen extension decides the writer (ADR-0041/ADR-0052). Each caret row
// sends the **full typed line** (`EXPORT DXF`, `EXPORT GLB`, …) — `EXPORT <TOKEN>`
// is an argument form of that same selector, not a command id of its own: the host
// opens the same picker with that format's extension suggested, and the chosen
// extension is still the authority (a name whose extension disagrees is refused
// before anything is written). The chrome therefore adds no catalog row and no
// `HOST_NAMES` entry — admission is `ARGUMENT_COMMANDS.EXPORT`, and every row
// inherits the parent's `canSave` capability gate because admission runs on the
// base selector.
//
// `COFF` is part of the host's token grammar (so a typed `EXPORT COFF` is never
// refused locally) but deliberately has no row here: the caret offers exactly the
// formats that read as a one-click choice. Per-format availability stays the
// host's answer — a carrier that compiled fewer connectors refuses the chosen
// format by name at run time (`unsupported-runtime: export.<id>`), and the chrome
// must not guess a disabled state it cannot see.
import type { RibbonCommandItem } from './ribbon-model.ts';

/** The plain picker first, then one row per format, in the panel's own order. */
const EXPORT_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'EXPORT', labelKey: 'file.export', icon: 'export', command: 'EXPORT' },
  { id: 'EXPORT DXF', labelKey: 'file.export.dxf', icon: 'export', command: 'EXPORT DXF' },
  { id: 'EXPORT DWG', labelKey: 'file.export.dwg', icon: 'export', command: 'EXPORT DWG' },
  { id: 'EXPORT GLTF', labelKey: 'file.export.gltf', icon: 'export', command: 'EXPORT GLTF' },
  { id: 'EXPORT GLB', labelKey: 'file.export.glb', icon: 'export', command: 'EXPORT GLB' },
  { id: 'EXPORT STL', labelKey: 'file.export.stl', icon: 'export', command: 'EXPORT STL' },
  { id: 'EXPORT OBJ', labelKey: 'file.export.obj', icon: 'export', command: 'EXPORT OBJ' },
  { id: 'EXPORT PLY', labelKey: 'file.export.ply', icon: 'export', command: 'EXPORT PLY' },
  { id: 'EXPORT OFF', labelKey: 'file.export.off', icon: 'export', command: 'EXPORT OFF' },
  { id: 'EXPORT IFC', labelKey: 'file.export.ifc', icon: 'export', command: 'EXPORT IFC' },
  { id: 'EXPORT SKP', labelKey: 'file.export.skp', icon: 'export', command: 'EXPORT SKP' },
]);

export const EXPORT_ITEM: RibbonCommandItem = Object.freeze({
  id: 'EXPORT',
  labelKey: 'file.export',
  icon: 'export',
  command: 'EXPORT',
  large: true,
  variants: EXPORT_VARIANTS,
});
