// The Parameters Manager store: one `cad_param_list` read plus the one write
// funnel (`cad_command` with a `-PARAMETERS …` line).
//
// Pure data + IPC only (no JSX, no React, no DOM), so the whole state machine is
// testable without a window — the same shape the layers palette uses.
//
// Rules, in one place:
//
//  * **Single flight** — a second mutation while one is in flight is refused
//    locally (`{kind:'busy'}`, no IPC). The window covers the queued command and
//    its read-back, so a fast double click cannot queue two edits.
//  * **Never optimistic** — an admitted command only re-reads `cad_param_list`;
//    the rows always come from the host's own answer.
//  * **Bounded read-back** — a queued write is read once after ~two host frames;
//    when the change is still not visible (a tick that began before the write
//    landed), one follow-up read closes the window. Two reads, never a poll loop.
//  * **Keep the last good list** — a refused write or a failed re-read leaves the
//    rows untouched and records the host's raw reason.
//  * **Reads are serialised** — two reads never interleave, so the newest answer
//    always wins.
import {
  PARAM_COMMAND,
  PARAM_LIST_COMMAND,
  createParamCommand,
  deleteParamCommand,
  parseParamListAnswer,
  renameParamCommand,
  setParamValueCommand,
} from './params.ts';
import type { ParamListAnswer, ParamRow } from './params.ts';

/**
 * Tauri IPC arguments, the subset of `@tauri-apps/api`'s `InvokeArgs` this
 * module sends. Declared here so the typed `invoke` stays assignable.
 */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;

/** The typed IPC call this module needs (`invoke` satisfies it). */
export type Invoke = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The store's state word. */
export type ParamsPhase = 'idle' | 'loading' | 'ready' | 'empty' | 'waiting' | 'error';

/** A failure the panel reports in the host's own words. */
export interface ParamsError {
  /** `read`: the list read failed; `mutation`: an admitted write was refused. */
  readonly kind: 'read' | 'mutation';
  /** The host's raw reason, never empty. */
  readonly reason: string;
}

/** The whole published state. */
export interface ParamsState {
  phase: ParamsPhase;
  active: string | null;
  rows: readonly ParamRow[];
  total: number;
  busy: boolean;
  error: ParamsError | null;
}

/** The outcome of one write request. */
export type ParamAck =
  | { readonly kind: 'ok' }
  | { readonly kind: 'error'; readonly reason: string }
  /** The panel could not build a command the typed grammar can carry. */
  | { readonly kind: 'invalid' }
  | { readonly kind: 'busy' };

/** The Parameters Manager store the component subscribes to. */
export interface ParamsStore {
  getState(): ParamsState;
  subscribe(listener: () => void): () => void;
  load(): Promise<void>;
  create(name: string, kind: string, expr: string): Promise<ParamAck>;
  setValue(name: string, expr: string): Promise<ParamAck>;
  rename(name: string, next: string): Promise<ParamAck>;
  remove(name: string): Promise<ParamAck>;
}

/** The host's "no session yet" code (a state, not an error). */
export const WAITING_CODE = 'cad_waiting';

export const EMPTY_PARAMS_STATE: ParamsState = Object.freeze({
  phase: 'idle',
  active: null,
  rows: [],
  total: 0,
  busy: false,
  error: null,
});

/**
 * How long to wait before reading a **queued** write back.
 *
 * `{accepted:true}` only means the command is queued: the desktop host applies
 * it on its owner tick (~16 ms), and `cad_param_list` is answered on the session
 * thread, so a read in the same turn would show the pre-write table. Two frames
 * is the documented window the layers palette uses too.
 */
export const PARAM_READ_DELAY_MS = 32;

/** The bounded wait before a read-back (a test seam through `wait`). */
export function nextTick(): Promise<void> {
  return new Promise<void>(resolve => { setTimeout(resolve, PARAM_READ_DELAY_MS); });
}

/** The store's knobs (the wait is a test seam). */
export interface ParamsStoreOptions {
  wait?: () => Promise<unknown>;
}

/** The machine code at the front of a host error string. */
function errorCode(reason: unknown): string {
  const text = reasonText(reason);
  const separator = text.indexOf(': ');
  return separator < 0 ? text.trim() : text.slice(0, separator).trim();
}

function reasonText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? '');
  return text || 'unknown';
}

/** Whether a read answered the "no session yet" code. */
export function isWaitingError(reason: unknown): boolean {
  return errorCode(reason) === WAITING_CODE;
}

/** Whether the host admitted (queued) the write. */
function accepted(response: unknown): boolean {
  return Boolean(response) && (response as { accepted?: unknown }).accepted === true;
}

