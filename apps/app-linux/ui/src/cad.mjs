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

/** The single IPC that sets the POLYGON tool's side count. */
export const POLYGON_SIDES_COMMAND = 'cad_set_polygon_sides';

// AutoCAD's `POLYSIDES` range. The host refuses a value outside it explicitly
// (never clamping), so the stepper only ever offers values inside it.
export const POLYGON_SIDES_MIN = 3;
export const POLYGON_SIDES_MAX = 1024;

/**
 * Status-bar view for the POLYGON side-count control.
 *
 * The host publishes the live value at `status.tool.params.sides` while POLYGON
 * is the active tool (`status.tool.key === 'draw.polygon'`); for every other
 * tool that field is `null`. Nothing is rendered unless POLYGON is active with a
 * real integer in the supported `3..=1024` range, so a missing/`null`/malformed
 * or out-of-range value yields `null` rather than an invented default.
 *
 * `admitted` is the shell's normal command admission (`canCommand`), which
 * already accounts for a pending command and a loading/unavailable host or an
 * unloaded vocabulary — the same gate the toggle chips use.
 *
 * `sides` (the *rendered* number) is always the real status value and is never
 * optimistically advanced. `pending` is the bounded delta of steps the host has
 * not echoed back yet (see [`stepPolygonPending`]): it only shifts the effective
 * target the next step builds on and the button bounds, so a rapid double-click
 * composes into `sides + 2` while the displayed number still follows the status.
 */
export function polygonSidesView(status, admitted, pending = null) {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || tool.key !== 'draw.polygon') return null;
  const sides = tool.params?.sides;
  if (!Number.isSafeInteger(sides) || sides < POLYGON_SIDES_MIN || sides > POLYGON_SIDES_MAX) return null;
  const delta = pending && pending.key === tool.key && pending.base === sides ? pending.delta : 0;
  const effective = sides + delta;
  return {
    sides,
    delta,
    effective,
    disabled: admitted !== true,
    // A step is offered only while the *effective* target stays inside the range
    // and the pending magnitude is under the cap: it is disabled, never clamped
    // into a different value.
    canDecrement: effective > POLYGON_SIDES_MIN && delta > -POLYGON_SIDES_PENDING_CAP,
    canIncrement: effective < POLYGON_SIDES_MAX && delta < POLYGON_SIDES_PENDING_CAP,
    decrement: effective - 1,
    increment: effective + 1,
  };
}

// Cap on the pending delta magnitude. 10 keeps a stuck queue from accumulating
// unbounded intent — at most ten un-echoed steps, i.e. a ~2.5 s burst at the
// 250 ms poll — while staying well above a normal click cadence, so the host
// virtually always catches up before the cap is ever felt.
export const POLYGON_SIDES_PENDING_CAP = 10;

/**
 * Reconcile the bounded pending delta against the latest real status.
 *
 * Returns the same `pending` while the host has not yet caught up (the status
 * value still equals the value the delta is measured against, `base`), and
 * `null` — dropping the delta — as soon as the status value changes, the tool
 * switches/cancels, the tool key changes, or the value becomes absent/`null`/
 * out-of-range. Returning the same reference on an unchanged poll lets the
 * caller `setState` without triggering a re-render.
 */
export function reconcilePolygonPending(pending, status) {
  const tool = status?.tool;
  const sides = tool?.params?.sides;
  const active = tool && typeof tool === 'object' && tool.key === 'draw.polygon'
    && Number.isSafeInteger(sides) && sides >= POLYGON_SIDES_MIN && sides <= POLYGON_SIDES_MAX;
  if (!active) return null;
  if (!pending || pending.key !== tool.key || pending.base !== sides) return null;
  return pending;
}

/**
 * Fold one step into the pending delta, or `null` when there is nothing to track.
 *
 * `target` is the value a step dispatches (`effective ± 1` from the view); the
 * new delta is `target - sides`, measured against the current status value. A
 * zero delta (a step that lands back on the status value) and any delta beyond
 * [`POLYGON_SIDES_PENDING_CAP`] collapse to `null`, so the cap can never be
 * exceeded even if a caller bypasses the view's disabled buttons.
 */
export function stepPolygonPending(pending, status, target) {
  const tool = status?.tool;
  const sides = tool?.params?.sides;
  if (!tool || typeof tool !== 'object' || !Number.isSafeInteger(sides)) return null;
  const delta = target - sides;
  if (!Number.isSafeInteger(delta) || delta === 0 || Math.abs(delta) > POLYGON_SIDES_PENDING_CAP) return null;
  return { key: tool.key, base: sides, delta };
}

/**
 * Ask the host to set the POLYGON side count.
 *
 * `cad_set_polygon_sides` is the one IPC for this parameter. A
 * `{accepted:true}` result is **queue admission**, exactly like `cad_command`:
 * the next real `cad_status` carries the new value, so the caller must not
 * optimistically mirror it. A value outside the supported range is refused here
 * before IPC (the host refuses it too rather than clamping); a pre-queue refusal
 * rejects with a string, returned as an explicit `failed` — never a silent
 * success or a fabricated local value.
 */
export async function setPolygonSides(invoke, sides) {
  if (!Number.isSafeInteger(sides) || sides < POLYGON_SIDES_MIN || sides > POLYGON_SIDES_MAX) {
    return { kind: 'refused', error: 'CAD_POLYGON_SIDES_OUT_OF_RANGE' };
  }
  try {
    const response = await invoke(POLYGON_SIDES_COMMAND, { sides });
    if (!response || response.accepted !== true) throw Error('CAD_POLYGON_SIDES_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
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
