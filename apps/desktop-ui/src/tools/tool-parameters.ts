// The active tool's typed parameters (ADR 0016 decision A): the status-bar views,
// the bounded pending delta, the typed-entry draft model and the one IPC that
// writes a value.
//
// The declarative half — the control spec table mirrored from the frozen host
// table, and which parameters each tool owns — lives in the same-directory
// sibling `tool-parameter-specs.ts`; this module imports it directly and owns the
// functions that read it.
import type { CadStatus, CadTool, Invoke } from '../cad/cad-status.ts';
import { TOOL_PARAMETERS, TOOL_PARAMETER_SPECS } from './tool-parameter-specs.ts';
import type {
  FlagToolParameterSpec, NumberToolParameterSpec, ToolParameterSpec,
} from './tool-parameter-specs.ts';

/**
 * The one general typed tool-parameter IPC (ADR 0016 decision A). It replaces
 * the removed narrow, polygon-only side-count IPC: every parameter now travels
 * this channel, carrying its stable machine key plus exactly one of
 * `number`/`flag`.
 *
 * `{accepted:true}` means **queued**, exactly like `cad_command` — never
 * completed. A malformed request (an unknown key, or neither/both value fields)
 * rejects with a string; the per-parameter range/kind and Viewer-mode refusals
 * surface later through `cad_status.error`.
 */
export const TOOL_PARAMETER_COMMAND = 'cad_set_tool_parameter';

// Cap on the pending delta magnitude, shared by every numeric parameter (it
// replaces the POLYGON-only cap). 10 keeps a stuck queue from accumulating
// unbounded intent — at most ten un-echoed steps, i.e. a ~2.5 s burst at the
// 250 ms poll — while staying well above a normal click cadence, so the host
// virtually always catches up before the cap is ever felt.
export const TOOL_PARAMETER_PENDING_CAP = 10;

/** The live boolean of a flag `spec` from the active tool, or `null`. */
function projectedFlag(spec: FlagToolParameterSpec, tool: CadTool | null | undefined): boolean | null {
  const raw = tool?.params?.[spec.statusKey];
  return typeof raw === 'boolean' ? raw : null;
}

/** The live number of a numeric `spec` from the active tool, or `null`. */
function projectedNumber(spec: NumberToolParameterSpec, tool: CadTool | null | undefined): number | null {
  const raw = tool?.params?.[spec.statusKey];
  const projectable = spec.projectable ?? spec.offered;
  return typeof raw === 'number' && projectable(raw) ? raw : null;
}

/** A flag parameter's status-bar view. */
export interface ToolParameterFlagView {
  readonly kind: 'flag';
  readonly key: string;
  readonly value: boolean;
  /** The value a dispatch would set (always the opposite of `value`). */
  readonly next: boolean;
  readonly labelKey: string;
  readonly hintKey: string;
  readonly icon?: string;
  /**
   * Whether the row sits outside the sibling code that scopes it (`activeWhen`):
   * always `false` for a row without one. Scope, not capability — the toggle still
   * dispatches (the host accepts the write and simply ignores it), but the chip is
   * muted and its title says the current setting does not use it.
   */
  readonly inactive: boolean;
  readonly disabled: boolean;
}

/**
 * One named code of a closed-set parameter: the machine value the IPC carries
 * plus the catalog key of the name a user reads.
 *
 * The `value` is the integer the one parameter IPC sends, exactly as a step would
 * send it, so naming a code never changes what travels on the wire.
 */
export interface ToolParameterChoice {
  /** The integer code the host accepts — exactly the value the IPC carries. */
  readonly value: number;
  /** Catalog key of the code's name (present in both catalogs). */
  readonly labelKey: string;
  /**
   * Whether the row currently stands on this code: the projected status value plus
   * the bounded pending delta. A pick therefore shows at once, and the host's next
   * echo — or an explicit refusal, which drops the delta — corrects it.
   */
  readonly selected: boolean;
  /** Whether picking it would dispatch: command admission **and** the row's scope. */
  readonly disabled: boolean;
}

