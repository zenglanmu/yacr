import { MTEXT_STEP_PROMPT_KEY, MTEXT_STEP_PROMPT_KEYS } from './text.ts';
import { TEXT_PICK_PROMPT_KEY, TEXT_STEP_PROMPT_KEY, textStepPromptKey, toolAwaitingText } from './text.ts';
import { OPEN_ENDED_PROMPTS } from './tool-open-ended.ts';
import type { CadStatus, CadTool, Invoke } from './cad-status.ts';

export { propertyGroupLabel, propertyRowLabel } from './property-labels.ts';
export { parseSelection } from './selection-wire.ts';
// The status validator and the import-progress/cancel affordances live in
// `cad-status.ts`, and the command funnel in `cad-command.ts` (this file is at
// the size ceiling); re-exporting keeps the chrome's import surface stable.
export { importProgressView, parseCadStatus, requestImportCancel } from './cad-status.ts';
export { commandEnabled, handleCommandKey, queueCadCommand, startCadPolling } from './cad-command.ts';
export type { CadCommandResult, CommandKeyEvent, PollClock } from './cad-command.ts';
export type {
  CadCamera, CadCommandLogEntry, CadImportProgress, CadMeasurementEcho, CadPopup, CadSelection,
  CadSelectionRow, CadSelectionRowKind, CadStatus, CadTool, CadToolParams, ImportCancelResult,
  ImportProgressView, Invoke, WorldPoint,
} from './cad-status.ts';

/** A status-flag chip: whether it is on, and whether its command is admitted. */
export interface StatusToggleView {
  readonly on: boolean;
  readonly disabled: boolean;
}

/** The grid-snap chip: its switch plus the real spacing the host reports. */
export interface GridSnapToggleView extends StatusToggleView {
  readonly spacing: number | null;
}

/** The single router command that flips idle hover preview. */
export const HOVER_PREVIEW_COMMAND = 'HOVERPREVIEW';

/**
 * Status-bar view for the idle hover-preview toggle.
 *
 * The server's `hoverPreview` flag is the single source of truth. A payload
 * without the field (older host) yields `null`, so the caller renders no chip at
 * all instead of guessing `false`. `admitted` is the shell's normal command
 * admission (`canCommand`), which already accounts for a pending command and a
 * loading/unavailable host.
 */
export function hoverPreviewToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.hoverPreview;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchHoverPreview(runCommand: (command: string) => void): void {
  runCommand(HOVER_PREVIEW_COMMAND);
}

/** The single router command that flips the object-snap master switch (F3). */
export const OBJECT_SNAP_COMMAND = 'OSNAP';

/**
 * Status-bar view for the object-snap master switch (ADR 0014 A.4).
 *
 * The server's `snapEnabled` flag is the single source of truth: it is the
 * session's live switch, not a local mirror and not the persisted
 * `view.overlays.snapHints` preference. A payload without the field (older host)
 * yields `null`, so the caller renders no chip at all instead of guessing
 * `false`. `admitted` is the shell's normal command admission (`canCommand`),
 * which already accounts for a pending command and a loading/unavailable host.
 */
export function objectSnapToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.snapEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchObjectSnap(runCommand: (command: string) => void): void {
  runCommand(OBJECT_SNAP_COMMAND);
}

/** The single router command that flips the display grid (`GRID`, F7). */
export const GRID_COMMAND = 'GRID';

/**
 * Status-bar view for the display-grid switch (`GRID`, F7).
 *
 * The server's `gridEnabled` flag is the single source of truth: it is the
 * session's live switch. An on chip does **not** by itself mean the grid is
 * drawn — the host also requires the persisted `view.overlays.grid` preference,
 * so the two gates compose with AND. This is the display grid only; grid snap
 * (`SNAP`/F9) is the separate [`gridSnapToggle`]. A payload without the field
 * yields `null`, so the caller renders no chip at all instead of guessing
 * `false`. `admitted` is the shell's normal command admission (`canCommand`).
 */
export function gridToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.gridEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchGrid(runCommand: (command: string) => void): void {
  runCommand(GRID_COMMAND);
}

/** The single router command that flips grid snap (`SNAP`, F9). */
export const GRID_SNAP_COMMAND = 'SNAP';

