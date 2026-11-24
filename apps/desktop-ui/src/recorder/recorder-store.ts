// The Action Recorder panel store (ADR-0056 D4): one host read plus the typed
// command lines every button sends.
//
// Pure data + IPC (no JSX, no DOM), so the state machine is testable without a
// window. Rules:
//
//  * **never optimistic** — a button only sends `cad_command`; the rows always
//    come from the host's own `cad_action_recorder` answer;
//  * **single flight** — one write at a time, with a read-back after it (the
//    desktop queues commands on the owner tick, so an immediate read would show
//    the previous state);
//  * **keep the host's words** — a refusal is shown raw (`actstop.path: …`,
//    `unsupported-runtime: action-recorder`), never replaced by a generic error;
//  * **no file picker** — the destination is the panel's typed path field.
import {
  ACTION_COMMAND,
  ACTION_RECORDER_COMMAND,
  RECORD_CONTINUE_COMMAND,
  parseActionRecorderAnswer,
  recordDeleteCommand,
  recordPlayCommand,
  recordStartCommand,
  recordStopCommand,
} from './recorder.ts';
import type { RecorderAnswer } from './recorder.ts';

/** The typed IPC call this store needs (`invoke` satisfies it). */
export type InvokeArgs = Record<string, unknown> | Uint8Array | number[] | ArrayBuffer;
export type Invoke = (command: string, args?: InvokeArgs) => Promise<unknown>;

/** The panel's state word. */
export type RecorderPhase = 'idle' | 'loading' | 'ready' | 'empty' | 'error';

/** A failure the panel reports in the host's own words. */
export interface RecorderError {
  readonly kind: 'read' | 'command';
  readonly reason: string;
}

/** The whole published state. */
export interface RecorderState {
  phase: RecorderPhase;
  answer: RecorderAnswer;
  busy: boolean;
  error: RecorderError | null;
}

/** The outcome of one panel action. */
export type RecorderAck =
  | { readonly kind: 'ok' }
  | { readonly kind: 'error'; readonly reason: string }
  | { readonly kind: 'busy' };

/** The Action Recorder store the component subscribes to. */
export interface RecorderStore {
  getState(): RecorderState;
  subscribe(listener: () => void): () => void;
  load(): Promise<void>;
  start(target: string): Promise<RecorderAck>;
  stop(path: string): Promise<RecorderAck>;
  play(name: string): Promise<RecorderAck>;
  remove(name: string): Promise<RecorderAck>;
  resume(): Promise<RecorderAck>;
}

const EMPTY_ANSWER: RecorderAnswer = Object.freeze({
  recording: null,
  macros: [],
  lastMacro: null,
  playback: null,
  lastError: null,
});

export const EMPTY_RECORDER_STATE: RecorderState = Object.freeze({
  phase: 'idle',
  answer: EMPTY_ANSWER,
  busy: false,
  error: null,
});

/** The bounded wait before a read-back (a test seam through `wait`). */
export const RECORDER_READ_DELAY_MS = 32;

/** The default wait; exported so tests can inject their own. */
export function nextTick(): Promise<void> {
  return new Promise<void>(resolve => { setTimeout(resolve, RECORDER_READ_DELAY_MS); });
}

export interface RecorderStoreOptions {
  wait?: () => Promise<unknown>;
}

function reasonText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? '');
  return text || 'unknown';
}

function accepted(response: unknown): boolean {
  return Boolean(response) && (response as { accepted?: unknown }).accepted === true;
}

/**
 * Build the Action Recorder store over one `invoke`.
 *
 * `send` builds the typed command line itself (the component never concatenates
 * a command), sends it through `cad_command`, then re-reads the host state.
 */
export function createRecorderStore(
  invoke: Invoke,
  { wait = nextTick }: RecorderStoreOptions = {},
): RecorderStore {
  let state: RecorderState = EMPTY_RECORDER_STATE;
  let busy = false;
  let reading: Promise<void> | null = null;
  const listeners = new Set<() => void>();

  function publish(next: RecorderState): void {
    state = { ...next };
    for (const listener of [...listeners]) listener();
  }

  const getState = (): RecorderState => state;

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }

  function apply(answer: RecorderAnswer): void {
    publish({
      ...state,
      phase: answer.recording || answer.macros.length > 0 || answer.playback ? 'ready' : 'empty',
      answer,
      error: null,
    });
  }

  async function read(retry = true): Promise<void> {
    try {
      if (state.phase === 'idle') publish({ ...state, phase: 'loading' });
      apply(parseActionRecorderAnswer(await invoke(ACTION_RECORDER_COMMAND)));
    } catch (error) {
      const reason = reasonText(error);
      publish({
        ...state,
        phase: state.phase === 'ready' || state.phase === 'empty' ? state.phase : 'error',
        error: { kind: 'read', reason },
      });
      if (retry && reason.includes('cad_waiting')) {
        await wait();
        await read(false);
      }
    }
  }

  function load(): Promise<void> {
    reading = reading ? reading.then(() => read(), () => read()) : read();
    return reading;
  }

  async function send(command: string): Promise<RecorderAck> {
    if (busy) return { kind: 'busy' };
    busy = true;
    publish({ ...state, busy: true, error: null });
    try {
      const response = await invoke(ACTION_COMMAND, { command });
      if (!accepted(response)) throw new Error('CAD_COMMAND_NOT_ACCEPTED');
    } catch (error) {
      busy = false;
      const reason = reasonText(error);
      publish({ ...state, busy: false, error: { kind: 'command', reason } });
      return { kind: 'error', reason };
    }
    // Let the owner tick apply the queued command before the read-back.
    await wait();
    await load();
    busy = false;
    publish({ ...state, busy: false });
    return { kind: 'ok' };
  }

  function start(target: string): Promise<RecorderAck> {
    return send(recordStartCommand(target));
  }

  function stop(path: string): Promise<RecorderAck> {
    // `ACTSTOP` with no typed path is legal: the host uses the path ACTRECORD
    // was started with, or refuses `actstop.path` when there is none.
    return send(recordStopCommand(path));
  }

  function play(name: string): Promise<RecorderAck> {
    return send(recordPlayCommand(name));
  }

  function remove(name: string): Promise<RecorderAck> {
    return send(recordDeleteCommand(name));
  }

  function resume(): Promise<RecorderAck> {
    return send(RECORD_CONTINUE_COMMAND);
  }

  return { getState, subscribe, load, start, stop, play, remove, resume };
}
