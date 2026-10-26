// The layers palette's model: the wire contract, its normalisation and the
// single-flight mutation state machine.
//
// Pure data + IPC only (no JSX, no React, no DOM), so the contract is unit
// testable without a window — the same shape `properties.mjs` and `options.mjs`
// use for the other editor surfaces.
//
// The host owns the layer table. This module never writes the DWG layer table and
// never touches a `databaseVisible` flag: `cad_set_layer_visible` is a
// **session-only temporary override**, `cad_restore_layers` drops every override,
// and `cad_set_active_layer` picks the layer new entities are drawn on.
//
// Wire shape (Rust side, frozen):
//
//   cad_layers                -> { layers: [{ name, visible, databaseVisible,
//                                          overridden, current,
//                                          color: { aci: number|null,
//                                                   rgb: "#rrggbb" },
//                                          linetype, lineweight, transparency,
//                                          locked, frozen, off, plottable }],
//                                  active: string | null, overridden: number }
//   cad_set_layer_visible  { name, visible } -> { accepted: true }   (queued)
//   cad_restore_layers                  -> { accepted: true }         (queued)
//   cad_set_active_layer   { name }     -> { accepted: true }         (queued)
//   cad_create_layer       { name }     -> { accepted: true }         (queued)
//   cad_rename_layer       { layer, name } -> { accepted: true }      (queued)
//   cad_delete_layer       { layer }    -> { accepted: true }         (queued)
//   cad_set_layer_property { layer, key, value } -> { accepted: true } (queued)
//
// The last four are **real layer-table writes** (one revision bump each, not
// undoable), unlike the two overrides and the active-layer pick. Their refusals
// are the database's own stable reasons (`layer.name_empty`,
// `layer.name_illegal_char`, `layer.duplicate_name`, `layer.default_undeletable`,
// `layer.in_use`, `layer.property.unsupported`, `layer.property.value`,
// `layer.linetype_missing`, plus the host's `layer.unknown` /
// `layer.ambiguous_name` name resolution) and none of them is repaired or
// swallowed here.
//
// The attribute fields on a row are **display/edit inputs, not behaviour**:
// `off` and `frozen` hide the layer through the existing `visible` path (a frozen
// or off layer reads `databaseVisible: false`), while `locked`, `plottable`, the
// colour, the linetype, the lineweight and the transparency are stored only — a
// layer colour does **not** recolour `ByLayer` entities (import flattening). The
// values are the host's own canonical write grammar: `lineweight` is `ByLayer` /
// `Default` / a 1/100 mm integer string, `transparency` is a percent-transparent
// string (`"0"` = opaque), and both can be sent straight back to
// `cad_set_layer_property`.
//
// Three consequences this module is built around:
//
//  * `accepted: true` means **queued, not applied**, so a success re-reads
//    `cad_layers()` only after the next tick — reading inside the same turn would
//    hand back the pre-mutation rows.
//  * A refusal **throws** (`layer.unknown: …`, `layer.no_document: …`,
//    `unsupported_on_surface_mode: …`). The rows are kept and the raw reason is
//    reported; nothing is retried behind the user's back.
//  * `cad_layers()` throws `cad_waiting` before a session exists. That is "not
//    ready", **not** an empty drawing: showing `layers.empty` ("No layers") there
//    would claim a drawing with no layers, so it is its own state.
//
// No drawing open answers `{ layers: [], active: null, overridden: 0 }`, and that
// empty answer is what the panel renders — no row is ever invented here.

/**
 * Tauri IPC arguments, the subset of `@tauri-apps/api`'s `InvokeArgs` this
 * module sends. Declared here so the typed `invoke` stays assignable.
 */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;

/** The typed IPC call this module needs (`invoke` satisfies it). */
export type Invoke = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** One layer's stored colour: the ACI identity (or `null`) plus `#rrggbb`. */
export interface LayerColor {
  /** The source ACI index, `null` for a true colour or when the host sent none. */
  readonly aci: number | null;
  /**
   * The resolved `#rrggbb` value, lowercase.
   *
   * An **empty string** means the host did not report a usable colour (a host
   * that answered an unexpected shape); it never means "black". A consumer falls
   * back to [`layerTone`] in that case instead of painting an invented colour.
   */
  readonly rgb: string;
}

