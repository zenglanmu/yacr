// The chrome's declarative admission tables, split verbatim out of `commands.ts`
// (which owns the functions that read them) so neither module approaches the
// 1000-line code gate. Plain sibling module, not a barrel: importers name it
// directly.
//
// Nothing here decides anything by itself — these are the frozen data rows the
// admission functions consult: the typed-argument grammar, the host capability
// gates, the gesture-only and document-scoped sets, the selection preconditions
// and the selectors the chrome answers without the router. Empty or wrong data
// here can only ever refuse, never silently admit.

/** The `cad_status` slice the admission rules read (all fields are the host's). */
export interface CommandStatusView {
  loading?: boolean;
  documentLabel?: string | null;
  canOpen?: boolean;
  canNew?: boolean;
  canSave?: boolean;
  canPlot?: boolean;
  canFit?: boolean;
  entityCount?: number | null;
  /** The `cad_status.selection` slice the selection rules count (`{count}`). */
  selection?: { readonly count?: unknown } | null;
}

/** A host capability boolean that gates a file-level selector. */
export type CapabilityGate = 'canNew' | 'canSave' | 'canPlot';

/** The parser for the one typed-argument form a selector accepts. */
export interface ArgumentSpec {
  readonly parse: (argument: string) => boolean;
}

/** Whether `text` is a plain positive decimal number (`2`, `0.5`, `1.25`). */
function isPositiveDecimal(text: string): boolean {
  if (!/^[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  const value = Number(text);
  return Number.isFinite(value) && value > 0;
}

/**
 * Whether `text` is a plain decimal number, sign and zero included (`3`, `2.5`,
 * `0`, `-5`).
 *
 * The point-style sysvars need the signed form where `SNAPUNIT` does not:
 * `PDMODE 0` is the DWG default (dot) and `PDSIZE 0` is the "size from the
 * viewport" default, so a positive-only gate would refuse the drawing's own
 * defaults; a negative `PDSIZE` is a percentage of the viewport height in the
 * DWG model, and the renderer reads the sign. The value is still a **single**
 * plain decimal (`5.`, `.5`, `1e3`, `+5` and `2 3` are not one) — the same
 * literal shape the other sysvar uses, so one grammar is typed either way.
 */
function isDecimalNumber(text: string): boolean {
  if (!/^-?[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  return Number.isFinite(Number(text));
}

/**
 * The selectors that take a typed argument, with the **local syntax** predicate
 * for that argument.
 *
 * This is an admission aid only: the chrome decides "does this line look like a
 * command that could run" and the host stays the authority for semantics (a
 * `ZOOM 0` that slipped through would be refused by the router and recorded for
 * `cad_status.error`, never clamped). `ZOOM` mirrors the host grammar —
 * a positive decimal scale factor (`> 1` zooms in, `< 1` zooms out), `ALL` or
 * `EXTENTS` (the `FIT` framing). `SNAPUNIT` is the session grid-snap spacing
 * sysvar and accepts the same positive-decimal shape (`25`, `0.5`, `2.5`).
 * `PDMODE`/`PDSIZE` (2026-10-26) are the drawing's POINT display-style sysvars:
 * one plain decimal each (`PDMODE 3`, `PDSIZE 2.5`), zero and negative included
 * because `PDMODE 0` (dot) and `PDSIZE 0` (viewport-relative) are the DWG
 * defaults and a negative `PDSIZE` is a viewport percentage. They change the
 * in-memory display style of the open drawing only — not undoable, and the
 * exporter writes neither — so nothing here may suggest persistence.
 * Every other selector takes no argument, so `LINE 2` and `SNAP 5` stay explicit
 * unsupported answers.
 */
export const ARGUMENT_COMMANDS: Readonly<Record<string, ArgumentSpec>> = Object.freeze({
  ZOOM: Object.freeze({
    parse: (argument: string) => argument === 'ALL' || argument === 'EXTENTS' || isPositiveDecimal(argument),
  }),
  SNAPUNIT: Object.freeze({
    parse: isPositiveDecimal,
  }),
  // Point display style (`$PDMODE` / `$PDSIZE`): the same one-number grammar as
  // `SNAPUNIT`, widened to the signed/zero form the DWG values themselves use.
  // The range semantics (which mode means dot/cross/…) and whether a value is
  // expressible stay the host's call — a value refused here is refused before
  // IPC with the shared `command.unsupported` sentence, and a value admitted
  // here is still validated by the router.
  PDMODE: Object.freeze({
    parse: isDecimalNumber,
  }),
  PDSIZE: Object.freeze({
    parse: isDecimalNumber,
  }),
});

/**
 * Host capability gates for the file-level selectors.
 *
 * `NEW`/`SAVE`/`PLOT` and their router aliases are real router commands, not
 * "host-owned" names to be hidden behind a local exclusion list: the host
 * publishes whether it can service them right now in `cad_status`
 * (`canNew`/`canSave`/`canPlot`) and those booleans decide admission. This table
 * maps each name onto the field that gates it; a name absent here has no
 * capability gate. `canNew` is about creating a blank drawing (no document
 * needed); `canSave`/`canPlot` additionally require an open document.
 */
export const CAPABILITY_GATES: Readonly<Record<string, CapabilityGate>> = Object.freeze({
  NEW: 'canNew',
  SAVE: 'canSave',
  SAVEAS: 'canSave',
  QSAVE: 'canSave',
  PLOT: 'canPlot',
  EXPORT: 'canPlot',
  PRINT: 'canPlot',
});

/**
 * Commands the Rust router accepts but a **click cannot perform** because they
 * are canvas drag gestures: `ORBIT` is the 3D primary drag, and its `yaw`/
 * `pitch` come from pointer motion, not from a button payload. Inventing an
 * orbit here would fabricate a camera move, so the affordance stays visible but
 * disabled with a gesture reason that tells the user how to orbit. This is an
 * explicit disable, never a local enable.
 */
export const GESTURE_ONLY_COMMANDS: readonly string[] = Object.freeze(['ORBIT']);

/**
 * Router-admitted selectors that only mean something with a drawing open:
 * `SWITCH2D3D` frames the document's 3D bounds and `PROJECTION` flips the
 * projection of that same camera, so on the start page there is nothing to
 * frame. They are disabled there with the catalog's own "no drawing is open"
 * sentence instead of a misleading "not supported yet".
 *
 * `MATCHPROP` needs a drawing *and* a selection; it is deliberately **not** in
 * this list — a document is not its only precondition, and the set means exactly
 * "a drawing is the whole requirement" (see `SELECTION_REQUIREMENTS`, which
 * answers the no-document case for selection-gated commands itself).
 */
export const DOCUMENT_SCOPED_COMMANDS: readonly string[] = Object.freeze(['SWITCH2D3D', 'PROJECTION']);

/** One command's precondition on the **current selection**. */
export interface SelectionRequirement {
  /** Minimum selected objects that can still produce a real result. */
  readonly count: number;
  /**
   * The catalog sentence that words the requirement. It is the desktop's own
   * copy of the host rule (`matchprop.needs_two`), so the disabled ribbon hint
   * and the command line's refusal say the same thing — and it is the *only*
   * place the source→targets rule is spelled out for the reader.
   */
  readonly reasonKey: string;
}

/**
 * Commands whose bare name acts on the **current selection**, with the smallest
 * selection that is not a guaranteed refusal.
 *
 * `MATCHPROP` is the selection-based property painter: the first selected entity
 * is the **source**, every other selected entity is a **target**, so one object
 * or none is never a partial success — the host refuses it outright
 * (`matchprop.needs_two`). Offering a live button in that state would be an
 * affordance that can only fail, so the same table disables it with a sentence
 * that says what is missing (`draw.kind.matchprop.hint`) *and* stops the command
 * line from sending a line that cannot succeed.
 *
 * Only *count* preconditions belong here. A command that merely needs "something"
 * selected (ERASE, MOVE, COPY) keeps the neighbours' contract: it stays enabled
 * and the host answers with an explicit refusal, because its own copy
 * (`draw.error.selection_required`) already says so at run time.
 *
 * `JOIN` (2026-10-26) is the second real count precondition: it welds the
 * selection into a single entity (one LINE or one ARC), so one object or none can
 * never be a partial success and the host refuses it outright. Its sentence is the
 * catalog's own `draw.kind.join.hint`, which is also what the disabled ribbon
 * button shows, so the button and the refusal word the same rule once.
 *
 * `LAYISO` needs at least one object — isolating "nothing" would leave every
 * layer hidden and no way back from the canvas — and it reuses the catalog's
 * existing "Select objects first" line (`draw.error.selection_required`): that is
 * already the sentence the host refuses a bare isolate with, so no second copy of
 * the same fact is invented. `LAYUNISO` takes **no** selection (`count: 0`): its
 * entry exists for the "no drawing is open" answer only — the requirement can
 * never be unmet, because nothing is what it restores.
 *
 * `LAYOFF`/`LAYON` (2026-10-26) follow the isolate pair exactly. `LAYOFF` turns
 * **off** every layer the current selection sits on (the current layer excepted),
 * so with nothing selected there is no layer to turn off — the same one-object
 * minimum as `LAYISO`, worded with the same catalog sentence the host refuses a
 * bare `LAYOFF` with (`draw.error.selection_required`). `LAYON` turns every layer
 * back on and takes **no** selection (`count: 0`), so its entry is the
 * "no drawing is open" answer only, exactly like `LAYUNISO`'s.
 */
export const SELECTION_REQUIREMENTS: Readonly<Record<string, SelectionRequirement>> = Object.freeze({
  MATCHPROP: Object.freeze({ count: 2, reasonKey: 'draw.kind.matchprop.hint' }),
  JOIN: Object.freeze({ count: 2, reasonKey: 'draw.kind.join.hint' }),
  // CHAMFER POLYLINE / FILLET POLYLINE (2026-10-27) are the Polyline modes of the
  // two corner commands: each rewrites **exactly one** preselected open polyline
  // in place (`CommandPayload::None`, so nothing is picked interactively). The
  // table states a *minimum* of one — the host is what refuses a selection of two
  // or a non-polyline, by name — and the sentence is this command's own hint, so
  // the disabled button and the refusal describe the same rule once. Zero
  // selected objects can never be a partial success, which is the whole reason
  // the row exists instead of leaving the host to answer at run time.
  'CHAMFER POLYLINE': Object.freeze({ count: 1, reasonKey: 'draw.kind.chamfer_polyline.hint' }),
  'FILLET POLYLINE': Object.freeze({ count: 1, reasonKey: 'draw.kind.fillet_polyline.hint' }),
  LAYISO: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYUNISO: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  LAYOFF: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYON: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  // AREA OBJECT (2026-10-27) measures **exactly one** closed object, so its rule is
  // a count of one: `SelectionRequirement.count` is a *minimum* here (the table's
  // shape does not express an upper bound), which means a selection of two is let
  // through locally and refused by the host **by name** — the same division of
  // labour every other selection rule uses. The sentence is this command's own
  // `measure.kind.area_object.hint`, which states the one-object rule and that the
  // command does not pick objects interactively, so the refusal and the catalogue
  // row cannot describe two different behaviours.
  'AREA OBJECT': Object.freeze({ count: 1, reasonKey: 'measure.kind.area_object.hint' }),
  // LAYERP restores the previous layer-override state, so no selection is ever the
  // reason it cannot run. Like LAYON/LAYUNISO its entry exists for the "no drawing
  // is open" answer only.
  LAYERP: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
});

/**
 * Selectors the **chrome** answers itself, before the router. They have a real
 * command-line entry (typing the word works) but no Rust command: `PROPERTIES`
 * opens the docked inspector and `LAYER` opens the docked layers palette, both
 * through the shell action of the same name.
 *
 * They are deliberately admitted regardless of the loaded host vocabulary (and
 * of whether a drawing is open): they touch no CAD state, exactly like the
 * `OPTIONS` dialog the command line already intercepts locally. `LAYER` is not a
 * router name at all — the router knows the layer *operations*
 * (`TOGGLELAYER`/`SETLAYERS`/`RESTORELAYERS`/`SETACTIVELAYER`), which the palette
 * drives through its own dedicated commands (`cad_layers` and friends).
 */
export const LOCAL_CHROME_COMMANDS: readonly string[] = Object.freeze(['PROPERTIES', 'LAYER']);
