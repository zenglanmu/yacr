import { admitCommandText } from './commands.mjs';
import { propertyRowLabel } from './property-labels.mjs';
import { parseSelection } from './selection-wire.mjs';
import { TEXT_PICK_PROMPT_KEY, TEXT_STEP_PROMPT_KEY, toolAwaitingText } from './text.mjs';
import { OPEN_ENDED_PROMPTS } from './tool-open-ended.mjs';

export { propertyGroupLabel, propertyRowLabel } from './property-labels.mjs';
export { parseSelection } from './selection-wire.mjs';

// Phase keys the backend reports (`cad_app::import_phase_key`). A phase outside
// this set is a contract violation, not a runtime surprise.
const IMPORT_PHASES = new Set(['reading', 'parsing', 'tables', 'entities', 'resolving', 'finishing']);

function parseImportProgress(value) {
  const count = field => Number.isSafeInteger(value[field]) && value[field] >= 0;
  const nullableCount = field => value[field] === null || count(field);
  if (value === null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !(value.phase === null || IMPORT_PHASES.has(value.phase))
    || !count('entitiesDone')
    || !nullableCount('entitiesTotal')
    || !nullableCount('bytes')
    || typeof value.cancellable !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  return value;
}

export function parseCadStatus(value) {
  const nullableString = field => value[field] === null || typeof value[field] === 'string';
  const count = field => Number.isSafeInteger(value[field]) && value[field] >= 0;
  const camera = value?.camera;
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !['documentLabel', 'error', 'backend', 'adapter'].every(nullableString)
    || !['loading', 'canOpen', 'canFit'].every(field => typeof value[field] === 'boolean')
    || !(value.entityCount === null || count('entityCount'))
    || !['importCount', 'frameCount'].every(count)
    || !(camera === null || (camera && Array.isArray(camera.center) && camera.center.length === 2
      && camera.center.every(Number.isFinite) && Number.isFinite(camera.scale) && camera.scale > 0))) {
    throw Error('CAD_STATUS_INVALID');
  }
  // Live import progress is optional (`null` when idle) but, when present, must
  // be well-formed; a malformed block is never rendered as a guessed bar.
  parseImportProgress(value.importProgress);
  // The selection block is always present; a malformed one is rejected outright.
  parseSelection(value.selection);
  // Idle hover preview is optional for older hosts, but a present flag must be a
  // real boolean rather than a truthy string the UI would render as "on".
  if (value.hoverPreview !== undefined && typeof value.hoverPreview !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The object-snap master switch is likewise optional for older hosts, but a
  // present flag must be a real boolean; a coercion would show a wrong chip.
  if (value.snapEnabled !== undefined && typeof value.snapEnabled !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The current snap candidate kind is optional (`null`/absent when there is no
  // candidate, or on an older host), but a present value must be a real string:
  // a coercion would render a wrong label.
  if (value.snapKind !== undefined && value.snapKind !== null && typeof value.snapKind !== 'string') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The pointer-crosshair gate is optional for older hosts, but a present flag
  // must be a real boolean: a truthy non-boolean would hide the system cursor
  // over a view that draws no crosshair, leaving the pointer invisible.
  if (value.crosshair !== undefined && typeof value.crosshair !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The active-tool block is `null`/absent when no tool runs, and an object when
  // one does. Its text-step flag (ADR 0019 A.2) is optional for older hosts, but
  // a present value must be a real boolean: a truthy non-boolean must never open
  // the text prompt, and the field — not the tool key or point count — is what
  // decides that mode.
  if (value.tool !== undefined && value.tool !== null
    && (typeof value.tool !== 'object' || Array.isArray(value.tool))) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (value.tool && value.tool.awaiting_text !== undefined
    && typeof value.tool.awaiting_text !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  return value;
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
export function hoverPreviewToggle(status, admitted) {
  const on = status?.hoverPreview;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchHoverPreview(runCommand) {
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
export function objectSnapToggle(status, admitted) {
  const on = status?.snapEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchObjectSnap(runCommand) {
  runCommand(OBJECT_SNAP_COMMAND);
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

// The host's own per-parameter value predicates (`cad_app::ToolParameter::validate`,
// ADR 0016 decision A), mirrored here so the UI never offers a value the host
// will refuse. The host refuses explicitly and never clamps, so an unoffered
// value is simply not shown as a step (and never silently rewritten).
const integerInRange = (min, max) => value => Number.isSafeInteger(value) && value >= min && value <= max;
const integerAtLeast = min => value => Number.isSafeInteger(value) && value >= min;
const finiteAtLeast = min => value => Number.isFinite(value) && value >= min;
const finiteAny = () => value => Number.isFinite(value);
const finiteAbove = low => value => Number.isFinite(value) && value > low;
// A closed set of integer codes (HATCH pattern type / style). The host refuses a
// code outside the set rather than clamping, so only the listed codes are offered.
const integerIn = (...values) => value => Number.isSafeInteger(value) && values.includes(value);
const closedSet = (...values) => Object.freeze({
  choices: Object.freeze([...values]),
  offered: integerIn(...values),
});
// Finite, non-zero and within a single turn (0 < |angle| <= 360): 360/-360 (the
// full-circle default) ARE offered targets, while 0 and anything beyond a turn
// (±720, …) are refused — a fill angle is always an explicitly-bounded sweep.
const partialTurn = () => value => Number.isFinite(value) && value !== 0 && Math.abs(value) <= 360;

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
export const TOOL_PARAMETER_SPECS = Object.freeze({
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
  // HATCH (ADR 0018 decision D, P6b). The five rows mirror
  // `cad_app::draw_tool::parameter`: the two closed sets use `choices` so the
  // stepper can only reach the implemented codes, spacing must be finite and
  // strictly positive, the angle is any finite number of degrees (no implicit
  // wrap — the host accepts it as typed) and `double` is a flag. Predefined
  // pattern names, gradients and associativity have no parameter here because
  // they are not implemented; nothing in this table may imply otherwise.
  hatchPatternType: {
    kind: 'number', statusKey: 'hatchPatternType', step: 1,
    ...closedSet(0, 1),
    labelKey: 'status.tool_parameter.hatch_pattern_type',
    hintKey: 'status.tool_parameter.hatch_pattern_type.hint',
  },
  hatchSpacing: {
    // A real length: the 1-unit step every other length uses would make every
    // spacing below the default 1 unreachable, so this one steps by 0.5. The
    // host accepts any finite spacing > 0; the stepper cannot enter arbitrary
    // decimals (there is no free-entry field).
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

/**
 * Which parameters belong to which active tool (`cad_status.tool.key`), and the
 * command whose admission gates the control (`shell.canCommand`).
 *
 * Only `draw.polygon` and `draw.hatch` are reachable today: POLYGON projects its
 * side count and HATCH its five parameters while the tool is active (HATCH
 * captures no points). The other tools are not implemented and the host projects
 * no value for their parameters, so nothing renders for them (a control appears
 * only when the tool is active **and** its value is projected). A later phase
 * makes a tool live by adding one row here (and, in the host, projecting its
 * values). The non-POLYGON/HATCH tool keys are the anticipated `draw.<kind>` form
 * of the frozen `CadTool.key` convention (ADR 0016 decisions B–E); the host must
 * use the same key for the row to light up.
 */
export const TOOL_PARAMETERS = Object.freeze({
  'draw.polygon': { command: 'POLYGON', keys: ['polygonSides'] },
  'draw.hatch': {
    command: 'HATCH',
    keys: ['hatchPatternType', 'hatchSpacing', 'hatchAngle', 'hatchDouble', 'hatchStyle'],
  },
  'draw.fillet': { command: 'FILLET', keys: ['filletRadius'] },
  'draw.chamfer': { command: 'CHAMFER', keys: ['chamferDistanceA', 'chamferDistanceB'] },
  'draw.align': { command: 'ALIGN', keys: ['alignScale'] },
  'draw.arrayrect': { command: 'ARRAYRECT', keys: ['arrayRows', 'arrayColumns', 'arrayRowSpacing', 'arrayColumnSpacing'] },
  'draw.arraypolar': { command: 'ARRAYPOLAR', keys: ['arrayItems', 'arrayFillAngle', 'arrayRotateItems'] },
});

/** The live value of `spec` from the active tool, or `null` when absent/invalid. */
function projectedValue(spec, tool) {
  const raw = tool?.params?.[spec.statusKey];
  if (spec.kind === 'flag') return typeof raw === 'boolean' ? raw : null;
  const projectable = spec.projectable ?? spec.offered;
  return typeof raw === 'number' && projectable(raw) ? raw : null;
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
export function toolParameterViews(status, admitted, pending = null) {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return [];
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry) return [];
  const disabled = !(typeof admitted === 'function' ? admitted(entry.command) : admitted === true);
  const views = [];
  for (const key of entry.keys) {
    const spec = TOOL_PARAMETER_SPECS[key];
    if (!spec) continue;
    const view = parameterView(key, spec, tool, disabled, pending);
    if (view) views.push(view);
  }
  return views;
}

function parameterView(key, spec, tool, disabled, pending) {
  const value = projectedValue(spec, tool);
  if (value === null) return null;
  if (spec.kind === 'flag') {
    return {
      kind: 'flag', key, value, next: !value,
      labelKey: spec.labelKey, hintKey: spec.hintKey, icon: spec.icon, disabled,
    };
  }
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
export function reconcileToolParameterPending(pending, status) {
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
  const value = projectedValue(spec, tool);
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
export function stepToolParameterPending(pending, status, key, target) {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry || !entry.keys.includes(key)) return null;
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number') return null;
  const base = projectedValue(spec, tool);
  if (base === null) return null;
  if (pending && (pending.key !== key || pending.base !== base)) return null;
  const delta = target - base;
  if (!Number.isFinite(delta) || delta === 0 || Math.abs(delta) > TOOL_PARAMETER_PENDING_CAP) return null;
  return { key, base, delta };
}

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
export async function setToolParameterNumber(invoke, key, number) {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number' || !spec.offered(number)) {
    return { kind: 'refused', error: 'CAD_TOOL_PARAMETER_OUT_OF_RANGE' };
  }
  try {
    const response = await invoke(TOOL_PARAMETER_COMMAND, { parameter: key, number });
    if (!response || response.accepted !== true) throw Error('CAD_TOOL_PARAMETER_NOT_ACCEPTED');
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
export async function setToolParameterFlag(invoke, key, flag) {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'flag' || typeof flag !== 'boolean') {
    return { kind: 'refused', error: 'CAD_TOOL_PARAMETER_KIND' };
  }
  try {
    const response = await invoke(TOOL_PARAMETER_COMMAND, { parameter: key, flag });
    if (!response || response.accepted !== true) throw Error('CAD_TOOL_PARAMETER_NOT_ACCEPTED');
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
export const TOOL_STEP_PROMPTS = Object.freeze({
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
export const TOOL_READY_PROMPTS = Object.freeze({
  'draw.dimcenter': 'tool.prompt.dim.center_ready',
  // TEXT's complement is its point plus its text, so "ready" is only reached
  // between the two states: the shared dimension copy ("Dimension defined; press
  // Enter to confirm") would be false for a text command, and the next step is
  // always the text. This names that step; the **input mode** still comes from
  // the host's `awaiting_text` alone (see [`toolStepPromptKey`]), so a race where
  // the point is counted but the flag has not arrived yet never opens the text
  // field — it only names what comes next.
  'draw.text': TEXT_STEP_PROMPT_KEY,
  // MLEADER's complement is its two points plus its text. Once the landing pick
  // is captured the capture is complete but the host may not have raised
  // `awaiting_text` yet, and the shared dimension copy ("Dimension defined") would
  // be false for a leader. This names the step that really comes next — the text
  // — without opening it: the **input mode** still comes from the host flag alone
  // (see [`toolStepPromptKey`]), so the field never appears before the host says
  // so.
  'draw.mleader': 'tool.prompt.mleader.type_text',
});

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
export const PRESELECT_PROMPTS = Object.freeze({
  'draw.hatch': Object.freeze({
    idle: 'tool.prompt.hatch.select_boundary',
    ready: 'tool.prompt.hatch.ready',
  }),
});

// Every prompt table stays reachable from this module; the open-ended one lives
// in `tool-open-ended.mjs` because this file is at the size ceiling.
export { OPEN_ENDED_PROMPTS };

/** The shared ready prompt for a capture that produced a dimension entity. */
const DEFAULT_READY_PROMPT = 'tool.prompt.dim.ready';

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
export function toolStepPromptKey(tool) {
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  // The host's `awaiting_text` names the text step outright (ADR 0019 A.2). It is
  // checked before any count-derived step: the text step is not a pick, and the
  // flag is the only source of truth for it.
  if (toolAwaitingText(tool)) return TEXT_STEP_PROMPT_KEY;
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
const SNAP_KIND_LABELS = {
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
export function snapKindLabelKey(status) {
  const kind = status?.snapKind;
  if (typeof kind !== 'string') return null;
  return SNAP_KIND_LABELS[kind]?.labelKey ?? null;
}

/** The catalog label for a locale-independent import phase key. */
function importPhaseLabel(t, phase) {
  switch (phase) {
    case 'reading': return t('import.phase.reading');
    case 'parsing': return t('import.phase.parsing');
    case 'tables': return t('import.phase.tables');
    case 'entities': return t('import.phase.entities');
    case 'resolving': return t('import.phase.resolving');
    case 'finishing': return t('import.phase.finishing');
    default: return t('import.phase.unknown');
  }
}

/**
 * A localised view of real import progress for the chrome.
 *
 * `ratio` is `null` when the importer never exposed a total (indeterminate), so
 * the bar animates instead of pretending a percentage; a determinate `ratio` is
 * the real `done / total`, clamped and never extrapolated to 100% before the
 * document is actually published. Returns `null` when there is no live import.
 */
export function importProgressView(progress, t) {
  if (!progress) return null;
  const determinate = progress.entitiesTotal !== null;
  const ratio = determinate
    ? (progress.entitiesTotal > 0 ? Math.min(1, progress.entitiesDone / progress.entitiesTotal) : 1)
    : null;
  const count = determinate
    ? t('import.progress.count', { done: progress.entitiesDone, total: progress.entitiesTotal })
    : t('import.progress.indeterminate', { done: progress.entitiesDone });
  const detail = progress.bytes === null
    ? count
    : `${count}${t('import.detail.separator')}${t('import.progress.bytes', { bytes: progress.bytes })}`;
  return {
    phase: importPhaseLabel(t, progress.phase),
    count,
    detail,
    ratio,
    determinate,
    cancellable: progress.cancellable,
  };
}

/**
 * A localised, honest view of the current selection for the properties panel.
 *
 * Three states, driven only by the real `selection` block (a missing/null block
 * is the explicit empty state, not a fabricated one):
 *
 * - `empty`  — no selection; `properties.empty`;
 * - `single` — the label→value rows of the one selected object;
 * - `multi`  — the selected count plus the labels of the keys whose values
 *   differ (`mixedLabel` is `null` when no key is actually mixed).
 */
export function selectionView(selection, t) {
  if (!selection || selection.empty || selection.count === 0) {
    return { kind: 'empty', count: 0, rows: [], mixedKeys: [], mixedLabel: null, countLabel: null };
  }
  if (selection.count === 1) {
    return {
      kind: 'single',
      count: 1,
      rows: selection.rows.map(row => ({ key: row.key, label: propertyRowLabel(t, row.key), value: row.value })),
      mixedKeys: [],
      mixedLabel: null,
      countLabel: null,
    };
  }
  const mixedKeys = selection.mixedKeys.map(key => propertyRowLabel(t, key));
  return {
    kind: 'multi',
    count: selection.count,
    rows: [],
    mixedKeys,
    mixedLabel: mixedKeys.length > 0 ? t('properties.mixed', { keys: mixedKeys.join(', ') }) : null,
    countLabel: t('properties.selected_count', { count: selection.count }),
  };
}

/**
 * Ask the host to cancel the in-flight open (`CANCEL`).
 *
 * Only offered while the host reports the import is still cancellable; refusal
 * (already terminal or probe mode) is explicit, never a silent success. A
 * `{kind:'accepted'}` result is queue admission, not a completed cancel — the
 * real outcome surfaces through the next `cad_status`.
 */
export async function requestImportCancel(invoke, status) {
  if (!status?.importProgress?.cancellable) return { kind: 'unavailable' };
  try {
    const response = await invoke('cad_command', { command: 'CANCEL' });
    if (!response || response.accepted !== true) throw Error('CAD_COMMAND_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * Whether the host capability gates allow `command` right now.
 *
 * Only the two capability-gated commands this build has gates for are checked
 * here (`OPEN`, `FIT`); the general admission decision comes from
 * [`admitCommandText`], which also consults the loaded host vocabulary.
 */
export function commandEnabled(status, pending, command) {
  return Boolean(status && !pending && !status.loading
    && (command === 'OPEN' ? status.canOpen : command === 'FIT' ? status.canFit && status.documentLabel !== null && status.entityCount !== null : false));
}

/**
 * Admit `text` through `cad_command`.
 *
 * The command vocabulary is the one loaded from the host (`admitCommandText`),
 * so an interactive tool (`LINE`, `MOVE`, `CONFIRM`, …) reaches the router; only
 * a name the host does not know, or an explicitly excluded host-owned command,
 * is refused before IPC. `{kind:'accepted'}` is queue admission, never
 * completion; a host-side refusal is an explicit `failed`, never a silent
 * success.
 */
export async function queueCadCommand(invoke, text, status, pending) {
  const admission = admitCommandText(text, { status, pending });
  const command = admission.command;
  if (admission.kind !== 'admitted') {
    return admission.kind === 'unavailable'
      ? { kind: 'failed', error: 'CAD_COMMAND_UNAVAILABLE' }
      : { kind: 'unsupported', command };
  }
  try {
    const response = await invoke('cad_command', { command });
    if (!response || response.accepted !== true) throw Error('CAD_COMMAND_NOT_ACCEPTED');
    // Acceptance is only queue admission, never completion or a database update.
    return { kind: 'accepted', command };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

export function handleCommandKey(event, submit) {
  // `isComposing` covers the standard IME report; `keyCode === 229` is the
  // legacy signal some IMEs send for the Enter that ends/commits composition.
  // Either way the press belongs to the IME, so Enter must not submit — for a
  // command *and* for a tool's text step.
  if (event.key !== 'Enter' || event.isComposing || event.keyCode === 229 || event.repeat) return;
  event.preventDefault();
  void submit();
}

export function startCadPolling(invoke, onStatus, onError, clock = globalThis) {
  let stopped = false;
  let timer;
  let failures = 0;
  async function poll() {
    try {
      const status = parseCadStatus(await invoke('cad_status'));
      if (!stopped) { failures = 0; onStatus(status); }
    } catch (error) {
      if (!stopped) { failures++; onStatus(null); onError(String(error)); }
    }
    if (!stopped) timer = clock.setTimeout(poll, failures ? Math.min(4000, 500 * 2 ** Math.min(failures, 3)) : 250);
  }
  void poll();
  return () => { stopped = true; clock.clearTimeout(timer); };
}