/** One layer as the host's answer describes it (the only row shape ever shown). */
export interface LayerRow {
  /** The layer name: non-empty, unique within one answer. */
  readonly name: string;
  /** **Effective** visibility (database flag plus a temporary override). */
  readonly visible: boolean;
  /** The drawing's own visibility flag, before any override. */
  readonly databaseVisible: boolean;
  /** Whether a session-only temporary override is in force. */
  readonly overridden: boolean;
  /** Whether this is the layer new entities are drawn on. */
  readonly current: boolean;
  /** The stored colour (an edit input; it does **not** recolour entities). */
  readonly color: LayerColor;
  /** The stored linetype **name** (`''` when the host reported none). */
  readonly linetype: string;
  /** The stored lineweight in the write grammar's vocabulary: `ByLayer`,
   * `Default` or a **1/100 mm** integer string (`'35'` = 0.35 mm). */
  readonly lineweight: string;
  /** Stored opacity as **percent transparent** (`'0'` = opaque), or
   * `'opacity:<raw>'` when the stored value is outside `0..=1`. */
  readonly transparency: string;
  /** Layer is locked (stored only: no behaviour yet). */
  readonly locked: boolean;
  /** Layer is frozen (hides the layer through the `visible` path). */
  readonly frozen: boolean;
  /** Layer is off (hides the layer through the `visible` path). */
  readonly off: boolean;
  /** Whether the layer plots (stored only: plotting is not implemented). */
  readonly plottable: boolean;
}

/** One normalised `cad_layers` answer. */
export interface LayersAnswer {
  readonly layers: LayerRow[];
  /** The current layer's name, or `null` when the host named none. */
  readonly active: string | null;
  /** How many rows carry a temporary override. */
  readonly overridden: number;
}

/** The palette's state word. */
export type LayersPhase = 'idle' | 'loading' | 'ready' | 'empty' | 'waiting' | 'error';

/** A failure the palette reports in the host's own words. */
export interface LayersError {
  /** `read`: a first read failed; `mutation`: an admitted write was refused. */
  readonly kind: 'read' | 'mutation';
  /** The host's raw reason, never empty. */
  readonly reason: string;
}

/** The palette's whole state, as published to `useSyncExternalStore`. */
export interface LayersState {
  phase: LayersPhase;
  layers: LayerRow[];
  active: string | null;
  overridden: number;
  busy: boolean;
  error: LayersError | null;
}

/** The outcome of one mutation request. */
export type LayerAck =
  | { readonly kind: 'ok' }
  | { readonly kind: 'error'; readonly reason: string }
  | { readonly kind: 'invalid' }
  | { readonly kind: 'busy' };

/** The outcome of one `cad_layers` read. */
export type LayersReadResult = ({ readonly kind: 'ok' } & LayersAnswer) | { readonly kind: 'error'; readonly reason: string };

/** The palette store the component subscribes to (`useSyncExternalStore`). */
export interface LayersStore {
  getState(): LayersState;
  subscribe(listener: () => void): () => void;
  load(): Promise<void>;
  setVisible(name: string, visible: boolean): Promise<LayerAck>;
  restore(): Promise<LayerAck>;
  setActive(name: string): Promise<LayerAck>;
  /** Create a new layer-table entry with this name. */
  createLayer(name: string): Promise<LayerAck>;
  /** Rename the layer currently named `layer` to `name`. */
  renameLayer(layer: string, name: string): Promise<LayerAck>;
  /** Delete the layer currently named `layer`. */
  deleteLayer(layer: string): Promise<LayerAck>;
  /**
   * Write one attribute of the layer currently named `layer`.
   *
   * `key` is one of the host's writable layer keys (`color`, `linetype`,
   * `lineweight`, `transparency`, `off`, `frozen`, `locked`, `plottable`) and
   * `value` follows that key's documented grammar — the **host's** units:
   * `lineweight` is 1/100 mm, `transparency` is percent transparent. A row's own
   * projected value can be sent back unchanged.
   */
  setProperty(layer: string, key: string, value: string): Promise<LayerAck>;
}

/** Reads the layer table plus the session's temporary overrides. */
export const LAYERS_COMMAND = 'cad_layers';
/** One temporary visibility override (never a write to the layer table). */
export const SET_LAYER_VISIBLE_COMMAND = 'cad_set_layer_visible';
/** Drops every temporary override at once. */
export const RESTORE_LAYERS_COMMAND = 'cad_restore_layers';
/** Picks the layer new entities are drawn on. */
export const SET_ACTIVE_LAYER_COMMAND = 'cad_set_active_layer';
/** Creates a new layer-table entry. */
export const CREATE_LAYER_COMMAND = 'cad_create_layer';
/** Renames an existing layer-table entry. */
export const RENAME_LAYER_COMMAND = 'cad_rename_layer';
/** Deletes a layer-table entry. */
export const DELETE_LAYER_COMMAND = 'cad_delete_layer';
/** Writes one stored attribute of a layer (colour, flags, linetype, …). */
export const SET_LAYER_PROPERTY_COMMAND = 'cad_set_layer_property';

