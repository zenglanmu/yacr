// The Home and Draw **Modify** panel item lists, split verbatim out of
// `ribbon-model.ts` so that module stays inside the code-size gate. This is plain
// data, not a barrel: `ribbon-model.ts` imports the two frozen arrays directly.
//
// Every item is a payload-free router command (`cad_command`), and every
// commented selection rule is enforced by `commands/gating.ts`; nothing here
// decides enablement.
import type { RibbonCommandItem } from './ribbon-model.ts';
import { PEDIT_ITEM } from './variants.ts';

/**
 * `TREX` split button: the two operations of the selection-based trim/extend.
 *
 * The parent `TREX` selector deliberately refuses a bare invocation (there is no
 * in-command operation prompt); the caret rows each carry the operation word and
 * are admitted by `ARGUMENT_COMMANDS.TREX`. Both read the current selection and
 * use every other selected object as a boundary.
 */
const TREX_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'TREX TRIM', labelKey: 'draw.kind.trex.trim', icon: 'trim', command: 'TREX TRIM' },
  { id: 'TREX EXTEND', labelKey: 'draw.kind.trex.extend', icon: 'extend', command: 'TREX EXTEND' },
]);

const TREX_ITEM: RibbonCommandItem = Object.freeze({
  id: 'TREX',
  labelKey: 'draw.kind.trex',
  icon: 'trim',
  command: 'TREX',
  hintKey: 'draw.kind.trex.hint',
  variants: TREX_VARIANTS,
});

/** `MPEDIT` split button: the three PEDIT options over the whole selection. */
const MPEDIT_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'MPEDIT CLOSE', labelKey: 'draw.kind.mpedit.close', icon: 'polyline', command: 'MPEDIT CLOSE' },
  { id: 'MPEDIT OPEN', labelKey: 'draw.kind.mpedit.open', icon: 'polyline', command: 'MPEDIT OPEN' },
  { id: 'MPEDIT REVERSE', labelKey: 'draw.kind.mpedit.reverse', icon: 'mirror', command: 'MPEDIT REVERSE' },
]);

const MPEDIT_ITEM: RibbonCommandItem = Object.freeze({
  id: 'MPEDIT',
  labelKey: 'draw.kind.mpedit',
  icon: 'polyline',
  command: 'MPEDIT',
  hintKey: 'draw.kind.mpedit.hint',
  variants: MPEDIT_VARIANTS,
});

/** `DRAWORDER` split button: FRONT/BACK/ABOVE/BELOW over the selection. */
const DRAWORDER_VARIANTS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'DRAWORDER FRONT', labelKey: 'draw.kind.draworder.front', icon: 'properties', command: 'DRAWORDER FRONT' },
  { id: 'DRAWORDER BACK', labelKey: 'draw.kind.draworder.back', icon: 'properties', command: 'DRAWORDER BACK' },
  { id: 'DRAWORDER ABOVE', labelKey: 'draw.kind.draworder.above', icon: 'properties', command: 'DRAWORDER ABOVE' },
  { id: 'DRAWORDER BELOW', labelKey: 'draw.kind.draworder.below', icon: 'properties', command: 'DRAWORDER BELOW' },
]);

const DRAWORDER_ITEM: RibbonCommandItem = Object.freeze({
  id: 'DRAWORDER',
  labelKey: 'draw.kind.draworder',
  icon: 'properties',
  command: 'DRAWORDER',
  hintKey: 'draw.kind.draworder.hint',
  variants: DRAWORDER_VARIANTS,
});

