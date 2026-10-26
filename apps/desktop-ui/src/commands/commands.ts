// Command catalog, admission rules and command-line autocomplete.
//
// Admission is **not** a hand-written mirror any more: the exact set of command
// names the Rust router accepts is loaded once at runtime from the `cad_commands`
// IPC command (backed by `cad_ui_core::session::admissible_names`, the single
// source of truth). Until that answer arrives the catalog is empty, so every
// command is explicitly unsupported — never "everything allowed by default".
//
// On top of the router's vocabulary the chrome keeps explicit **refusal** rules:
// gesture-only names (a click cannot supply a drag payload), document-scoped
// selectors (meaningless on the start page), the selectors the chrome answers
// itself, the host **capability booleans** (`NEW`/`SAVE`/`PLOT` and their
// aliases), and the commands that act on the **current selection** and can
// therefore only run once that selection is big enough (`SELECTION_REQUIREMENTS`).
// No name is disabled merely because it is "host-owned": the host publishes
// `canNew`/`canSave`/`canPlot` in `cad_status` and those booleans decide. Every
// refusal is explicit and named, not a silent omission — the buttons stay visible
// but disabled with the real reason key.
import { selectionCount } from '../cad/selection-wire.ts';

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

/** A typed command line split into its base name and its argument. */
export interface SplitCommandText {
  readonly name: string;
  readonly argument: string | null;
  readonly text: string;
}

/** One row of the host's alias table (`cad_command_aliases`). */
export interface CommandAlias {
  readonly alias: string;
  readonly target: string;
}

/** One raw `{ alias, command }` pair as the host reports it. */
export interface CommandAliasInput {
  readonly alias?: unknown;
  readonly command?: unknown;
}

/** One entry of the chrome's display catalog. */
export interface CommandCatalogEntry {
  readonly id: string;
  readonly labelKey: string;
  readonly icon: string;
  readonly group: string;
}

/** One autocomplete row: a canonical command or an alias shorthand. */
export interface CommandSuggestion {
  readonly id: string;
  readonly labelKey: string | null;
  readonly icon: string;
  readonly supported: boolean;
  readonly group?: string;
  readonly target?: string;
}

export interface FilterCommandSuggestionsOptions {
  readonly history?: readonly string[];
  readonly limit?: number;
  readonly catalog?: readonly CommandCatalogEntry[];
  readonly aliases?: readonly CommandAlias[];
}

export interface AvailabilityOptions {
  readonly status?: CommandStatusView | null;
  readonly pending?: boolean;
}

export interface ToolCommandOptions {
  readonly toolActive?: boolean;
  /**
   * Whether a **grip drag** is armed (`cad_status.gripDrag`).
   *
   * A drag is not a tool (`tool` stays `null` while it is armed), so it only
   * widens `Escape`: the host drops the armed drag on `CANCEL`, and without this
   * flag the drag would be un-cancellable from the keyboard. `Enter` stays
   * tool-only, so a drag never sends a `CONFIRM` for a capture that does not exist.
   */
  readonly gripDragActive?: boolean;
  readonly typing?: boolean;
  readonly composing?: boolean;
  readonly repeat?: boolean;
}

/** What `cad_command` would do with a typed line. */
export type AdmissionKind = 'empty' | 'admitted' | 'unavailable' | 'unsupported';

export interface CommandAdmission {
  readonly kind: AdmissionKind;
  readonly command: string;
  readonly reasonKey: string | null;
}

interface ScoredCatalogSuggestion extends CommandCatalogEntry {
  supported: boolean;
  score: number;
  usage: number;
}

interface ScoredAliasSuggestion {
  id: string;
  target: string;
  icon: string;
  labelKey: string | null;
  supported: boolean;
  score: number;
  usage: number;
}

type ScoredSuggestion = ScoredCatalogSuggestion | ScoredAliasSuggestion;

/** Normalize typed input the same way the host does (trim + upper-case). */
export function normalizeCommandText(text: unknown): string {
  return String(text ?? '').trim().toUpperCase();
}

