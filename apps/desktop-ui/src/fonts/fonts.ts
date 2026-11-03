// The Font Manager's model: the `cad_fonts` wire contract, its strict
// validation, the read-only row projection and the one-shot read state.
//
// Pure data + IPC only (no JSX, no React, no DOM), so the contract is testable
// without a window — the same shape `layers/layers.ts` uses for the layer table.
//
// The host owns the font registry. This module never registers a face, never
// touches the font package on disk and never retries a failed load: `cad_fonts`
// is a **report**, and the panel shows exactly what it answered.
//
// Wire shape (the Rust side owns it):
//
//   cad_fonts -> {
//     available: boolean,
//     reason: string | null,        // catalog key when unavailable
//     fontsDir: string | null,      // effective package directory
//     catalogEntries: number,
//     locale: "latin" | "zh-Hans" | "zh-Hant" | "ja" | "ko",
//     defaultFace: string | null,
//     localeMissing: string | null, // detail when this locale has no CJK face
//     faces: [{ key, file, kind, source, status, detail }]
//   }
//
// Two rules this module is built around:
//
//  * **No coercion.** A malformed answer throws (`FONTS_PAYLOAD_INVALID`), it is
//    never turned into an empty success. An empty `faces` array is a real host
//    answer ("no faces registered"), not the same thing as an unreadable reply.
//  * **`available: false` is its own state.** Its named `reason` is what the
//    panel shows; the empty state would claim the registry answered with nothing.
//
// `cad_waiting` (the host has not run its first owner tick yet) is a **loading**
// state, not an error and not an empty registry.

/** Tauri IPC arguments, the subset of the API this module sends. */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;

/** The typed IPC call this module needs (`invoke` satisfies it). */
export type Invoke = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The command that reports the font registry. */
export const FONTS_COMMAND = 'cad_fonts';

/** The chrome-local command-line selector for the panel. */
export const FONTS_SELECTOR = 'FONTS';

/** The host's "no session yet" code: a loading state, never an error. */
export const FONTS_WAITING_CODE = 'cad_waiting';

/** The stable code every malformed payload carries. */
export const FONTS_PAYLOAD_INVALID = 'FONTS_PAYLOAD_INVALID';

/** Font technologies the report can name. */
export type FontKind = 'mesh' | 'shx' | 'system' | 'unknown';

/** Where a face came from. */
export type FontSource = 'package' | 'explicit' | 'system';

/** The closed set of face states. */
export type FontStatus = 'loaded' | 'failed' | 'unresolved' | 'unsupported';

/** The locales the host classifies its default face for. */
export type FontLocale = 'latin' | 'zh-Hans' | 'zh-Hant' | 'ja' | 'ko';

/** One face as the host's answer describes it (the only row shape ever shown). */
export interface FontFace {
  /** Registered key or requested name: the row's display identity. */
  readonly key: string;
  /** Resolved file, or `null` when the host named none. */
  readonly file: string | null;
  /** The font technology. */
  readonly kind: FontKind;
  /** Where the face came from. */
  readonly source: FontSource;
  /** The face's state. */
  readonly status: FontStatus;
  /** The host's detail for this face, or `null`. */
  readonly detail: string | null;
}

/** One validated `cad_fonts` answer. */
export interface FontsAnswer {
  readonly available: boolean;
  /** Catalog key naming why the report is unavailable, or `null`. */
  readonly reason: string | null;
  /** The configured/effective package directory, or `null`. */
  readonly fontsDir: string | null;
  /** How many entries the font catalog holds. */
  readonly catalogEntries: number;
  /** The locale the host classified its fallback face for. */
  readonly locale: FontLocale;
  /** The face the host falls back to, or `null`. */
  readonly defaultFace: string | null;
  /** Detail when this locale has no CJK face, or `null`. */
  readonly localeMissing: string | null;
  /** Distinct font keys the open drawing references (lazy-loader input). */
  readonly requested: readonly string[];
  /** Whether the drawing has text even when it names no font. */
  readonly textNeeded: boolean;
  /** The registered/requested faces, in the host's order. */
  readonly faces: readonly FontFace[];
}

/** The three row groups the panel renders. */
export type FontGroup = 'loaded' | 'failed' | 'unresolved';

/** The badge tone of a face state (the class suffix the panel renders). */
export type FontTone = 'ok' | 'error' | 'muted';