/** The Home tab's Modify panel, in display order. */
export const HOME_MODIFY_ITEMS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM', large: true },
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
  { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
  // MATCHPROP (MA) is the selection-based property painter, so it sits
  // beside ERASE/EXPLODE — the panel's other commands that act on the
  // **current** selection. It is a small button with no variant popover:
  // there is no Settings sub-prompt (which properties to copy), the
  // command always transfers all five display properties, and a caret
  // here would imply choices the command does not have. Its enabled state
  // is the shared admission rule plus its own selection requirement
  // (`SELECTION_REQUIREMENTS`), so with fewer than two selected objects it
  // reads disabled with the sentence that names the rule.
  { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', command: 'MATCHPROP' },
  { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
  // The three single-curve edits (2026-11-01) sit directly after the
  // TRIM/EXTEND pair, the curve-editing commands they belong with; JOIN
  // still closes the panel, so no shipped button moves.
  //
  // BREAK captures **two picks** — the first is both the curve pick and
  // the first break point, the second is the other end of the removed
  // span — so it is not a pre-selection tool: pressing it starts a
  // capture the way TRIM does. Small button, no variant popover (the
  // command has no options sub-prompt), glyph `trim`.
  { id: 'BREAK', labelKey: 'draw.kind.break', icon: 'trim', command: 'BREAK' },
  // DIVIDE and LENGTHEN take **zero picks**: they act on the current
  // selection (exactly one curve) and their settings are the shared tool
  // parameters, so they carry neither a pick prompt nor a caret here. The
  // glyphs are borrowed (`point` for the marks DIVIDE creates, `scale`
  // for the length LENGTHEN changes) — the icon set has no dedicated
  // divide/lengthen mark.
  { id: 'DIVIDE', labelKey: 'draw.kind.divide', icon: 'point', command: 'DIVIDE' },
  { id: 'LENGTHEN', labelKey: 'draw.kind.lengthen', icon: 'scale', command: 'LENGTHEN' },
  { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', command: 'CHAMFER' },
  { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', command: 'FILLET' },
  // The Polyline modes sit directly after the two-pick pair they belong
  // to: the same corner treatment, one preselected open polyline instead
  // of two picks. Small buttons with no variant popover (there is no
  // options sub-prompt and no TRIMMODE switch), enabled by the shared
  // admission plus their own one-object selection requirement.
  { id: 'CHAMFER POLYLINE', labelKey: 'draw.kind.chamfer_polyline', icon: 'chamfer', command: 'CHAMFER POLYLINE' },
  { id: 'FILLET POLYLINE', labelKey: 'draw.kind.fillet_polyline', icon: 'fillet', command: 'FILLET POLYLINE' },
  { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
  { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
  { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
  { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', command: 'STRETCH' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', command: 'MIRROR', icon: 'mirror' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', command: 'SCALE', icon: 'scale' },
  // JOIN (2026-10-26) closes the Modify panel the way AutoCAD's does. It
  // is a small button (it is not a primary edit) with no variant popover —
  // the command has no options sub-prompt — and its enablement is the
  // shared admission **plus** its own two-object selection requirement, so
  // with fewer objects it reads disabled with the rule's sentence.
  { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', command: 'JOIN' },
  // SELECTSIMILAR (AutoCAD 2026) closes the Modify panel after JOIN: a
  // type-only v1 that grows the current selection by geometry kind. It is
  // a small button with no variant popover (there is no attribute match
  // to configure in v1), and it is selection-gated by
  // `SELECTION_REQUIREMENTS` — with nothing selected it reads disabled
  // with its own hint.
  { id: 'SELECTSIMILAR', labelKey: 'draw.kind.selectsimilar', icon: 'selectAll', command: 'SELECTSIMILAR' },
  // REVERSE (AutoCAD 2026) is a selection-based direction flip: no
  // capture, no options, one or more LINE/polyline/spline objects. Small
  // button with no popover; enablement is the shared admission plus its
  // own one-object selection requirement.
  { id: 'REVERSE', labelKey: 'draw.kind.reverse', icon: 'mirror', command: 'REVERSE' },
  // PEDIT (AutoCAD 2026) is the polyline option subset: the one selected
  // model-space polyline is the whole input and the option is typed
  // (`PEDIT CLOSE|OPEN|REVERSE`), because this build has no in-command option
  // prompt. It is a split button (the shared item lives in `variants.ts`); each
  // row is an argument form of the base selector, gated by
  // `ARGUMENT_COMMANDS.PEDIT` plus the one-object selection rule.
  PEDIT_ITEM,
  // OOPS restores the last ERASE: no capture, no selection, `undo` glyph.
  { id: 'OOPS', labelKey: 'draw.kind.oops', icon: 'undo', command: 'OOPS' },
  // OVERKILL v1: exact duplicates only, two-object selection minimum.
  { id: 'OVERKILL', labelKey: 'draw.kind.overkill', icon: 'erase', command: 'OVERKILL' },
  // BREAKATPOINT starts its own two-step capture (object pick, break point); it
  // is not a preselection command and owns no options popover.
  { id: 'BREAKATPOINT', labelKey: 'draw.kind.breakatpoint', icon: 'trim', command: 'BREAKATPOINT' },
  // HATCHTOBACK (AutoCAD 2026) lives in AutoCAD's Draw Order drop-down; this
  // build has no draw-order menu, so it is an ordinary small router button here.
  // It takes no selection and is document-scoped (an open drawing is the whole
  // requirement); no options popover exists.
  { id: 'HATCHTOBACK', labelKey: 'draw.kind.hatchtoback', icon: 'hatch', command: 'HATCHTOBACK' },
  // TEXTTOFRONT (AutoCAD 2026) is the selection-based draw-order command: it
  // brings the selected text/leaders/dimensions in front of every other
  // model-space object. It is a small router button (no capture, no options
  // popover), enabled by the shared one-object selection requirement. Paper/block
  // draw order and the all-objects form are NOT IMPLEMENTED; the hint says so.
  { id: 'TEXTTOFRONT', labelKey: 'draw.kind.texttofront', icon: 'properties', command: 'TEXTTOFRONT', hintKey: 'draw.kind.texttofront.hint' },
  // The 2026-11-07 payload-free selection batch. SSX/SELECTCOUNT compare the
  // first selection's five properties; the hide/isolate trio is session-only
  // visibility state. All five are plain router buttons (no capture, no options
  // popover); selection-gated rows live in `commands/gating.ts`. The typed
  // QSELECT/FILTER/GETSEL/-FIND/FIND forms deliberately have no button: a button
  // would have to invent the filter or the search text.
  { id: 'SSX', labelKey: 'draw.kind.ssx', icon: 'selectAll', command: 'SSX' },
  { id: 'SELECTCOUNT', labelKey: 'draw.kind.selectcount', icon: 'properties', command: 'SELECTCOUNT' },
  { id: 'HIDEOBJECTS', labelKey: 'draw.kind.hideobjects', icon: 'clearSelection', command: 'HIDEOBJECTS' },
  { id: 'ISOLATEOBJECTS', labelKey: 'draw.kind.isolateobjects', icon: 'layers', command: 'ISOLATEOBJECTS' },
  { id: 'UNISOLATEOBJECTS', labelKey: 'draw.kind.unisolateobjects', icon: 'layers', command: 'UNISOLATEOBJECTS' },
  // The 2026-11-07 selection batch. TREX/MPEDIT/DRAWORDER are split buttons (the
  // operation word is the command's argument); EXTRIM is a plain button whose
  // selection form uses the first selected object as the boundary.
  TREX_ITEM,
  { id: 'EXTRIM', labelKey: 'draw.kind.extrim', icon: 'trim', command: 'EXTRIM', hintKey: 'draw.kind.extrim.hint' },
  MPEDIT_ITEM,
  DRAWORDER_ITEM,
]);

/** The Draw tab's Modify panel, in display order. */

/** The Draw tab's Modify panel, in display order. */
export const DRAW_MODIFY_ITEMS: readonly RibbonCommandItem[] = Object.freeze([
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
  { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
  // Same slot as on the Home tab: directly after the other selection-acting
  // commands (see the Home/modify comment for why it is a small button with
  // no popover and why its enablement is selection-gated).
  { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', command: 'MATCHPROP' },
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
  { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
  { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM' },
  { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
  // Same slot as on the Home tab (see the Home/modify comment for the
  // two-pick BREAK shape and the zero-pick DIVIDE/LENGTHEN pair), so both
  // modify panels read in the same order.
  { id: 'BREAK', labelKey: 'draw.kind.break', icon: 'trim', command: 'BREAK' },
  { id: 'DIVIDE', labelKey: 'draw.kind.divide', icon: 'point', command: 'DIVIDE' },
  { id: 'LENGTHEN', labelKey: 'draw.kind.lengthen', icon: 'scale', command: 'LENGTHEN' },
  { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', command: 'CHAMFER' },
  { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', command: 'FILLET' },
  // The Polyline modes sit directly after the two-pick pair they belong
  // to: the same corner treatment, one preselected open polyline instead
  // of two picks. Small buttons with no variant popover (there is no
  // options sub-prompt and no TRIMMODE switch), enabled by the shared
  // admission plus their own one-object selection requirement.
  { id: 'CHAMFER POLYLINE', labelKey: 'draw.kind.chamfer_polyline', icon: 'chamfer', command: 'CHAMFER POLYLINE' },
  { id: 'FILLET POLYLINE', labelKey: 'draw.kind.fillet_polyline', icon: 'fillet', command: 'FILLET POLYLINE' },
  { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
  { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
  { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
  { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', command: 'STRETCH' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
  // Same slot as on the Home tab: JOIN closes the Modify panel (AutoCAD's
  // own order puts Join last), small, no popover, selection-gated.
  { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', command: 'JOIN' },
  // SELECTSIMILAR (AutoCAD 2026) is the type-only v1 selection grower; it
  // sits in the same tail slot on both tabs and stays selection-gated.
  { id: 'SELECTSIMILAR', labelKey: 'draw.kind.selectsimilar', icon: 'selectAll', command: 'SELECTSIMILAR' },
  // Same tail slot as the Home tab: REVERSE is selection-based, so it
  // carries no pick prompt and no popover.
  { id: 'REVERSE', labelKey: 'draw.kind.reverse', icon: 'mirror', command: 'REVERSE' },
  // Same tail slot as the Home tab: PEDIT is the polyline option subset, a
  // split button whose rows are `PEDIT CLOSE|OPEN|REVERSE` (the shared item
  // lives in `variants.ts`). Bare `PEDIT` is a named needs-option refusal.
  PEDIT_ITEM,
  { id: 'OOPS', labelKey: 'draw.kind.oops', icon: 'undo', command: 'OOPS' },
  // OVERKILL v1: exact duplicates only, two-object selection minimum.
  { id: 'OVERKILL', labelKey: 'draw.kind.overkill', icon: 'erase', command: 'OVERKILL' },
  // BREAKATPOINT starts its own two-step capture (object pick, break point); it
  // is not a preselection command and owns no options popover.
  { id: 'BREAKATPOINT', labelKey: 'draw.kind.breakatpoint', icon: 'trim', command: 'BREAKATPOINT' },
  // HATCHTOBACK (AutoCAD 2026) lives in AutoCAD's Draw Order drop-down; this
  // build has no draw-order menu, so it is an ordinary small router button here.
  // It takes no selection and is document-scoped (an open drawing is the whole
  // requirement); no options popover exists.
  { id: 'HATCHTOBACK', labelKey: 'draw.kind.hatchtoback', icon: 'hatch', command: 'HATCHTOBACK' },
  // TEXTTOFRONT (AutoCAD 2026) is the selection-based draw-order command: it
  // brings the selected text/leaders/dimensions in front of every other
  // model-space object. It is a small router button (no capture, no options
  // popover), enabled by the shared one-object selection requirement. Paper/block
  // draw order and the all-objects form are NOT IMPLEMENTED; the hint says so.
  { id: 'TEXTTOFRONT', labelKey: 'draw.kind.texttofront', icon: 'properties', command: 'TEXTTOFRONT', hintKey: 'draw.kind.texttofront.hint' },
  // The 2026-11-07 payload-free selection batch. SSX/SELECTCOUNT compare the
  // first selection's five properties; the hide/isolate trio is session-only
  // visibility state. All five are plain router buttons (no capture, no options
  // popover); selection-gated rows live in `commands/gating.ts`. The typed
  // QSELECT/FILTER/GETSEL/-FIND/FIND forms deliberately have no button: a button
  // would have to invent the filter or the search text.
  { id: 'SSX', labelKey: 'draw.kind.ssx', icon: 'selectAll', command: 'SSX' },
  { id: 'SELECTCOUNT', labelKey: 'draw.kind.selectcount', icon: 'properties', command: 'SELECTCOUNT' },
  { id: 'HIDEOBJECTS', labelKey: 'draw.kind.hideobjects', icon: 'clearSelection', command: 'HIDEOBJECTS' },
  { id: 'ISOLATEOBJECTS', labelKey: 'draw.kind.isolateobjects', icon: 'layers', command: 'ISOLATEOBJECTS' },
  { id: 'UNISOLATEOBJECTS', labelKey: 'draw.kind.unisolateobjects', icon: 'layers', command: 'UNISOLATEOBJECTS' },
]);
