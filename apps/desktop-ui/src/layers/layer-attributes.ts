// The attribute surface of one layer row: which color the row's dot paints,
// which state chips it marks, which flags a press writes and which read-only
// attributes its details line reports.
//
// Pure data + predicates (no JSX, no React, no DOM), the same shape
// `layer-labels.ts` uses for refusals: the palette is layout, this module is the
// decision, so every rule below is unit-testable without a window.
//
// Three rules it exists to keep in one place:
//
//  * **A color is only painted as the drawing's when the host reported one.** The
//    row's `color.rgb` is the host's own `#rrggbb`. When it is absent or not a hex
//    color, the dot falls back to `layerTone` — the muted, name-derived marker —
//    and `fromColor: false` tells the palette to say so in the dot's accessible
//    name instead of presenting a hash as if it were the layer's color.
//  * **A chip is the row's own state, never a recomputed guess.** Every flag is
//    read straight off the row the host answered; a field the host did not send
//    reads as `false` here exactly as it does in `parseLayersAnswer`, so the
//    palette can never show a state the wire did not carry.
//  * **A press sends the host's grammar, and the copy carries the behavior.**
//    Flags travel as `'true'`/`'false'`, the color as `#RRGGBB`. What a flag
//    actually *does* is not a code comment only: `off`/`frozen` hide the layer
//    through the existing `visible` path (freezing's own rules are not
//    implemented), and `locked`/`plottable` are stored without changing anything
//    yet — each of those facts is a catalog key (`layers.<flag>.hint`) this module
//    points at, so the palette shows it as the chip's hint.
//
// What is deliberately **not** here: pickers for the linetype, the lineweight or
// the transparency. The wire carries no linetype table to choose from, so those
// three are read-only, verbatim projections (see [`layerAttributes`]).
import { layerTone } from './layers.ts';
import type { LayerRow } from './layers.ts';

/** The only color shape the palette paints as the drawing's own: `#rrggbb`. */
const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;

/** The color a row's dot paints, and whether it is the drawing's own. */
export interface LayerSwatch {
  /** A `#rrggbb` string, always paintable as CSS. */
  readonly color: string;
  /**
   * `true` when `color` is the host's stored layer color, `false` when it is the
   * name-derived marker. A surface uses this to keep the marker distinguishable
   * (and to say so in the dot's accessible name) instead of claiming a hash color
   * is the drawing's.
   */
  readonly fromColor: boolean;
}

/**
 * The color the row's swatch paints: the host's `#rrggbb` when it sent a usable
 * one, otherwise the muted name-derived tone.
 *
 * Nothing is invented: `fromColor: false` is the honest answer for a row whose
 * color field was empty or not a hex color, and the dot then reads as the
 * identity marker it is.
 */
export function layerSwatch(row: LayerRow | null | undefined): LayerSwatch {
  const rgb = typeof row?.color?.rgb === 'string' ? row.color.rgb : '';
  if (HEX_COLOR.test(rgb)) return { color: rgb.toLowerCase(), fromColor: true };
  return { color: layerTone(row).color, fromColor: false };
}

/** The four stored flags a row may write (the host's writable flag keys). */
export type LayerFlagKey = 'off' | 'frozen' | 'locked' | 'plottable';

/** One flag the palette can write, with the catalog keys that name it. */
export interface LayerFlag {
  readonly key: LayerFlagKey;
  /** The catalog key naming the flag's current state. */
  readonly labelKey: string;
  /** The catalog key carrying the honest behavior behind the flag. */
  readonly hintKey: string;
  /** The stored state, read from the row. */
  readonly value: boolean;
  /** The wire value a press sends: the opposite of `value`, `'true'`/`'false'`. */
  readonly next: string;
}

/**
 * The flag table: the four keys, their catalog keys and the order they appear in
 * (the host's own `off`, `frozen`, `locked`, `plottable` grouping).
 */
const FLAGS: ReadonlyArray<{
  readonly key: LayerFlagKey;
  readonly labelKey: string;
  readonly hintKey: string;
}> = [
  { key: 'off', labelKey: 'layers.off', hintKey: 'layers.off.hint' },
  { key: 'frozen', labelKey: 'layers.frozen', hintKey: 'layers.frozen.hint' },
  { key: 'locked', labelKey: 'layers.locked', hintKey: 'layers.locked.hint' },
  { key: 'plottable', labelKey: 'layers.plottable', hintKey: 'layers.plottable.hint' },
];

/** The `'true'`/`'false'` the flag grammar takes for `value`. */
export function flagPayload(value: unknown): string {
  return value === true ? 'true' : 'false';
}

/** One row's stored state for one flag (`false` when the row is gone). */
function storedFlag(row: LayerRow | null | undefined, key: LayerFlagKey): boolean {
  return row?.[key] === true;
}

/**
 * The four flags of one row, in a fixed order, each with the press that flips it.
 *
 * The palette renders these as the engaged-state toggles of the row's details
 * panel: the state comes from the row, and `next` is what a press sends.
 */
export function layerFlags(row: LayerRow | null | undefined): LayerFlag[] {
  return FLAGS.map(flag => {
    const value = storedFlag(row, flag.key);
    return { ...flag, value, next: flagPayload(!value) };
  });
}

/** One chip the resting row marks, plus the press that clears the mark. */
export interface LayerMark {
  readonly key: LayerFlagKey;
  /** The catalog key naming the state the chip marks. */
  readonly labelKey: string;
  /** The catalog key carrying the honest behavior behind the state. */
  readonly hintKey: string;
  /** The wire value a press sends to clear the mark. */
  readonly next: string;
}

