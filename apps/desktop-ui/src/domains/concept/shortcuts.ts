// SketchUp's documented default single-key shortcuts, as a pure table.
//
// It is **one** table: `ConceptShortcuts` is the only call site, and a resolved
// name still goes through `shell.canCommand` + `shell.submitCommand`, the same
// one funnel the band/menu use. The table never invents an acceptance.
//
// The official table assigns Offset to **F** and Orbit to **O** (the brief's
// "O = Offset" is not the reference mapping). Keys with no working kernel tool
// still resolve to the closest stable command id — PRESSPULL/MATERIALS are
// explicit host refusals, never silent no-ops. Keys with no honest command at
// all (Image Igloo, Back Edges) are not in the table.
import type { AcceleratorContext } from '../../commands/accelerators.ts';

/** The command a plain SketchUp key means, or `null`. */
export const CONCEPT_SHORTCUTS: Readonly<Record<string, string>> = Object.freeze({
  e: 'ERASE',
  l: 'LINE',
  r: 'RECTANG',
  c: 'CIRCLE',
  a: 'ARC',
  m: 'MOVE',
  p: 'PRESSPULL',
  q: 'ROTATE',
  s: 'SCALE',
  f: 'OFFSET',
  t: 'DIST',
  o: 'ORBIT',
  h: 'PAN',
  z: 'ZOOMIN',
  b: 'MATERIALS',
  g: 'BLOCK',
});

/** The command a Shift+letter chord means, or `null`. */
const SHIFTED: Readonly<Record<string, string>> = Object.freeze({
  z: 'FIT',
});

/**
 * The fields the concept table reads. It reuses the shared accelerator context
 * so the same typing/popup/text-step ownership rules apply.
 */
export interface ConceptShortcutEvent {
  readonly key?: string;
  readonly ctrlKey?: boolean;
  readonly shiftKey?: boolean;
  readonly metaKey?: boolean;
  readonly altKey?: boolean;
  readonly isComposing?: boolean;
  readonly keyCode?: number;
}

/**
 * One key press's concept-domain outcome.
 *
 * `'CANCEL'` is the Select state (Space): with a tool active it cancels the
 * capture, exactly like Escape; with no tool it is already the idle state, so
 * the press is consumed but sends nothing.
 */
export type ConceptShortcut =
  | { readonly kind: 'command'; readonly command: string }
  | { readonly kind: 'select' }
  | { readonly kind: 'none' };

/**
 * Resolve one press.
 *
 * Declines exactly where the shared table does: IME composition, a text entry,
 * an open popup, an armed grip drag, or a command's terminal text step. It does
 * **not** decline merely because a tool is active: SketchUp's letter keys switch
 * tools mid-capture, and the host owns whatever cancellation/restart the new
 * command needs. `meta`/`alt`/`ctrl` are never ours.
 */
export function conceptShortcut(
  event: ConceptShortcutEvent = {},
  context: AcceleratorContext = {},
): ConceptShortcut {
  if (event.isComposing === true || event.keyCode === 229) return { kind: 'none' };
  if (context.typing === true || context.popupOpen === true) return { kind: 'none' };
  if (context.textStep === true || context.gripDragActive === true) return { kind: 'none' };
  if (event.ctrlKey === true || event.metaKey === true || event.altKey === true) return { kind: 'none' };
  const key = typeof event.key === 'string' ? event.key : '';
  if (key === ' ') return { kind: 'select' };
  if (key.length !== 1) return { kind: 'none' };
  const letter = key.toLowerCase();
  const command = event.shiftKey === true ? SHIFTED[letter] : CONCEPT_SHORTCUTS[letter];
  return command ? { kind: 'command', command } : { kind: 'none' };
}