/**
 * The host's "no session yet" code. Not an error state: the panel says so in the
 * host's own words (`host.poll_not_ready`) instead of claiming an empty drawing.
 */
export const WAITING_CODE = 'cad_waiting';

/** A plain JSON object, or `null` for anything else (arrays included). */
function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/**
 * The typed word that opens the docked palette.
 *
 * It is a chrome-local selector, like `PROPERTIES`: the command line intercepts
 * it **before** the router, so no `cad_command` is ever sent for it. The router
 * knows the layer *operations* (`TOGGLELAYER`, `SETLAYERS`, `RESTORELAYERS`,
 * `SETACTIVELAYER`) but not a payload-free `LAYER`, which is why the chrome
 * answers the word itself.
 */
export function isLayersCommand(text: unknown): boolean {
  return String(text ?? '').trim().toUpperCase() === 'LAYER';
}

/** The answer for "the host could not be asked at all". */
export const EMPTY_LAYERS_ANSWER: LayersAnswer = Object.freeze({ layers: [], active: null, overridden: 0 });

/**
 * Normalise one `cad_layers` answer.
 *
 * The host is the authority: nothing is added, inferred or re-ordered. A row is
 * kept only when it has a non-empty string name (a row without a name can neither
 * be shown nor addressed) and a repeated name is dropped after its first
 * occurrence, because two rows with one name are one layer. Missing booleans read
 * as `false` and missing strings as `''` — the contract always sends them, so this
 * only matters for a host that answered something unexpected, and claiming an eye
 * state or a colour it did not send would be worse than showing the plain row.
 */
export function parseLayersAnswer(answer: unknown): LayersAnswer {
  const source = recordOf(answer);
  const rows: unknown[] = Array.isArray(source?.layers) ? source.layers : [];
  const seen = new Set<string>();
  const layers: LayerRow[] = [];
  for (const row of rows) {
    const fields = recordOf(row);
    if (!fields) continue;
    const name = typeof fields.name === 'string' ? fields.name : '';
    if (!name || seen.has(name)) continue;
    seen.add(name);
    layers.push({
      name,
      visible: fields.visible === true,
      databaseVisible: fields.databaseVisible === true,
      overridden: fields.overridden === true,
      current: fields.current === true,
      color: parseLayerColor(fields.color),
      linetype: textOf(fields.linetype),
      lineweight: textOf(fields.lineweight),
      transparency: textOf(fields.transparency),
      locked: fields.locked === true,
      frozen: fields.frozen === true,
      off: fields.off === true,
      plottable: fields.plottable === true,
    });
  }
  const active = typeof source?.active === 'string' && source.active ? source.active : null;
  // The counter is the host's number when it sent a usable one; otherwise it is
  // counted from the rows of the same answer (never estimated).
  const reported = source?.overridden;
  const overridden = typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= 0
    ? reported
    : overrideCount(layers);
  return { layers, active, overridden };
}