/** A numeric parameter's status-bar view: a bounded stepper, optionally a named chooser. */
export interface ToolParameterNumberView {
  readonly kind: 'number';
  readonly key: string;
  readonly value: number;
  readonly delta: number;
  readonly effective: number;
  readonly text: string;
  /** The increment the `-`/`+` buttons and the arrow keys use. */
  readonly step: number;
  /** Whether the value may also be typed in place (a real length), not only stepped. */
  readonly typed: boolean;
  /**
   * Whether the row is a **colour swatch** rather than a number to step (`spec.swatch`).
   * The wire value is still the integer ACI index this view carries; the row paints
   * that index from the shared palette and opens the shared colour chooser, which
   * can only hand back another index of the same palette.
   */
  readonly swatch: boolean;
  /**
   * The named closed set this row chooses among, in the host's own code order, or
   * `null` for a plain stepper. A named row is still a numeric row — the same
   * `choices` the `offered` predicate keeps, the same one IPC — so the chooser can
   * never offer a code a step could not, and the bounds below stay meaningful.
   */
  readonly choices: readonly ToolParameterChoice[] | null;
  /**
   * How the named set is laid out, or `null` for a plain stepper: `'segments'` is
   * the one-row chooser, `'grid'` the compact chip that opens the names as a grid
   * (MTEXT's 3×3 attachment codes — see the spec's `choiceLayout`). Presentation
   * only: a pick is the same integer through the same one IPC either way.
   */
  readonly choiceLayout: 'segments' | 'grid' | null;
  /**
   * Whether the row sits outside the sibling code that scopes it (`activeWhen`):
   * always `false` for a row without one. This is **scope, not capability** — the
   * host would still accept a write — so it is reported separately from `disabled`
   * (which is command admission) and only mutes the chooser.
   */
  readonly inactive: boolean;
  readonly labelKey: string;
  readonly hintKey: string;
  readonly icon?: string;
  readonly disabled: boolean;
  readonly decrement: number;
  readonly increment: number;
  readonly canDecrement: boolean;
  readonly canIncrement: boolean;
}

/** One status-bar parameter view. */
export type ToolParameterView = ToolParameterFlagView | ToolParameterNumberView;

/** A bounded pending step the host has not echoed back yet. */
export interface ToolParameterPending {
  readonly key: string;
  /** The status value the delta is measured against. */
  readonly base: number;
  readonly delta: number;
}

/**
 * The status-bar views for every parameter of the currently active tool.
 *
 * Returns `[]` unless a tool is active with a map entry; each parameter yields a
 * numeric view (a bounded stepper, or a named chooser when its spec names the
 * codes) or a flag view, and a parameter whose value is absent (not
 * projected, `null`, or malformed/out of range) yields nothing at all — never an
 * invented default. `admitted(command)` is the shell's normal command admission
 * (`canCommand`), which already accounts for a pending command and a
 * loading/unavailable host or an unloaded vocabulary — the same gate the toggle
 * chips use.
 *
 * A numeric view's `value` (the *rendered* number) is always the real status
 * value and is never optimistically advanced. `effective` is `value + delta`,
 * where `delta` is the bounded pending magnitude the host has not echoed back yet
 * (see [`stepToolParameterPending`]); it only shifts the target the next step
 * builds on and the button bounds, so a rapid double-click composes into
 * `value + 2·step` while the displayed number still follows the status. A named
 * chooser marks the code at `effective`, so a pick reads as chosen at once and the
 * host's echo (or its refusal, which drops the delta) settles it.
 */
export function toolParameterViews(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean | ((command: string) => boolean),
  pending: ToolParameterPending | null = null,
): ToolParameterView[] {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return [];
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry) return [];
  const disabled = !(typeof admitted === 'function' ? admitted(entry.command) : admitted === true);
  const views: ToolParameterView[] = [];
  for (const key of entry.keys) {
    const spec = TOOL_PARAMETER_SPECS[key];
    if (!spec) continue;
    const view = parameterView(key, spec, tool, disabled, pending);
    if (view) views.push(view);
  }
  return views;
}

/**
 * Whether a row scoped by `activeWhen` applies under the current sibling code.
 *
 * An absent, `null` or non-numeric sibling value never claims the row applies: the
 * strip can only make a scope claim it can prove from `cad_status`, and the host
 * projects every parameter of an active tool together.
 */
function activeUnder(spec: ToolParameterSpec, tool: CadTool): boolean {
  const condition = spec.activeWhen;
  if (!condition) return true;
  const raw = tool.params?.[condition.statusKey];
  return typeof raw === 'number' && condition.values.includes(raw);
}