/**
 * Status-bar view for the grid-snap switch (`SNAP`, F9).
 *
 * The server's `snapGridEnabled` flag is the single source of truth: it is the
 * session's live switch, not a local mirror and not the display grid. While on,
 * the host quantises a captured point to the nearest multiple of the session
 * `snapSpacing` in X and Y (never Z) before the `ORTHO` constraint; `spacing` is
 * that real projected value, present only when it is a usable number (> 0), so
 * the chip never advertises a spacing the host did not report. A payload without
 * the field (older host) yields `null`, so the caller renders no chip at all
 * instead of guessing `false`. `admitted` is the shell's normal command
 * admission (`canCommand`).
 */
export function gridSnapToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): GridSnapToggleView | null {
  const on = status?.snapGridEnabled;
  if (typeof on !== 'boolean') return null;
  const spacing = status?.snapSpacing;
  return {
    on,
    spacing: typeof spacing === 'number' && Number.isFinite(spacing) && spacing > 0 ? spacing : null,
    disabled: admitted !== true,
  };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchGridSnap(runCommand: (command: string) => void): void {
  runCommand(GRID_SNAP_COMMAND);
}

/** The single router command that flips the ORTHO drafting constraint (F8). */
export const ORTHO_COMMAND = 'ORTHO';

/**
 * Status-bar view for the orthographic drafting constraint (`ORTHO`, F8).
 *
 * The server's `orthoEnabled` flag is the single source of truth: it is the
 * session's live switch, not a local mirror and not a persisted preference. While
 * on, the host constrains the next point entry (and the matching rubber band) to a
 * horizontal or vertical direction from the tool's previous point; it is not grid
 * snap (`SNAP`/F9) and it never moves a point in Z. A payload without the field
 * (older host) yields `null`, so the caller renders no chip at all instead of
 * guessing `false`. `admitted` is the shell's normal command admission
 * (`canCommand`).
 */
export function orthoToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.orthoEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchOrtho(runCommand: (command: string) => void): void {
  runCommand(ORTHO_COMMAND);
}

/**
 * The global drafting-aid accelerators: function key -> router command.
 *
 * A **name** table, not a second key handler. The shell has exactly one global
 * `keydown` listener (`App.jsx`), and it is the only place a function key may be
 * claimed: it resolves the key through [`draftingAcceleratorCommand`] and
 * submits the returned command through the ordinary funnel (`submitCommand`) —
 * the same path every chip uses. `F7` is AutoCAD's own key for the display grid,
 * `F3` its object-snap key, `F8` its orthographic-constraint key and `F9` its
 * grid-snap key.
 */
export const DRAFTING_ACCELERATORS: Readonly<Record<string, string>> = Object.freeze({
  F3: 'OSNAP', F7: 'GRID', F8: 'ORTHO', F9: 'SNAP',
});

/** The router command a drafting-aid function key means, or `null`. */
export function draftingAcceleratorCommand(key: unknown): string | null {
  if (typeof key !== 'string') return null;
  return DRAFTING_ACCELERATORS[key] ?? null;
}

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

/** The host's own per-parameter value predicate (never a clamp). */
type ParameterPredicate = (value: number) => boolean;

// The host's own per-parameter value predicates (`cad_app::ToolParameter::validate`,
// ADR 0016 decision A), mirrored here so the UI never offers a value the host
// will refuse. The host refuses explicitly and never clamps, so an unoffered
// value is simply not shown as a step (and never silently rewritten).
const integerInRange = (min: number, max: number): ParameterPredicate =>
  (value: number): boolean => Number.isSafeInteger(value) && value >= min && value <= max;
const integerAtLeast = (min: number): ParameterPredicate =>
  (value: number): boolean => Number.isSafeInteger(value) && value >= min;
const finiteAtLeast = (min: number): ParameterPredicate =>
  (value: number): boolean => Number.isFinite(value) && value >= min;
const finiteAny = (): ParameterPredicate => (value: number): boolean => Number.isFinite(value);
const finiteAbove = (low: number): ParameterPredicate =>
  (value: number): boolean => Number.isFinite(value) && value > low;
// A closed set of integer codes (HATCH pattern type / style). The host refuses a
// code outside the set rather than clamping, so only the listed codes are offered.
const integerIn = (...values: number[]): ParameterPredicate =>
  (value: number): boolean => Number.isSafeInteger(value) && values.includes(value);

/** A closed-set parameter's exact accepted integer codes. */
interface ClosedSet {
  readonly choices: readonly number[];
  readonly offered: ParameterPredicate;
}

const closedSet = (...values: number[]): ClosedSet => Object.freeze({
  choices: Object.freeze([...values]),
  offered: integerIn(...values),
});
// Finite, non-zero and within a single turn (0 < |angle| <= 360): 360/-360 (the
// full-circle default) ARE offered targets, while 0 and anything beyond a turn
// (±720, …) are refused — a fill angle is always an explicitly-bounded sweep.
const partialTurn = (): ParameterPredicate =>
  (value: number): boolean => Number.isFinite(value) && value !== 0 && Math.abs(value) <= 360;

/** Fields every tool-parameter control spec shares. */
interface ToolParameterSpecBase {
  /** The field of `cad_status.tool.params` that projects the live value. */
  readonly statusKey: string;
  /** Catalog key of the control's label (present in both catalogs). */
  readonly labelKey: string;
  /** Catalog key of the control's hint (present in both catalogs). */
  readonly hintKey: string;
  /** An optional status-bar icon, present only where one exists. */
  readonly icon?: string;
  /** Whether a projected value may be rendered; defaults to `offered`. */
  readonly projectable?: ParameterPredicate;
}

/** A `'number'` parameter: a bounded stepper. */
export interface NumberToolParameterSpec extends ToolParameterSpecBase {
  readonly kind: 'number';
  /** The increment a numeric stepper offers (integers step by 1; angles by 15°). */
  readonly step: number;
  /** The host's own predicate: whether it would **accept** a value as a target. */
  readonly offered: ParameterPredicate;
  /** For a closed-set parameter, the exact integer codes the host accepts. */
  readonly choices?: readonly number[];
}

/** A `'flag'` parameter: a toggle. */
export interface FlagToolParameterSpec extends ToolParameterSpecBase {
  readonly kind: 'flag';
}

/** The per-parameter control spec, mirrored from the frozen host table. */
export type ToolParameterSpec = NumberToolParameterSpec | FlagToolParameterSpec;

/** Every parameter spec, keyed by its stable machine key. */
export type ToolParameterSpecMap = Readonly<Record<string, ToolParameterSpec>>;

/**
 * The per-parameter control spec, mirrored from the frozen host table
 * (`cad_app::draw_tool::parameter`, ADR 0016 decision A).
 *
 * - `kind`      — `'number'` (bounded stepper) or `'flag'` (toggle);
 * - `statusKey` — the field of `cad_status.tool.params` that projects the live
 *   value (POLYGON's legacy `sides`; later phases project each parameter under
 *   its own machine key);
 * - `step`      — the increment a numeric stepper offers (integer parameters step
 *   by 1; the fill angle steps by 15°);
 * - `offered`   — the host's own predicate: whether the host would **accept** a
 *   value as a target (the stepper never offers a value this rejects);
 * - `choices`   — for a closed-set numeric parameter, the exact integer codes the
 *   host accepts (and therefore the only ones the stepper can reach). It is
 *   machine metadata, not copy: the status bar has no choice control and the
 *   catalog carries no per-choice label keys, so the choice names are spoken by
 *   the parameter's `hintKey` ("0 Normal, 1 Outer, 2 Ignore");
 * - `projectable` — whether a value from `cad_status` may be **rendered**. It
 *   defaults to `offered` (an out-of-range projection is a contract violation and
 *   renders nothing), so no parameter currently overrides it;
 * - `labelKey` / `hintKey` — catalog keys (present in both catalogs);
 * - `icon`      — an optional status-bar icon, present only where one exists.
 */
export const TOOL_PARAMETER_SPECS: ToolParameterSpecMap = Object.freeze({
  polygonSides: {
    kind: 'number', statusKey: 'sides', step: 1, icon: 'polygon',
    offered: integerInRange(3, 1024),
    labelKey: 'status.tool_parameter.polygon_sides',
    hintKey: 'status.tool_parameter.polygon_sides.hint',
  },
  filletRadius: {
    kind: 'number', statusKey: 'filletRadius', step: 1,
    offered: finiteAtLeast(0),
    labelKey: 'status.tool_parameter.fillet_radius',
    hintKey: 'status.tool_parameter.fillet_radius.hint',
  },
  chamferDistanceA: {
    kind: 'number', statusKey: 'chamferDistanceA', step: 1,
    offered: finiteAtLeast(0),
    labelKey: 'status.tool_parameter.chamfer_distance_a',
    hintKey: 'status.tool_parameter.chamfer_distance_a.hint',
  },
  chamferDistanceB: {
    kind: 'number', statusKey: 'chamferDistanceB', step: 1,
    offered: finiteAtLeast(0),
    labelKey: 'status.tool_parameter.chamfer_distance_b',
    hintKey: 'status.tool_parameter.chamfer_distance_b.hint',
  },
  alignScale: {
    kind: 'flag', statusKey: 'alignScale',
    labelKey: 'status.tool_parameter.align_scale',
    hintKey: 'status.tool_parameter.align_scale.hint',
  },
  arrayRows: {
    kind: 'number', statusKey: 'arrayRows', step: 1,
    offered: integerAtLeast(1),
    labelKey: 'status.tool_parameter.array_rows',
    hintKey: 'status.tool_parameter.array_rows.hint',
  },
  arrayColumns: {
    kind: 'number', statusKey: 'arrayColumns', step: 1,
    offered: integerAtLeast(1),
    labelKey: 'status.tool_parameter.array_columns',
    hintKey: 'status.tool_parameter.array_columns.hint',
  },
  arrayRowSpacing: {
    kind: 'number', statusKey: 'arrayRowSpacing', step: 1,
    offered: finiteAny(),
    labelKey: 'status.tool_parameter.array_row_spacing',
    hintKey: 'status.tool_parameter.array_row_spacing.hint',
  },
  arrayColumnSpacing: {
    kind: 'number', statusKey: 'arrayColumnSpacing', step: 1,
    offered: finiteAny(),
    labelKey: 'status.tool_parameter.array_column_spacing',
    hintKey: 'status.tool_parameter.array_column_spacing.hint',
  },
  arrayItems: {
    kind: 'number', statusKey: 'arrayItems', step: 1,
    offered: integerAtLeast(1),
    labelKey: 'status.tool_parameter.array_items',
    hintKey: 'status.tool_parameter.array_items.hint',
  },
  arrayFillAngle: {
    kind: 'number', statusKey: 'arrayFillAngle', step: 15,
    offered: partialTurn(),
    labelKey: 'status.tool_parameter.array_fill_angle',
    hintKey: 'status.tool_parameter.array_fill_angle.hint',
  },
  arrayRotateItems: {
    kind: 'flag', statusKey: 'arrayRotateItems',
    labelKey: 'status.tool_parameter.array_rotate_items',
    hintKey: 'status.tool_parameter.array_rotate_items.hint',
  },
  // HATCH (ADR 0018 decision D, P6b). The six rows mirror
  // `cad_app::draw_tool::parameter`: the three closed sets use `choices` so the
  // stepper can only reach the implemented codes, spacing must be finite and
  // strictly positive (it doubles as the pattern scale / `HPSCALE`), the angle
  // is any finite number of degrees (no implicit wrap — the host accepts it as
  // typed) and `double` is a flag. Pattern type `2` selects one of the
  // **three** predefined patterns the host transcribed (`ANSI31`, `ANSI32`,
  // `ANSI37`) by its 1-based index; custom pattern names, `.pat` files,
  // gradients and associativity have no parameter here because they are not
  // implemented.
  hatchPatternType: {
    kind: 'number', statusKey: 'hatchPatternType', step: 1,
    ...closedSet(0, 1, 2),
    labelKey: 'status.tool_parameter.hatch_pattern_type',
    hintKey: 'status.tool_parameter.hatch_pattern_type.hint',
  },
  hatchPattern: {
    // The 1-based index of the implemented predefined pattern table. The upper
    // bound is the host table length (ANSI31, ANSI32, ANSI37), so an offered
    // value can never name a pattern the host does not implement.
    kind: 'number', statusKey: 'hatchPattern', step: 1,
    ...closedSet(1, 2, 3),
    labelKey: 'status.tool_parameter.hatch_pattern',
    hintKey: 'status.tool_parameter.hatch_pattern.hint',
  },
  hatchSpacing: {
    // A real length: the 1-unit step every other length uses would make every
    // spacing below the default 1 unreachable, so this one steps by 0.5. The
    // host accepts any finite spacing > 0; it is also the predefined pattern
    // scale (`HPSCALE`). The stepper cannot enter arbitrary decimals (there is
    // no free-entry field).
    kind: 'number', statusKey: 'hatchSpacing', step: 0.5,
    offered: finiteAbove(0),
    labelKey: 'status.tool_parameter.hatch_spacing',
    hintKey: 'status.tool_parameter.hatch_spacing.hint',
  },
  hatchAngle: {
    kind: 'number', statusKey: 'hatchAngle', step: 15,
    offered: finiteAny(),
    labelKey: 'status.tool_parameter.hatch_angle',
    hintKey: 'status.tool_parameter.hatch_angle.hint',
  },
  hatchDouble: {
    kind: 'flag', statusKey: 'hatchDouble',
    labelKey: 'status.tool_parameter.hatch_double',
    hintKey: 'status.tool_parameter.hatch_double.hint',
  },
  hatchStyle: {
    kind: 'number', statusKey: 'hatchStyle', step: 1,
    ...closedSet(0, 1, 2),
    labelKey: 'status.tool_parameter.hatch_style',
    hintKey: 'status.tool_parameter.hatch_style.hint',
  },
});

/** Which parameters belong to an active tool, and the command that gates them. */
export interface ToolParameterOwner {
  readonly command: string;
  readonly keys: readonly string[];
}

/** Every tool that owns typed parameters, keyed by its stable tool key. */
export type ToolParameterMap = Readonly<Record<string, ToolParameterOwner>>;

/**
 * Which parameters belong to which active tool (`cad_status.tool.key`), and the
 * command whose admission gates the control (`shell.canCommand`).
 *
 * Only `draw.polygon` and `draw.hatch` are reachable today: POLYGON projects its
 * side count and HATCH its six parameters while the tool is active (HATCH
 * captures no points). The other tools are not implemented and the host projects
 * no value for their parameters, so nothing renders for them (a control appears
 * only when the tool is active **and** its value is projected). A later phase
 * makes a tool live by adding one row here (and, in the host, projecting its
 * values). The non-POLYGON/HATCH tool keys are the anticipated `draw.<kind>` form
 * of the frozen `CadTool.key` convention (ADR 0016 decisions B–E); the host must
 * use the same key for the row to light up.
 */
export const TOOL_PARAMETERS: ToolParameterMap = Object.freeze({
  'draw.polygon': { command: 'POLYGON', keys: ['polygonSides'] },
  'draw.hatch': {
    command: 'HATCH',
    keys: ['hatchPatternType', 'hatchPattern', 'hatchSpacing', 'hatchAngle', 'hatchDouble', 'hatchStyle'],
  },
  'draw.fillet': { command: 'FILLET', keys: ['filletRadius'] },
  'draw.chamfer': { command: 'CHAMFER', keys: ['chamferDistanceA', 'chamferDistanceB'] },
  'draw.align': { command: 'ALIGN', keys: ['alignScale'] },
  'draw.arrayrect': { command: 'ARRAYRECT', keys: ['arrayRows', 'arrayColumns', 'arrayRowSpacing', 'arrayColumnSpacing'] },
  'draw.arraypolar': { command: 'ARRAYPOLAR', keys: ['arrayItems', 'arrayFillAngle', 'arrayRotateItems'] },
});

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
  readonly disabled: boolean;
}