/**
 * Build the Parameters Manager store over one `invoke`.
 *
 * The store is a plain object, not a hook, so its single-flight, error-retention
 * and re-read behaviour are testable without React. The component subscribes
 * through `useSyncExternalStore`.
 */
export function createParamsStore(invoke: Invoke, { wait = nextTick }: ParamsStoreOptions = {}): ParamsStore {
  let state: ParamsState = EMPTY_PARAMS_STATE;
  let busy = false;
  let reading: Promise<void> | null = null;
  const listeners = new Set<() => void>();

  function publish(next: ParamsState): void {
    state = { ...next };
    for (const listener of [...listeners]) listener();
  }

  const getState = (): ParamsState => state;

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  function applyAnswer(answer: ParamListAnswer): void {
    publish({
      ...state,
      phase: answer.rows.length > 0 ? 'ready' : 'empty',
      active: answer.active,
      rows: answer.rows,
      total: answer.total,
      error: null,
    });
  }

  function failRead(reason: string): void {
    if (isWaitingError(reason)) {
      // No session (yet): the rows described a session that is not answering, so
      // they are dropped rather than shown as current. A state, not an error.
      publish({ ...state, phase: 'waiting', active: null, rows: [], total: 0, error: null });
      return;
    }
    publish({
      ...state,
      phase: state.rows.length > 0 ? 'ready' : 'error',
      error: { kind: 'read', reason },
    });
  }

  async function read(retry = true): Promise<void> {
    try {
      if (state.phase === 'idle') publish({ ...state, phase: 'loading' });
      const value = await invoke(PARAM_LIST_COMMAND);
      applyAnswer(parseParamListAnswer(value));
    } catch (error) {
      const reason = reasonText(error);
      failRead(reason);
      // `cad_waiting` is usually the one-frame window before the host's first
      // session exists; one follow-up read keeps the panel from sitting there.
      if (retry && isWaitingError(reason)) {
        await wait();
        await read(false);
      }
    }
  }

  function load(): Promise<void> {
    // The first read starts in this turn (the skeleton is published with it);
    // later reads wait for the one in flight.
    reading = reading ? reading.then(() => read(), () => read()) : read();
    return reading;
  }

  function hasRow(name: string): boolean {
    return state.rows.some(row => row.name === name.trim());
  }

  /** Send one built command; a missing command is `invalid` before any IPC. */
  async function send(command: string | null): Promise<ParamAck> {
    if (command === null) return { kind: 'invalid' };
    try {
      const response = await invoke(PARAM_COMMAND, { command });
      if (!accepted(response)) throw new Error('CAD_COMMAND_NOT_ACCEPTED');
      return { kind: 'ok' };
    } catch (error) {
      return { kind: 'error', reason: reasonText(error) };
    }
  }

  async function mutate(
    build: () => string | null,
    applied?: () => boolean,
  ): Promise<ParamAck> {
    const command = build();
    if (busy) return { kind: 'busy' };
    if (command === null) return { kind: 'invalid' };
    busy = true;
    // The previous message belongs to the previous attempt: clear it as the new
    // one goes out, so a stale refusal never sits above a fresh state.
    publish({ ...state, busy: true, error: null });
    const result = await send(command);
    if (result.kind !== 'ok') {
      busy = false;
      publish({
        ...state,
        busy: false,
        error: { kind: 'mutation', reason: result.kind === 'error' ? result.reason : 'invalid' },
      });
      return result;
    }
    // Queued, not applied: let the host run it, then read its own answer.
    await wait();
    await load();
    // A tick that began before the write landed would still show the old rows.
    // One follow-up read (and only one) closes that window.
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
    create(name: string, kind: string, expr: string): Promise<ParamAck> {
      return mutate(
        () => createParamCommand(name, kind, expr),
        () => hasRow(name),
      );
    },
    setValue(name: string, expr: string): Promise<ParamAck> {
      // An expression's evaluated result is the host's computation, so there is
      // no local predicate worth retrying for: one read-back is the contract.
      return mutate(() => setParamValueCommand(name, expr));
    },
    rename(name: string, next: string): Promise<ParamAck> {
      return mutate(
        () => renameParamCommand(name, next),
        // A half-applied rename is not a state the database can produce, so both
        // halves are required: the new name resolves and the old one is gone.
        () => hasRow(next) && !hasRow(name),
      );
    },
    remove(name: string): Promise<ParamAck> {
      return mutate(
        () => deleteParamCommand(name),
        () => !hasRow(name),
      );
    },
  };
}