/** A non-empty string field, or `''` (never a fabricated value). */
function textOf(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

/** The `#rrggbb` shape a colour must have to be paintable as CSS. */
const RGB_PATTERN = /^#[0-9a-fA-F]{6}$/;

/**
 * Normalise one row's `color`.
 *
 * `aci` is kept only when it is a safe integer index (`1..=255`); `rgb` only when
 * it is a real `#rrggbb`. Anything else answers `{aci: null, rgb: ''}` — the row
 * has no colour it can honestly paint, and an empty `rgb` makes a consumer fall
 * back to [`layerTone`] instead of a fabricated swatch.
 */
export function parseLayerColor(value: unknown): LayerColor {
  const fields = recordOf(value);
  const aci = fields?.aci;
  const rgb = fields?.rgb;
  return {
    aci: typeof aci === 'number' && Number.isSafeInteger(aci) && aci >= 1 && aci <= 255 ? aci : null,
    rgb: typeof rgb === 'string' && RGB_PATTERN.test(rgb) ? rgb.toLowerCase() : '',
  };
}

/**
 * The rows whose **effective** visibility is on (database flag plus a temporary
 * override). The palette deliberately lists hidden layers too — a hidden layer has
 * to stay reachable to be turned back on — so this is a projection for callers
 * that need the eye state, never the palette's row source.
 */
export function visibleLayers(layers: LayerRow[] | null | undefined): LayerRow[] {
  return (Array.isArray(layers) ? layers : []).filter(layer => layer?.visible === true);
}

/**
 * Case-insensitive substring search over the layer names. A trimmed empty needle
 * returns the very same array (no copy, no re-render churn); an empty list is
 * never turned into a placeholder row.
 */
export function filterLayers(layers: LayerRow[] | null | undefined, needle: unknown): LayerRow[] {
  const rows = Array.isArray(layers) ? layers : [];
  const query = String(needle ?? '').trim().toLowerCase();
  if (!query) return rows;
  return rows.filter(layer => String(layer?.name ?? '').toLowerCase().includes(query));
}

/** How many layers carry a temporary visibility override. */
export function overrideCount(layers: LayerRow[] | null | undefined): number {
  return (Array.isArray(layers) ? layers : []).filter(layer => layer?.overridden === true).length;
}

/**
 * FNV-1a over the layer name: the stable input of [`layerTone`].
 *
 * A hash, not a semantic: the same name always answers the same number, and a
 * different name almost always answers a different one.
 */
export function layerNameHash(name: unknown): number {
  const text = String(name ?? '');
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

/** The identity dot's saturation/lightness: muted on purpose (see `layerTone`). */
const TONE_SATURATION = 0.24;
const TONE_LIGHTNESS = 0.62;

/** One layer's identity dot: a hue plus the `#rrggbb` it renders as. */
export interface LayerTone {
  readonly hue: number;
  readonly color: string;
}

function hslToHex(hue: number, saturation: number, lightness: number): string {
  const channel = (offset: number): string => {
    const k = (offset + hue / 30) % 12;
    const amplitude = saturation * Math.min(lightness, 1 - lightness);
    const value = lightness - amplitude * Math.max(-1, Math.min(k - 3, Math.min(9 - k, 1)));
    return Math.round(value * 255).toString(16).padStart(2, '0');
  };
  return `#${channel(0)}${channel(8)}${channel(4)}`;
}

/**
 * A stable per-layer swatch, derived from the layer **name**.
 *
 * This is **not** the drawing's layer colour. The real stored colour *is* on the
 * wire now (`LayerRow.color`, `#rrggbb` plus the ACI identity), so a surface that
 * wants the real colour paints `row.color.rgb` — this function is the palette's
 * current identity dot, kept until the layer palette switches to the stored
 * colour. What it is instead is a marker: the same muted hue for the same name on
 * every open, so a row stays recognisable even when the host reported no usable
 * colour (`color.rgb === ''`). Deliberately low saturation and mid lightness: it
 * reads as a marker next to the name, not as a colour sample a user could mistake
 * for their own layer colour.
 *
 * Accepts a layer object or a bare name. Returns `{ hue, color }` where `color` is
 * a `#rrggbb` string (computed here rather than as a CSS `hsl()` so the value does
 * not depend on the webview's colour-syntax support).
 */
export function layerTone(layer: string | { readonly name?: unknown } | null | undefined): LayerTone {
  const name = typeof layer === 'string' ? layer : layer?.name;
  const hash = layerNameHash(name);
  const hue = hash % 360;
  return { hue, color: hslToHex(hue, TONE_SATURATION, TONE_LIGHTNESS) };
}

/**
 * Step a list cursor with wraparound, for `ArrowUp`/`ArrowDown`/`Home`/`End`.
 *
 * `current` outside `0..count-1` means "no row is focused": a positive step then
 * lands on the first row and a negative one on the last, which is exactly what
 * `Home` (`nextFocusIndex(-1, 1, n)`) and `End` (`nextFocusIndex(-1, -1, n)`)
 * need. An empty list has no position, so it answers `-1`.
 */
export function nextFocusIndex(current: unknown, delta: unknown, count: unknown): number {
  const size = typeof count === 'number' && Number.isSafeInteger(count) && count > 0 ? count : 0;
  if (!size) return -1;
  const step = typeof delta === 'number' && Number.isSafeInteger(delta) ? delta : 0;
  const at = typeof current === 'number' && Number.isSafeInteger(current) ? current : -1;
  if (at < 0 || at >= size) return step < 0 ? size - 1 : 0;
  return ((at + step) % size + size) % size;
}

/** The raw text of an IPC failure, never empty. */
function reasonText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? '');
  return text || 'unknown';
}

