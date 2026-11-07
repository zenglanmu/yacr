// The Home tab's Layers panel items, split out of `ribbon-model.ts` (which owns
// the tab/panel tree) so that file stays under the 1000-line code gate. Plain
// sibling module, not a barrel: `ribbon-model.ts` imports the array verbatim.
//
// The carrier rule lives in `ribbon-model.ts`'s `PALETTE_COMMANDS`: this panel
// holds the layer operations whose effect is only observable in the layers
// palette, so a carrier that never renders the palette drops the panel whole.
import type { RibbonCommandItem } from './ribbon-model.ts';

export const LAYER_PANEL_ITEMS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER', large: true },
  // Layer isolate / unisolate (2026-10-26) sit beside the palette opener
  // they operate on: both are payload-free router commands acting on the
  // **current selection** (LAYISO needs at least one object, LAYUNISO
  // none), so they are small buttons with the shared admission and no
  // popover. They are dropped with the palette on a carrier that never
  // renders it (`PALETTE_COMMANDS`), because the isolation state is only
  // visible there.
  { id: 'LAYISO', labelKey: 'layers.isolate', icon: 'eyeOff', command: 'LAYISO' },
  { id: 'LAYUNISO', labelKey: 'layers.unisolate', icon: 'eye', command: 'LAYUNISO' },
  // LAYERP (2026-10-27) restores the previous layer-override state, so it
  // sits directly beside LAYUNISO — the two are the layer run's two
  // "bring back what was there" commands, and the pair reads in the order
  // the reader reaches for them (unisolate everything, or go back one
  // step). It is a payload-free router command with no popover, and the
  // carrier rule is the panel's: the state it restores is only visible in
  // the palette, so it is dropped with it (`PALETTE_COMMANDS`).
  { id: 'LAYERP', labelKey: 'layers.previous', icon: 'eye', command: 'LAYERP' },
  // Layer off / on (2026-10-26) close the panel: LAYOFF turns off every
  // layer the selection sits on (needs one object), LAYON turns every
  // layer back on (needs none). Same conventions as the isolate pair —
  // small, payload-free router commands, no popover — and the same
  // carrier rule: the resulting layer state is only visible in the
  // palette, so they are dropped with it (`PALETTE_COMMANDS`).
  { id: 'LAYOFF', labelKey: 'draw.kind.layoff', icon: 'eyeOff', command: 'LAYOFF' },
  { id: 'LAYON', labelKey: 'draw.kind.layon', icon: 'eye', command: 'LAYON' },
  // The AutoCAD 2026 layer-selection run closes the panel. Every entry is a
  // **pre-selection** router command (the current session selection is the
  // input; there is no interactive "Select objects" prompt in this build) except
  // LAYTHW, which takes no selection at all. `LAYCUR` and `SETBYLAYER` reuse
  // the `layers` glyph, the freeze/thaw pair the palette's existing visibility
  // pair (`eyeOff`/`eye`), and the lock/unlock pair the same `layers` glyph
  // because the icon set has no lock mark. Small buttons, no popover: none has
  // an options sub-prompt.
  { id: 'LAYCUR', labelKey: 'draw.kind.laycur', icon: 'layers', command: 'LAYCUR' },
]);
