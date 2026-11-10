// Global error capture: the window events and the wasm panic hook all feed the
// same [`reportError`] store.
//
// Idempotent, and guarded at every boundary: a capture path that throws would
// turn a diagnostic into a second failure. When there is no `window` (the node
// suite never imports this module) it does nothing.
//
// The host side of the panic hand-off is deliberately small:
//
//   * `window.__yacr_report_panic(jsonOrObject)` — called by the wasm glue when a
//     panic is caught;
//   * `window.__YACR_PANIC_LOG__` — an array a pre-init script may append to. It
//     is drained once at install, so a panic raised while the wasm session was
//     still booting is not lost before this listener existed.
import { reportError } from './error-report.ts';

const PANIC_GLOBAL = '__yacr_report_panic';
const PANIC_LOG_GLOBAL = '__YACR_PANIC_LOG__';

let installed = false;

interface PanicDetail {
  readonly message: string;
  readonly location?: string;
  readonly stack?: string;
}

/** The globals this module owns, read structurally so no `declare global` is needed. */
function globals(): Record<string, unknown> {
  return globalThis as unknown as Record<string, unknown>;
}

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() !== '' ? value : undefined;
}

/**
 * Validate a panic payload. Accepts a JSON string or an object shaped
 * `{ message, location?, stack? }`; anything without a message is refused
 * instead of being invented.
 */
function readPanicDetail(input: unknown): PanicDetail | null {
  let value: unknown = input;
  if (typeof value === 'string') {
    try {
      value = JSON.parse(value);
    } catch {
      // A bare string is still a usable message.
      return typeof value === 'string' && value.trim() !== '' ? { message: value } : null;
    }
  }
  if (typeof value === 'string') return nonEmptyString(value) ? { message: value } : null;
  if (typeof value !== 'object' || value === null) return null;
  const record = value as Record<string, unknown>;
  const message = nonEmptyString(record.message);
  if (!message) return null;
  return {
    message,
    location: nonEmptyString(record.location),
    stack: nonEmptyString(record.stack),
  };
}

function reportPanic(input: unknown): void {
  try {
    const detail = readPanicDetail(input);
    if (!detail) return;
    // The parsed fields are the whole payload; keeping the raw JSON as `detail`
    // would print the same message/location/stack a second time under the stack.
    reportError({
      kind: 'panic',
      message: detail.message,
      location: detail.location,
      stack: detail.stack,
    });
  } catch {
    // The capture hook itself must never throw.
  }
}

function messageOf(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value !== null && typeof value === 'object') {
    const message = (value as { message?: unknown }).message;
    if (typeof message === 'string' && message !== '') return message;
  }
  return value === undefined || value === null ? '' : String(value);
}

function stackOf(value: unknown): string | undefined {
  if (value === null || typeof value !== 'object') return undefined;
  return nonEmptyString((value as { stack?: unknown }).stack);
}

const onWindowError: EventListener = event => {
  try {
    const detail = event as ErrorEvent;
    const message = typeof detail.message === 'string' && detail.message !== ''
      ? detail.message
      : detail.error === undefined || detail.error === null ? '' : String(detail.error);
    const filename = typeof detail.filename === 'string' ? detail.filename : '';
    const line = Number.isFinite(detail.lineno) ? detail.lineno : 0;
    const column = Number.isFinite(detail.colno) ? detail.colno : 0;
    const location = filename === '' && line === 0 && column === 0
      ? undefined
      : `${filename}:${line}:${column}`;
    reportError({ kind: 'uncaught', message, location, stack: stackOf(detail.error) });
  } catch {
    // Never let capture break the page.
  }
};

const onUnhandledRejection: EventListener = event => {
  try {
    const reason = (event as PromiseRejectionEvent).reason;
    reportError({ kind: 'rejection', message: messageOf(reason), stack: stackOf(reason) });
  } catch {
    // Never let capture break the page.
  }
};

const onPanicEvent: EventListener = event => {
  try {
    reportPanic((event as CustomEvent<unknown>).detail);
  } catch {
    // Never let capture break the page.
  }
};

/** Drain a pre-init panic log once, oldest first. */
function drainPanicLog(): void {
  try {
    const root = globals();
    const log = root[PANIC_LOG_GLOBAL];
    if (!Array.isArray(log) || log.length === 0) return;
    const entries = log.slice();
    log.length = 0;
    for (const entry of entries) reportPanic(entry);
  } catch {
    // A malformed global is not a reason to fail boot.
  }
}

/**
 * Install the capture listeners and the panic bridge. Calling it twice is a
 * no-op, so a hot reload or a second mount cannot double-report.
 */
export function installGlobalErrorCapture(): void {
  if (installed || typeof window === 'undefined') return;
  installed = true;
  try {
    window.addEventListener('error', onWindowError);
  } catch {
    // A host without the event still gets the panic bridge below.
  }
  try {
    window.addEventListener('unhandledrejection', onUnhandledRejection);
  } catch {
    // As above.
  }
  try {
    window.addEventListener('yacr:panic', onPanicEvent);
  } catch {
    // As above.
  }
  try {
    globals()[PANIC_GLOBAL] = (input: unknown) => reportPanic(input);
  } catch {
    // A frozen global is a host defect; the listeners still run.
  }
  drainPanicLog();
}
