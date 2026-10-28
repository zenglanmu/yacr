// The browser transport: a **thin forwarder** to the wasm CAD session
// (`apps/app-web`), which serves the desktop command set in-process.
//
// The session on the page is the real one (`apps/app-web/src/page.rs`): a real
// `CadSession`, the real command router, the real `cad-db`, the real input seam
// and the real popup state machine. Its `invoke(name, args_json)` export serves
// exactly the 20 wire commands the desktop `#[tauri::command]`s serve, answering
// one envelope: `{"ok":true,"value":…}` or `{"ok":false,"error":"<reason>"}`. So
// there is nothing to map here — one name in, one envelope out — and three rules
// shape every line of this module:
//
// 1. **Nothing is invented.** The status, the command vocabulary and the alias
//    table are the session's own JSON, parsed and handed on; the command answers
//    are the session's own `value`, and a payload that cannot be read is a thrown
//    `web-adapter:` error, never an empty object the chrome would render as "no
//    document, nothing wrong".
// 2. **No empty success.** A refusal is `{ok:false,error}` and is rethrown
//    **verbatim**, so the command line shows the host's own words
//    (`unsupported-runtime: <name>` for a name the carrier does not serve,
//    `popup.no_text_selection: …` for a refused dialog, …). An envelope that is
//    neither shape is a loud failure, never a success.
// 3. **One host-specific behaviour: `OPEN`.** The wasm session has no
//    filesystem: a picker is a DOM capability and the bytes come from the page, so
//    `OPEN` is intercepted here (a hidden `<input type="file">` plus the session's
//    `open_bytes`) and everything else is forwarded. `cad_status.canOpen` is
//    lifted the same way and for the same reason: the session reports it `false`
//    because the session cannot open, the transport reports `true` because it can.
//    The input carries **no** `accept` filter (it sends Android to the media
//    picker) and the selected file is validated by extension instead — see
//    `pickDrawingFile`/`openThroughHost`.
//
// The two other names that are not forwarded are reports the chrome *sends*, not
// asks for: `canvas_session_status` (the session's own drawing identity) and
// `canvas_runtime_failed` (a failure step). On this carrier the session already
// owns both facts, so there is nobody to relay them to: the adapter remembers
// them (for diagnostics and tests) and resolves - which is not an empty success,
// because the command has no answer by contract. The failure is also printed as
// the stable marker line, so a dead session is loud in the console as well.
// (`host/invoke.ts` prefers the Tauri bridge for these two when one exists, which
// is where the X11 host's marker line and exit code live.)
//
// The module arrives as `unknown` from the page global the `dom-canvas` bootstrap
// publishes (`src/cad/canvas-host.ts`), is awaited through its `ready`, and its
// exports are checked before anything is called. A rejected init or a missing
// export fails loudly with `web-adapter: canvas module unavailable: …` - the
// honest recovery is a page reload, never a fabricated status.
//
// # Why this loader also publishes the bootstrap
//
// The bootstrap is injected by the `dom-canvas` **pixel** transport
// (`loadCanvasModule`, `src/cad/canvas-host.ts`), and that transport is only
// mounted once the chrome sees `surfaceMode: "dom-canvas"` - which is a field of
// the `cad_status` answer this adapter serves. In a plain browser nothing else
// injects it, so "the global is absent, fail" would deadlock the page: no status,
// no carrier flag, no transport, no module, no status.
//
// So an absent global is not reported as absent; the loader asks the one module
// that knows how to inject `/canvas/bootstrap.js` to bring it up (`loadCanvasModule`
// is cached per page, so the transport's later call reuses this very attempt), and
// only then fails - with the loader's own words, which name the URL and the fetch
// probe. A page that genuinely cannot load the session (missing artifact, CSP
// without `'wasm-unsafe-eval'`, a rejected init) still gets a loud
// `web-adapter: canvas module unavailable: …`, never a fabricated status.

import {
  CANVAS_GLOBAL, CANVAS_ID, CANVAS_STEP_UNCLASSIFIED, canvasCssSize, loadCanvasModule, parseCanvasStatus,
} from '../cad/canvas-host.ts';
import type { CanvasEnv } from '../cad/canvas-host.ts';
import { rectFromElement } from '../cad/metrics.ts';

