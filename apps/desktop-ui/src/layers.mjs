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
//                                          overridden, current }],
//                                  active: string | null, overridden: number }
//   cad_set_layer_visible  { name, visible } -> { accepted: true }   (queued)
//   cad_restore_layers                  -> { accepted: true }         (queued)
//   cad_set_active_layer   { name }     -> { accepted: true }         (queued)
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

/** Reads the layer table plus the session's temporary overrides. */
export const LAYERS_COMMAND = 'cad_layers';
/** One temporary visibility override (never a write to the layer table). */
export const SET_LAYER_VISIBLE_COMMAND = 'cad_set_layer_visible';
/** Drops every temporary override at once. */
export const RESTORE_LAYERS_COMMAND = 'cad_restore_layers';
/** Picks the layer new entities are drawn on. */
export const SET_ACTIVE_LAYER_COMMAND = 'cad_set_active_layer';

/**
 * The host's "no session yet" code. Not an error state: the panel says so in the
 * host's own words (`host.poll_not_ready`) instead of claiming an empty drawing.
 */
export const WAITING_CODE = 'cad_waiting';

/**
 * The typed word that opens the docked palette.
 *
 * It is a chrome-local selector, like `PROPERTIES`: the command line intercepts
 * it **before** the router, so no `cad_command` is ever sent for it. The router
 * knows the layer *operations* (`TOGGLELAYER`, `SETLAYERS`, `RESTORELAYERS`,
 * `SETACTIVELAYER`) but not a payload-free `LAYER`, which is why the chrome
 * answers the word itself.
 */
export function isLayersCommand(text) {
  return String(text ?? '').trim().toUpperCase() === 'LAYER';
}

/** The answer for "the host could not be asked at all". */
export const EMPTY_LAYERS_ANSWER = Object.freeze({ layers: [], active: null, overridden: 0 });

/**
 * Normalise one `cad_layers` answer.
 *
 * The host is the authority: nothing is added, inferred or re-ordered. A row is
 * kept only when it has a non-empty string name (a row without a name can neither
 * be shown nor addressed) and a repeated name is dropped after its first
 * occurrence, because two rows with one name are one layer. Missing booleans read
 * as `false` — the contract always sends them, so this only matters for a host
 * that answered something unexpected, and claiming an eye state it did not send
 * would be worse than showing the plain row.
 */
export function parseLayersAnswer(answer) {
  const source = answer && typeof answer === 'object' ? answer : null;
  const rows = Array.isArray(source?.layers) ? source.layers : [];
  const seen = new Set();
  const layers = [];
  for (const row of rows) {
    if (!row || typeof row !== 'object') continue;
    const name = typeof row.name === 'string' ? row.name : '';
    if (!name || seen.has(name)) continue;
    seen.add(name);
    layers.push({
      name,
      visible: row.visible === true,
      databaseVisible: row.databaseVisible === true,
      overridden: row.overridden === true,
      current: row.current === true,
    });
  }
  const active = typeof source?.active === 'string' && source.active ? source.active : null;
  // The counter is the host's number when it sent a usable one; otherwise it is
  // counted from the rows of the same answer (never estimated).
  const overridden = Number.isSafeInteger(source?.overridden) && source.overridden >= 0
    ? source.overridden
    : overrideCount(layers);
  return { layers, active, overridden };
}

/**
 * The rows whose **effective** visibility is on (database flag plus a temporary
 * override). The palette deliberately lists hidden layers too — a hidden layer has
 * to stay reachable to be turned back on — so this is a projection for callers
 * that need the eye state, never the palette's row source.
 */
export function visibleLayers(layers) {
  return (Array.isArray(layers) ? layers : []).filter(layer => layer?.visible === true);
}

/**
 * Case-insensitive substring search over the layer names. A trimmed empty needle
 * returns the very same array (no copy, no re-render churn); an empty list is
 * never turned into a placeholder row.
 */
export function filterLayers(layers, needle) {
  const rows = Array.isArray(layers) ? layers : [];
  const query = String(needle ?? '').trim().toLowerCase();
  if (!query) return rows;
  return rows.filter(layer => String(layer?.name ?? '').toLowerCase().includes(query));
}

/** How many layers carry a temporary visibility override. */
export function overrideCount(layers) {
  return (Array.isArray(layers) ? layers : []).filter(layer => layer?.overridden === true).length;
}

/**
 * FNV-1a over the layer name: the stable input of [`layerTone`].
 *
 * A hash, not a semantic: the same name always answers the same number, and a
 * different name almost always answers a different one.
 */