/** A numeric parameter's status-bar view: a bounded stepper. */
export interface ToolParameterNumberView {
  readonly kind: 'number';
  readonly key: string;
  readonly value: number;
  readonly delta: number;
  readonly effective: number;
  readonly text: string;
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
 * numeric stepper view or a flag view, and a parameter whose value is absent (not
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
 * `value + 2·step` while the displayed number still follows the status.
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
      labelKey: spec.labelKey, hintKey: spec.hintKey, icon: spec.icon, disabled,
    };
  }
  const value = projectedNumber(spec, tool);
  if (value === null) return null;
  const delta = pending && pending.key === key && pending.base === value ? pending.delta : 0;
  const effective = value + delta;
  return {
    kind: 'number', key, value, delta, effective, text: String(value),
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
 * Per-step capture prompts for tools whose picks have names (ADR 0017 P5c–P5e).
 *
 * The dimension commands capture named picks — the linear/aligned pair three
 * (two extension-line origins, then the dimension-line location), the radial
 * pair two (an arc/circle entity pick, then the dimension-line location),
 * DIMCENTER one (an arc/circle entity pick), DIMANGULAR three (two straight-line
 * entity picks, then the arc location) and DIMORDINATE two (a feature point, then
 * the leader endpoint) — and a bare point count ("1 point") does not tell the user
 * *what* to pick next. This table names each pick; the step is derived from the
 * host's real `tool.key` and `tool.points` (the captured count), never inferred
 * from the pointer stream. Every other tool has no entry, so it renders no prompt
 * at all and its chip is unchanged.
 *
 * `location_linear` vs `location_aligned` differ on purpose: for DIMLINEAR the
 * location pick also decides horizontal vs vertical, which the user must know
 * before clicking.
 *
 * The radial pair's second pick is deliberately **not** described as positioning
 * the text: the location pick only sets the leader's direction from the centre,
 * and the synthesis derives the text placement itself. Its prompt asks only for
 * the "dimension location" so it never claims the user places the text.
 *
 * DIMANGULAR is the **two-line** form only: its first two steps are line entity
 * picks, never a vertex point. The two lines' infinite intersection is the vertex
 * and the third pick is the arc location, which selects which of the four angles
 * is measured — the backend's rule is "the sector the location falls in". The copy
 * asks for the arc location and makes no quadrant-picker claim, because no such
 * picker exists. The three-point (vertex + two side points) construction is
 * supported by the command payload only and has **no interactive capture**, so no
 * prompt may imply a vertex-first workflow.
 *
 * DIMORDINATE's two steps are free points (no entity pick) and the interactive
 * tool never forces an axis: the backend derives it from the documented auto rule
 * (a greater Y difference measures the X ordinate; the unstated equal case is our
 * declared choice of Y). The prompts name the two points only, so they promise no
 * axis control the tool does not offer.
 *
 * MLEADER's two steps are free points too — the arrowhead position, then the
 * landing point — followed by the same terminal text step TEXT uses. Its prompts
 * name those two picks and nothing else: the command has **no `Options`
 * sub-prompt** (leader type, content type, max points, angular constraint), so no
 * step copy or control may imply one.
 *
 * Only the **named picks** live here, as an ordered list (a step is chosen by the
 * captured count). A completed capture's ready prompt is not positional, so it is
 * resolved from [`TOOL_READY_PROMPTS`] below.
 */
export const TOOL_STEP_PROMPTS: Readonly<Record<string, readonly string[]>> = Object.freeze({
  'draw.textedit': ['tool.prompt.textedit.select_entity'],
  'draw.dimlinear': [
    'tool.prompt.dim.origin_first',
    'tool.prompt.dim.origin_second',
    'tool.prompt.dim.location_linear',
  ],
  'draw.dimaligned': [
    'tool.prompt.dim.origin_first',
    'tool.prompt.dim.origin_second',
    'tool.prompt.dim.location_aligned',
  ],
  'draw.dimradius': [
    'tool.prompt.dim.select_arc',
    'tool.prompt.dim.location_radius',
  ],
  'draw.dimdiameter': [
    'tool.prompt.dim.select_arc',
    'tool.prompt.dim.location_diameter',
  ],
  'draw.dimcenter': [
    'tool.prompt.dim.select_arc',
  ],
  'draw.dimangular': [
    'tool.prompt.dim.select_line',
    'tool.prompt.dim.select_second_line',
    'tool.prompt.dim.location_angular',
  ],
  'draw.dimordinate': [
    'tool.prompt.dim.feature',
    'tool.prompt.dim.leader_endpoint',
  ],
  // TEXT captures one point (its insertion point) and then takes its text. Only
  // the point is a **named pick** here; the text step is not positional (it has
  // no captured count of its own) and is named from the host's `awaiting_text`
  // flag by [`toolStepPromptKey`] — never by counting points.
  'draw.text': [
    TEXT_PICK_PROMPT_KEY,
  ],
  // MTEXT (2026-10-26) captures two named corners: the first is the insertion
  // point, the opposite one only defines the wrap width. Its terminal text step
  // is named from `awaiting_text` by [`textStepPromptKey`], never by a count.
  'draw.mtext': MTEXT_STEP_PROMPT_KEYS,
  // MLEADER (ADR 0019 decision F.2) captures exactly two points — the arrowhead
  // position, then the landing point — and then the **same** terminal text step
  // TEXT uses. Only those two picks belong here: the text step is named from the
  // host's `awaiting_text` flag by [`toolStepPromptKey`], never from a point
  // count, so this list must stay at two entries. The two names are
  // MLEADER-specific because a leader's first pick is its arrowhead, not a
  // generic "point", and its second is the landing the text hangs from.
  'draw.mleader': [
    'tool.prompt.mleader.arrow',
    'tool.prompt.mleader.landing',
  ],
});

/**
 * Per-tool "ready" prompt override, keyed by the active tool key (ADR 0017 P5d).
 *
 * The named-pick table above must stay an **ordered list** (a step is chosen by
 * the captured count), so the ready prompt — which is not positional — lives in
 * this parallel table. Once the exact complement is captured the tool is ready
 * and waits for Enter; the shared default `tool.prompt.dim.ready` says a
 * **dimension** is defined, which is correct for the real dimension commands.
 * DIMCENTER is the one exception: it emits plain line geometry (a centre mark or
 * centrelines), not a dimension entity, so it must not claim a dimension was
 * defined and gets `tool.prompt.dim.center_ready` instead. DIMANGULAR and
 * DIMORDINATE both commit real `DIMENSION` entities, so they deliberately have no
 * entry here and fall through to the shared dimension ready copy; reusing the
 * centre-mark copy for them would be false. A tool not listed here falls back to
 * the shared dimension ready prompt.
 */
export const TOOL_READY_PROMPTS: Readonly<Record<string, string>> = Object.freeze({
  'draw.dimcenter': 'tool.prompt.dim.center_ready',
  // TEXT's complement is its point plus its text, so "ready" is only reached
  // between the two states: the shared dimension copy ("Dimension defined; press
  // Enter to confirm") would be false for a text command, and the next step is
  // always the text. This names that step; the **input mode** still comes from
  // the host's `awaiting_text` alone (see [`toolStepPromptKey`]), so a race where
  // the point is counted but the flag has not arrived yet never opens the text
  // field — it only names what comes next.
  'draw.text': TEXT_STEP_PROMPT_KEY,
  // MTEXT's complement is its two corners plus its text, so it has the same
  // "the next step is the text" override, with MTEXT's own multiline wording. The
  // input mode still comes from the host's `awaiting_text` alone.
  'draw.mtext': MTEXT_STEP_PROMPT_KEY,
  // MLEADER's complement is its two points plus its text. Once the landing pick
  // is captured the capture is complete but the host may not have raised
  // `awaiting_text` yet, and the shared dimension copy ("Dimension defined") would
  // be false for a leader. This names the step that really comes next — the text
  // — without opening it: the **input mode** still comes from the host flag alone
  // (see [`toolStepPromptKey`]), so the field never appears before the host says
  // so.
  'draw.mleader': 'tool.prompt.mleader.type_text',
});

/** A pre-selection tool's two prompts: waiting for a selection, and ready. */
export interface PreselectPrompt {
  readonly idle: string;
  readonly ready: string;
}

/**
 * Pre-selection tools: the boundary already exists before the command starts, so
 * the chip has no pick to name and cannot use a point index (ADR 0018 decision D,
 * P6b).
 *
 * HATCH's point arity is the zero-point exact complement — `tool.points` stays 0
 * from the moment the tool starts to the moment it commits — so an entry in
 * [`TOOL_STEP_PROMPTS`] could not express it: an empty step list makes that
 * derivation report "ready" before anything is selected, and any non-empty list
 * would invent a pick the tool never takes. The host's real `tool.ready`
 * (`DrawPreview::can_confirm`, true only for a non-empty selection) is the
 * authority, so idle and ready are the only two states. Keeping this in its own
 * table also guarantees a pre-selection tool can never inherit the shared
 * dimension ready copy.
 */
export const PRESELECT_PROMPTS: Readonly<Record<string, PreselectPrompt>> = Object.freeze({
  'draw.hatch': Object.freeze({
    idle: 'tool.prompt.hatch.select_boundary',
    ready: 'tool.prompt.hatch.ready',
  }),
});

// Every prompt table stays reachable from this module; the open-ended one lives
// in `tool-open-ended.ts` because this file is at the size ceiling.
export { OPEN_ENDED_PROMPTS };

/** The shared ready prompt for a capture that produced a dimension entity. */
const DEFAULT_READY_PROMPT: string = 'tool.prompt.dim.ready';

/**
 * The catalog key for the next expected pick of the active `tool`, or `null`.
 *
 * A tool the host reports as `awaiting_text` names its **text step** first — that
 * step is terminal and is not a pick, so no point count can express it; the flag
 * is the single source of truth ([`toolAwaitingText`]).
 *
 * A pre-selection tool (see [`PRESELECT_PROMPTS`]) captures no picks at all, so
 * its chip is chosen by the host's readiness instead: the "select the boundary
 * objects first" copy while it waits for a selection, the "boundary ready" copy
 * once the host reports it can be confirmed. `ready` must be the literal `true`;
 * a missing or malformed readiness never claims the selection is sufficient.
 *
 * `null` for a tool with no named steps and no pre-selection prompt (every other
 * existing tool) so the chip stays exactly as it was. Once the exact complement
 * is captured the capture is ready and waits for Enter, so the ready prompt names
 * that instead of repeating the last pick (a per-tool override from
 * [`TOOL_READY_PROMPTS`], else the shared dimension ready prompt). The point
 * count is read as the host's real value; a malformed one is treated as "no
 * points yet" rather than a fabricated later step.
 *
 * An open-ended tool ([`OPEN_ENDED_PROMPTS`]) has no fixed complement, so the
 * count only distinguishes "base dimension still missing" from "capturing
 * points". The base pick is the tool's first captured step — the same shape
 * DIMRADIUS/DIMCENTER already have — and the command refuses a capture with zero
 * measured points, so `points <= 1` always names the point step and the host's
 * `ready` is what turns the chip into the ready copy. No step count is ever
 * assumed: the point prompt repeats for as many points as the user picks.
 */
export function toolStepPromptKey(tool: CadTool | null | undefined): string | null {
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  // The host's `awaiting_text` names the text step outright (ADR 0019 A.2). It is
  // checked before any count-derived step: the text step is not a pick, and the
  // flag is the only source of truth for it. The wording is per tool
  // ([`textStepPromptKey`]) so MTEXT's multiline copy is never TEXT's.
  if (toolAwaitingText(tool)) return textStepPromptKey(tool);
  const preselect = PRESELECT_PROMPTS[tool.key];
  if (preselect) return tool.ready === true ? preselect.ready : preselect.idle;
  const points = Number.isSafeInteger(tool.points) && tool.points >= 0 ? tool.points : 0;
  const openEnded = OPEN_ENDED_PROMPTS[tool.key];
  if (openEnded) {
    if (points === 0) return openEnded.select;
    // Readiness never wins without a measured point: a capture with only the base
    // pick is refused by the command, so claiming "ready" there would be false.
    if (points > 1 && tool.ready === true) return DEFAULT_READY_PROMPT;
    return openEnded.point;
  }
  const steps = TOOL_STEP_PROMPTS[tool.key];
  if (!steps) return null;
  if (points < steps.length) return steps[points];
  return TOOL_READY_PROMPTS[tool.key] ?? DEFAULT_READY_PROMPT;
}

// The snap engine's stable machine keys (`cad_measure::SnapKind::as_str`) and
// their catalog labels. A key outside this set is rendered as nothing rather
// than a guessed label.
const SNAP_KIND_LABELS: Readonly<Record<string, { readonly labelKey: string }>> = {
  endpoint: { labelKey: 'snap.kind.endpoint' },
  midpoint: { labelKey: 'snap.kind.midpoint' },
  center: { labelKey: 'snap.kind.center' },
  quadrant: { labelKey: 'snap.kind.quadrant' },
  perpendicular: { labelKey: 'snap.kind.perpendicular' },
  intersection: { labelKey: 'snap.kind.intersection' },
};

/**
 * The catalog label key for the current object-snap candidate kind, or `null`.
 *
 * The server's `snapKind` is the single source of truth (ADR 0014 A.7): it is
 * the current best candidate's machine kind while a point prompt is active, and
 * `null` otherwise. An absent/`null`/unknown kind yields `null`, so the caller
 * renders nothing extra instead of a placeholder chip or a guessed label.
 */
export function snapKindLabelKey(status: Partial<CadStatus> | null | undefined): string | null {
  const kind = status?.snapKind;
  if (typeof kind !== 'string') return null;
  return SNAP_KIND_LABELS[kind]?.labelKey ?? null;
}