/** The session exports this transport calls, after validation. */
export interface WebShellModule {
  /** The wasm session's init promise (published by the bootstrap). */
  readonly ready?: unknown;
  /** JSON `cad_status` snapshot (already `camelCase`, carrier fields included). */
  shell_status(): string;
  /** JSON array of command names this carrier can execute. */
  shell_commands(): string;
  /** JSON array of `{alias, command}` rows this carrier resolves. */
  shell_aliases(): string;
  /**
   * The one wire entry: serve a Tauri command name in-process.
   *
   * Answers the `{"ok":true,"value":…}` / `{"ok":false,"error":…}` envelope as a
   * JSON string (`apps/app-web/src/direct.rs`). The name set is the desktop's, so
   * this transport needs no table of its own.
   */
  invoke(name: string, argsJson: string): unknown;
  /**
   * Feed one wire pointer input (the desktop `probe_input` body: the `ProbeInput`
   * object itself, not an envelope).
   *
   * Answers a JSON ack; a *refusal* is an answer (`accepted: false` with a stable
   * reason), and only a malformed argument object throws.
   */
  probe_input(argsJson: string): unknown;
  /**
   * Open caller-supplied DWG bytes, replacing the session's document.
   *
   * Not part of the wire command set: this is the canvas transport's own open
   * (the `OPEN` command is served here, on top of it). Absent on a module that
   * never got that far, which is why it is optional and checked per call.
   */
  open_bytes?(canvasId: string, label: string, cssW: number, cssH: number, dpr: number, bytes: Uint8Array): unknown;
}

/** The page global the bootstrap publishes the module on. */
export interface WebModuleEnv {
  readonly __YACR_CANVAS_MODULE__?: unknown;
}

/** The export names, in the order a missing one is reported. */
export const WEB_SHELL_EXPORTS: readonly string[] = [
  'shell_status', 'shell_commands', 'shell_aliases', 'invoke', 'probe_input',
];

/** The command names this transport serves, spelled as the chrome sends them. */
export const WEB_STATUS_COMMAND = 'cad_status';
export const WEB_COMMANDS_COMMAND = 'cad_commands';
export const WEB_ALIASES_COMMAND = 'cad_command_aliases';
export const WEB_COMMAND_COMMAND = 'cad_command';
/** One wire pointer input; the session's own `probe_input` export answers it. */
export const WEB_INPUT_COMMAND = 'probe_input';
/** A report the chrome sends; this carrier has nobody to relay it to. */
export const WEB_SESSION_COMMAND = 'canvas_session_status';
/** A failure report the chrome sends; printed and remembered, never answered. */
export const WEB_FAILURE_COMMAND = 'canvas_runtime_failed';

/**
 * The command the **transport** implements, on top of the wasm session's own
 * vocabulary.
 *
 * Opening needs a DOM file picker and the file's bytes, neither of which a wasm
 * session can produce; the session's `open_bytes` then does the import. So `OPEN`
 * is advertised by this adapter (and `cad_status.canOpen` is lifted here) while
 * the session keeps reporting `canOpen: false`, which is the truth about the
 * session.
 */
export const WEB_OPEN_COMMAND = 'OPEN';

/** The host-implemented commands added to the session's vocabulary, in order. */
export const WEB_HOST_COMMANDS: readonly string[] = [WEB_OPEN_COMMAND];

/**
 * The file extensions this carrier opens and the importer reads.
 *
 * Used **after** the picker returns, never as an `accept` filter (see
 * [`pickDrawingFile`]).
 */
export const WEB_OPEN_EXTENSIONS: readonly string[] = Object.freeze(['.dwg', '.dxf']);

/** The stable prefix of every module-unavailable failure. */
export const WEB_UNAVAILABLE_PREFIX = 'web-adapter: canvas module unavailable';

/** The stable marker line a failed session prints (`yacr: unsupported-runtime: <step>`). */
export const WEB_MARKER_PREFIX = 'yacr: unsupported-runtime: ';

/** The lower-cased extension of `name` (including the dot), or `''`. */
function fileExtension(name: unknown): string {
  if (typeof name !== 'string') return '';
  const dot = name.lastIndexOf('.');
  return dot <= 0 ? '' : name.slice(dot).toLowerCase();
}

/**
 * Whether `name` is a file this carrier may hand to the importer.
 *
 * Checked on the **selected** file's name, because the picker deliberately does
 * not filter by type: the browser's `accept` list would otherwise decide the
 * intent, and on Android it decides the *wrong* one (see [`pickDrawingFile`]).
 */