/**
 * The named choices of a closed set, or `null` when the row is a plain stepper.
 *
 * The codes come from the spec's own `choices` — the same list the `offered`
 * predicate keeps — so the chooser can never show a code the host would refuse,
 * and it lists them all: a code without a catalog name (a broken table) demotes
 * the whole row to the stepper instead of silently dropping a value the host
 * accepts. Every name is a catalog key, never copy: this module carries none.
 */
function choiceViews(
  spec: NumberToolParameterSpec,
  selectedCode: number,
  disabled: boolean,
  inactive: boolean,
): readonly ToolParameterChoice[] | null {
  const keys = spec.choiceLabelKeys;
  const codes = spec.choices;
  if (!keys || !codes) return null;
  const choices: ToolParameterChoice[] = [];
  for (const value of codes) {
    const labelKey = keys[value];
    if (typeof labelKey !== 'string' || labelKey.length === 0) return null;
    choices.push({
      value,
      labelKey,
      selected: value === selectedCode,
      disabled: disabled || inactive,
    });
  }
  return Object.freeze(choices);
}

function parameterView(
  key: string,
  spec: ToolParameterSpec,
  tool: CadTool,
  disabled: boolean,
  pending: ToolParameterPending | null,
): ToolParameterView | null {
  if (spec.kind === 'flag') {
    const value = projectedFlag(spec, tool);
    if (value === null) return null;
    return {
      kind: 'flag', key, value, next: !value,
      inactive: !activeUnder(spec, tool),
      labelKey: spec.labelKey, hintKey: spec.hintKey, icon: spec.icon, disabled,
    };
  }
  const value = projectedNumber(spec, tool);
  if (value === null) return null;
  const delta = pending && pending.key === key && pending.base === value ? pending.delta : 0;
  const effective = value + delta;
  const inactive = !activeUnder(spec, tool);
  const choices = choiceViews(spec, effective, disabled, inactive);
  return {
    kind: 'number', key, value, delta, effective, text: String(value),
    step: spec.step, typed: spec.typed === true, swatch: spec.swatch === true,
    choices,
    // A row without a named set has no layout (`null`); a named one keeps the
    // spec's own — `'segments'` unless it asks for the compact grid.
    choiceLayout: choices ? (spec.choiceLayout ?? 'segments') : null,
    inactive,
    labelKey: spec.labelKey, hintKey: spec.hintKey, icon: spec.icon, disabled,
    decrement: effective - spec.step,
    increment: effective + spec.step,
    // A step is offered only while the *effective* target is a value the host
    // accepts and the pending magnitude is under the cap: it is disabled, never
    // clamped into a different value.
    canDecrement: spec.offered(effective - spec.step) && delta > -TOOL_PARAMETER_PENDING_CAP,
    canIncrement: spec.offered(effective + spec.step) && delta < TOOL_PARAMETER_PENDING_CAP,
  };
}

/**
 * Reconcile the bounded pending delta against the latest real status.
 *
 * Returns the same `pending` while the host has not yet caught up (the status
 * value still equals the value the delta is measured against, `base`), and
 * `null` — dropping the delta — as soon as the value changes, the tool
 * switches/cancels, the parameter leaves the active tool's map, the value becomes
 * absent/`null`/invalid, or the status carries an `error`. The error rule matters
 * because an in-queue host refusal (queue full, Viewer edge) surfaces through
 * `status.error` **without** changing the value: without it the delta would stay
 * stuck at its `base` and the next click would compose an absolute target up to
 * the cap away from the real value. Returning the same reference on an unchanged
 * poll lets the caller `setState` without triggering a re-render.
 */
export function reconcileToolParameterPending(
  pending: ToolParameterPending | null | undefined,
  status: Partial<CadStatus> | null | undefined,
): ToolParameterPending | null {
  if (!pending || typeof pending !== 'object') return null;
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  // A real host error (never an empty/absent string) drops the in-flight delta,
  // so the next step builds on the value the host actually holds.
  if (status?.error) return null;
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry || !entry.keys.includes(pending.key)) return null;
  const spec = TOOL_PARAMETER_SPECS[pending.key];
  if (!spec || spec.kind !== 'number') return null;
  const value = projectedNumber(spec, tool);
  if (value === null || value !== pending.base) return null;
  return pending;
}