/**
 * Split a typed command line into its base name and typed argument.
 *
 * `ZOOM 2` -> `{ name: 'ZOOM', argument: '2', text: 'ZOOM 2' }`; a name-only
 * line keeps `argument: null`. One or more spaces/tabs separate the two and
 * surrounding whitespace is trimmed (via [`normalizeCommandText`]), while the
 * argument keeps its own internal text — `ZOOM 2 3` yields `'2 3'`, so the
 * argument predicate (not a second split) decides about it. This is the same
 * grammar the Rust router reads, so a value the chrome admits is the value the
 * host validates.
 */
export function splitCommandText(text: unknown): SplitCommandText {
  const normalized = normalizeCommandText(text);
  if (!normalized) return { name: '', argument: null, text: normalized };
  const match = /^([^ \t]+)[ \t]+(.+)$/.exec(normalized);
  return match
    ? { name: match[1], argument: match[2], text: normalized }
    : { name: normalized, argument: null, text: normalized };
}

/** Whether `text` is a plain positive decimal number (`2`, `0.5`, `1.25`). */
function isPositiveDecimal(text: string): boolean {
  if (!/^[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  const value = Number(text);
  return Number.isFinite(value) && value > 0;
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
});

/**
 * The loaded host vocabulary, uppercased and deduped. Empty until
 * [`setCommandCatalog`] receives the `cad_commands` answer.
 */
let loadedCatalog: readonly string[] = Object.freeze([]);

/** Replace the admission catalog with the names the host reported. */
export function setCommandCatalog(names?: Iterable<unknown> | null): readonly string[] {
  const seen = new Set<string>();
  for (const name of names ?? []) {
    const id = normalizeCommandText(name);
    if (id) seen.add(id);
  }
  loadedCatalog = Object.freeze([...seen]);
  return loadedCatalog;
}

/** The current admission catalog (empty until the host has answered). */
export function commandCatalog(): readonly string[] {
  return loadedCatalog;
}

/**
 * The host's AutoCAD-style alias table (`cad_command_aliases`), as frozen
 * `{ alias, target }` rows. Empty until [`setCommandAliases`] receives the
 * host's answer, and it stays empty when that answer fails — "no aliases known"
 * is the honest state, never a guessed table.
 *
 * Aliases are a **suggestion** aid only: admission still goes through
 * [`admitCommandText`] and the host's `cad_command`, which canonicalises the
 * typed text itself. A missing alias table therefore costs discoverability, never
 * correctness.
 */
let loadedAliases: readonly CommandAlias[] = Object.freeze([]);
// Bumped on every replacement so a memoized suggestion list recomputes when the
// table arrives (the rows are module state, not React state).
let aliasRevision = 0;

/**
 * Replace the alias table with the `{ alias, command }` pairs the host reported.
 *
 * Rows are normalized like typed text (trim + upper-case) and a row missing
 * either half is dropped rather than completed from a guess. The first row for
 * an alias wins: the host's table has no conflicts (it is contract-tested), so
 * this only bounds a malformed answer.
 */
export function setCommandAliases(pairs?: Iterable<CommandAliasInput> | null): readonly CommandAlias[] {
  const byAlias = new Map<string, CommandAlias>();
  for (const pair of pairs ?? []) {
    const alias = normalizeCommandText(pair?.alias);
    const target = normalizeCommandText(pair?.command);
    if (!alias || !target) continue;
    if (!byAlias.has(alias)) byAlias.set(alias, Object.freeze({ alias, target }));
  }
  loadedAliases = Object.freeze([...byAlias.values()]);
  aliasRevision += 1;
  return loadedAliases;
}

/** The current alias table (empty until the host has answered). */
export function commandAliases(): readonly CommandAlias[] {
  return loadedAliases;
}

/** A counter that changes whenever the alias table is replaced. */
export function commandAliasRevision(): number {
  return aliasRevision;
}

/**
 * The canonical command `text` is an alias for, or `null`.
 *
 * Case-insensitive and trimmed, exactly like the host's normalization. Only real
 * alias rows answer: a canonical name (which is not an alias) returns `null`, so
 * a caller can tell "this is a shorthand" from "this is already the command".
 */
export function aliasTarget(text: unknown): string | null {
  const alias = normalizeCommandText(text);
  if (!alias) return null;
  return loadedAliases.find(row => row.alias === alias)?.target ?? null;
}

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
 */
export const SELECTION_REQUIREMENTS: Readonly<Record<string, SelectionRequirement>> = Object.freeze({
  MATCHPROP: Object.freeze({ count: 2, reasonKey: 'draw.kind.matchprop.hint' }),
  JOIN: Object.freeze({ count: 2, reasonKey: 'draw.kind.join.hint' }),
  LAYISO: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYUNISO: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
});

/** The selection requirement of `id`, or `null` when it has none. */
export function selectionRequirement(id: unknown): SelectionRequirement | null {
  return SELECTION_REQUIREMENTS[normalizeCommandText(id)] ?? null;
}

/**
 * Whether the current selection satisfies `id`'s requirement.
 *
 * A command with no requirement, or one asked without a status (the host has not
 * answered yet), is never blocked here — an unknown selection is not evidence of
 * an empty one, and the status-less path already refuses everything it cannot
 * prove elsewhere. With **no drawing** it is always `false`: there is no
 * selection block to read, so the requirement can never be met (the callers say
 * "no drawing is open" in that case rather than "select more objects").
 */
export function selectionRequirementMet(
  id: unknown,
  status: CommandStatusView | null | undefined,
): boolean {
  const requirement = selectionRequirement(id);
  if (!requirement || !status) return true;
  if (status.documentLabel === null) return false;
  return selectionCount(status.selection) >= requirement.count;
}

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

/** Whether `id` is one of the chrome-local selectors above. */
export function isLocalChromeCommand(id: unknown): boolean {
  return LOCAL_CHROME_COMMANDS.includes(normalizeCommandText(id));
}

/**
 * The catalog key the chrome uses for "open a drawing first". The catalog
 * already words this sentence once (`plot.no_document`); reusing the key keeps
 * one wording for one fact instead of growing a second copy.
 */
export const NO_DOCUMENT_KEY = 'plot.no_document';

/**
 * The catalog key for "another operation is in progress" (`linux.busy`). A
 * capability can read `false` merely because the host is still importing or
 * publishing, and the honest sentence there is the busy one — never
 * `command.unsupported`, which would claim a permanent limitation for a
 * temporary state. The key is reused as-is; no new copy is invented here.
 */
export const BUSY_KEY = 'linux.busy';

export function isSupportedCommand(id: unknown): boolean {
  const name = normalizeCommandText(id);
  if (!name || GESTURE_ONLY_COMMANDS.includes(name)) return false;
  // A chrome-local selector is supported by definition: the shell answers it
  // without the router, so it cannot be missing from the host vocabulary.
  if (LOCAL_CHROME_COMMANDS.includes(name)) return true;
  return loadedCatalog.includes(name);
}

/**
 * Why a false capability blocks `name`: `canNew` needs no document, so a false
 * value there means the host cannot create a blank drawing at all;
 * `canSave`/`canPlot` also require an open document, and on the start page that
 * is the whole reason (with the status not loading — that case is [`BUSY_KEY`]).
 */
function capabilityRefusalKey(name: string, status: CommandStatusView): string {
  return CAPABILITY_GATES[name] === 'canNew' || status.documentLabel !== null
    ? 'command.unsupported'
    : NO_DOCUMENT_KEY;
}

/**
 * The catalog key for why `id` is unavailable in this chrome: the real gesture
 * hint for a drag-only command (`ORBIT`), the busy sentence while the host is
 * loading or a command is already in flight, "no drawing is open" for a
 * save/plot gate that only lacks a document, and otherwise the generic
 * unsupported reason. It keeps the refusal explicit instead of telling the user
 * a command "is not supported" when the real answer is "drag on the canvas" or
 * "a load is still running".
 *
 * This is the one reason table: the command line (through
 * [`admitCommandText`] and [`queueCadCommand`]), the ribbon hints and the QAT /
 * start-panel titles all resolve their sentence through it, so a disabled button
 * and a refused submission can never word the same fact differently.
 */
export function unavailableReasonKey(
  id: unknown,
  { status = null, pending = false }: AvailabilityOptions = {},
): string {
  const name = normalizeCommandText(id);
  if (GESTURE_ONLY_COMMANDS.includes(name)) return 'view.orbit.gesture';
  // Busy/loading is answered **before** the capability table, for every command.
  // A capability gate used to be the only way into this branch, so a gate-less
  // command (ERASE, MATCHPROP, …) told the reader "Command not available" while a
  // load or another command was merely still running — the permanent-limitation
  // sentence for a temporary state. The module's own contract is the opposite
  // ("the busy sentence while the host is loading or a command is already in
  // flight"), so the temporary fact is worded first.
  if (status && (pending || status.loading)) return BUSY_KEY;
  // A selection precondition is answered next: MATCHPROP has no capability gate,
  // yet with fewer than two selected objects the real reason is the selection, not
  // "unsupported". With no drawing open the honest reason is the catalog's own "no
  // drawing is open" sentence — there is no selection to count.
  const requirement = SELECTION_REQUIREMENTS[name];
  if (requirement && status) {
    if (status.documentLabel === null) return NO_DOCUMENT_KEY;
    if (!selectionRequirementMet(name, status)) return requirement.reasonKey;
  }
  const gate = CAPABILITY_GATES[name];
  if (!gate || !status) return 'command.unsupported';
  if (!status[gate]) return capabilityRefusalKey(name, status);
  return 'command.unsupported';
}

/**
 * Catalog key for a host tool selector (`draw.line` -> `tool.draw.line`), or
 * `null` when there is no selector so the caller renders no name.
 */
export function toolLabelKey(toolKey?: unknown): string | null {
  const key = String(toolKey ?? '').trim().toLowerCase();
  return key ? `tool.${key}` : null;
}

/**
 * The CAD command a global key press should submit while an interactive tool (or
 * an armed grip drag) owns the pointer, or `null` when the press is not ours.
 *
 * `Enter` confirms the active **tool**; `Escape` cancels whatever owns the
 * pointer — the active tool or an armed grip drag — but only when the key was not
 * typed into a field (`typing`), was not an IME composition, and was not an
 * auto-repeat. A missing tool *and* no drag returns `null` for every key, so
 * Escape keeps its panel-closing behaviour when nothing is in progress.
 *
 * The drag case deliberately only widens **Escape**: a grip drag is not a tool
 * (`cad_status.tool` is `null`), so it must not enable `CONFIRM` or any other
 * tool-only key. `CANCEL` is the one selector the host needs from the keyboard
 * here — it drops the armed drag as a pure state drop (`reset_grip_gesture`).
 */
export function toolCommandForKey(key: string, {
  toolActive = false,
  gripDragActive = false,
  typing = false,
  composing = false,
  repeat = false,
}: ToolCommandOptions = {}): 'CONFIRM' | 'CANCEL' | null {
  if (typing || composing || repeat) return null;
  if (key === 'Escape') return toolActive || gripDragActive ? 'CANCEL' : null;
  if (!toolActive) return null;
  if (key === 'Enter') return 'CONFIRM';
  return null;
}

/**
 * Every command name the app exposes in the chrome, in AutoCAD-style listing
 * order (file, draw, modify, view, measure, annotate, classify). `labelKey` is a
 * catalog key; `icon` names an entry in the icon set.
 *
 * This is a **display** table only: whether an entry is enabled comes from the
 * loaded host vocabulary ([`isSupportedCommand`]), so a command missing here can
 * still be typed and admitted when the router knows it.
 */
export const COMMAND_CATALOG: readonly CommandCatalogEntry[] = Object.freeze([
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', group: 'file' },
  { id: 'FIT', labelKey: 'toolbar.fit', icon: 'fit', group: 'view' },
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', group: 'file' },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', group: 'file' },
  // PLOT is the same router selector as the `EXPORT`/`PRINT` aliases (the host
  // writes SVG/PDF through the platform save picker and refuses `.png` for the
  // GUI). It sits after SAVE because it is a file-level output action, and it is
  // gated by `canPlot` like the aliases, not by anything local.
  { id: 'PLOT', labelKey: 'file.plot', icon: 'save', group: 'file' },
  { id: 'UNDO', labelKey: 'toolbar.undo', icon: 'undo', group: 'modify' },
  { id: 'REDO', labelKey: 'toolbar.redo', icon: 'redo', group: 'modify' },
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', group: 'draw' },
  { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', group: 'draw' },
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', group: 'draw' },
  { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', group: 'draw' },
  { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', group: 'draw' },
  { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', group: 'draw' },
  { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', group: 'draw' },
  { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', group: 'draw' },
  { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', group: 'draw' },
  // Single-line TEXT (ADR 0019 decision A): a point and then its text, which is
  // the command's terminal step and travels on `cad_set_tool_text`.
  { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', group: 'draw' },
  // MTEXT (2026-10-26): the multiline counterpart of TEXT. The bare selector
  // starts the two-corner tool (first corner = insertion point, opposite corner =
  // wrap width) and the text is the same terminal `cad_set_tool_text` step. It
  // has no variant entry: rotation/attachment/line spacing/columns/background are
  // deliberately not implemented, and nothing here may imply them.
  { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', group: 'draw' },
  // MLEADER (ADR 0019 decision F.2): two points — the arrowhead position, then
  // the landing point — and then that same terminal text step. It carries no
  // variant entry because the command has no `Options` sub-prompt (leader type /
  // content type / max points / angular constraint), and nothing here may imply
  // one.
  { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', group: 'draw' },
  { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', group: 'draw' },
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', group: 'modify' },
  { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', group: 'modify' },
  { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', group: 'modify' },
  { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', group: 'modify' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', group: 'modify' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', group: 'modify' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', group: 'modify' },
  { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', group: 'modify' },
  { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', group: 'modify' },
  { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', group: 'modify' },
  { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', group: 'modify' },
  { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', group: 'modify' },
  { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', group: 'modify' },
  { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', group: 'modify' },
  { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', group: 'modify' },
  { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', group: 'modify' },
  // MATCHPROP (the selection-based property painter) sits after the other
  // selection-acting modify commands. It is a **payload-free** router command, so
  // it is listed and typeable; whether it can run is decided by
  // `SELECTION_REQUIREMENTS` (it needs a source *and* at least one target), so a
  // one-object selection is refused with the sentence that says so.
  { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', group: 'modify' },
  // JOIN (2026-10-26) closes the modify run: welding the current selection into
  // one entity acts on that selection exactly like MATCHPROP, so it is appended
  // after it (no shipped row moves). It is a payload-free router command, and the
  // "at least two objects" rule is `SELECTION_REQUIREMENTS`' — the host refuses a
  // one-object join outright, so the button is never a live affordance that can
  // only fail. The glyph is the polyline one: the icon set has no dedicated join
  // mark yet, and the merge output is a curve either way.
  { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', group: 'modify' },
  { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', group: 'measure' },
  { id: 'MEASUREPOLYLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', group: 'measure' },
  { id: 'MEASUREANGLE', labelKey: 'measure.kind.angle', icon: 'angle', group: 'measure' },
  { id: 'AREA', labelKey: 'measure.kind.area', icon: 'area', group: 'measure' },
  // ID (2026-10-26) is the point-coordinate readout: one point pick, reported as
  // X/Y/Z plus the object snap that produced it. It is the fifth measurement kind
  // and is listed last there so no shipped entry moves. It reuses the `point`
  // glyph (a crosshair on a marker) because the command is "which point is this",
  // and it has no variant popover: there is no options sub-prompt.
  { id: 'ID', labelKey: 'measure.kind.id', icon: 'point', group: 'measure' },
  { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', group: 'annotate' },
  { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', group: 'annotate' },
  { id: 'DIMRADIUS', labelKey: 'draw.kind.dimradius', icon: 'dimradius', group: 'annotate' },
  { id: 'DIMDIAMETER', labelKey: 'draw.kind.dimdiameter', icon: 'dimdiameter', group: 'annotate' },
  { id: 'DIMCENTER', labelKey: 'draw.kind.dimcenter', icon: 'dimcenter', group: 'annotate' },
  { id: 'DIMANGULAR', labelKey: 'draw.kind.dimangular', icon: 'dimangular', group: 'annotate' },
  { id: 'DIMORDINATE', labelKey: 'draw.kind.dimordinate', icon: 'dimordinate', group: 'annotate' },
  // Baseline/continue dimensions (ADR 0017 decision C, P5f): both pick an
  // existing linear dimension first, then accept any number of points and commit
  // with Enter — never a fixed three-point capture like the linear pair.
  { id: 'DIMBASELINE', labelKey: 'draw.kind.dimbaseline', icon: 'dimbaseline', group: 'annotate' },
  { id: 'DIMCONTINUE', labelKey: 'draw.kind.dimcontinue', icon: 'dimcontinue', group: 'annotate' },
  { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', group: 'annotate' },
  // Layer isolate / unisolate (2026-10-26). Unlike `LAYER` they are **router**
  // commands (`LAYISO`/`LAYUNISO`), not chrome-local selectors, so they are
  // admitted by the host vocabulary alone. Their glyphs are the layers palette's
  // own visibility pair — isolating hides the other layers (dimmed eye),
  // unisolating shows them again (plain eye) — because the operation is a
  // visibility change, not a palette toggle.
  { id: 'LAYISO', labelKey: 'layers.isolate', icon: 'eyeOff', group: 'annotate' },
  { id: 'LAYUNISO', labelKey: 'layers.unisolate', icon: 'eye', group: 'annotate' },
  { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', group: 'annotate' },
  { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', group: 'annotate' },
  { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', group: 'view' },
  { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', group: 'view' },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', group: 'view' },
  // RESETVIEW is a payload-free router command (`CommandId::ResetView`) that
  // restores the drawing's saved view; it has no capability gate and no
  // document-scoped precondition of its own.
  { id: 'RESETVIEW', labelKey: 'view.reset', icon: 'view', group: 'view' },
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', group: 'view' },
  // 3D view selectors: both are payload-free router commands (typing them in the
  // command line already works), and both need an open drawing — the ribbon
  // disables them on the start page. `STANDARDVIEW` is deliberately **absent**:
  // it carries a payload and travels on `cad_standard_view`, never on
  // `cad_command`, so it is not a name this catalog may offer for typing.
  { id: 'SWITCH2D3D', labelKey: 'ribbon.command.view_switch2d3d', icon: 'view', group: 'view' },
  { id: 'PROJECTION', labelKey: 'view.projection', icon: 'axes', group: 'view' },
  { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', group: 'view' },
  { id: 'SNAP', labelKey: 'status.toggle.snap', icon: 'snap', group: 'view' },
  { id: 'ORTHO', labelKey: 'status.toggle.ortho', icon: 'angle', group: 'view' },
  { id: 'OSNAP', labelKey: 'status.toggle.osnap', icon: 'snap', group: 'view' },
]);

const CATALOG_BY_ID = new Map<string, CommandCatalogEntry>(COMMAND_CATALOG.map(command => [command.id, command]));

export function commandById(id: unknown): CommandCatalogEntry | null {
  return CATALOG_BY_ID.get(normalizeCommandText(id)) ?? null;
}

/**
 * The suggestion rank of an alias row: after canonical prefix matches (`0`),
 * before canonical mid-string matches (`1`). See [`filterCommandSuggestions`].
 */
const ALIAS_SUGGESTION_SCORE = 0.5;

/**
 * Autocomplete over the command catalog **plus** the host's alias table.
 * Ordering follows the command-window convention: prefix matches before
 * mid-string matches, then commands used more often in history float up, then
 * alphabetical for stability. `query` is the raw typed text.
 *
 * Alias rows (`{ id: <alias>, target: <canonical> }`) are matched as a *prefix*
 * of the alias only (`l` finds `L`, `L → LINE`; `ine` does not). They rank
 * between canonical prefix matches and canonical mid-string matches, so an exact
 * canonical match is never pushed down by a shorthand, and a shorthand is never
 * starved by a canonical name that merely *contains* the needle (`PLINE` for
 * `l`). Among themselves they follow the same usage-frequency order. The `limit`
 * applies to the merged list. An empty alias table (or a failed
 * `cad_command_aliases` answer) changes nothing but the extra rows.
 */
export function filterCommandSuggestions(query: unknown, {
  history = [],
  limit = 8,
  catalog = COMMAND_CATALOG,
  aliases = commandAliases(),
}: FilterCommandSuggestionsOptions = {}): CommandSuggestion[] {
  // The needle is the **base name**: a typed argument belongs to the
  // submission, not to the lookup, so `ZOOM 2` keeps suggesting `ZOOM` while
  // the line (argument included) is what reaches the host.
  const needle = splitCommandText(query).name;
  const usage = new Map<string, number>();
  history.forEach((entry, index) => {
    const id = splitCommandText(entry).name;
    usage.set(id, (usage.get(id) ?? 0) + (history.length - index));
  });
  const matches: ScoredSuggestion[] = [];
  for (const command of catalog) {
    const at = needle ? command.id.indexOf(needle) : 0;
    if (needle && at === -1) continue;
    matches.push({
      ...command,
      supported: isSupportedCommand(command.id),
      score: needle ? (at === 0 ? 0 : 1) : 2,
      usage: usage.get(command.id) ?? 0,
    });
  }
  // Alias rows only make sense once something is typed: an empty query is the
  // browse-the-catalog case, not a shorthand lookup.
  if (needle) {
    for (const { alias, target } of aliases) {
      if (!alias.startsWith(needle)) continue;
      const command = commandById(target);
      matches.push({
        id: alias,
        target,
        icon: command?.icon ?? 'terminal',
        labelKey: command?.labelKey ?? null,
        supported: isSupportedCommand(alias),
        score: ALIAS_SUGGESTION_SCORE,
        usage: usage.get(alias) ?? 0,
      });
    }
  }
  matches.sort((a, b) => a.score - b.score || b.usage - a.usage || a.id.localeCompare(b.id));
  return matches.slice(0, limit).map(({ score, usage: _usage, ...rest }) => rest);
}

/** The suggestion shown highlighted after `query` changes. */
export function defaultSuggestionIndex(suggestions: readonly CommandSuggestion[], query: unknown): number {
  if (!suggestions.length) return -1;
  // A typed argument does not change which name is highlighted: `ZOOM 2` still
  // highlights the `ZOOM` row.
  const needle = splitCommandText(query).name;
  if (!needle) return 0;
  const exact = suggestions.findIndex(suggestion => suggestion.id === needle);
  return exact === -1 ? 0 : exact;
}

/**
 * Step the history cursor. `null` means "the draft line". Up moves to older
 * entries, Down moves back toward the draft.
 */
export function stepHistoryCursor(history: readonly string[], index: number | null, direction: number): number | null {
  if (!history.length) return null;
  const size = history.length;
  if (direction < 0) {
    const base = index === null ? size : index;
    return Math.max(0, base - 1);
  }
  if (index === null) return null;
  const next = index + 1;
  return next >= size ? null : next;
}

/** Text for the current history cursor position. */
export function historyText(history: readonly string[], index: number | null, draft: string): string {
  return index === null || index < 0 || index >= history.length ? draft : history[index];
}

/** Push `entry` onto a bounded history (newest last, no adjacent duplicates). */
export function pushHistory(history: readonly string[], entry: unknown, limit = 64): readonly string[] {
  const value = String(entry ?? '').trim();
  if (!value) return history;
  const withoutDuplicate = history.at(-1) === value ? history : [...history, value];
  return withoutDuplicate.length > limit ? withoutDuplicate.slice(withoutDuplicate.length - limit) : withoutDuplicate;
}

/**
 * Admission for the command line: what would `cad_command` do with this text?
 * Uses the vocabulary loaded from the host ([`isSupportedCommand`]) plus the
 * capability gates, without touching IPC.
 *
 * A line may carry the one typed argument form ([`ARGUMENT_COMMANDS`]):
 * admission runs on the **base name** through the same vocabulary/capability
 * gates, and the returned `command` is the **full normalized line** so the host
 * receives (and validates) the value itself. An argument on a selector that
 * takes none, or an argument outside the local syntax predicate, is an explicit
 * `unsupported` answer with the shared reason key — the host remains the
 * authority for semantics, and a bare `ZOOM` keeps its existing payload-refusal
 * path unchanged.
 *
 * Every non-admitted answer carries the `reasonKey` that words **why** (see
 * [`unavailableReasonKey`]) so the caller never has to re-derive — or invent — a
 * second explanation for the same refusal.
 */
export function admitCommandText(text: unknown, { status = null, pending = false }: AvailabilityOptions = {}): CommandAdmission {
  const { name: id, argument, text: command } = splitCommandText(text);
  if (!id) return { kind: 'empty', command: id, reasonKey: null };
  // A typed argument is gated by its base selector and its local grammar before
  // anything else: `LINE 2` and `ZOOM 0` never reach IPC.
  if (argument !== null) {
    const spec = ARGUMENT_COMMANDS[id];
    if (!spec || !spec.parse(argument)) {
      return { kind: 'unsupported', command, reasonKey: 'command.unsupported' };
    }
  }
  // A local selector is admitted without a status/document: it only toggles
  // chrome. The submit path intercepts it before `queueCadCommand`.
  if (LOCAL_CHROME_COMMANDS.includes(id)) return { kind: 'admitted', command, reasonKey: null };
  if (!isSupportedCommand(id)) {
    return { kind: 'unsupported', command, reasonKey: unavailableReasonKey(id, { status, pending }) };
  }
  // No status means the host has not answered yet: nothing is admitted on a
  // guess, and the honest reason is that this build cannot say yet.
  if (!status) return { kind: 'unavailable', command, reasonKey: 'command.unsupported' };
  if (pending || status.loading) return { kind: 'unavailable', command, reasonKey: BUSY_KEY };
  if (id === 'OPEN' && !status.canOpen) return { kind: 'unavailable', command, reasonKey: 'command.unsupported' };
  const gate = CAPABILITY_GATES[id];
  if (gate && !status[gate]) {
    return { kind: 'unavailable', command, reasonKey: capabilityRefusalKey(id, status) };
  }
  if (DOCUMENT_SCOPED_COMMANDS.includes(id) && status.documentLabel === null) {
    return { kind: 'unavailable', command, reasonKey: NO_DOCUMENT_KEY };
  }
  // A selection precondition is checked before the command reaches the host: with
  // no drawing open there is no selection to read, and with too small a selection
  // MATCHPROP can only be refused (`matchprop.needs_two`). Either way the line is
  // answered here with the sentence that says what to do first.
  const requirement = SELECTION_REQUIREMENTS[id];
  if (requirement && !selectionRequirementMet(id, status)) {
    return {
      kind: 'unavailable',
      command,
      reasonKey: status.documentLabel === null ? NO_DOCUMENT_KEY : requirement.reasonKey,
    };
  }
  if (id === 'FIT' && !(status.canFit && status.documentLabel !== null && status.entityCount !== null)) {
    return { kind: 'unavailable', command, reasonKey: 'command.unsupported' };
  }
  return { kind: 'admitted', command, reasonKey: null };
}
