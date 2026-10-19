import { admitCommandText } from './commands.mjs';

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

// The frozen selection block (`cad-app::selection::SelectionProperties`). It is
// always present and structurally exact: `empty == (count == 0)`, `rows` only
// carry the single selected object, and `mixedKeys` are only meaningful for a
// real multi-selection. A block that breaks any of these invariants is a
// contract violation, never something to coerce into a fabricated empty state.
function parseSelection(selection) {
  if (!selection || typeof selection !== 'object' || Array.isArray(selection)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const { count, empty, rows, mixedKeys } = selection;
  if (!Number.isSafeInteger(count) || count < 0
    || typeof empty !== 'boolean' || empty !== (count === 0)
    || !Array.isArray(rows) || !Array.isArray(mixedKeys)) {
    throw Error('CAD_STATUS_INVALID');
  }
  // Rows describe one object only: a host that ships rows for an empty or
  // multi-selection is malformed, not a shortcut we may silently truncate.
  if (rows.length > 0 && count !== 1) throw Error('CAD_STATUS_INVALID');
  // Row keys are machine keys the catalog may not know; that is tolerated (the
  // UI shows the raw key). A missing/typed key or a non-string value is not.
  if (!rows.every(row => row && typeof row === 'object' && !Array.isArray(row)
    && typeof row.key === 'string' && row.key.length > 0 && typeof row.value === 'string')) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (mixedKeys.length > 0 && count <= 1) throw Error('CAD_STATUS_INVALID');
  if (!mixedKeys.every(key => typeof key === 'string' && key.length > 0)) {
    throw Error('CAD_STATUS_INVALID');
  }
  return selection;
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
});

/**
 * Which parameters belong to which active tool (`cad_status.tool.key`), and the
 * command whose admission gates the control (`shell.canCommand`).
 *
 * Only `draw.polygon` is reachable today — the other tools are not implemented
 * and the host projects no value for their parameters, so nothing renders for
 * them (a control appears only when the tool is active **and** its value is
 * projected). A later phase makes a tool live by adding one row here (and, in
 * the host, projecting its values). The non-POLYGON tool keys are the anticipated
 * `draw.<kind>` form of the frozen `CadTool.key` convention (ADR 0016 decisions
 * B–E); the host must use the same key for the row to light up.
 */
export const TOOL_PARAMETERS = Object.freeze({
  'draw.polygon': { command: 'POLYGON', keys: ['polygonSides'] },
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
 * Per-step capture prompts for tools whose picks have names (ADR 0017 P5c).
 *
 * The dimension commands are three-pick captures — two extension-line origins,
 * then the dimension-line location — and a bare point count ("1 point") does not
 * tell the user *what* to pick next. This table names each pick; the step is
 * derived from the host's real `tool.key` and `tool.points` (the captured count),
 * never inferred from the pointer stream. Every other tool has no entry, so it
 * renders no prompt at all and its chip is unchanged.
 *
 * `location_linear` vs `location_aligned` differ on purpose: for DIMLINEAR the
 * location pick also decides horizontal vs vertical, which the user must know
 * before clicking.
 */
export const TOOL_STEP_PROMPTS = Object.freeze({
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
});

/**
 * The catalog key for the next expected pick of the active `tool`, or `null`.
 *
 * `null` for a tool with no named steps (every existing tool) so the chip stays
 * exactly as it was. Once the exact complement is captured the capture is ready
 * and waits for Enter, so the ready prompt names that instead of repeating the
 * last pick. The point count is read as the host's real value; a malformed one
 * is treated as "no points yet" rather than a fabricated later step.
 */
export function toolStepPromptKey(tool) {
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  const steps = TOOL_STEP_PROMPTS[tool.key];
  if (!steps) return null;
  const points = Number.isSafeInteger(tool.points) && tool.points >= 0 ? tool.points : 0;
  return points < steps.length ? steps[points] : 'tool.prompt.dim.ready';
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
 * The catalog label for a stable machine property key.
 *
 * Keys come from `cad-app::selection` (`PropertyRow::new`). The set is frozen
 * here so every key has an explicit, reviewed translation; any key the catalog
 * does not know is rendered **as itself** — never a guessed label and never
 * dropped.
 */
export function propertyRowLabel(t, key) {
  switch (key) {
    case 'id': return t('properties.row.id');
    case 'handle': return t('properties.row.handle');
    case 'type': return t('properties.row.type');
    case 'layer': return t('properties.row.layer');
    case 'space': return t('properties.row.space');
    case 'draw_order': return t('properties.row.draw_order');
    case 'instance': return t('properties.row.instance');
    case 'sub_element': return t('properties.row.sub_element');
    case 'status': return t('properties.row.status');
    case 'start': return t('properties.row.start');
    case 'end': return t('properties.row.end');
    case 'length': return t('properties.row.length');
    case 'vertices': return t('properties.row.vertices');
    case 'closed': return t('properties.row.closed');
    case 'bulges': return t('properties.row.bulges');
    case 'center': return t('properties.row.center');
    case 'normal': return t('properties.row.normal');
    case 'radius': return t('properties.row.radius');
    case 'start_angle': return t('properties.row.start_angle');
    case 'sweep': return t('properties.row.sweep');
    case 'ratio': return t('properties.row.ratio');
    case 'degree': return t('properties.row.degree');
    case 'control_points': return t('properties.row.control_points');
    case 'position': return t('properties.row.position');
    case 'triangles': return t('properties.row.triangles');
    case 'block': return t('properties.row.block');
    case 'text': return t('properties.row.text');
    case 'height': return t('properties.row.height');
    case 'font': return t('properties.row.font');
    case 'shape': return t('properties.row.shape');
    case 'size': return t('properties.row.size');
    case 'opaque_type': return t('properties.row.opaque_type');
    case 'opaque_version': return t('properties.row.opaque_version');
    case 'payload_bytes': return t('properties.row.payload_bytes');
    case 'pixels': return t('properties.row.pixels');
    case 'file': return t('properties.row.file');
    case 'visible': return t('properties.row.visible');
    case 'boundary': return t('properties.row.boundary');
    case 'inverted': return t('properties.row.inverted');
    case 'children': return t('properties.row.children');
    case 'subtype': return t('properties.row.subtype');
    case 'measurement': return t('properties.row.measurement');
    case 'display': return t('properties.row.display');
    // An unknown machine key is shown by its raw key, never fabricated.
    default: return key;
  }
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
  if (event.key !== 'Enter' || event.isComposing || event.repeat) return;
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
