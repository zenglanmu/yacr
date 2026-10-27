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
//
// The declarative tables live in same-directory siblings — `catalog.ts` (the display
// catalog) and `gating.ts` (the refusal/gating rows) — which this module imports
// directly. The functions that read them all stay here.
import { selectionCount } from '../cad/selection-wire.ts';
import { COMMAND_CATALOG } from './catalog.ts';
import type { CommandCatalogEntry } from './catalog.ts';
import {
  ARGUMENT_COMMANDS, CAPABILITY_GATES, DOCUMENT_SCOPED_COMMANDS, GESTURE_ONLY_COMMANDS,
  LOCAL_CHROME_COMMANDS, SELECTION_REQUIREMENTS,
} from './gating.ts';
import type { CommandStatusView, SelectionRequirement } from './gating.ts';

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

/** Whether `id` is one of the chrome-local selectors (`LOCAL_CHROME_COMMANDS`). */
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
  const split = splitCommandText(text);
  const command = split.text;
  // A canonical selector may contain a space (`MEASURE AREA`, `AREA OBJECT`,
  // `ZOOM EXTENTS`). The host router resolves the **whole** trimmed line against its
  // command table first and only then falls back to base-plus-argument, so the
  // chrome must use the same order: otherwise a real selector is refused locally as
  // "`BASE` with a bad argument" while the host would have accepted it.
  const whole = split.argument !== null && isSupportedCommand(command) ? command : null;
  const id = whole ?? split.name;
  const argument = whole === null ? split.argument : null;
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