/** Catalog key for each closed status value. */
export const FONT_STATUS_KEYS: Readonly<Record<FontStatus, string>> = Object.freeze({
  loaded: 'fonts.status.loaded',
  failed: 'fonts.status.failed',
  unresolved: 'fonts.status.unresolved',
  unsupported: 'fonts.status.unsupported',
});

/** Catalog key for each closed source value. */
export const FONT_SOURCE_KEYS: Readonly<Record<FontSource, string>> = Object.freeze({
  package: 'fonts.source.package',
  explicit: 'fonts.source.explicit',
  system: 'fonts.source.system',
});

/** Which section a face belongs to. */
export function fontGroup(status: FontStatus): FontGroup {
  if (status === 'loaded') return 'loaded';
  return status === 'unresolved' ? 'unresolved' : 'failed';
}

/** The badge tone of one state: loaded is ok, real failures are error. */
export function fontTone(status: FontStatus): FontTone {
  if (status === 'loaded') return 'ok';
  return status === 'unresolved' ? 'muted' : 'error';
}

/** One renderable font row (all values are the host's, never inferred). */
export interface FontRow extends FontFace {
  readonly group: FontGroup;
  readonly statusKey: string;
  readonly sourceKey: string;
  readonly tone: FontTone;
}

/** The rows of an answer, grouped in the panel's fixed section order. */
export interface FontRowGroups {
  readonly loaded: FontRow[];
  readonly failed: FontRow[];
  readonly unresolved: FontRow[];
}

/** The counts the panel's section headings and summary read. */
export interface FontSummary {
  readonly total: number;
  readonly loaded: number;
  readonly failed: number;
  readonly unresolved: number;
}

/** Project one face into its render row; nothing is completed from a guess. */
export function fontRow(face: FontFace): FontRow {
  return {
    ...face,
    group: fontGroup(face.status),
    statusKey: FONT_STATUS_KEYS[face.status],
    sourceKey: FONT_SOURCE_KEYS[face.source],
    tone: fontTone(face.status),
  };
}

/** Group the host's faces for the three read-only sections. */
export function projectFontRows(faces: readonly FontFace[] | null | undefined): FontRowGroups {
  const groups: { loaded: FontRow[]; failed: FontRow[]; unresolved: FontRow[] } = {
    loaded: [],
    failed: [],
    unresolved: [],
  };
  for (const face of Array.isArray(faces) ? faces : []) {
    const row = fontRow(face);
    groups[row.group].push(row);
  }
  return groups;
}

/** Count the groups (the summary never guesses a number). */
export function fontSummary(groups: FontRowGroups | null | undefined): FontSummary {
  const loaded = groups?.loaded.length ?? 0;
  const failed = groups?.failed.length ?? 0;
  const unresolved = groups?.unresolved.length ?? 0;
  return { total: loaded + failed + unresolved, loaded, failed, unresolved };
}

