// What one colour value means, and what a chooser choice writes back.
//
// Pure data + predicates (no JSX, no React, no DOM), the same shape
// `layer-attributes.ts` and `properties.ts` use: the component is layout, this
// module is the decision, so every rule below is unit-testable without a window.
//
// The one rule it exists to keep: **the chooser never invents a spelling.** A
// parsed value is read in the grammar the host's write path documents
// (`crates/cad-app/src/property_value`: `ByLayer`, `ByBlock`, `aci:0..=256`,
// `#RRGGBB`; `crates/cad-app/src/layer_property`: `aci:1..=255`, `#RRGGBB`,
// `rgb:#RRGGBB`, `r,g,b`), and every choice writes one of those spellings through
// the surface's existing write channel. There is no second parser and no second
// value form: a swatch writes `aci:N`, a symbol writes its own token, and the
// free-form entry is normalised by the caller's own grammar check.
import { ACI_COLORS, ACI_INDEXES, aciHex } from './aci-palette.ts';
import { nextFocusIndex } from '../layers/layers.ts';

/** One parsed colour value: the shape decides what the chooser can show. */
export type ColorValue =
  /** Nothing to show: an untouched field or a mixed row. */
  | { readonly kind: 'empty' }
  /** `ByLayer` / `ByBlock` (or a token this build does not know, kept verbatim). */
  | { readonly kind: 'symbolic'; readonly symbol: string }
  /** An indexed palette colour, with the `#rrggbb` it paints. */
  | { readonly kind: 'aci'; readonly index: number; readonly hex: string }
  /** A true colour (`#rrggbb`), which is not an index. */
  | { readonly kind: 'hex'; readonly hex: string }
  /** Any other string the row carries: shown as text, never as a colour. */
  | { readonly kind: 'unparsed'; readonly text: string };

/**
 * The symbolic colours the **entity** colour grammar accepts, in AutoCAD's own
 * order. `crates/cad-app/src/property_value` parses both, case-insensitively, and
 * projects them back as these exact tokens.
 *
 * `ByBlock` is the token AutoCAD's Properties palette labels *ByObject*: the
 * literal string `ByObject` is **not** in the grammar, so it is not offered here
 * (the host would refuse it) and the entry writes and reads back `ByBlock`.
 */
export const ENTITY_COLOR_SYMBOLS: readonly string[] = Object.freeze(['ByLayer', 'ByBlock']);

/**
 * The symbolic colours the **layer** colour grammar accepts: none.
 *
 * `crates/cad-app/src/layer_property` refuses `ByLayer`/`ByBlock` **by name**
 * ("a layer has no layer above it"), so a layer chooser that offered them would
 * offer two guaranteed refusals. A layer's colour is always concrete.
 */
export const LAYER_COLOR_SYMBOLS: readonly string[] = Object.freeze([]);

/** The symbolic token matched case-insensitively, in its canonical spelling. */
function canonicalSymbol(text: string): string | null {
  for (const symbol of ['ByLayer', 'ByBlock'] as const) {
    if (text.toLowerCase() === symbol.toLowerCase()) return symbol;
  }
  return null;
}

/**
 * Parse one row value.
 *
 * Nothing is guessed: a string outside the documented grammar is `unparsed` (the
 * chooser still shows it as the current text and offers nothing as selected), and
 * `aci:0` / `aci:256` — legal grammar for the symbolic variants, but not colours —
 * parse as `symbolic` so the chooser marks ByBlock/ByLayer rather than a swatch.
 *
 * The layer grammar's two extra spellings (`rgb:#RRGGBB`, `r,g,b`) are `unparsed`
 * here on purpose: the wire's layer row projects `color.rgb` as `#rrggbb`, and a
 * typed `r,g,b` is normalised to `#RRGGBB` by `colorPayload` before it is ever
 * written, so neither can reach this function as a current value.
 */