/**
 * Fold one step into the pending delta, or `null` when there is nothing to track.
 *
 * `target` is the value a step dispatches (`effective ± step` from the view); the
 * new delta is `target - base`, measured against the current status value. A zero
 * delta (a step that lands back on the status value) and any delta beyond
 * [`TOOL_PARAMETER_PENDING_CAP`] collapse to `null`, so the cap can never be
 * exceeded even if a caller bypasses the view's disabled buttons.
 *
 * `pending` is the in-flight delta the caller's `target` was built on. Folding is
 * only meaningful onto the same parameter and the same `base` value: a mismatch
 * means the caller stepped off a stale target (the same class of drift the
 * reconcile guards against), so the step is refused rather than composed into an
 * absolute target the host never saw.
 */
export function stepToolParameterPending(
  pending: ToolParameterPending | null | undefined,
  status: Partial<CadStatus> | null | undefined,
  key: string,
  target: number,
): ToolParameterPending | null {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry || !entry.keys.includes(key)) return null;
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number') return null;
  const base = projectedNumber(spec, tool);
  if (base === null) return null;
  if (pending && (pending.key !== key || pending.base !== base)) return null;
  const delta = target - base;
  if (!Number.isFinite(delta) || delta === 0 || Math.abs(delta) > TOOL_PARAMETER_PENDING_CAP) return null;
  return { key, base, delta };
}

/** What one tool-parameter IPC did: queue admission, an explicit refusal, a failure. */
export type ToolParameterResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'refused'; readonly error: string }
  | { readonly kind: 'failed'; readonly error: string };

/**
 * Ask the host to set one numeric tool parameter.
 *
 * `cad_set_tool_parameter` is the one IPC for every parameter. A
 * `{accepted:true}` result is **queue admission**, exactly like `cad_command`:
 * the next real `cad_status` carries the new value, so the caller must not
 * optimistically mirror it. A value the host would refuse is refused here before
 * IPC (the host refuses it too rather than clamping); a pre-queue refusal rejects
 * with a string, returned as an explicit `failed` — never a silent success or a
 * fabricated local value.
 */