/** Whether typed text is the local `FONTS` selector (case-insensitive). */
export function isFontsCommand(text: unknown): boolean {
  return String(text ?? '').trim().toUpperCase() === FONTS_SELECTOR;
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** A malformed payload is refused by name, never repaired into a success. */
function invalid(detail: string): never {
  throw new Error(`${FONTS_PAYLOAD_INVALID}: ${detail}`);
}

/** A catalog-key-shaped string (`fonts.loaded`, `host.poll_not_ready`, …). */
const KEY_SHAPE = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$/;

const FONT_KINDS: ReadonlySet<string> = new Set(['mesh', 'shx', 'system', 'unknown']);
const FONT_SOURCES: ReadonlySet<string> = new Set(['package', 'explicit', 'system']);
const FONT_STATUSES: ReadonlySet<string> = new Set(['loaded', 'failed', 'unresolved', 'unsupported']);
const FONT_LOCALES: ReadonlySet<string> = new Set(['latin', 'zh-Hans', 'zh-Hant', 'ja', 'ko']);

function textField(fields: Record<string, unknown>, name: string): string {
  const value = fields[name];
  if (typeof value !== 'string' || value.length === 0) invalid(`${name} must be a non-empty string`);
  return value;
}

function nullableText(fields: Record<string, unknown>, name: string): string | null {
  const value = fields[name];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string') invalid(`${name} must be a string or null`);
  return value;
}

function nullableTextNonEmpty(fields: Record<string, unknown>, name: string): string | null {
  const value = nullableText(fields, name);
  if (value !== null && value.length === 0) invalid(`${name} must be null or non-empty`);
  return value;
}

function enumField<T extends string>(fields: Record<string, unknown>, name: string, allowed: ReadonlySet<string>): T {
  const value = fields[name];
  if (typeof value !== 'string' || !allowed.has(value)) invalid(`${name} is not one of the known values`);
  return value as T;
}

/** Validate and normalise one face row. Every field is required. */
function parseFace(value: unknown, index: number): FontFace {
  const fields = recordOf(value);
  if (!fields) invalid(`faces[${index}] must be an object`);
  return {
    key: textField(fields, 'key'),
    file: nullableTextNonEmpty(fields, 'file'),
    kind: enumField<FontKind>(fields, 'kind', FONT_KINDS),
    source: enumField<FontSource>(fields, 'source', FONT_SOURCES),
    status: enumField<FontStatus>(fields, 'status', FONT_STATUSES),
    detail: nullableText(fields, 'detail'),
  };
}

/**
 * Validate one `cad_fonts` answer.
 *
 * Throws [`FONTS_PAYLOAD_INVALID`] on any field that is not the contract's exact
 * shape: a missing boolean is not `false`, a missing array is not empty, and a
 * row with an unknown status is not dropped. `available: false` must carry a
 * catalog-key reason; an available report must not.
 */
export function parseFontsAnswer(answer: unknown): FontsAnswer {
  const fields = recordOf(answer);
  if (!fields) invalid('the answer must be an object');

  const available = fields.available;
  if (typeof available !== 'boolean') invalid('available must be a boolean');

  const reason = nullableText(fields, 'reason');
  if (available) {
    if (reason !== null) invalid('an available report must not carry a reason');
  } else if (reason === null || !KEY_SHAPE.test(reason)) {
    invalid('an unavailable report must carry a named catalog reason');
  }

  const fontsDir = nullableTextNonEmpty(fields, 'fontsDir');
  const catalogEntries = fields.catalogEntries;
  if (typeof catalogEntries !== 'number' || !Number.isSafeInteger(catalogEntries) || catalogEntries < 0) {
    invalid('catalogEntries must be a non-negative safe integer');
  }
  const locale = enumField<FontLocale>(fields, 'locale', FONT_LOCALES);
  const defaultFace = nullableTextNonEmpty(fields, 'defaultFace');
  const localeMissing = nullableTextNonEmpty(fields, 'localeMissing');

  const requestedSource = fields.requested;
  if (!Array.isArray(requestedSource)) invalid('requested must be an array');
  const requested = requestedSource.map((key, index) => {
    if (typeof key !== 'string' || key.length === 0) invalid(`requested[${index}] must be a non-empty string`);
    return key;
  });
  if (typeof fields.textNeeded !== 'boolean') invalid('textNeeded must be a boolean');
  const textNeeded = fields.textNeeded;

  const facesSource = fields.faces;
  if (!Array.isArray(facesSource)) invalid('faces must be an array');
  const faces = facesSource.map((face, index) => parseFace(face, index));
  const seen = new Set<string>();
  for (const face of faces) {
    if (seen.has(face.key)) invalid(`duplicate face key ${JSON.stringify(face.key)}`);
    seen.add(face.key);
  }

  return { available, reason, fontsDir, catalogEntries, locale, defaultFace, localeMissing, requested, textNeeded, faces };
}

/** The raw text of an IPC failure, never empty. */
function reasonText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? '');
  return text || 'unknown';
}

/**
 * The machine code at the front of a host error string (`cad_waiting`,
 * `unsupported-runtime: …`). Kept separate from the sentence so the waiting
 * state is recognised without parsing prose.
 */
export function errorCode(text: unknown): string {
  const raw = String(text ?? '').trim();
  if (!raw) return '';
  const separator = raw.indexOf(':');
  return (separator === -1 ? raw : raw.slice(0, separator)).trim();
}

/** Whether a failure reason is the host's "no session yet" answer. */
export function isFontsWaiting(reason: unknown): boolean {
  return errorCode(reason) === FONTS_WAITING_CODE;
}

/** The outcome of one `cad_fonts` read. */
export type FontsReadResult = { readonly kind: 'ok'; readonly answer: FontsAnswer } | { readonly kind: 'error'; readonly reason: string };

