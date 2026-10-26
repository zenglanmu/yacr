// The frozen `cad_status` snapshot: its wire types, the runtime validator the
// chrome runs on every poll, and the import-progress/cancel affordances.
//
// The types are the host's own wire shape (`cad_ui_core::session::status`,
// serialized with `camelCase` except the two documented snake_case overrides
// `tool.awaiting_text` and `tool.text_prefill`). The validator is deliberately
// **narrower** than the type: it enforces exactly the fields the chrome cannot
// proceed without, and tolerates the additive blocks an older host may omit, so
// those are typed optional for the same reason. A malformed payload throws
// `CAD_STATUS_INVALID` and is never coerced into a guessed state.

import { parseSelection } from './selection-wire.ts';
import type { Translator } from '../i18n.ts';

/** The `invoke` callable the shell threads in (Tauri IPC, or a test double). */
export type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/** A world (drawing) point: exactly three finite numbers, `[x, y, z]`. */
export type WorldPoint = readonly [number, number, number];

/** The camera block of the status (`{center, scale}`). */
export interface CadCamera {
  /** World point at the centre of the viewport. */
  readonly center: readonly [number, number];
  /** World units per logical pixel (strictly positive). */
  readonly scale: number;
}

/** Live import progress; present only while an import is genuinely running. */
export interface CadImportProgress {
  /** Stable machine key of the latest phase, or `null` before the first tick. */
  readonly phase: string | null;
  /** Real entities normalised/inserted so far. */
  readonly entitiesDone: number;
  /** Total entities expected, or `null` when the format does not expose it. */
  readonly entitiesTotal: number | null;
  /** Source bytes measured so far, only when actually known. */
  readonly bytes: number | null;
  /** Whether a cancel request can still be issued for this import. */
  readonly cancellable: boolean;
}

/** The closed set of editing-control machine keys a selection row may carry. */
export type CadSelectionRowKind = 'text' | 'number' | 'enum' | 'bool';

/** One property row: a value plus its stable machine key and read-side metadata. */
export interface CadSelectionRow {
  readonly key: string;
  /** Shared value, or `null` when the values differ across the selection. */
  readonly value: string | null;
  /** Stable panel group machine key (`entity`, `geometry`, `text`, `display`). */
  readonly group: string;
  /** Whether the values differ across the selection (`value` is `null`). */
  readonly mixed: boolean;
  /** Whether a future write path can change the value for every selected ref. */
  readonly editable: boolean;
  readonly kind: CadSelectionRowKind;
  /** Real named-table values for an `enum` row (sorted, capped by the app). */
  readonly options: readonly string[];
}

/** The `selection` block: always present, always a real selection. */
export interface CadSelection {
  readonly count: number;
  /** Whether nothing is selected (`count === 0`). */
  readonly empty: boolean;
  readonly rows: readonly CadSelectionRow[];
  /** Machine keys of the mixed rows; empty for any single selection. */
  readonly mixedKeys: readonly string[];
  /** The round-trippable current text of a single selected text entity, or `null`. */
  readonly text: string | null;
  /** Wire name: `text_multiline` (see the module header). */
  readonly text_multiline: boolean;
}

/** The typed tool-parameter block (`tool.params`), keyed by machine key. */
export type CadToolParams = Readonly<Record<string, unknown>>;

/** The active interactive tool, projected from the host's real capture state. */
export interface CadTool {
  /** Stable machine key (`"draw.line"`, `"measure.distance"`, …). */
  readonly key: string;
  /** Points captured so far (the real count from the preview). */
  readonly points: number;
  /** Whether the tool can be confirmed right now. */
  readonly ready: boolean;
  /** Additive: the real open-endedness (PLINE/SPLINE), absent on an older host. */
  readonly openEnded?: boolean;
  /** Wire name: `awaiting_text`; absent on an older host means "no text step". */
  readonly awaiting_text?: boolean;
  /** Wire name: `text_prefill`; the TEXTEDIT prefill, or `null`. */
  readonly text_prefill?: string | null;
  /** Typed tool parameters; `null` fields for a tool that has no such parameter. */
  readonly params?: CadToolParams;
}

/** Typed measurement echo carried by a `result` command-log entry. */
export interface CadMeasurementEcho {
  readonly algorithm: string;
  readonly value: number;
  readonly unit: string | null;
  readonly precision: string | null;
  readonly points: readonly WorldPoint[];
}

/** One entry of the command-window echo; it carries no message/text field. */
export interface CadCommandLogEntry {
  readonly sequence: number;
  readonly kind: 'command' | 'result' | 'error';
  readonly command: string | null;
  readonly code: string | null;
  readonly measurement: CadMeasurementEcho | null;
}

/** The open modal popup of the host (`{ kind, label, modal }`). */
export interface CadPopup {
  /** Stable popup kind key (`"about"`, `"textEdit"` or `"options"`). */
  readonly kind: string;
  /** Frozen legacy wire label; addresses no window (the popup is in-DOM). */
  readonly label: string;
  /** Whether the host holds the modal input gate for this popup. */
  readonly modal: boolean;
}