export function layerNameHash(name) {
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

function hslToHex(hue, saturation, lightness) {
  const channel = offset => {
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
 * This is **not** the drawing's layer colour: the DWG layer table's ACI colour is
 * not on the wire, so painting one would be a fabricated fact. What it is instead
 * is an identity dot — the same muted hue for the same name on every open, so a
 * row stays recognisable without ever claiming to be the real colour. Deliberately
 * low saturation and mid lightness: it reads as a marker next to the name, not as
 * a colour sample a user could mistake for their own layer colour.
 *
 * Accepts a layer object or a bare name. Returns `{ hue, color }` where `color` is
 * a `#rrggbb` string (computed here rather than as a CSS `hsl()` so the value does
 * not depend on the webview's colour-syntax support).
 */
export function layerTone(layer) {
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
export function nextFocusIndex(current, delta, count) {
  const size = Number.isSafeInteger(count) && count > 0 ? count : 0;
  if (!size) return -1;
  const step = Number.isSafeInteger(delta) ? delta : 0;
  const at = Number.isSafeInteger(current) ? current : -1;
  if (at < 0 || at >= size) return step < 0 ? size - 1 : 0;
  return ((at + step) % size + size) % size;
}

/** The raw text of an IPC failure, never empty. */
function reasonText(error) {
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
export function errorCode(text) {
  const raw = String(text ?? '').trim();
  if (!raw) return '';
  const separator = raw.indexOf(':');
  return (separator === -1 ? raw : raw.slice(0, separator)).trim();
}

/** Whether a failure reason is the host's "no session yet" answer. */
export function isWaitingError(reason) {
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
export function ackAdmitted(ack) {
  if (!ack || typeof ack !== 'object') return true;
  return ack.admitted !== false && ack.accepted !== false;
}

function ackReason(ack) {
  if (ack && typeof ack === 'object' && typeof ack.error === 'string' && ack.error) return ack.error;
  return 'CAD_COMMAND_NOT_ACCEPTED';
}

async function sendAck(invoke, command, args) {
  try {
    const ack = await invoke(command, args);
    return ackAdmitted(ack) ? { kind: 'ok' } : { kind: 'error', reason: ackReason(ack) };
  } catch (error) {
    return { kind: 'error', reason: reasonText(error) };
  }
}

function validName(name) {
  return typeof name === 'string' && name.length > 0;
}

/**
 * Read the layer table. Never throws: a failure is a `{kind:'error'}` result so a
 * caller can keep the rows it already shows and report the real reason.
 */
export async function refreshLayers(invoke) {
  try {
    const answer = await invoke(LAYERS_COMMAND);
    return { kind: 'ok', ...parseLayersAnswer(answer) };
  } catch (error) {
    return { kind: 'error', reason: reasonText(error) };
  }
}

/** Turn one layer's effective visibility on or off (a session-only override). */
export function setLayerVisible(invoke, name, visible) {
  if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, SET_LAYER_VISIBLE_COMMAND, { name, visible: visible === true });
}

/** Drop every temporary visibility override. */
export function restoreLayers(invoke) {
  return sendAck(invoke, RESTORE_LAYERS_COMMAND, undefined);
}

/** Make one layer the layer new entities are drawn on. */
export function setActiveLayer(invoke, name) {
  if (!validName(name)) return Promise.resolve({ kind: 'invalid' });
  return sendAck(invoke, SET_ACTIVE_LAYER_COMMAND, { name });
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
export const EMPTY_LAYERS_STATE = Object.freeze({
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
export function nextTick() {
  return new Promise(resolve => { setTimeout(resolve, LAYER_READ_DELAY_MS); });
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
export function createLayersStore(invoke, { wait = nextTick } = {}) {
  let state = EMPTY_LAYERS_STATE;
  let busy = false;
  let reading = null;
  const listeners = new Set();

  function publish(next) {
    state = { ...next };
    for (const listener of [...listeners]) listener();
  }

  const getState = () => state;

  function subscribe(listener) {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  function applyAnswer(answer) {
    publish({
      ...state,
      phase: answer.layers.length > 0 ? 'ready' : 'empty',
      layers: answer.layers,
      active: answer.active,
      overridden: answer.overridden,
      error: null,
    });
  }

  function failRead(reason) {
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

  async function read(retry = true) {
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

  function load() {
    // The very first read starts in this turn (so the skeleton is published with
    // the same call that asks the host); later reads wait for the one in flight.
    reading = reading ? reading.then(read, read) : read();
    return reading;
  }

  /** Whether one row of the current projection shows `name` as `visible`. */
  function showsVisibility(name, visible) {
    const row = state.layers.find(layer => layer.name === name);
    return row ? row.visible === visible : false;
  }

  async function mutate(send, applied) {
    if (busy) return { kind: 'busy' };
    busy = true;
    // The previous message belongs to the previous attempt: clear it as the new
    // one goes out, so a stale error never sits above a fresh state.
    publish({ ...state, busy: true, error: null });
    const result = await send();
    if (result.kind !== 'ok') {
      busy = false;
      publish({ ...state, busy: false, error: { kind: 'mutation', reason: result.reason } });
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
    setVisible: (name, visible) => (validName(name)
      ? mutate(
        () => setLayerVisible(invoke, name, visible),
        () => showsVisibility(name, visible === true),
      )
      : Promise.resolve({ kind: 'invalid' })),
    restore: () => mutate(
      () => restoreLayers(invoke),
      () => state.overridden === 0,
    ),
    setActive: (name) => (validName(name)
      ? mutate(
        () => setActiveLayer(invoke, name),
        () => state.active === name,
      )
      : Promise.resolve({ kind: 'invalid' })),
  };
}
