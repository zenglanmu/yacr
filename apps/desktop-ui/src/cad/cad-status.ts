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
import { parseOrbitCapability } from './host-profile.ts';
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

/**
 * The authoritative 3D/2D view orientation (`cad_status.view`), used by the
 * ViewCube. Every value is the host camera's own state, never a UI guess: a
 * host that cannot report a finite basis sends `null` and the cube refuses to
 * rotate rather than fabricating an orientation.
 *
 * The block is also carried on the per-frame volatile event (a free orbit
 * changes it every frame), so it must not re-render the shell tree.
 */
export interface CadViewOrientation {
  /** `"2d"` or `"3d"` — the host's real view mode. */
  readonly mode: '2d' | '3d';
  /** `"orthographic"` or `"perspective"`. */
  readonly projection: 'orthographic' | 'perspective';
  readonly eye: WorldPoint;
  readonly target: WorldPoint;
  readonly up: WorldPoint;
}

/**
 * The host's navigation-chrome switches (`cad_status.navigation`): AutoCAD's
 * `NAVVCUBEDISPLAY` bit code plus the two `DISPLAYVIEWCUBEIN2D/3D` registry
 * gates and the navbar visibility. Always host truth — the chrome only hides
 * or shows from these values.
 */
export interface CadNavigationSettings {
  readonly navbarVisible: boolean;
  /** `0..=3`: 1 = 3D styles, 2 = 2D styles, 3 = both (AutoCAD bit code). */
  readonly viewCubeDisplay: number;
  readonly viewCubeIn2d: boolean;
  readonly viewCubeIn3d: boolean;
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

/**
 * The closed set of values a plugin `Editor` prompt can ask for
 * (`cad_status.pendingPrompt.kind`, serialized lowercase by the host).
 */
export type CadPendingPromptKind =
  | 'point'
  | 'entity'
  | 'selection'
  | 'string'
  | 'double'
  | 'integer'
  | 'keyword';

/**
 * The pending plugin `Editor` prompt (`cad_status.pendingPrompt`).
 *
 * A prompt is **answer data**, not a command: while one is pending the user's
 * typed line completes it (`cad_set_tool_text`), Enter is `CONFIRM` and Escape
 * is `CANCEL`. `message` is the sentence the shell shows (the plugin's own, or
 * the host's default for the kind), `keywords` the accepted options in the
 * caller's spelling, and `error` the last invalid-input sentence while the
 * prompt stays pending (re-prompt semantics).
 *
 * `keywords`/`error` are never interpreted here — the host matches the answer,
 * so this type carries them verbatim.
 */
export interface CadPendingPrompt {
  readonly kind: CadPendingPromptKind;
  readonly message: string;
  readonly keywords: readonly string[];
  readonly error: string | null;
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

/** One layout tab the drawing exposes (`cad_status.layouts[]`). */
export interface CadLayoutTab {
  /** The stable `LayoutId` the host resolves `SPACE #<id>` against. */
  readonly id: number;
  /** The drawing's own layout name (never invented for an unnamed layout). */
  readonly name: string;
  /** Whether this build can draw the layout's viewports. */
  readonly supported: boolean;
  /** `"<stable-code>: <message>"` when unsupported, `null` when supported. */
  readonly reason: string | null;
  /** Real viewport count from the layout table. */
  readonly viewportCount: number;
}

/** The drawing space the host is currently showing (`cad_status.activeSpace`). */
export type CadActiveSpace =
  | { readonly kind: 'model' }
  | { readonly kind: 'paper'; readonly id: number };

/** The closed set of external-reference states the host can report. */
export type CadReferenceKind =
  | 'loaded'
  | 'missing'
  | 'browser-unavailable'
  | 'refused-path'
  | 'unloaded'
  | 'cycle'
  | 'depth-exceeded'
  | 'skipped-overlay'
  | 'unsupported';

/** One external reference (`cad_status.references[]`). */
export interface CadReferenceStatus {
  /** The reference name stored in the drawing. */
  readonly name: string;
  /** The raw path stored in the drawing (never rewritten by the chrome). */
  readonly path: string;
  /** The path the host really opened, when it did. */
  readonly resolvedPath: string | null;
  readonly status: CadReferenceKind;
  /** `"<stable-code>: <message>"`, or `null` when the host gave no reason. */
  readonly reason: string | null;
  /** Whether the reference is an overlay rather than an attached reference. */
  readonly overlay: boolean;
  /** Nesting depth of the reference in the drawing's reference graph. */
  readonly depth: number;
}

/** One drawing block definition (`cad_status.blocks[]`). */
export interface CadBlockStatus {
  /** The stored table name, or `null` for a definition without metadata. */
  readonly name: string | null;
  /** The definition's real member count. */
  readonly memberCount: number;
  /** The stored insertion base point, or `null` when the block has no metadata. */
  readonly basePoint: WorldPoint | null;
}

/** One captured member reference of a session group. */
export interface CadGroupMember {
  /** The entity id as a decimal string (a `u128` never loses precision). */
  readonly entityId: string;
  /** The nested INSERT chain, root first; empty for a top-level entity. */
  readonly instancePath: readonly string[];
}

/** One session-only entity group (`cad_status.groups[]`). */
export interface CadGroupStatus {
  readonly name: string;
  readonly memberCount: number;
  /** The captured references, in capture order. */
  readonly members: readonly CadGroupMember[];
}

/** One image clip boundary summary (`cad_status.images[].clip`). */
export interface CadImageClipStatus {
  /** Stored vertex count; `2` is the rectangle form. */
  readonly vertexCount: number;
  /** Whether the boundary keeps the inside (`true`) or the outside (`false`). */
  readonly inside: boolean;
}

/** One raster IMAGE entity (`cad_status.images[]`). */
export interface CadImageStatus {
  /** The entity id as a decimal string. */
  readonly id: string;
  /** Stable space key (`model`, `paper`, `block`). */
  readonly space: 'model' | 'paper' | 'block';
  /** The layout/block id for paper/block space, `null` for model space. */
  readonly spaceId: number | null;
  /** The logical resource key stored on the entity, if any. */
  readonly file: string | null;
  /** Pixel size `[width, height]` as stored. */
  readonly pixels: readonly [number, number];
  /** The entity's stored visibility flag. */
  readonly visible: boolean;
  /** The stored clip summary, or `null` when the image is unclipped. */
  readonly clip: CadImageClipStatus | null;
}

/**
 * The host's carrier profile (`cad_status.hostProfile`), one always-present
 * nested object on the current wire.
 *
 * The Rust host serialises all four keys on every status. They are `unknown`
 * here because `cad/surface.ts` and `cad/view3d.ts` are the narrowing readers —
 * the JSON contract is authoritative, not a second declaration of it.
 *
 * The object itself is optional on [`CadStatus`] (and in the parser) **only**
 * for compatibility with a recorded pre-P1 payload (a captured wire sample or an
 * external evidence ledger can predate the field): a status without this block
 * must still parse and keep the documented legacy defaults (`native-below`,
 * palettes available, standard views undeclared). In-repo fixtures all carry the
 * block. A host that does send a profile is validated strictly for every key it
 * carries.
 */
export interface CadHostProfile {
  /** `native-below` or `dom-canvas` on the wire; readers narrow it. */
  readonly surfaceMode?: unknown;
  /** `native` or `wasm` on the wire; readers narrow it. */
  readonly documentOwner?: unknown;
  /** Whether this carrier renders the desktop palettes (a real boolean). */
  readonly palettesAvailable?: unknown;
  /** `null` (undeclared) or the authoritative array of view keys. */
  readonly standardViews?: unknown;
  /**
   * Whether this carrier can orbit at all (`cad_orbit`).
   *
   * `false` is an **affirmative** limitation (the 2D-only `dom-canvas` wasm
   * session, which has no 3D camera and would answer
   * `unsupported-runtime: cad_orbit`); `true` is an explicit yes; `null`/absent
   * is undeclared and keeps the desktop rule (allowed once a 3D drawing is open).
   * The front-end admission helper (`cad/orbit.ts`) reads exactly this, so a
   * gesture the carrier cannot honour is disabled with a reason instead of
   * reaching the transport. Only a real boolean or `null` is accepted; the rule
   * itself lives in `cad/host-profile.ts` (`parseOrbitCapability`).
   */
  readonly orbit?: unknown;
}

/**
 * Host-reported performance metrics for the performance/diagnostics panel.
 *
 * Every field is `null` when the host or its backend cannot measure it — an
 * absent number is never rendered as `0`. The GPU numbers are the **renderer's
 * own accounting**, not device-wide usage; `vramTotalBytes` is the advertised
 * device-local VRAM proxy (Vulkan only), not free memory.
 */
export interface CadPerformance {
  /** Process CPU over the last sample window, percent of one core. */
  readonly cpuPercent: number | null;
  /** Resident set size of this process in bytes. */
  readonly processRssBytes: number | null;
  /** Virtual address space of this process in bytes. */
  readonly processVirtualBytes: number | null;
  /** Total physical memory of the machine in bytes. */
  readonly systemTotalBytes: number | null;
  /** Currently available physical memory in bytes. */
  readonly systemAvailableBytes: number | null;
  /** Bytes this renderer currently charges against the GPU device. */
  readonly gpuMemoryUsedBytes: number | null;
  /** Peak of that accounting since the renderer started. */
  readonly gpuMemoryPeakBytes: number | null;
  /** Driver-reserved bytes from the wgpu allocator (whole Device), if any. */
  readonly gpuMemoryReservedBytes: number | null;
  /** Advertised device-local VRAM proxy (largest heap; Vulkan only). */
  readonly vramTotalBytes: number | null;
}

/**
 * The `cad_status` snapshot consumed by the chrome.
 *
 * Every field the validator below enforces is required; the additive blocks an
 * older host may omit (`hoverPreview`, `snapEnabled`, `snapKind`, `crosshair`,
 * `commandLog`, `popup`, `gripDrag`, `hostProfile`) are optional, and the
 * consumers already read them tolerantly (`?? []`, `!= null`, `Boolean(...)`).
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
  /**
   * The host's persisted UI theme (`"dark"` or `"light"`).
   *
   * Optional only for a recorded pre-theme payload; the readers default to
   * `"dark"` (the historical chrome). The host remains the authority: a theme
   * toggle is a real setting write, and the chrome follows this value.
   */
  readonly theme?: string;
  /**
   * The real view orientation for the ViewCube, or `null` when the host has no
   * authoritative camera (no document, or a non-finite basis). Optional for an
   * older host; absent means "cannot report" and disables cube rotation.
   */
  readonly view?: CadViewOrientation | null;
  /**
   * The host's navigation-chrome switches, or absent on an older host (the
   * readers then apply the AutoCAD defaults: navbar on, ViewCube display 3,
   * 2D gate off, 3D gate on).
   */
  readonly navigation?: CadNavigationSettings;
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
   * The pending plugin `Editor` prompt, or `null`/absent when none is open.
   *
   * Additive: an older host sends no field at all, which reads as "no prompt"
   * (`pendingPromptOf`). When present, `null` is the host's own "nothing is
   * pending" answer and an object must be well-formed — an unknown `kind` is a
   * parse refusal, never a coerced prompt.
   */
  readonly pendingPrompt?: CadPendingPrompt | null;
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
  /**
   * The carrier profile: which surface runs this chrome, who owns the CAD
   * document, and the carrier's palette / standard-view capabilities.
   *
   * Optional **only** so a pre-P1 payload still parses (see [`CadHostProfile`]);
   * the current Rust wire always sends it. When a key is absent the readers apply
   * the historical defaults — never a guessed capability.
   */
  readonly hostProfile?: CadHostProfile;
  /** Layout tabs of the open drawing; absent on an older host means `[]`. */
  readonly layouts?: readonly CadLayoutTab[];
  /** The active drawing space; absent on an older host means model space. */
  readonly activeSpace?: CadActiveSpace;
  /** External references of the open drawing; absent means `[]`. */
  readonly references?: readonly CadReferenceStatus[];
  /** Drawing block definitions; absent means `[]`. */
  readonly blocks?: readonly CadBlockStatus[];
  /** Session-only entity groups; absent means `[]`. */
  readonly groups?: readonly CadGroupStatus[];
  /** Raster IMAGE entities in every space; absent means `[]`. */
  readonly images?: readonly CadImageStatus[];
  /**
   * Host-reported performance metrics, or `null`/absent when the host does not
   * publish them. Absence is an explicit "not measured", never a fabricated
   * zero: the panel renders an em dash for each missing value.
   */
  readonly performance?: CadPerformance | null;
}

/** The open drawing's layout tabs; an older host's omission is an empty list. */
export function layoutTabsOf(status: Partial<CadStatus> | null | undefined): readonly CadLayoutTab[] {
  return status?.layouts ?? [];
}

/** The active space; an older host's omission means model space. */
export function activeSpaceOf(status: Partial<CadStatus> | null | undefined): CadActiveSpace {
  return status?.activeSpace ?? { kind: 'model' };
}

/** The open drawing's external references; an omission is an empty list. */
export function referencesOf(status: Partial<CadStatus> | null | undefined): readonly CadReferenceStatus[] {
  return status?.references ?? [];
}

/** The drawing's block definitions; an omission is an empty list. */
export function blocksOf(status: Partial<CadStatus> | null | undefined): readonly CadBlockStatus[] {
  return status?.blocks ?? [];
}

/** The session's entity groups; an omission is an empty list. */
export function groupsOf(status: Partial<CadStatus> | null | undefined): readonly CadGroupStatus[] {
  return status?.groups ?? [];
}

/** The drawing's raster IMAGE entities; an omission is an empty list. */
export function imagesOf(status: Partial<CadStatus> | null | undefined): readonly CadImageStatus[] {
  return status?.images ?? [];
}

/**
 * The pending plugin prompt, or `null` when none is open.
 *
 * The one reader for the block: the `native-below` and `dom-canvas` carriers
 * both report it on `cad_status`, and an older host without the field means "no
 * prompt" — never a guessed one. The parser has already validated it.
 */
export function pendingPromptOf(
  status: Partial<CadStatus> | null | undefined,
): CadPendingPrompt | null {
  return status?.pendingPrompt ?? null;
}

// Phase keys the backend reports (`cad_app::import_phase_key`). A phase outside
// this set is a contract violation, not a runtime surprise.
const IMPORT_PHASES = new Set(['reading', 'parsing', 'tables', 'entities', 'resolving', 'finishing']);

/** The closed external-reference state set (`cad_ui_core` reference status). */
const REFERENCE_KINDS = new Set([
  'loaded', 'missing', 'browser-unavailable', 'refused-path', 'unloaded', 'cycle',
  'depth-exceeded', 'skipped-overlay', 'unsupported',
]);

/** The closed plugin-prompt kind set (`cad_ui_core` `pendingPrompt.kind`). */
const PENDING_PROMPT_KINDS = new Set([
  'point', 'entity', 'selection', 'string', 'double', 'integer', 'keyword',
]);

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

/**
 * Parse the ViewCube orientation block. `null` is the host's honest "no
 * authoritative camera"; a present block must carry a real finite basis and one
 * of the two documented modes/projections.
 */
export function parseCadViewOrientation(value: unknown): CadViewOrientation | null {
  if (value === null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const view = value as Record<string, unknown>;
  if ((view.mode !== '2d' && view.mode !== '3d')
    || (view.projection !== 'orthographic' && view.projection !== 'perspective')
    || !isWorldPoint(view.eye)
    || !isWorldPoint(view.target)
    || !isWorldPoint(view.up)) {
    throw Error('CAD_STATUS_INVALID');
  }
  return value as CadViewOrientation;
}

/**
 * Parse the pending plugin prompt block.
 *
 * The block is `null` on the host's own "nothing pending" answer; a present
 * object must carry a documented `kind`, a string `message`, a string-only
 * `keywords` list and `error` as a string or `null`. An unknown kind is a
 * contract violation — the chrome must never guess what an answer means.
 */
export function parseCadPendingPrompt(value: unknown): CadPendingPrompt {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const prompt = value as Record<string, unknown>;
  if (typeof prompt.kind !== 'string' || !PENDING_PROMPT_KINDS.has(prompt.kind)
    || typeof prompt.message !== 'string'
    || !Array.isArray(prompt.keywords)
    || !prompt.keywords.every((keyword: unknown) => typeof keyword === 'string')
    || !(prompt.error === null || typeof prompt.error === 'string')) {
    throw Error('CAD_STATUS_INVALID');
  }
  return value as CadPendingPrompt;
}

/** Parse the navigation switches: both real booleans and a 0..3 bit code. */
export function parseCadNavigation(value: unknown): CadNavigationSettings {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const navigation = value as Record<string, unknown>;
  const display = navigation.viewCubeDisplay;
  if (typeof navigation.navbarVisible !== 'boolean'
    || typeof navigation.viewCubeIn2d !== 'boolean'
    || typeof navigation.viewCubeIn3d !== 'boolean'
    || typeof display !== 'number' || !Number.isInteger(display)
    || display < 0 || display > 3) {
    throw Error('CAD_STATUS_INVALID');
  }
  return value as CadNavigationSettings;
}

/** Whether `value` is one well-formed `layouts[]` row. */
function isLayoutTab(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const tab = value as Record<string, unknown>;
  return nonNegativeInteger(tab.id)
    && typeof tab.name === 'string'
    && typeof tab.supported === 'boolean'
    && (tab.reason === null || typeof tab.reason === 'string')
    && nonNegativeInteger(tab.viewportCount);
}

/** Whether `value` is one well-formed `references[]` row. */
function isReferenceStatus(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const reference = value as Record<string, unknown>;
  return typeof reference.name === 'string'
    && typeof reference.path === 'string'
    && (reference.resolvedPath === null || typeof reference.resolvedPath === 'string')
    && typeof reference.status === 'string'
    && REFERENCE_KINDS.has(reference.status)
    && (reference.reason === null || typeof reference.reason === 'string')
    && typeof reference.overlay === 'boolean'
    && nonNegativeInteger(reference.depth);
}

/** Whether `value` is one well-formed `blocks[]` row. */
function isBlockStatus(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const block = value as Record<string, unknown>;
  return (block.name === null || typeof block.name === 'string')
    && nonNegativeInteger(block.memberCount)
    && (block.basePoint === null || isWorldPoint(block.basePoint));
}

/** Whether `value` is one well-formed group member row. */
function isGroupMember(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const member = value as Record<string, unknown>;
  return typeof member.entityId === 'string'
    && Array.isArray(member.instancePath)
    && member.instancePath.every((id: unknown) => typeof id === 'string');
}

/** Whether `value` is one well-formed `groups[]` row. */
function isGroupStatus(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const group = value as Record<string, unknown>;
  return typeof group.name === 'string'
    && nonNegativeInteger(group.memberCount)
    && Array.isArray(group.members)
    && group.members.every(isGroupMember)
    && group.members.length === group.memberCount;
}

/** Whether `value` is one well-formed image pixel pair. */
function isPixelSize(value: unknown): boolean {
  return Array.isArray(value) && value.length === 2 && value.every(Number.isFinite);
}

/** Whether `value` is one well-formed image clip summary. */
function isImageClip(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const clip = value as Record<string, unknown>;
  return nonNegativeInteger(clip.vertexCount) && typeof clip.inside === 'boolean';
}

/** Whether `value` is one well-formed `images[]` row. */
function isImageStatus(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const image = value as Record<string, unknown>;
  const space = image.space;
  return typeof image.id === 'string'
    && (space === 'model' || space === 'paper' || space === 'block')
    && (image.spaceId === null || nonNegativeInteger(image.spaceId))
    && (image.file === null || typeof image.file === 'string')
    && isPixelSize(image.pixels)
    && typeof image.visible === 'boolean'
    && (image.clip === null || isImageClip(image.clip));
}

/** The keys a `cad_status.performance` block must carry when it is present. */
const PERFORMANCE_FIELDS = [
  'cpuPercent',
  'processRssBytes',
  'processVirtualBytes',
  'systemTotalBytes',
  'systemAvailableBytes',
  'gpuMemoryUsedBytes',
  'gpuMemoryPeakBytes',
  'gpuMemoryReservedBytes',
  'vramTotalBytes',
] as const;

/** A metric that is either a real non-negative finite number or `null`. */
function isOptionalMetric(value: unknown): boolean {
  return value === null
    || (typeof value === 'number' && Number.isFinite(value) && value >= 0);
}

/**
 * Validate a present `performance` block.
 *
 * All keys are required when the block exists (the Rust wire always serializes
 * them); a missing key is a contract violation, never a guessed `null`. `null`
 * itself is the host's explicit "no metrics on this carrier".
 */
function parsePerformance(value: unknown): void {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const record = value as Record<string, unknown>;
  if (!PERFORMANCE_FIELDS.every(field => field in record && isOptionalMetric(record[field]))) {
    throw Error('CAD_STATUS_INVALID');
  }
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
  // The carrier profile. The block is optional as a whole: a recorded pre-P1
  // payload has no `hostProfile` at all, and it must still parse — the readers
  // then apply the documented legacy
  // defaults (`native-below`, palettes available, standard views undeclared)
  // rather than a guessed capability. When the block is present, every key it
  // carries is validated against the frozen JSON contract: an unknown
  // `surfaceMode`/`documentOwner`, a non-boolean palette flag or a non-array
  // standard-view list is a contract violation, never a value to coerce.
  const hostProfile = status.hostProfile;
  if (hostProfile !== undefined) {
    if (!hostProfile || typeof hostProfile !== 'object' || Array.isArray(hostProfile)) {
      throw Error('CAD_STATUS_INVALID');
    }
    const profile = hostProfile as Record<string, unknown>;
    if (profile.surfaceMode !== undefined
      && profile.surfaceMode !== 'native-below' && profile.surfaceMode !== 'dom-canvas') {
      throw Error('CAD_STATUS_INVALID');
    }
    if (profile.documentOwner !== undefined
      && profile.documentOwner !== 'native' && profile.documentOwner !== 'wasm') {
      throw Error('CAD_STATUS_INVALID');
    }
    if (profile.palettesAvailable !== undefined && typeof profile.palettesAvailable !== 'boolean') {
      throw Error('CAD_STATUS_INVALID');
    }
    // `null` is the declared "no standard views are known" answer: it means the
    // carrier never declared a list and keeps the historic allow-all rule. An
    // array is authoritative (including `[]`, which declares none), so anything
    // other than `null` or an array of strings is rejected here.
    if (profile.standardViews !== undefined && profile.standardViews !== null) {
      if (!Array.isArray(profile.standardViews)
        || !profile.standardViews.every((key: unknown) => typeof key === 'string')) {
        throw Error('CAD_STATUS_INVALID');
      }
    }
    // The orbit capability: its own reader validates it (`cad/host-profile.ts`).
    parseOrbitCapability(profile.orbit);
  }
  // The additive layout/reference blocks: absent is the older-host answer
  // (`[]` / model / `[]`), but a present block is validated strictly so a
  // malformed row can never become a guessed tab or reference state.
  if (status.layouts !== undefined
    && (!Array.isArray(status.layouts) || !status.layouts.every(isLayoutTab))) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (status.activeSpace !== undefined) {
    const space = status.activeSpace;
    if (!space || typeof space !== 'object' || Array.isArray(space)) {
      throw Error('CAD_STATUS_INVALID');
    }
    const kind = (space as Record<string, unknown>).kind;
    if (kind !== 'model' && kind !== 'paper') throw Error('CAD_STATUS_INVALID');
    if (kind === 'paper' && !nonNegativeInteger((space as Record<string, unknown>).id)) {
      throw Error('CAD_STATUS_INVALID');
    }
  }
  if (status.references !== undefined
    && (!Array.isArray(status.references) || !status.references.every(isReferenceStatus))) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (status.blocks !== undefined
    && (!Array.isArray(status.blocks) || !status.blocks.every(isBlockStatus))) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (status.groups !== undefined
    && (!Array.isArray(status.groups) || !status.groups.every(isGroupStatus))) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (status.images !== undefined
    && (!Array.isArray(status.images) || !status.images.every(isImageStatus))) {
    throw Error('CAD_STATUS_INVALID');
  }
  // Performance metrics are optional (an older host, or a carrier that publishes
  // none), but a present block must be well-formed: a malformed number is never
  // rendered as a measured value.
  if (status.performance !== undefined && status.performance !== null) {
    parsePerformance(status.performance);
  }
  // The UI theme is optional for a recorded pre-theme payload; a present value
  // must be one of the two documented names (never a free-form CSS token).
  if (status.theme !== undefined && status.theme !== 'dark' && status.theme !== 'light') {
    throw Error('CAD_STATUS_INVALID');
  }
  // The ViewCube orientation is optional (`null`/absent when the host cannot
  // report one), but a present block must be a real finite camera basis.
  if (status.view !== undefined) parseCadViewOrientation(status.view);
  // The pending plugin prompt is optional (`null`/absent when none is open), but
  // a present block must be complete and its kind documented: this block is what
  // routes the user's next line as *answer data*, so a malformed one is refused
  // outright rather than coerced into a guessed prompt.
  if (status.pendingPrompt !== undefined && status.pendingPrompt !== null) {
    parseCadPendingPrompt(status.pendingPrompt);
  }
  // The navigation switches are optional for an older host, but a present block
  // must be complete: a partial block would hide a real host state.
  if (status.navigation !== undefined) parseCadNavigation(status.navigation);
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