/**
 * The machine code at the front of a host error string.
 *
 * The host answers `layer.unknown: WALLS`, `cad_waiting` or
 * `unsupported_on_surface_mode`, so the code is everything up to the first `: `
 * separator (or the whole trimmed text when there is none). Kept separate from
 * the sentence so one refusal code can be recognised without parsing prose.
 */
export function errorCode(text: unknown): string {
  const raw = String(text ?? '').trim();
  if (!raw) return '';
  const separator = raw.indexOf(':');
  return (separator === -1 ? raw : raw.slice(0, separator)).trim();
}

/** Whether a failure reason is the host's "no session yet" answer. */
export function isWaitingError(reason: unknown): boolean {
  return errorCode(reason) === WAITING_CODE;
}

/**
 * Whether a mutation ack admitted the request.
 *
 * The contract words the ack as `{ accepted: true }` — **queued**, never applied —
 * and a refusal throws instead of answering. The older shape also allowed an
 * `admitted` flag, so both spellings are honoured; only an ack that explicitly
 * says `false` is read as a refusal, and an ack with neither flag is not treated
 * as a stronger promise than "the host took it".
 */
export function ackAdmitted(ack: unknown): boolean {
  const fields = recordOf(ack);
  if (!fields) return true;
  return fields.admitted !== false && fields.accepted !== false;
}

function ackReason(ack: unknown): string {
  const fields = recordOf(ack);
  const error = fields?.error;
  if (typeof error === 'string' && error) return error;
  return 'CAD_COMMAND_NOT_ACCEPTED';
}

async function sendAck(invoke: Invoke, command: string, args: InvokeArgs | undefined): Promise<LayerAck> {
  try {
    const ack = await invoke(command, args);
    return ackAdmitted(ack) ? { kind: 'ok' } : { kind: 'error', reason: ackReason(ack) };
  } catch (error) {
    return { kind: 'error', reason: reasonText(error) };
  }
}

function validName(name: unknown): name is string {
  return typeof name === 'string' && name.length > 0;
}

/** The boolean a layer flag's value spells, or `null` when it spells none. */
function flagValue(value: string): boolean | null {
  if (value === 'true' || value === '1') return true;
  if (value === 'false' || value === '0') return false;
  return null;
}

/** The ACI index `aci:<n>` names (`1..=255`), or `null`. */
function aciValue(value: string): number | null {
  const match = /^aci:(\d+)$/i.exec(value);
  if (!match) return null;
  const index = Number(match[1]);
  return Number.isSafeInteger(index) && index >= 1 && index <= 255 ? index : null;
}

/**
 * The `#rrggbb` a colour value resolves to (`#RRGGBB`, `rgb:#RRGGBB` or
 * `r,g,b`), or `null` when it is not one of those spellings.
 *
 * Used only by [`showsProperty`]'s read-back check; the authoritative parse is the
 * host's.
 */
function rgbValue(value: string): string | null {
  const explicit = /^rgb:(.*)$/i.exec(value);
  const body = (explicit ? explicit[1] : value).trim();
  const hex = RGB_PATTERN.test(body) ? body.toLowerCase() : null;
  if (hex !== null) return hex;
  const parts = body.split(',').map(part => part.trim());
  if (parts.length !== 3) return null;
  const channels: string[] = [];
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) return null;
    const channel = Number(part);
    if (channel > 255) return null;
    channels.push(channel.toString(16).padStart(2, '0'));
  }
  return `#${channels.join('')}`;
}

/** The canonical percent string a transparency value spells, or `null`. */
function percentValue(value: string): string | null {
  if (!/^\d{1,3}$/.test(value)) return null;
  const percent = Number(value);
  return percent <= 100 ? String(percent) : null;
}

/**
 * Read the layer table. Never throws: a failure is a `{kind:'error'}` result so a
 * caller can keep the rows it already shows and report the real reason.
 */
export async function refreshLayers(invoke: Invoke): Promise<LayersReadResult> {
  try {
    const answer = await invoke(LAYERS_COMMAND);
    return { kind: 'ok', ...parseLayersAnswer(answer) };
  } catch (error) {
    return { kind: 'error', reason: reasonText(error) };
  }
}

/** Turn one layer's effective visibility on or off (a session-only override). */
export function setLayerVisible(invoke: Invoke, name: unknown, visible: unknown): Promise<LayerAck> {
  if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, SET_LAYER_VISIBLE_COMMAND, { name, visible: visible === true });
}