/**
 * The `cad_status` snapshot consumed by the chrome.
 *
 * Every field the validator below enforces is required; the additive blocks an
 * older host may omit (`hoverPreview`, `snapEnabled`, `snapKind`, `crosshair`,
 * `commandLog`, `popup`, `gripDrag`) are optional, and the consumers already read
 * them tolerantly (`?? []`, `!= null`, `Boolean(...)`).
 */
export interface CadStatus {
  readonly documentLabel: string | null;
  readonly entityCount: number | null;
  readonly loading: boolean;
  readonly importProgress: CadImportProgress | null;
  readonly error: string | null;
  readonly canNew: boolean;
  readonly canOpen: boolean;
  readonly canSave: boolean;
  readonly canPlot: boolean;
  readonly canFit: boolean;
  readonly backend: string | null;
  readonly adapter: string | null;
  readonly camera: CadCamera | null;
  readonly selection: CadSelection;
  readonly tool?: CadTool | null;
  readonly hoverPreview?: boolean;
  readonly snapEnabled?: boolean;
  readonly gridEnabled: boolean;
  readonly snapGridEnabled: boolean;
  /** Session grid-snap spacing in drawing units (finite, strictly positive). */
  readonly snapSpacing: number;
  readonly orthoEnabled: boolean;
  readonly snapKind?: string | null;
  readonly crosshair?: boolean;
  readonly cursorWorld: WorldPoint | null;
  readonly importCount: number;
  readonly frameCount: number;
  readonly commandLog?: readonly CadCommandLogEntry[];
  readonly popup?: CadPopup | null;
  /**
   * Whether a grip drag is armed (`cad_ui_core::session::CadStatus::grip_drag`).
   *
   * Additive, optional for an older host. It is **not** an active tool: `tool`
   * stays `null` while it is `true`; the chrome reads it only to route Escape to
   * `CANCEL`, which the host answers by dropping the armed drag (nothing was
   * written before the release). A `dom-canvas` carrier never arms a drag and
   * therefore always reports `false`.
   */
  readonly gripDrag?: boolean;
}

// Phase keys the backend reports (`cad_app::import_phase_key`). A phase outside
// this set is a contract violation, not a runtime surprise.
const IMPORT_PHASES = new Set(['reading', 'parsing', 'tables', 'entities', 'resolving', 'finishing']);

/** Whether `raw` is a non-negative safe integer (a real count, never a guess). */
function nonNegativeInteger(raw: unknown): boolean {
  return typeof raw === 'number' && Number.isSafeInteger(raw) && raw >= 0;
}