export function isDrawingFileName(name: unknown): boolean {
  return WEB_OPEN_EXTENSIONS.includes(fileExtension(name));
}

/** The stable refusal for a selected file this carrier cannot import. */
export function drawingFileRefusal(name: unknown): string {
  return `web-adapter: ${WEB_COMMAND_COMMAND}: OPEN refused ${JSON.stringify(String(name ?? ''))}:`
    + ` only ${WEB_OPEN_EXTENSIONS.join(' / ')} files can be opened on this carrier`;
}

/**
 * The vocabulary the chrome should load: the session's names **plus** the
 * transport's own ([`WEB_HOST_COMMANDS`]), deduped, first occurrence winning.
 *
 * Order is the session's order first, then the host command, so the list is stable
 * across polls and the session's own vocabulary keeps its published order. Every
 * name is trimmed and upper-cased exactly like `setCommandCatalog` normalises it,
 * so two spellings of one selector cannot both be advertised.
 */
export function commandVocabulary(names: readonly unknown[]): string[] {
  const merged: string[] = [];
  const seen = new Set<string>();
  for (const raw of [...names, ...WEB_HOST_COMMANDS]) {
    if (typeof raw !== 'string') {
      throw new Error(`web-adapter: ${WEB_COMMANDS_COMMAND}: the canvas module reported a non-string command name`);
    }
    const name = raw.trim().toUpperCase();
    if (!name || seen.has(name)) continue;
    seen.add(name);
    merged.push(name);
  }
  return merged;
}

/** The device pixel ratio the picker flow sizes the session with (1 when absent). */
function documentDpr(doc: Document | null): number {
  const ratio = doc?.defaultView?.devicePixelRatio
    ?? (globalThis as { readonly devicePixelRatio?: number }).devicePixelRatio;
  return typeof ratio === 'number' && Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
}

/**
 * Ask the user for a drawing file, resolving `null` when the dialog is dismissed.
 *
 * A hidden `<input type="file">` is created, appended, clicked **inside the user
 * gesture** (the command line's Enter) and removed once it settles:
 *
 * * `change` with a selection -> that file;
 * * `change` with an empty list, or `cancel` (the dismissed dialog, Chromium
 *   113+/WebKit) -> `null`.
 *
 * `cancel` is what keeps a dismissed dialog from hanging the promise forever.
 *
 * # No `accept` filter, deliberately
 *
 * The Chrome/Android intent resolver maps an `accept` list onto MIME types, so
 * `accept=".dwg,.dxf"` becomes `image/vnd.dwg` + `image/vnd.dxf` — both
 * registered **image** types. Android then routes `ACTION_GET_CONTENT` to the
 * media **Photo Picker** ("No photos or videos"), where a drawing saved to
 * `/sdcard/Download` cannot be selected at all. With no `accept` attribute the
 * intent carries no type filter and the system opens the **document** picker
 * (Files/Downloads), which is the only picker that can see a `.dwg` there.
 *
 * The format is therefore validated on the selected file's name
 * ([`isDrawingFileName`], in [`openThroughHost`]), so dropping the filter does not
 * widen what the importer is handed: a wrong type is refused with a named error
 * instead of being passed on. `multiple` stays off: opening replaces the one
 * document this session carries.
 */
function pickDrawingFile(doc: Document): Promise<File | null> {
  return new Promise<File | null>((resolve, reject) => {
    const input = doc.createElement('input');
    input.type = 'file';
    input.multiple = false;
    input.style.display = 'none';
    let settled = false;
    const finish = (file: File | null): void => {
      if (settled) return;
      settled = true;
      input.remove();
      resolve(file);
    };
    input.addEventListener('change', () => finish(input.files?.[0] ?? null));
    input.addEventListener('cancel', () => finish(null));
    (doc.body ?? doc.documentElement).appendChild(input);
    try {
      input.click();
    } catch (error) {
      input.remove();
      reject(new Error(`web-adapter: ${WEB_COMMAND_COMMAND}: the file picker could not open: ${String(error)}`, { cause: error }));
    }
  });
}