/**
 * Read the font report. Never throws: a rejected transport and a malformed
 * payload both arrive as a `{kind:'error'}` result, so the panel can show the
 * real reason instead of an invented empty list.
 */
export async function refreshFonts(invoke: Invoke): Promise<FontsReadResult> {
  try {
    const answer = await invoke(FONTS_COMMAND);
    return { kind: 'ok', answer: parseFontsAnswer(answer) };
  } catch (error) {
    return { kind: 'error', reason: reasonText(error) };
  }
}

/** The panel's state word. */
export type FontsPhase = 'idle' | 'loading' | 'ready' | 'empty' | 'unavailable' | 'waiting' | 'error';

/** The panel's whole state, as published to `useSyncExternalStore`. */
export interface FontsState {
  phase: FontsPhase;
  answer: FontsAnswer | null;
  error: { readonly kind: 'read'; readonly reason: string } | null;
}

/** The store the component subscribes to (`useSyncExternalStore`). */
export interface FontsStore {
  getState(): FontsState;
  subscribe(listener: () => void): () => void;
  /** Read once (a second call while a read is in flight joins it). */
  load(): Promise<void>;
}

/** The state before the first read. */
export const EMPTY_FONTS_STATE: FontsState = Object.freeze({
  phase: 'idle',
  answer: null,
  error: null,
});

/**
 * The bounded wait before the one follow-up read after `cad_waiting`.
 *
 * The code means the host has not run its first owner tick yet (the effective
 * font package is resolved there), so one short wait is enough to tell "not
 * ready yet" from "this carrier never answers". It is never a poll loop.
 */
export const FONTS_READ_DELAY_MS = 32;

/** The default wait (injectable through [`createFontsStore`]). */
export function fontsNextTick(): Promise<void> {
  return new Promise<void>(resolve => { setTimeout(resolve, FONTS_READ_DELAY_MS); });
}

/** The knobs [`createFontsStore`] accepts (the wait is a test seam). */
export interface FontsStoreOptions {
  wait?: () => Promise<unknown>;
}

/**
 * Build the panel's one-shot read store.
 *
 * Rules, in one place:
 *  * **Single flight** — callers share the in-flight read; a second call never
 *    interleaves a second answer.
 *  * **No retry loop** — a `cad_waiting` answer gets exactly one follow-up read
 *    after a bounded wait. Every other failure is final until the panel remounts.
 *  * **Unavailable is not empty** — `{available:false}` becomes its own phase and
 *    keeps the named reason; it never reaches `empty`.
 *  * **Malformed is an error** — the strict parser throws, `refreshFonts` reports
 *    the code, and no row is fabricated.
 */
export function createFontsStore(invoke: Invoke, { wait = fontsNextTick }: FontsStoreOptions = {}): FontsStore {
  let state: FontsState = EMPTY_FONTS_STATE;
  let reading: Promise<void> | null = null;
  const listeners = new Set<() => void>();

  function publish(next: FontsState): void {
    state = { ...next };
    for (const listener of [...listeners]) listener();
  }

  const getState = (): FontsState => state;

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  async function read(retry = true): Promise<void> {
    try {
      if (state.phase === 'idle') publish({ ...state, phase: 'loading' });
      const result = await refreshFonts(invoke);
      if (result.kind !== 'ok') {
        if (isFontsWaiting(result.reason)) {
          publish({ ...state, phase: 'waiting', answer: null, error: null });
          if (retry) {
            await wait();
            await read(false);
          }
          return;
        }
        publish({ ...state, phase: 'error', answer: null, error: { kind: 'read', reason: result.reason } });
        return;
      }
      const answer = result.answer;
      const phase: FontsPhase = answer.available
        ? (answer.faces.length > 0 ? 'ready' : 'empty')
        : 'unavailable';
      publish({ phase, answer, error: null });
    } catch (error) {
      // A listener (or an unexpected failure) must never leave the panel stuck in
      // `loading`: it is reported through the same read-failure path.
      publish({ ...state, phase: 'error', answer: null, error: { kind: 'read', reason: reasonText(error) } });
    }
  }

  function load(): Promise<void> {
    reading = reading ? reading.then(() => read(), () => read()) : read();
    return reading;
  }

  return { getState, subscribe, load };
}
