// Colour value types and the standard ACI table.
//
// Local and pure: no wire, no host. The nine standard indexes are the drawing's
// nominal colours, matching the host's canonical palette
// (`apps/desktop-ui/src/color/aci-palette.ts`, mirrored from
// `crates/cad-app/src/property_value/aci.rs`). Indexes `0` (ByBlock) and `256`
// (ByLayer) are symbolic, not colours, and every index outside `1..9` has no
// entry here.
//
// `aciToRgb` answers `undefined` — explicitly — for anything it does not know.
// It never invents `#000000`: a caller that wants a fallback must choose one and
// say so, because a fabricated colour is indistinguishable from a real one.

import { invalidArgument } from './errors.ts';

/** A resolved ACI colour: the index it came from and its `#rrggbb` value. */
export interface Color {
  readonly aci: number;
  readonly rgb: string;
}

/** The four ways an entity names its colour (AutoCAD's `ColorMethod`). */
export const ColorMethod = {
  ByLayer: 'byLayer',
  ByBlock: 'byBlock',
  ByAci: 'byAci',
  ByColor: 'byColor',
} as const;

/** `'byLayer' | 'byBlock' | 'byAci' | 'byColor'`. */
export type ColorMethod = (typeof ColorMethod)[keyof typeof ColorMethod];

/** An entity's colour setting, unresolved until the layer/block cascade says so. */
export type EntityColor =
  | { readonly method: 'byLayer' }
  | { readonly method: 'byBlock' }
  | { readonly method: 'byAci'; readonly aci: number }
  | { readonly method: 'byColor'; readonly rgb: string };

/** ACI `1..9` (`#rrggbb`); `undefined` for every other index and for non-integers. */
const STANDARD_ACI: Readonly<Record<number, string>> = Object.freeze({
  1: '#ff0000',
  2: '#ffff00',
  3: '#00ff00',
  4: '#00ffff',
  5: '#0000ff',
  6: '#ff00ff',
  7: '#ffffff',
  8: '#808080',
  9: '#c0c0c0',
});

/** The `#rrggbb` of one standard ACI index, or `undefined` when there is none. */
export function aciToRgb(aci: number): string | undefined {
  if (typeof aci !== 'number' || !Number.isSafeInteger(aci)) return undefined;
  return Object.prototype.hasOwnProperty.call(STANDARD_ACI, aci)
    ? STANDARD_ACI[aci]
    : undefined;
}

/** A `Color` for one standard ACI index; refuses an index with no known value. */
export function colorFromAci(aci: number): Color {
  const rgb = aciToRgb(aci);
  if (rgb === undefined) throw invalidArgument('aci');
  return { aci, rgb };
}

/** The colour surface exposed on `CadApi.colors`. */
export interface ColorLibrary {
  readonly aciToRgb: typeof aciToRgb;
  readonly colorFromAci: typeof colorFromAci;
}

/** The pure colour library; no host, no bridge, no CSS/DOM dependency. */
export function createColorLibrary(): ColorLibrary {
  return { aciToRgb, colorFromAci };
}
