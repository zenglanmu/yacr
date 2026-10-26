// The canonical AutoCAD Color Index (ACI) palette, `0..=255`.
//
// This is a **mirror** of `crates/cad-app/src/property_value/aci.rs` — the one
// table the host's colour grammar resolves (`aci:N`) and therefore the one table
// the chooser may paint. It is generated from that file's `ACI_COLORS` and
// `test/properties-color.test.ts` re-reads the Rust source and compares every
// triple, so a palette edit on either side fails a test instead of leaving the
// picker showing a colour the drawing would not store.
//
// ACI `0` is the ByBlock placeholder and `256` is ByLayer: neither is a colour,
// so neither appears in the grid (they are the chooser's symbolic entries) and
// [`aciHex`] answers `null` for them. Index `7` is white, the drawing's nominal
// dark-background default.

/** The 256 sRGB triples as `#rrggbb`, index = ACI index (`0` is the placeholder). */
export const ACI_COLORS: readonly string[] = Object.freeze([
  '#000000', '#ff0000', '#ffff00', '#00ff00', '#00ffff', '#0000ff', '#ff00ff', '#ffffff',
  '#808080', '#c0c0c0', '#ff0000', '#ff7f7f', '#cc0000', '#cc6666', '#990000', '#994c4c',
  '#7f0000', '#7f3f3f', '#4c0000', '#4c2626', '#ff3f00', '#ff9f7f', '#cc3300', '#cc7f66',
  '#992600', '#995f4c', '#7f1f00', '#7f4f3f', '#4c1300', '#4c2f26', '#ff7f00', '#ffbf7f',
  '#cc6600', '#cc9966', '#994c00', '#99724c', '#7f3f00', '#7f5f3f', '#4c2600', '#4c3926',
  '#ffbf00', '#ffdf7f', '#cc9900', '#ccb266', '#997200', '#99854c', '#7f5f00', '#7f6f3f',
  '#4c3900', '#4c4226', '#ffff00', '#ffff7f', '#cccc00', '#cccc66', '#999900', '#99994c',
  '#7f7f00', '#7f7f3f', '#4c4c00', '#4c4c26', '#bfff00', '#dfff7f', '#99cc00', '#b2cc66',
  '#729900', '#85994c', '#5f7f00', '#6f7f3f', '#394c00', '#424c26', '#7fff00', '#bfff7f',
  '#66cc00', '#99cc66', '#4c9900', '#72994c', '#3f7f00', '#5f7f3f', '#264c00', '#394c26',
  '#3fff00', '#9fff7f', '#33cc00', '#7fcc66', '#269900', '#5f994c', '#1f7f00', '#4f7f3f',
  '#134c00', '#2f4c26', '#00ff00', '#7fff7f', '#00cc00', '#66cc66', '#009900', '#4c994c',
  '#007f00', '#3f7f3f', '#004c00', '#264c26', '#00ff3f', '#7fff9f', '#00cc33', '#66cc7f',
  '#009926', '#4c995f', '#007f1f', '#3f7f4f', '#004c13', '#264c2f', '#00ff7f', '#7fffbf',
  '#00cc66', '#66cc99', '#00994c', '#4c9972', '#007f3f', '#3f7f5f', '#004c26', '#264c39',
  '#00ffbf', '#7fffdf', '#00cc99', '#66ccb2', '#009972', '#4c9985', '#007f5f', '#3f7f6f',
  '#004c39', '#264c42', '#00ffff', '#7fffff', '#00cccc', '#66cccc', '#009999', '#4c9999',
  '#007f7f', '#3f7f7f', '#004c4c', '#264c4c', '#00bfff', '#7fdfff', '#0099cc', '#66b2cc',
  '#007299', '#4c8599', '#005f7f', '#3f6f7f', '#00394c', '#26424c', '#007fff', '#7fbfff',
  '#0066cc', '#6699cc', '#004c99', '#4c7299', '#003f7f', '#3f5f7f', '#00264c', '#26394c',
  '#003fff', '#7f9fff', '#0033cc', '#667fcc', '#002699', '#4c5f99', '#001f7f', '#3f4f7f',
  '#00134c', '#262f4c', '#0000ff', '#7f7fff', '#0000cc', '#6666cc', '#000099', '#4c4c99',
  '#00007f', '#3f3f7f', '#00004c', '#26264c', '#3f00ff', '#9f7fff', '#3300cc', '#7f66cc',
  '#260099', '#5f4c99', '#1f007f', '#4f3f7f', '#13004c', '#2f264c', '#7f00ff', '#bf7fff',
  '#6600cc', '#9966cc', '#4c0099', '#724c99', '#3f007f', '#5f3f7f', '#26004c', '#39264c',
  '#bf00ff', '#df7fff', '#9900cc', '#b266cc', '#720099', '#854c99', '#5f007f', '#6f3f7f',
  '#39004c', '#42264c', '#ff00ff', '#ff7fff', '#cc00cc', '#cc66cc', '#990099', '#994c99',
  '#7f007f', '#7f3f7f', '#4c004c', '#4c264c', '#ff00bf', '#ff7fdf', '#cc0099', '#cc66b2',
  '#990072', '#994c85', '#7f005f', '#7f3f6f', '#4c0039', '#4c2642', '#ff007f', '#ff7fbf',
  '#cc0066', '#cc6699', '#99004c', '#994c72', '#7f003f', '#7f3f5f', '#4c0026', '#4c2639',
  '#ff003f', '#ff7f9f', '#cc0033', '#cc667f', '#990026', '#994c5f', '#7f001f', '#7f3f4f',
  '#4c0013', '#4c262f', '#333333', '#5b5b5b', '#848484', '#adadad', '#d6d6d6', '#ffffff',
]);

/**
 * The `#rrggbb` of one ACI index, or `null` for `0`/`256` (symbolic, not colours)
 * and for anything outside the table.
 */
export function aciHex(index: unknown): string | null {
  if (typeof index !== 'number' || !Number.isSafeInteger(index)) return null;
  if (index < 1 || index > 255) return null;
  return ACI_COLORS[index] ?? null;
}

/** The ACI indexes the chooser offers as swatches: every real entry, `1..=255`. */
export const ACI_INDEXES: readonly number[] = Object.freeze(
  Array.from({ length: 255 }, (_, offset) => offset + 1),
);
