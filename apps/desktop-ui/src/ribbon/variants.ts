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
