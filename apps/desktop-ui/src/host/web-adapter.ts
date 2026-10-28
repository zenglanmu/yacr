// The browser transport: the four `shell_*` exports of the wasm CAD session
// (`apps/app-web/src/shell.rs`), mapped onto the command names the shared chrome
// already speaks (`cad_status`, `cad_commands`, `cad_command_aliases`,
// `cad_command`).
//
// This module is the **only** place where a browser-shaped command answer is
// produced. Two rules shape every line of it:
//
// 1. **Nothing is invented.** The status, the command vocabulary and the alias
//    table are the session's own JSON, parsed and handed on; a payload that
//    cannot be read is a thrown `web-adapter:` error, never an empty object the
//    chrome would render as "no document, nothing wrong".
// 2. **No empty success.** The browser carrier is view/navigate only (open, pan,
//    zoom, resize), so every other command - the layer writes, the settings, the
//    view payloads, the popups, `probe_input` - rejects with
//    `unsupported-runtime: <command>`. A resolved `undefined` would look like a
//    completed write to the chrome and to any future caller.
//
// The two exceptions are reports the chrome *sends*, not asks for:
// `canvas_session_status` (the session's own drawing identity) and
// `canvas_runtime_failed` (a failure step). On this carrier the session already
// owns both facts, so there is nobody to relay them to: the adapter remembers
// them (for diagnostics and tests) and resolves - which is not an empty success,
// because the command has no answer by contract. The failure is also printed as
// the stable marker line, so a dead session is loud in the console as well.
//
// The module arrives as `unknown` from the page global the `dom-canvas`
// bootstrap publishes (`src/cad/canvas-host.ts`), is awaited through its `ready`,
// and its four exports are checked before anything is called. A rejected init or a
// missing export fails loudly with `web-adapter: canvas module unavailable: …` -
// the honest recovery is a page reload, never a fabricated status.
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

import { CANVAS_GLOBAL, CANVAS_STEP_UNCLASSIFIED, loadCanvasModule } from '../cad/canvas-host.ts';
import type { CanvasEnv } from '../cad/canvas-host.ts';

/** The four exports the browser transport calls, after validation. */
export interface WebShellModule {
  /** The wasm session's init promise (published by the bootstrap). */
  readonly ready?: unknown;
  /** JSON `cad_status` snapshot (already `camelCase`, carrier fields included). */
  shell_status(): string;
  /** JSON array of command names this carrier can execute. */
  shell_commands(): string;
  /** JSON array of `[alias, canonical]` pairs this carrier resolves. */
  shell_aliases(): string;
  /** One command; refuses by throwing (`unsupported-runtime: cad_command: …`). */
  shell_command(command: string): string;
}

/** The page global the bootstrap publishes the module on. */
export interface WebModuleEnv {
  readonly __YACR_CANVAS_MODULE__?: unknown;
}

/** The four export names, in the order a missing one is reported. */
export const WEB_SHELL_EXPORTS: readonly string[] = ['shell_status', 'shell_commands', 'shell_aliases', 'shell_command'];

/** The command names this transport serves, spelled as the chrome sends them. */
export const WEB_STATUS_COMMAND = 'cad_status';
export const WEB_COMMANDS_COMMAND = 'cad_commands';
export const WEB_ALIASES_COMMAND = 'cad_command_aliases';
export const WEB_COMMAND_COMMAND = 'cad_command';
/** A report the chrome sends; this carrier has nobody to relay it to. */
export const WEB_SESSION_COMMAND = 'canvas_session_status';
/** A failure report the chrome sends; printed and remembered, never answered. */
export const WEB_FAILURE_COMMAND = 'canvas_runtime_failed';

/** The stable prefix of every command this carrier does not implement. */
export const WEB_UNSUPPORTED_PREFIX = 'unsupported-runtime: ';

/** The stable prefix of every module-unavailable failure. */
export const WEB_UNAVAILABLE_PREFIX = 'web-adapter: canvas module unavailable';

/** The stable marker line a failed session prints (`yacr: unsupported-runtime: <step>`). */
export const WEB_MARKER_PREFIX = 'yacr: unsupported-runtime: ';

/** The refusal one unmapped command rejects with. */
export function unsupportedRuntimeMessage(command: string): string {
  return `${WEB_UNSUPPORTED_PREFIX}${command}`;
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
 * export throw), then the four shell exports are checked. Every failure is
 * `web-adapter: canvas module unavailable: …` plus the reason.
 */
export async function loadWebShellModule(env: WebModuleEnv = globalThis): Promise<WebShellModule> {
  const scope = env as WebModuleEnv & Partial<CanvasEnv> & Record<string, unknown>;
  let published = scope[CANVAS_GLOBAL];
  if (plainRecord(published) === null) {
    // One attempt, not a retry loop: `loadCanvasModule` caches its promise per
    // page, so the pixel transport's later call joins this attempt instead of
    // appending a second script tag, and a failed init stays failed.
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
 * `shape` is the JSON kind the wire contract promises (`object` for the status,
 * `array` for the two vocabularies). A non-string answer, unparsable JSON or the
 * wrong kind is a `web-adapter: <command>: …` error - the chrome never receives a
 * coerced or guessed payload.
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

/** What a caller may inject (the console is the only side effect). */
export interface WebAdapterDeps {
  /** Where the marker line goes; defaults to the global console. */
  readonly console?: { error(...args: unknown[]): void };
}

/** The browser transport [`createWebAdapter`] returns. */
export interface WebAdapter {
  /** Serve one command; rejects loudly for everything this carrier cannot do. */
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
 * The browser transport over one wasm session module.
 *
 * See the module header for the map and the two no-answer report commands.
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
        case WEB_STATUS_COMMAND:
          return readJson(command, 'shell_status', 'object');
        case WEB_COMMANDS_COMMAND:
          return readJson(command, 'shell_commands', 'array');
        case WEB_ALIASES_COMMAND:
          return readJson(command, 'shell_aliases', 'array');
        case WEB_COMMAND_COMMAND: {
          const module = await moduleOf();
          const name = argRecord(args).command;
          if (typeof name !== 'string' || name.length === 0) {
            throw new Error(`web-adapter: ${command}: missing command`);
          }
          try {
            // A refusal is the session's own message (`unsupported-runtime:
            // cad_command: …`); it is rethrown **verbatim** so the command line
            // shows the host's words, never a rewritten copy.
            return parseJson(command, module.shell_command(name), 'object');
          } catch (error) {
            throw error instanceof Error ? error : new Error(String(error));
          }
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
          throw new Error(unsupportedRuntimeMessage(command));
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