/**
 * The host `OPEN` flow: pick a file, check it is a drawing, read its bytes, hand
 * them to the session.
 *
 * Resolves `{accepted: true}` on a completed open **and** on a cancelled dialog: the
 * command was accepted and executed either way, and a cancelled picker is not a
 * failure (the chrome would otherwise paint an error for a deliberate cancel, and
 * nothing about the document changed). A selected file that is not a drawing is a
 * real failure and rejects with [`drawingFileRefusal`] before any byte is read —
 * the picker no longer filters by type (see [`pickDrawingFile`]), so this is the
 * one place that keeps a non-drawing out of the importer. Every other real failure
 * — no canvas, no `open_bytes`, an unreadable file, a refused import — rejects with
 * the real message instead.
 */
async function openThroughHost(module: WebShellModule, doc: Document | null): Promise<unknown> {
  if (doc === null) {
    throw new Error(`web-adapter: ${WEB_COMMAND_COMMAND}: no document to open a file from on this carrier`);
  }
  const canvas = doc.getElementById(CANVAS_ID);
  if (canvas === null) {
    throw new Error(`web-adapter: ${WEB_COMMAND_COMMAND}: no #${CANVAS_ID} element to size the open session from`);
  }
  const size = canvasCssSize(rectFromElement(canvas, documentDpr(doc)));
  if (size === null) {
    throw new Error(`web-adapter: ${WEB_COMMAND_COMMAND}: #${CANVAS_ID} has no usable size to open into`);
  }
  if (typeof module.open_bytes !== 'function') {
    throw new Error(`web-adapter: ${WEB_COMMAND_COMMAND}: the canvas module has no open_bytes export`);
  }
  const file = await pickDrawingFile(doc);
  if (file === null) return { accepted: true };
  // The picker shows every file the document provider offers, so the format gate
  // lives here: a wrong type is named and refused, never handed to the importer.
  if (!isDrawingFileName(file.name)) throw new Error(drawingFileRefusal(file.name));
  const bytes = new Uint8Array(await file.arrayBuffer());
  let status: unknown;
  try {
    status = await module.open_bytes(CANVAS_ID, file.name, size.width, size.height, size.dpr, bytes);
  } catch (error) {
    // A wasm `Result<_, JsValue>` rejects with a bare string; normalise it so the
    // caller's `String(error)` shows the session's sentence, not `<object>`.
    throw error instanceof Error ? error : new Error(String(error), { cause: error });
  }
  // `open_bytes` answers with the session's own status JSON. A non-`ok` answer
  // would mean the document was not installed, which must not look accepted.
  const parsed = parseCanvasStatus(status);
  if (parsed !== null && parsed.ok === false) {
    const step = typeof parsed.step === 'string' ? parsed.step : 'open';
    const detail = typeof parsed.error === 'string' ? parsed.error : String(status);
    throw new Error(`web-adapter: ${WEB_COMMAND_COMMAND}: ${step}: ${detail}`);
  }
  return { accepted: true };
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function plainRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/**
 * Await the published module and hand back its validated call surface.
 *
 * An absent `globalThis.__YACR_CANVAS_MODULE__` first asks the `dom-canvas`
 * loader to inject `/canvas/bootstrap.js` (see the module header for why the page
 * cannot have done it yet); everything else about the load is unchanged: `ready`
 * is awaited (a rejected wasm init must surface here, named, not as a random
 * export throw), then the exports are checked. Every failure is
 * `web-adapter: canvas module unavailable: …` plus the reason.
 */
export async function loadWebShellModule(env: WebModuleEnv = globalThis): Promise<WebShellModule> {
  const scope = env as WebModuleEnv & Partial<CanvasEnv> & Record<string, unknown>;
  let published = scope[CANVAS_GLOBAL];
  if (plainRecord(published) === null) {
    // One attempt, not a retry loop: `loadCanvasModule` shares its in-flight
    // promise per page, so the pixel transport's later call joins this attempt
    // instead of appending a second script tag, and a failed init stays failed.
    try {
      await loadCanvasModule(undefined, scope as CanvasEnv);
    } catch (error) {
      throw new Error(`${WEB_UNAVAILABLE_PREFIX}: ${CANVAS_GLOBAL} is not published and the bootstrap did not load: ${String(error)}`, { cause: error });
    }
    published = scope[CANVAS_GLOBAL];
  }
  const record = plainRecord(published);
  if (record === null) {
    throw new Error(`${WEB_UNAVAILABLE_PREFIX}: ${CANVAS_GLOBAL} is not published (the dom-canvas bootstrap has not run)`);
  }
  if (record.ready === undefined) {
    throw new Error(`${WEB_UNAVAILABLE_PREFIX}: ${CANVAS_GLOBAL}.ready is missing (the dom-canvas bootstrap is incomplete)`);
  }
  try {
    await record.ready;
  } catch (error) {
    throw new Error(`${WEB_UNAVAILABLE_PREFIX}: init failed: ${String(error)}`, { cause: error });
  }
  for (const name of WEB_SHELL_EXPORTS) {
    if (typeof record[name] !== 'function') {
      throw new Error(`${WEB_UNAVAILABLE_PREFIX}: missing export ${name}`);
    }
  }
  return published as WebShellModule;
}

/**
 * One command's JSON answer, parsed.
 *
 * `shape` is the JSON kind the wire contract promises (`object` for the status and
 * for every command envelope, `array` for the two vocabularies). A non-string
 * answer, unparsable JSON or the wrong kind is a `web-adapter: <command>: …`
 * error - the chrome never receives a coerced or guessed payload.
 */
function parseJson(command: string, raw: unknown, shape: 'object' | 'array'): unknown {
  if (typeof raw !== 'string') {
    throw new Error(`web-adapter: ${command}: expected a JSON string from the canvas module`);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(`web-adapter: ${command}: ${String(error)}`, { cause: error });
  }
  const ok = shape === 'array'
    ? Array.isArray(parsed)
    : parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed);
  if (!ok) throw new Error(`web-adapter: ${command}: expected a JSON ${shape} from the canvas module`);
  return parsed;
}