export async function setToolParameterNumber(
  invoke: Invoke,
  key: string,
  number: number,
): Promise<ToolParameterResult> {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number' || !spec.offered(number)) {
    return { kind: 'refused', error: 'CAD_TOOL_PARAMETER_OUT_OF_RANGE' };
  }
  try {
    const response = await invoke(TOOL_PARAMETER_COMMAND, { parameter: key, number });
    if (!response || (response as { accepted?: unknown }).accepted !== true) throw Error('CAD_TOOL_PARAMETER_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * Ask the host to set one flag tool parameter.
 *
 * The caller passes the **opposite** of the value it read from the status (never
 * a fabricated one). Admission semantics match [`setToolParameterNumber`]: the
 * result is queue admission, and a refusal is explicit.
 */
export async function setToolParameterFlag(
  invoke: Invoke,
  key: string,
  flag: boolean,
): Promise<ToolParameterResult> {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'flag' || typeof flag !== 'boolean') {
    return { kind: 'refused', error: 'CAD_TOOL_PARAMETER_KIND' };
  }
  try {
    const response = await invoke(TOOL_PARAMETER_COMMAND, { parameter: key, flag });
    if (!response || (response as { accepted?: unknown }).accepted !== true) throw Error('CAD_TOOL_PARAMETER_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * The catalog key of the refusal shown for a typed value the host would refuse.
 *
 * It is a catalog key, not copy: the strip translates it with the parameter's own
 * localized name, so the message can never drift from the label it is about.
 */
export const TOOL_PARAMETER_INVALID_KEY = 'status.tool_parameter.invalid';

/**
 * The catalog key of the reason a scoped row is inert right now.
 *
 * It is a catalog key, not copy: the strip prefixes it to the row's own hint, so a
 * greyed chooser says *why* it is greyed without a second hand-written sentence
 * per parameter. Scope is not capability — the host would still accept the write —
 * so the sentence talks about what the current setting uses, never about support.
 */
export const TOOL_PARAMETER_INACTIVE_KEY = 'status.tool_parameter.inactive';

/**
 * The local draft of one typed numeric parameter.
 *
 * A draft is **text the user is typing**, never a copy of the host value: while
 * `text` is `null` the field renders the live `cad_status` number, and once the
 * user types, only what they typed is shown — a poll landing mid-edit can never
 * overwrite it. `messageKey` is the catalog refusal the field is currently
 * showing, and it is cleared by the next keystroke and by Escape.
 */
export interface ToolParameterDraft {
  /** What the field shows instead of the host value, or `null` when untouched. */
  readonly text: string | null;
  /** Catalog key of the refusal being shown, or `null` when there is none. */
  readonly messageKey: string | null;
}

/** No draft: the field shows the host value and no refusal. */
export const NO_TOOL_PARAMETER_DRAFT: ToolParameterDraft = Object.freeze({ text: null, messageKey: null });

/** Fold one keystroke into the draft; typing clears the previous refusal. */
export function typeToolParameterDraft(_draft: ToolParameterDraft | null | undefined, text: string): ToolParameterDraft {
  return { text: typeof text === 'string' ? text : '', messageKey: null };
}

/**
 * Drop the draft: Escape (and a `-`/`+` step, which is an absolute target built
 * from the status) puts the host value back in the field.
 */
export function cancelToolParameterDraft(): ToolParameterDraft {
  return NO_TOOL_PARAMETER_DRAFT;
}

/** What one commit of the draft decided. */
export type ToolParameterDraftCommit =
  /** Nothing was typed: no write, no message — an untouched field's blur. */
  | { readonly kind: 'idle' }
  /** Refused before dispatch: the draft keeps the text and names the message. */
  | { readonly kind: 'refused'; readonly draft: ToolParameterDraft }
  /** Accepted by the parameter's own guard: the caller dispatches `number`. */
  | { readonly kind: 'write'; readonly number: number; readonly draft: ToolParameterDraft };

// A plain decimal (optionally signed, optionally with an exponent) and nothing
// else. This is the field's **syntax**, not its range: `1.75` and `2` parse, an
// empty field and `abc` do not, and the range stays exactly the host's own
// `offered` predicate below — no stricter and no looser bound is invented here.
const DECIMAL_TEXT = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;

/**
 * Decide what one commit of a typed value does — never a silent clamp.
 *
 * `0`, a negative number, `NaN`, an empty field, a non-numeric string and any
 * value the parameter's own `offered` predicate refuses all come back `refused`
 * with [`TOOL_PARAMETER_INVALID_KEY`], so the caller shows the catalog message
 * and **nothing is dispatched**. A value the host accepts (including a decimal
 * such as `1.75`) is returned **verbatim** as a number: no rounding, no
 * re-derivation from the steps.
 */
export function commitToolParameterDraft(
  key: string,
  draft: ToolParameterDraft | null | undefined,
): ToolParameterDraftCommit {
  const raw = typeof draft?.text === 'string' ? draft.text.trim() : null;
  if (raw === null) return { kind: 'idle' };
  const spec = TOOL_PARAMETER_SPECS[key];
  const number = DECIMAL_TEXT.test(raw) ? Number(raw) : Number.NaN;
  if (!spec || spec.kind !== 'number' || !spec.offered(number)) {
    return { kind: 'refused', draft: { text: draft?.text ?? raw, messageKey: TOOL_PARAMETER_INVALID_KEY } };
  }
  return { kind: 'write', number, draft: NO_TOOL_PARAMETER_DRAFT };
}

/**
 * The value one arrow-key step from the field offers, or `null` when the step is
 * not a value the host accepts.
 *
 * The step starts from the draft when the field holds a usable number and from
 * `base` (the view's effective target, the same one the `-`/`+` buttons step
 * from) otherwise, so an arrow key never composes off half-typed text. A target
 * outside the parameter's range is simply not offered — refused, never clamped.
 */
export function stepToolParameterDraft(
  key: string,
  draft: ToolParameterDraft | null | undefined,
  base: number,
  direction: 1 | -1,
): number | null {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number') return null;
  const typed = commitToolParameterDraft(key, draft);
  const from = typed.kind === 'write' ? typed.number : base;
  const target = from + direction * spec.step;
  return Number.isFinite(target) && spec.offered(target) ? target : null;
}
