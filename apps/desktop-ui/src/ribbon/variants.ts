// Ribbon split-button variant lists (pure data, no React).
//
// Split out of `ribbon-model.ts` (which is at the file-size ceiling) so a new
// variant family can be added without pushing that module over the limit. This is
// a plain sibling module, not a barrel: the model imports the frozen items it
// places in a panel.
//
// A variant row is an ordinary payload-free router command, exactly like a
// top-level ribbon item: its enablement is the loaded host vocabulary alone.
import type { RibbonCommandItem } from './ribbon-model.ts';

/**
 * The Circle split button: the plain `CIRCLE` plus its three definition modes.
 *
 * The parent stays the ordinary CIRCLE button (centre, then radius) and keeps its
 * `large` tile on both draw panels; the caret offers the three further modes the
 * host advertises as **separate selectors** (`CIRCLE 2P`, `CIRCLE 3P`,
 * `CIRCLE D`). The structure copies the `FIT` and `AREA` popovers: the first row
 * repeats the parent's own selector so the menu is also a "what the button does"
 * list.
 *
 * Every row is a router command with no parameter — the picks are the whole input
 * — and every row reuses the `circle` glyph, because all four produce the same
 * entity and the icon set has no per-mode badge; the label states the definition
 * method instead. `CIRCLE D`'s second pick is a **distance from the centre that
 * is the diameter**, so its label must say "diameter" and never imply a radius
 * pick (the step prompt spells the halving out).
 *
 * One frozen value used by both draw panels: the two tabs must offer the same
 * four modes, and a shared (frozen) object is the cheapest way to keep them
 * identical.
 */
const CIRCLE_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', command: 'CIRCLE' },
  { id: 'CIRCLE 2P', labelKey: 'draw.kind.circle2p', icon: 'circle', command: 'CIRCLE 2P' },
  { id: 'CIRCLE 3P', labelKey: 'draw.kind.circle3p', icon: 'circle', command: 'CIRCLE 3P' },
  { id: 'CIRCLE D', labelKey: 'draw.kind.circlediameter', icon: 'circle', command: 'CIRCLE D' },
]);

export const CIRCLE_ITEM: RibbonCommandItem = Object.freeze({
  id: 'CIRCLE',
  labelKey: 'draw.kind.circle',
  icon: 'circle',
  command: 'CIRCLE',
  large: true,
  variants: CIRCLE_VARIANTS,
});

/**
 * The Polygon split button: the plain **inscribed** `POLYGON` plus AutoCAD's
 * `Circumscribed about circle` (`C`) variant.
 *
 * The parent keeps the unchanged inscribed row (centre, then a vertex on the
 * polygon) and its small-button slot on both draw panels; the caret offers the
 * circumscribed construction as its own canonical **multi-word** selector
 * (`POLYGON CIRCUMSCRIBED`), never an alias of `POLYGON`: the second pick means
 * different things (a polygon vertex vs a point on the **tangent** circle) and
 * the committed rings differ.
 *
 * Both rows are router commands with no parameter — the picks are the whole input
 * and the side count is the session `POLYSIDES` sysvar — and both reuse the
 * `polygon` glyph, because the output is the same LWPOLYLINE kind and the icon set
 * has no per-variant badge; the label states the construction instead. There is
 * no `Edge` row: that construction is NOT IMPLEMENTED and a caret entry would
 * promise it.
 */
const POLYGON_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', command: 'POLYGON' },
  {
    id: 'POLYGON CIRCUMSCRIBED',
    labelKey: 'draw.kind.polygon_circumscribed',
    icon: 'polygon',
    command: 'POLYGON CIRCUMSCRIBED',
  },
]);

export const POLYGON_ITEM: RibbonCommandItem = Object.freeze({
  id: 'POLYGON',
  labelKey: 'draw.kind.polygon',
  icon: 'polygon',
  command: 'POLYGON',
  variants: POLYGON_VARIANTS,
});

/**
 * The `MEASUREGEOM` split button: AutoCAD's inquiry router over the measurement
 * tools this build already has.
 *
 * Every row sends the **full typed line** (`MEASUREGEOM DISTANCE`, …), never the
 * bare name: the bare selector is the host's `measuregeom.needs_option` refusal,
 * and the router maps each option onto the existing tool (or the read-only
 * `RADIUS` report). The parent repeats `DISTANCE` so the button is the ordinary
 * one-click distance measurement while the caret names the other options.
 *
 * `VOLUME` is listed because AutoCAD has it and the host must answer with its
 * named `measuregeom.volume_unsupported` code — but it is **not** a working
 * measurement: this build has no 3D solid/volume model. The label/hint say so
 * and no number is ever fabricated. The two new labels are `measure.kind.radius`
 * and `measure.kind.volume`; the four tool rows reuse the existing measurement
 * labels so the menu reads exactly like the Measure panel.
 */
const MEASUREGEOM_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'MEASUREGEOM DISTANCE', labelKey: 'measure.kind.distance', icon: 'distance', command: 'MEASUREGEOM DISTANCE' },
  { id: 'MEASUREGEOM ANGLE', labelKey: 'measure.kind.angle', icon: 'angle', command: 'MEASUREGEOM ANGLE' },
  { id: 'MEASUREGEOM AREA', labelKey: 'measure.kind.area', icon: 'area', command: 'MEASUREGEOM AREA' },
  { id: 'MEASUREGEOM POLYLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', command: 'MEASUREGEOM POLYLINE' },
  { id: 'MEASUREGEOM RADIUS', labelKey: 'measure.kind.radius', icon: 'dimradius', command: 'MEASUREGEOM RADIUS' },
  {
    id: 'MEASUREGEOM VOLUME',
    labelKey: 'measure.kind.volume',
    icon: 'area',
    command: 'MEASUREGEOM VOLUME',
    hintKey: 'measure.kind.measuregeom_volume.hint',
  },
]);

export const MEASUREGEOM_ITEM: RibbonCommandItem = Object.freeze({
  id: 'MEASUREGEOM',
  labelKey: 'measure.kind.measuregeom',
  icon: 'distance',
  command: 'MEASUREGEOM DISTANCE',
  large: true,
  hintKey: 'measure.kind.measuregeom.hint',
  variants: MEASUREGEOM_VARIANTS,
});