/** One argument object as the JSON string the session's `invoke` takes. */
function commandArgs(args: unknown): string {
  if (args === undefined || args === null) return '{}';
  try {
    return JSON.stringify(args) ?? '{}';
  } catch (error) {
    throw new Error(`web-adapter: the arguments are not JSON: ${String(error)}`, { cause: error });
  }
}

/**
 * The source a [`createWebAdapter`] call takes.
 *
 * A validated module (tests, and any caller that already loaded one) or a
 * loader - the production path, `() => loadWebShellModule()`, so the page global
 * is read **lazily**: the adapter object exists before the bootstrap has run, and
 * a command issued too early fails with the loud unavailable error instead of
 * capturing a stale `undefined`.
 */
export type WebShellSource = WebShellModule | (() => Promise<WebShellModule>);

/** One drawing identity relayed by the session, as this carrier keeps it. */
export interface CanvasSessionRelay {
  readonly label: string | null;
  readonly batches: number | null;
  readonly draws: number | null;
}

/** One failure step reported by the chrome, as this carrier keeps it. */
export interface CanvasRuntimeFailure {
  /** The step the session (or the loader) reported. */
  readonly step: string;
  /** The session's own detail text. */
  readonly detail: string;
  /** The stable marker line printed for this step. */
  readonly marker: string;
}

/** What a caller may inject (the console and the picker's document). */
export interface WebAdapterDeps {
  /** Where the marker line goes; defaults to the global console. */
  readonly console?: { error(...args: unknown[]): void };
  /**
   * The document the `OPEN` picker and the live `#cad-canvas` live in.
   *
   * Defaults to the page's `globalThis.document`; injected so the host flow is
   * exercised against a fake DOM in tests (a real `<input type="file">` cannot be
   * driven from Node).
   */
  readonly document?: Document | null;
}

/** The browser transport [`createWebAdapter`] returns. */
export interface WebAdapter {
  /** Serve one command; rejects loudly when the session refuses or the module cannot be read. */
  invoke(command: string, args?: unknown): Promise<unknown>;
  /** The last `canvas_session_status` report, or `null`. */
  readonly lastSession: CanvasSessionRelay | null;
  /** The last `canvas_runtime_failed` report, or `null`. */
  readonly lastFailure: CanvasRuntimeFailure | null;
}