/** Drop every temporary visibility override. */
export function restoreLayers(invoke: Invoke): Promise<LayerAck> {
  return sendAck(invoke, RESTORE_LAYERS_COMMAND, undefined);
}

/** Make one layer the layer new entities are drawn on. */
export function setActiveLayer(invoke: Invoke, name: unknown): Promise<LayerAck> {
  if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, SET_ACTIVE_LAYER_COMMAND, { name });
}

/**
 * Create a new layer-table entry.
 *
 * An empty name is refused locally (`{kind:'invalid'}`, no IPC): the database
 * would refuse it too, but sending a request whose only possible answer is
 * `layer.name_empty` wastes the host's queue slot. Every other validation (the
 * illegal character set, uniqueness, trimming) stays the host's, so its reason
 * arrives unchanged.
 */
export function createLayer(invoke: Invoke, name: unknown): Promise<LayerAck> {
  if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, CREATE_LAYER_COMMAND, { name });
}

/**
 * Rename the layer currently named `layer` to `name`.
 *
 * Both fields are required: `layer` addresses the row the panel has, `name` is
 * the new name. The host resolves `layer` against the drawing's own table, so a
 * stale row is an explicit `layer.unknown` refusal rather than a write to the
 * wrong layer.
 */
export function renameLayer(invoke: Invoke, layer: unknown, name: unknown): Promise<LayerAck> {
  if (!validName(layer) || !validName(name)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, RENAME_LAYER_COMMAND, { layer, name });
}

/**
 * Delete the layer currently named `layer`.
 *
 * The host refuses the default layer `0` (`layer.default_undeletable`) and any
 * layer that still contains objects (`layer.in_use`); nothing is deleted here.
 */
export function deleteLayer(invoke: Invoke, layer: unknown): Promise<LayerAck> {
  if (!validName(layer)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, DELETE_LAYER_COMMAND, { layer });
}

/**
 * Write one stored attribute of the layer currently named `layer`.
 *
 * All three fields are required: `layer` addresses the row the panel has, `key` is
 * one of the host's writable layer keys and `value` is that key's grammar in the
 * **host's** units (`lineweight` = 1/100 mm, `transparency` = percent
 * transparent). Nothing is normalised or converted here: a row's own projected
 * value can be sent back unchanged, and anything the grammar refuses comes back as
 * the host's reason (`layer.property.unsupported`, `layer.property.value`,
 * `layer.linetype_missing`).
 *
 * An empty layer name, key or value is refused locally (`{kind:'invalid'}`, no
 * IPC): no key in the grammar accepts an empty value, and sending a request whose
 * only possible answer is a refusal wastes the host's queue slot.
 */
export function setLayerProperty(
  invoke: Invoke,
  layer: unknown,
  key: unknown,
  value: unknown,
): Promise<LayerAck> {
  if (!validName(layer) || !validName(key)) return Promise.resolve({ kind: 'invalid' });
  if (typeof value !== 'string' || value.trim() === '') return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, SET_LAYER_PROPERTY_COMMAND, { layer, key, value });
}

/**
 * The palette's state before anything was read.
 *
 * `phase` is the panel's state word: `idle`/`loading` (first read, skeleton),
 * `ready` (rows), `empty` (the host really answered "no layers"), `waiting` (the
 * host has no session yet — `cad_waiting`, never rendered as an empty drawing) and
 * `error` (a first read failed, so there is nothing to show). `error` on a panel
 * that already has rows is an *inline* note above the list, not a different phase:
 * a failed re-read must never blank what the user is looking at.
 */
export const EMPTY_LAYERS_STATE: LayersState = Object.freeze({
  phase: 'idle',
  layers: [],
  active: null,
  overridden: 0,
  busy: false,
  error: null,
});

/**
 * How long to wait before reading a **queued** mutation back.
 *
 * `{accepted:true}` only means the write is queued: the host drains the queue and
 * re-mirrors the layer projection on its own owner tick, which runs at a ~16 ms
 * frame cadence. A read in the same turn would therefore hand back the
 * pre-mutation rows, so the read waits two frames.
 */
export const LAYER_READ_DELAY_MS = 32;

/**
 * The bounded wait before a read-back. Injectable through
 * [`createLayersStore`] so the store's tests never depend on wall time.
 */
export function nextTick(): Promise<void> {
  return new Promise<void>(resolve => { setTimeout(resolve, LAYER_READ_DELAY_MS); });
}

/** The knobs [`createLayersStore`] accepts (the wait is a test seam). */
export interface LayersStoreOptions {
  wait?: () => Promise<unknown>;
}