export function parseColorValue(value: unknown): ColorValue {
  const text = typeof value === 'string' ? value.trim() : '';
  if (!text) return { kind: 'empty' };
  const symbol = canonicalSymbol(text);
  if (symbol) return { kind: 'symbolic', symbol };
  const indexed = /^aci:(\d+)$/i.exec(text);
  if (indexed) {
    const index = Number(indexed[1]);
    if (Number.isSafeInteger(index) && index === 0) return { kind: 'symbolic', symbol: 'ByBlock' };
    if (Number.isSafeInteger(index) && index === 256) return { kind: 'symbolic', symbol: 'ByLayer' };
    const hex = aciHex(index);
    return hex ? { kind: 'aci', index, hex } : { kind: 'unparsed', text };
  }
  if (/^#[0-9a-f]{6}$/i.test(text)) return { kind: 'hex', hex: text.toLowerCase() };
  return { kind: 'unparsed', text };
}

/** What the trigger paints: a real colour, a symbolic token, or neither. */
export interface ColorSwatchView {
  /** A paintable `#rrggbb`, or `null` when the value is not a colour. */
  readonly hex: string | null;
  /** The symbolic token (`ByLayer`/`ByBlock`), or `null` when there is none. */
  readonly symbol: string | null;
}

/** What the trigger paints for one value (never a fabricated colour). */
export function colorSwatch(value: unknown): ColorSwatchView {
  const parsed = parseColorValue(value);
  if (parsed.kind === 'symbolic') return { hex: null, symbol: parsed.symbol };
  if (parsed.kind === 'aci' || parsed.kind === 'hex') return { hex: parsed.hex, symbol: null };
  return { hex: null, symbol: null };
}

/** The catalog key naming one symbolic colour, or `null` for an unknown token. */
export function symbolLabelKey(symbol: unknown): string | null {
  switch (symbol) {
    case 'ByLayer':
      return 'color.symbol.bylayer';
    case 'ByBlock':
      return 'color.symbol.byblock';
    default:
      return null;
  }
}

/** The index the value currently selects, or `null` (a true colour selects none). */
export function selectedAciIndex(value: unknown): number | null {
  const parsed = parseColorValue(value);
  return parsed.kind === 'aci' ? parsed.index : null;
}

/** The wire value one grid swatch writes: the grammar's own `aci:N`. */
export function aciChoice(index: number): string {
  return `aci:${index}`;
}

/** Whether `value` is exactly this symbolic entry (case-insensitively). */
export function isSelectedSymbol(value: unknown, symbol: string): boolean {
  const parsed = parseColorValue(value);
  return parsed.kind === 'symbolic' && parsed.symbol.toLowerCase() === symbol.toLowerCase();
}

/** One swatch of the standard palette: the index and the colour it paints. */
export interface ColorSwatchCell {
  readonly index: number;
  readonly hex: string;
}

/**
 * The standard indexed palette, `1..=255`, in index order.
 *
 * Index `0` (ByBlock) and `256` (ByLayer) are symbolic, not colours, so they are
 * the chooser's symbol entries instead of swatches. Built once from the mirrored
 * table, so the grid and `aciHex` can never disagree.
 */
export const COLOR_GRID: readonly ColorSwatchCell[] = Object.freeze(
  ACI_INDEXES.map(index => ({ index, hex: ACI_COLORS[index] })),
);

/** How many swatches one grid row holds (17 rows of 15 for `1..=255`). */
export const COLOR_GRID_COLUMNS = 15;

/**
 * The cursor step one key means on the swatch grid, or `null` when the key is not
 * a grid key (so `<Enter>`, `<Space>`, `<Escape>` and typing stay untouched).
 *
 * Only the relative moves live here: `Home`/`End` are absolute and are answered by
 * [`swatchCursor`], which is also where the wrapping rule is applied.
 */
export function swatchDelta(key: string, columns: number = COLOR_GRID_COLUMNS): number | null {
  const width = Number.isSafeInteger(columns) && columns > 0 ? columns : COLOR_GRID_COLUMNS;
  switch (key) {
    case 'ArrowLeft':
      return -1;
    case 'ArrowRight':
      return 1;
    case 'ArrowUp':
      return -width;
    case 'ArrowDown':
      return width;
    default:
      return null;
  }
}

/**
 * The swatch the cursor moves to for one key, or `null` when the key is not a grid
 * key.
 *
 * Arrows step one cell sideways or one row up/down and **wrap** at both ends (the
 * same rule `nextFocusIndex` applies to the layers list, imported rather than
 * re-spelled); `Home`/`End` are the first and last swatch. `column` is how many
 * swatches one row holds, so a caller that changes the CSS grid must change it too.
 */
export function swatchCursor(
  current: unknown,
  key: string,
  columns: number = COLOR_GRID_COLUMNS,
  count: number = COLOR_GRID.length,
): number | null {
  const size = Number.isSafeInteger(count) && count > 0 ? count : COLOR_GRID.length;
  if (key === 'Home') return 0;
  if (key === 'End') return size - 1;
  const delta = swatchDelta(key, columns);
  if (delta === null) return null;
  return nextFocusIndex(current, delta, size);
}
