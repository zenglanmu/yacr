// The AutoCAD-style control accelerators for the document commands.
//
// `docs/shortcuts.md` promises these chords. Until this table existed only the
// CAD shortcut menu (clipboard, select all), the title bar/ribbon (undo, redo)
// and the command line could reach the same commands, so the documented
// accelerators did nothing. This module is the **one** mapping table; `App.tsx`
// owns the **one** call site.
//
// It is deliberately pure and free of React/DOM types: the caller passes the
// key/modifier fields it already has, so the table is unit-testable under
// `node --test` with no DOM, and every focus/popup/tool decision stays outside
// it, where the shell can see the real event target and session state.
//
// What this table is **not**: a second admission. A resolved name still has to
// pass `shell.canCommand` — the same `admitCommandText` the command line, the
// ribbon and the QAT use — and the shell prevents the webview default *only*
// when it dispatches. A chord whose command is not admitted is left to the
// webview untouched; nothing here invents an acceptance or a refusal.

/** The key-event fields the accelerator table reads (plain values, no DOM types). */
export interface AcceleratorKeyEvent {
  readonly key?: string;
  readonly ctrlKey?: boolean;
  readonly shiftKey?: boolean;
  readonly metaKey?: boolean;
  readonly altKey?: boolean;
  /** `KeyboardEvent.isComposing`: an IME composition owns the press. */
  readonly isComposing?: boolean;
  /** Legacy IME commit marker (`KeyboardEvent.keyCode === 229`). */
  readonly keyCode?: number;
}

/**
 * Who else owns the keyboard right now.
 *
 * The shell fills this from state it already has (`cad_status`, the lane gates);
 * the table never guesses, and an absent flag means `false` — a chord is not
 * declined because a field was forgotten.
 */
export interface AcceleratorContext {
  /** Focus is a text entry: `INPUT`/`SELECT`/`TEXTAREA` or `contentEditable`. */
  readonly typing?: boolean;
  /** A host dialog is open (`cad_status.popup`) and owns the modal gate. */
  readonly popupOpen?: boolean;
  /** A CAD tool is active (`cad_status.tool.key`). */
  readonly toolActive?: boolean;
  /** The active tool is awaiting its terminal text step (`tool.awaiting_text`). */
  readonly textStep?: boolean;
  /** A grip drag is armed (`cad_status.gripDrag`) — not a tool, but it owns the press. */
  readonly gripDragActive?: boolean;
}

/**
 * The `Ctrl`+letter chords that exist in their unshifted form only.
 *
 * The two documented shifted forms are handled by name in the function below:
 * `Ctrl+Shift+S` is still `SAVE`, and `Ctrl+Shift+Z` is `REDO`.
 */
const CONTROL_LETTER_COMMANDS: Readonly<Record<string, string>> = Object.freeze({
  a: 'SELECTALL',
  c: 'COPYCLIP',
  x: 'CUTCLIP',
  v: 'PASTECLIP',
  o: 'OPEN',
  n: 'NEW',
  p: 'PLOT',
});

/**
 * The CAD command a key press means, or `null` when the press is not a CAD
 * accelerator and must stay exactly as it was.
 *
 * Decision order (every `return null` happens before the chord is even looked
 * up, so a declined press is never prevented or dispatched):
 *
 * 1. an IME composition (`isComposing`, or the legacy `keyCode === 229`);
 * 2. a text entry has focus — native copy/paste/undo/select-all and the
 *    webview's IME stay untouched in the command line, the property fields, the
 *    text editors and the layer panels;
 * 3. a host dialog is open (its overlay owns the keyboard);
 * 4. a CAD tool is active, its text step is pending, or a grip drag is armed —
 *    deliberately: Enter/Escape are the owners there, and firing `COPYCLIP` or
 *    `UNDO` mid-tool would act on a capture the user is still making;
 * 5. `metaKey`/`altKey` is held, or the press is not `Ctrl` at all (`Super` and
 *    `AltGr` chords belong to the desktop, and a plain key is nobody's chord);
 * 6. `Ctrl+Home` → `FIT` (AutoCAD's zoom extents);
 * 7. otherwise the letters: `Ctrl+Z` → `UNDO`, `Ctrl+Shift+Z`/`Ctrl+Y` → `REDO`,
 *    `Ctrl+Shift+S`/`Ctrl+S` → `SAVE`, and the unshifted-only rows above (any
 *    other shifted letter is declined on purpose).
 */
export function acceleratorCommand(
  event: AcceleratorKeyEvent = {},
  context: AcceleratorContext = {},
): string | null {
  // 1. An IME owns every key of its composition, Ctrl chords included.
  if (event.isComposing === true || event.keyCode === 229) return null;
  // 2. Text entry keeps native editing: a CAD command must never steal
  //    copy/paste/undo/select-all from a focused field.
  if (context.typing === true) return null;
  // 3. A host dialog is modal: its overlay owns the accelerators it defines.
  if (context.popupOpen === true) return null;
  // 4. Tool ownership is Enter/Escape's, and an armed grip drag is the same
  //    situation on the pointer side (the host writes nothing until release).
  if (context.toolActive === true || context.textStep === true || context.gripDragActive === true) {
    return null;
  }
  // 5. Only the control modifier is ours.
  if (event.metaKey === true || event.altKey === true) return null;
  if (event.ctrlKey !== true) return null;
  const key = typeof event.key === 'string' ? event.key : '';
  const shift = event.shiftKey === true;
  // 6. Ctrl+Home frames the drawing (AutoCAD's `FIT`). Named keys keep their
  //    exact spelling; only single characters are case-folded below.
  if (key === 'Home') return shift ? null : 'FIT';
  // 7. Letters. Only the documented shifted forms exist: `Ctrl+Shift+S` is still
  //    `SAVE`, and `Ctrl+Shift+Z` is `REDO` (`Ctrl+Y` is the same redo). Every
  //    other letter is unshifted-only, which keeps `Ctrl+Shift+C/X/V` clear of
  //    AutoCAD's COPYBASE/CUTBASE/PASTEBASE family — commands this build does
  //    not implement, so they must not degrade silently into plain copy/paste.
  if (key.length !== 1) return null;
  const letter = key.toLowerCase();
  if (letter === 'z') return shift ? 'REDO' : 'UNDO';
  if (letter === 's') return 'SAVE';
  if (letter === 'y') return shift ? null : 'REDO';
  if (shift) return null;
  return CONTROL_LETTER_COMMANDS[letter] ?? null;
}

/**
 * The command the shell may run for this press: the mapped name, but only when
 * the caller's admission function (the shell's `canCommand`, i.e.
 * `admitCommandText`) allows it.
 *
 * `null` is the whole decline path — no dispatch **and** no webview-default
 * suppression — so "gated by the same capability as every other affordance"
 * is one tested rule rather than a convention the call site has to remember.
 * `admitted` is called only for a chord that mapped, never for ordinary keys.
 */
export function acceleratorDispatch(
  event: AcceleratorKeyEvent,
  context: AcceleratorContext,
  admitted: (command: string) => boolean,
): string | null {
  const command = acceleratorCommand(event, context);
  return command !== null && admitted(command) === true ? command : null;
}