/**
 * Build the palette's store over one `invoke`.
 *
 * The store is a plain object, not a hook, so its behaviour (single flight, error
 * retention, re-read after every admitted mutation) is testable without React. The
 * component subscribes through `useSyncExternalStore`.
 *
 * Rules, in one place:
 *  * **Single flight** — a second mutation while one is in flight is refused
 *    locally (`{kind:'busy'}`, no IPC, no queue). The window covers the queued
 *    mutation **and** its read-back, so a fast double click cannot slip a second
 *    override in between. The buttons are disabled while `busy`; this is the
 *    keyboard/auto-repeat backstop rather than the UI's only guard.
 *  * **Never optimistic** — an admitted mutation only re-reads `cad_layers()`; the
 *    rows always come from the host's own answer.
 *  * **Bounded read-back** — a queued write is read once after ~two host frames;
 *    if the projection still does not show it (a tick that began before the write
 *    landed), one follow-up read closes the window. Two reads, never a poll loop.
 *  * **Keep the last good list** — a refused mutation or a failed re-read leaves
 *    the rows untouched and records the raw reason.
 *  * **Reads are serialised** — two reads never interleave, so the newest answer
 *    always wins and a document change during a read still gets its own read.
 */
export function createLayersStore(invoke: Invoke, { wait = nextTick }: LayersStoreOptions = {}): LayersStore {
  let state: LayersState = EMPTY_LAYERS_STATE;
  let busy = false;
  let reading: Promise<void> | null = null;
  const listeners = new Set<() => void>();

  function publish(next: LayersState): void {
    state = { ...next };
    for (const listener of [...listeners]) listener();
  }

  const getState = (): LayersState => state;

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  function applyAnswer(answer: LayersAnswer): void {
    publish({
      ...state,
      phase: answer.layers.length > 0 ? 'ready' : 'empty',
      layers: answer.layers,
      active: answer.active,
      overridden: answer.overridden,
      error: null,
    });
  }

  function failRead(reason: string): void {
    if (isWaitingError(reason)) {
      // No session (yet). The rows described a session that is not answering, so
      // they are dropped rather than shown as if they were current — and this is
      // a state, not an error: there is nothing for the user to fix.
      publish({ ...state, phase: 'waiting', layers: [], active: null, overridden: 0, error: null });
      return;
    }
    publish({
      ...state,
      phase: state.layers.length > 0 ? 'ready' : 'error',
      error: { kind: 'read', reason },
    });
  }

  async function read(retry = true): Promise<void> {
    try {
      if (state.phase === 'idle') publish({ ...state, phase: 'loading' });
      const result = await refreshLayers(invoke);
      if (result.kind !== 'ok') {
        failRead(result.reason);
        // `cad_waiting` is usually the one-frame window before the host's first
        // mirror, so a single follow-up read (and only one) keeps the panel from
        // sitting in "not ready" until the user asks again.
        if (retry && isWaitingError(result.reason)) {
          await wait();
          await read(false);
        }
        return;
      }
      applyAnswer(result);
    } catch (error) {
      // A listener (or an unexpected answer) blew up: never leave the panel
      // stuck in `loading` — report it through the same read-failure path.
      failRead(reasonText(error));
    }
  }

  function load(): Promise<void> {
    // The very first read starts in this turn (so the skeleton is published with
    // the same call that asks the host); later reads wait for the one in flight.
    reading = reading ? reading.then(() => read(), () => read()) : read();
    return reading;
  }

  /** Whether one row of the current projection shows `name` as `visible`. */
  function showsVisibility(name: string, visible: boolean): boolean {
    const row = state.layers.find(layer => layer.name === name);
    return row ? row.visible === visible : false;
  }

  /** Whether the current projection has a row named `name` (host names are trimmed). */
  function hasLayer(name: string): boolean {
    const needle = name.trim();
    return state.layers.some(layer => layer.name === needle);
  }

  /**
   * Whether the current projection already shows `key = value` on `layer`.
   *
   * This is **not** a validator: it only decides whether one more read-back is
   * worth it (the documented two-frame window). It therefore compares the values
   * the host itself projects (`LayerRow`), normalising the spellings the grammar
   * accepts; a spelling it cannot interpret answers "not shown yet", which costs
   * one extra read and never a wrong claim.
   */
  function showsProperty(layer: string, key: string, value: string): boolean {
    const row = state.layers.find(entry => entry.name === layer.trim());
    if (!row) return false;
    const wanted = value.trim();
    switch (key) {
      case 'off':
      case 'frozen':
      case 'locked':
      case 'plottable': {
        const flag = flagValue(wanted);
        return flag !== null && row[key] === flag;
      }
      case 'color': {
        const aci = aciValue(wanted);
        if (aci !== null) return row.color.aci === aci;
        const rgb = rgbValue(wanted);
        return rgb !== null && row.color.rgb === rgb;
      }
      case 'linetype':
        return row.linetype.toLowerCase() === wanted.toLowerCase();
      case 'lineweight':
        return row.lineweight.toLowerCase() === wanted.toLowerCase();
      case 'transparency': {
        const percent = percentValue(wanted);
        return percent !== null && row.transparency === percent;
      }
      default:
        // An unknown key cannot be interpreted here. The write was still admitted
        // (the host decides), so no extra read is claimed to be necessary.
        return true;
    }
  }

  // The wire senders, under their own names so the store actions below
  // (`createLayer` …) can never be mistaken for the module-level helpers.
  const createLayerRequest = (name: string): Promise<LayerAck> => createLayer(invoke, name);
  const renameLayerRequest = (layer: string, name: string): Promise<LayerAck> =>
    renameLayer(invoke, layer, name);
  const deleteLayerRequest = (layer: string): Promise<LayerAck> => deleteLayer(invoke, layer);
  const setPropertyRequest = (layer: string, key: string, value: string): Promise<LayerAck> =>
    setLayerProperty(invoke, layer, key, value);

  async function mutate(send: () => Promise<LayerAck>, applied?: () => boolean): Promise<LayerAck> {
    if (busy) return { kind: 'busy' };
    busy = true;
    // The previous message belongs to the previous attempt: clear it as the new
    // one goes out, so a stale error never sits above a fresh state.
    publish({ ...state, busy: true, error: null });
    const result = await send();
    if (result.kind !== 'ok') {
      busy = false;
      // `busy`/`invalid` are refused before the send (single flight, name check),
      // so the reachable failure here is the host's own reason; naming the kind
      // otherwise keeps the panel honest instead of showing an empty note.
      const reason = result.kind === 'error' ? result.reason : result.kind;
      publish({ ...state, busy: false, error: { kind: 'mutation', reason } });
      return result;
    }
    // Queued, not applied: let the host run it, then read the host's own answer.
    await wait();
    await load();
    // A tick that began before the write landed would still show the old rows.
    // One follow-up read (and only one) closes that window; after it, whatever
    // the host says stands.
    if (typeof applied === 'function' && !applied()) {
      await wait();
      await load();
    }
    busy = false;
    publish({ ...state, busy: false });
    return { kind: 'ok' };
  }

  return {
    getState,
    subscribe,
    load,
    setVisible(name: string, visible: boolean): Promise<LayerAck> {
      if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
      return mutate(
        () => setLayerVisible(invoke, name, visible),
        () => showsVisibility(name, visible === true),
      );
    },
    restore(): Promise<LayerAck> {
      return mutate(
        () => restoreLayers(invoke),
        () => state.overridden === 0,
      );
    },
    setActive(name: string): Promise<LayerAck> {
      if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
      return mutate(
        () => setActiveLayer(invoke, name),
        () => state.active === name,
      );
    },
    createLayer(name: string): Promise<LayerAck> {
      if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
      return mutate(
        () => createLayerRequest(name),
        () => hasLayer(name),
      );
    },
    renameLayer(layer: string, name: string): Promise<LayerAck> {
      if (!validName(layer) || !validName(name)) return Promise.resolve({ kind: 'invalid' });
      return mutate(
        () => renameLayerRequest(layer, name),
        // The new name resolves and the old one is gone: a half-applied rename is
        // not a state the database can produce, so both halves are required.
        () => hasLayer(name) && !hasLayer(layer),
      );
    },
    deleteLayer(layer: string): Promise<LayerAck> {
      if (!validName(layer)) return Promise.resolve({ kind: 'invalid' });
      return mutate(
        () => deleteLayerRequest(layer),
        () => !hasLayer(layer),
      );
    },
    setProperty(layer: string, key: string, value: string): Promise<LayerAck> {
      if (!validName(layer) || !validName(key)) return Promise.resolve({ kind: 'invalid' });
      if (typeof value !== 'string' || value.trim() === '') return Promise.resolve({ kind: 'invalid' });
      return mutate(
        () => setPropertyRequest(layer, key, value),
        () => showsProperty(layer, key, value),
      );
    },
  };
}