function parseImportProgress(value: unknown): CadImportProgress | null {
  if (value === null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const progress = value as Record<string, unknown>;
  const count = (field: string): boolean => nonNegativeInteger(progress[field]);
  const nullableCount = (field: string): boolean => progress[field] === null || count(field);
  if (!(progress.phase === null || IMPORT_PHASES.has(progress.phase as string))
    || !count('entitiesDone')
    || !nullableCount('entitiesTotal')
    || !nullableCount('bytes')
    || typeof progress.cancellable !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  return value as CadImportProgress;
}

/** Whether `value` is a well-formed world point: exactly three finite numbers. */
function isWorldPoint(value: unknown): value is WorldPoint {
  return Array.isArray(value) && value.length === 3 && value.every(Number.isFinite);
}

/** Whether `value` is a well-formed `{center, scale}` camera block. */
function isCamera(value: unknown): value is CadCamera {
  if (!value || typeof value !== 'object') return false;
  const camera = value as { center?: unknown; scale?: unknown };
  return Array.isArray(camera.center) && camera.center.length === 2
    && camera.center.every(Number.isFinite)
    && typeof camera.scale === 'number' && Number.isFinite(camera.scale) && camera.scale > 0;
}

export function parseCadStatus(value: unknown): CadStatus {
  // Still unvalidated: every read below goes through this loose record view and
  // the checks themselves are the validator. A non-object short-circuits before
  // any field helper runs, exactly as the guard order below spells out.
  const status = value as Record<string, unknown>;
  const nullableString = (field: string): boolean =>
    status[field] === null || typeof status[field] === 'string';
  const count = (field: string): boolean => nonNegativeInteger(status[field]);
  const camera = status?.camera;
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !['documentLabel', 'error', 'backend', 'adapter'].every(nullableString)
    || !['loading', 'canOpen', 'canNew', 'canSave', 'canPlot', 'canFit', 'gridEnabled', 'orthoEnabled', 'snapGridEnabled'].every(field => typeof status[field] === 'boolean')
    || !(status.entityCount === null || count('entityCount'))
    || !['importCount', 'frameCount'].every(count)
    || !(camera === null || isCamera(camera))) {
    throw Error('CAD_STATUS_INVALID');
  }
  // The grid-snap spacing is required and must be a real, usable spacing: a
  // finite number strictly greater than zero. The host never sends 0/NaN (the
  // session setter refuses those), so a malformed value is a contract violation
  // rather than something to render as a guessed spacing.
  if (!(typeof status.snapSpacing === 'number' && Number.isFinite(status.snapSpacing) && status.snapSpacing > 0)) {
    throw Error('CAD_STATUS_INVALID');
  }
  // Live import progress is optional (`null` when idle) but, when present, must
  // be well-formed; a malformed block is never rendered as a guessed bar.
  parseImportProgress(status.importProgress);
  // The selection block is always present; a malformed one is rejected outright.
  parseSelection(status.selection);
  // Idle hover preview is optional for older hosts, but a present flag must be a
  // real boolean rather than a truthy string the UI would render as "on".
  if (status.hoverPreview !== undefined && typeof status.hoverPreview !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The object-snap master switch is likewise optional for older hosts, but a
  // present flag must be a real boolean; a coercion would show a wrong chip.
  if (status.snapEnabled !== undefined && typeof status.snapEnabled !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The current snap candidate kind is optional (`null`/absent when there is no
  // candidate, or on an older host), but a present value must be a real string:
  // a coercion would render a wrong label.
  if (status.snapKind !== undefined && status.snapKind !== null && typeof status.snapKind !== 'string') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The pointer-crosshair gate is optional for older hosts, but a present flag
  // must be a real boolean: a truthy non-boolean would hide the system cursor
  // over a view that draws no crosshair, leaving the pointer invisible.
  if (status.crosshair !== undefined && typeof status.crosshair !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The armed-grip-drag flag is optional for older hosts, but a present value
  // must be a real boolean: a truthy non-boolean would swallow Escape (or, worse,
  // claim a drag that no host gesture owns).
  if (status.gripDrag !== undefined && typeof status.gripDrag !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The cursor's world coordinate is required: `null` (no reliable 2D cursor) or
  // exactly three finite numbers. Anything else is rejected outright — a
  // malformed point (a string, a 2-element array, a NaN) is never rendered as a
  // drawing coordinate the host did not compute.
  if (!(status.cursorWorld === null || isWorldPoint(status.cursorWorld))) {
    throw Error('CAD_STATUS_INVALID');
  }
  // The active-tool block is `null`/absent when no tool runs, and an object when
  // one does. Its text-step flag (ADR 0019 A.2) is optional for older hosts, but
  // a present value must be a real boolean: a truthy non-boolean must never open
  // the text prompt, and the field — not the tool key or point count — is what
  // decides that mode.
  const tool = status.tool;
  if (tool !== undefined && tool !== null && (typeof tool !== 'object' || Array.isArray(tool))) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (tool !== undefined && tool !== null) {
    const awaitingText = (tool as Record<string, unknown>).awaiting_text;
    if (awaitingText !== undefined && typeof awaitingText !== 'boolean') {
      throw Error('CAD_STATUS_INVALID');
    }
  }
  return value as CadStatus;
}

/** The catalog label for a locale-independent import phase key. */
function importPhaseLabel(t: Translator, phase: string | null): string {
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

/** A localised view of real import progress for the chrome. */
export interface ImportProgressView {
  readonly phase: string;
  readonly count: string;
  readonly detail: string;
  /** `null` while the importer exposed no total (indeterminate). */
  readonly ratio: number | null;
  readonly determinate: boolean;
  readonly cancellable: boolean;
}

/**
 * A localised view of real import progress for the chrome.
 *
 * `ratio` is `null` when the importer never exposed a total (indeterminate), so
 * the bar animates instead of pretending a percentage; a determinate `ratio` is
 * the real `done / total`, clamped and never extrapolated to 100% before the
 * document is actually published. Returns `null` when there is no live import.
 */
export function importProgressView(
  progress: CadImportProgress | null | undefined,
  t: Translator,
): ImportProgressView | null {
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

/** What `CANCEL` did: queue admission, an explicit refusal, or nothing to cancel. */
export type ImportCancelResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'unavailable' }
  | { readonly kind: 'failed'; readonly error: string };

/**
 * Ask the host to cancel the in-flight open (`CANCEL`).
 *
 * Only offered while the host reports the import is still cancellable; refusal
 * (already terminal or probe mode) is explicit, never a silent success. A
 * `{kind:'accepted'}` result is queue admission, not a completed cancel — the
 * real outcome surfaces through the next `cad_status`.
 */
export async function requestImportCancel(
  invoke: Invoke,
  status: Partial<CadStatus> | null | undefined,
): Promise<ImportCancelResult> {
  if (!status?.importProgress?.cancellable) return { kind: 'unavailable' };
  try {
    const response = await invoke('cad_command', { command: 'CANCEL' });
    if (!response || (response as { accepted?: unknown }).accepted !== true) throw Error('CAD_COMMAND_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