/** A non-negative safe integer, or `null` (never a rounded or coerced count). */
function canvasCount(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/** The argument record of a report command (`{}` for anything else). */
function argRecord(args: unknown): Record<string, unknown> {
  return plainRecord(args) ?? {};
}

/**
 * Forward one command to the session's `invoke` and unwrap its envelope.
 *
 * The session composes every word: a success yields the `value` the desktop
 * `#[tauri::command]` would return, and a refusal throws its `error` **unchanged**
 * (including the `unsupported-runtime: <name>` answer for a name this carrier does
 * not serve, which is why this module needs no name list of its own). An answer
 * that is neither shape - or a refusal with no reason at all - is a transport
 * failure, never a success.
 */
async function forward(module: WebShellModule, command: string, args: unknown): Promise<unknown> {
  const answer = parseJson(command, await module.invoke(command, commandArgs(args)), 'object') as Record<string, unknown>;
  if (answer.ok === true) return answer.value;
  if (answer.ok === false) {
    const reason = answer.error;
    if (typeof reason === 'string' && reason.length > 0) throw new Error(reason);
    throw new Error(`web-adapter: ${command}: the session refused without a reason (${JSON.stringify(answer)})`);
  }
  throw new Error(`web-adapter: ${command}: the session answer is not an {ok, value|error} envelope`);
}

/**
 * The browser transport over one wasm session module.
 *
 * See the module header for the forwarding rule and the two no-answer reports.
 */
export function createWebAdapter(source: WebShellSource, deps: WebAdapterDeps = {}): WebAdapter {
  const moduleOf: () => Promise<WebShellModule> = typeof source === 'function'
    ? source
    : async () => source;
  const log = deps.console ?? (globalThis as { readonly console?: { error(...args: unknown[]): void } }).console;
  let lastSession: CanvasSessionRelay | null = null;
  let lastFailure: CanvasRuntimeFailure | null = null;

  /** Read one JSON answer from a named export, loudly on any failure. */
  async function readJson(command: string, name: 'shell_status' | 'shell_commands' | 'shell_aliases', shape: 'object' | 'array'): Promise<unknown> {
    const module = await moduleOf();
    return parseJson(command, module[name](), shape);
  }

  return {
    async invoke(command: string, args?: unknown): Promise<unknown> {
      switch (command) {
        case WEB_STATUS_COMMAND: {
          const status = await readJson(command, 'shell_status', 'object') as Record<string, unknown>;
          // `canOpen` is lifted from the session's answer to this transport's:
          // the session cannot open a file (no DOM, no picker, no bytes), this
          // transport can ([`openThroughHost`]). Everything else — `canFit`
          // included — stays exactly what the session reported.
          return { ...status, canOpen: true };
        }
        case WEB_COMMANDS_COMMAND: {
          const names = await readJson(command, 'shell_commands', 'array') as unknown[];
          return commandVocabulary(names);
        }
        case WEB_ALIASES_COMMAND:
          return readJson(command, 'shell_aliases', 'array');
        case WEB_INPUT_COMMAND: {
          // The lane sends the desktop body (`{ input: ProbeInput }`); the session
          // export takes the `ProbeInput` object itself.
          const module = await moduleOf();
          const wrapped = plainRecord(args);
          const input = wrapped !== null && 'input' in wrapped ? wrapped.input : args;
          return parseJson(command, module.probe_input(commandArgs(input)), 'object');
        }
        case WEB_COMMAND_COMMAND: {
          const module = await moduleOf();
          const name = argRecord(args).command;
          if (typeof name !== 'string' || name.length === 0) {
            throw new Error(`web-adapter: ${command}: missing command`);
          }
          // The one command the transport implements itself (see the header).
          if (name.trim().toUpperCase() === WEB_OPEN_COMMAND) {
            return openThroughHost(module, deps.document ?? globalThis.document ?? null);
          }
          return forward(module, command, args);
        }
        case WEB_SESSION_COMMAND: {
          const record = argRecord(args);
          lastSession = {
            label: typeof record.label === 'string' ? record.label : null,
            batches: canvasCount(record.batches),
            draws: canvasCount(record.draws),
          };
          return undefined;
        }
        case WEB_FAILURE_COMMAND: {
          const record = argRecord(args);
          const step = typeof record.step === 'string' && record.step.length > 0 ? record.step : CANVAS_STEP_UNCLASSIFIED;
          const detail = typeof record.detail === 'string' ? record.detail : String(record.detail ?? '');
          lastFailure = { step, detail, marker: `${WEB_MARKER_PREFIX}${step}` };
          log?.error?.(`${WEB_MARKER_PREFIX}${step}`, detail);
          return undefined;
        }
        default:
          // Every wire command (and the honest `unsupported-runtime: <name>` for
          // anything else) is the session's own answer.
          return forward(await moduleOf(), command, args);
      }
    },
    get lastSession(): CanvasSessionRelay | null {
      return lastSession;
    },
    get lastFailure(): CanvasRuntimeFailure | null {
      return lastFailure;
    },
  };
}