/**
 * The state chips a row shows while its details panel is closed.
 *
 * Only the states worth a mark appear: `off`, `frozen` and `locked` while they are
 * set, and `plottable` while it is **not**. That keeps a plain row exactly as
 * compact as it was before attributes arrived (a 300px dock cannot hold four
 * bidirectional chips next to the name) and it keeps the marks column meaning
 * what it already means — "something about this row is not the default". A press
 * on a mark clears that very state, which is the one direction a reader of the
 * mark already expects; the other direction is a press in the details panel.
 *
 * `plottable` is the one flag whose notable state is `false` (plotting is the
 * default), so its mark is the `layers.no_plot` wording rather than
 * `layers.plottable` — and it is only marked when the row itself said
 * `plottable: false`, never because the field was absent.
 *
 * [`layerMarkSummary`] then caps what a *resting* row spells out: the dock cannot
 * show four chips next to a name, so a row with more than [`RESTING_MARK_LIMIT`]
 * states shows the first two and a count of the rest (which opens the details line).
 */
export function layerMarks(row: LayerRow | null | undefined): LayerMark[] {
  const marks: LayerMark[] = [];
  for (const flag of FLAGS) {
    const value = storedFlag(row, flag.key);
    // `plottable` marks only a row that said "not plottable" itself: for a flag
    // whose default is the notable state, a missing field must not be marked as
    // if it had been reported.
    const notable = flag.key === 'plottable' ? row?.plottable === false : value;
    if (!notable) continue;
    const notPlotting = flag.key === 'plottable';
    marks.push({
      key: flag.key,
      labelKey: notPlotting ? 'layers.no_plot' : flag.labelKey,
      hintKey: notPlotting ? 'layers.no_plot.hint' : flag.hintKey,
      next: flagPayload(!value),
    });
  }
  return marks;
}

/**
 * How many state chips a resting row spells out.
 *
 * The 300px dock leaves the marks column room for about two chips once the row's
 * own marks and controls (current, override, attributes, rename/delete) are
 * counted, so two is the most a row can say without eliding its name. Most rows
 * carry none.
 */
export const RESTING_MARK_LIMIT = 2;

/** A row's resting marks: the ones it spells out, and how many it holds back. */
export interface LayerMarkSummary {
  /** The chips the row shows, in the flags' own order. */
  readonly marks: LayerMark[];
  /** How many further states are set but not spelled out (`0` when none). */
  readonly hidden: number;
}

/**
 * Split a row's marks into the chips a row shows and the count it holds back.
 *
 * Nothing is dropped silently: a row with more than [`RESTING_MARK_LIMIT`] states
 * says so with the `hidden` count (the palette renders it as the `layers.marks_more`
 * chip, which opens the details line where all four flags are real toggles). The
 * order is the host's own grouping, so the same row always spells out the same two.
 */
export function layerMarkSummary(row: LayerRow | null | undefined): LayerMarkSummary {
  const marks = layerMarks(row);
  return { marks: marks.slice(0, RESTING_MARK_LIMIT), hidden: Math.max(0, marks.length - RESTING_MARK_LIMIT) };
}

/** The summary of a row that shows no marks at all (and a stable empty answer). */
export const EMPTY_MARK_SUMMARY: LayerMarkSummary = Object.freeze({ marks: [], hidden: 0 });

/** One read-only attribute of the details line. */
export interface LayerAttribute {
  readonly key: 'linetype' | 'lineweight' | 'transparency';
  /** The catalog key naming the attribute. */
  readonly labelKey: string;
  /** The host's own string, verbatim (never converted or re-scaled). */
  readonly value: string;
}

/** A non-empty string field, or `''` (never a fabricated value). */
function textOf(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

/**
 * The three read-only attributes, in the palette's order, verbatim.
 *
 * The strings are the host's own write grammar (`ByLayer` / `Default` / a 1/100 mm
 * integer, and percent transparent) and are **not** converted: a read-only line
 * that re-scaled them would be inventing a unit the wire never promised. The
 * panel's note says they are shown as the drawing stores them.
 */
export function layerAttributes(row: LayerRow | null | undefined): LayerAttribute[] {
  return [
    { key: 'linetype', labelKey: 'layers.linetype', value: textOf(row?.linetype) },
    { key: 'lineweight', labelKey: 'layers.lineweight', value: textOf(row?.lineweight) },
    { key: 'transparency', labelKey: 'layers.transparency', value: textOf(row?.transparency) },
  ];
}

/**
 * The wire value a colour control's value means, or `''` when the layer's colour
 * grammar takes none of it.
 *
 * Two spellings reach this surface now, and both are the **layer** grammar's own
 * (one parser, no new capability):
 *
 *  * a true colour, `#RRGGBB` — upper-cased here so one colour is never sent under
 *    two casings (the wire's row projection hands back lowercase, so a typed
 *    uppercase has to mean the same colour);
 *  * an indexed palette colour, `aci:1..=255` — what a swatch in the shared
 *    colour chooser writes, so a pick keeps the drawing's ACI identity instead of
 *    silently becoming a true colour. `aci:0` (ByBlock) and `aci:256` (ByLayer)
 *    are **refused**: a layer has no layer above it, and this surface offers no
 *    symbolic entry for the same reason.
 *
 * Anything else — an empty field, a named colour, a shorthand hex, `ByLayer`, a
 * three-part `r,g,b` this control never emits — answers `''` and is never sent.
 */
export function colorPayload(value: unknown): string {
  const raw = typeof value === 'string' ? value.trim() : '';
  const indexed = /^aci:(\d+)$/i.exec(raw);
  if (indexed) {
    const index = Number(indexed[1]);
    return Number.isSafeInteger(index) && index >= 1 && index <= 255 ? `aci:${index}` : '';
  }
  return HEX_COLOR.test(raw) ? `#${raw.slice(1).toUpperCase()}` : '';
}
