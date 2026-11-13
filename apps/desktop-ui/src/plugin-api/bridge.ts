// The WebView's implementation of the package's `CadHostBridge`.
//
// This is the seam between the frozen contract package and this host's wire:
// `hostInfo()` reads the `cad_api_status` snapshot (validated by
// `plugin-api/descriptor.ts`), `request()` serves the reflected reads, and
// `mutate()` serves the one queued mutation. It owns no semantics of its own -
// every capability answer comes from the host's own table, and every refusal is
// the host's or the adapter's name for a call it cannot serve.
//
// # Two lanes
//
// `request` is the **read** lane: one synchronous `cad_api_call` and the reply's
// `value` is handed back **unshaped** (the client package decodes it).
//
// `mutate` is the **write** lane: `cad_api_submit` queues the request and returns
// an acknowledgement, then `cad_api_poll` is drained until the outcome carrying
// our correlation id appears. The id and the polling are this adapter's business;
// the package only learns that the mutation finished, and what it produced. A
// commit therefore never resolves before the host reported what happened.
//
// # Implemented operations
//
// Five reflected reads are wired (M2.2–M2.4):
// `ApplicationServices.documentManager.mdiActiveDocument`,
// `ApplicationServices.documentManager.documents`,
// `DatabaseServices.database.symbolTables`, `DatabaseServices.database.revision`
// and `EditorInput.editor.currentSelection`.
// Three live-session operations are wired over the submit lane (M3.2/M3.3):
// `DatabaseServices.transaction.commit`, `DatabaseServices.database.getObjectId`
// and `DatabaseServices.transaction.getObject`.
// The editor prompts and the immediate `selectAll` (P2a) also use that lane: a
// prompt completes only when the user answers, so it gets a ten-minute wait
// instead of the five-second machine budget.
//
// Every other request - including a `supported` row this adapter has not wired -
// is refused locally as `api.adapter-missing: <namespace>.<operation>` and the
// host is **never** called for it. Adding one operation means adding its `case`
// below, not loosening the `default`.
//
// `IMPLEMENTED_OPERATIONS` is exported for the tests: it is the single list of
// what this build can answer on either lane, and every descriptor row that flips
// to `supported` must appear here.

import type { CadStatus } from '../cad/cad-status.ts';
import type { Invoke } from '../host/invoke.ts';
import { API_NAMESPACES, CadApiError } from '../../../../packages/cad-plugin-api/src/index.ts';
import type {
  CadApiHostInfo,
  CadApiNamespace,
  CadApiRequest,
  CadHostBridge,
} from '../../../../packages/cad-plugin-api/src/index.ts';
import { hostInfoFrom, parseApiStatusPayload } from './descriptor.ts';

/** What the bridge needs: the transport seam and the last status snapshot. */
export interface CadBridgeDeps {
  /** The one IPC seam (`host/invoke.ts`); never the Tauri package directly. */
  readonly invoke: Invoke;
  /** The chrome's latest `cad_status`, read at `hostInfo()` time. */
  readonly status: () => CadStatus | null;
  /**
   * The first poll delay in milliseconds (default 8; it doubles per attempt up to
   * 32). Injected by the tests so they do not sleep for seconds.
   */
  readonly pollDelayMs?: number;
  /**
   * How many `cad_api_poll` calls one mutation may make (by default enough to
   * cover its timeout at the capped delay). Injected by the tests.
   */
  readonly maxPolls?: number;
  /**
   * How long a prompt may stay unanswered (default ten minutes). A prompt waits
   * for a human, so it does not share the five-second budget of a commit.
   * Injected by the tests.
   */
  readonly promptTimeoutMs?: number;
}

/**
 * The wire operations this build actually implements, as
 * `<namespace>.<operation>` (the same label the descriptor uses).
 *
 * Every name listed here must have a `case` in `request()` (reads) or go through
 * `mutate` (writes) for real, and `test/plugin-api/bridge.test.ts` refuses a
 * descriptor row that is `supported` without a matching entry — so a row can never
 * be advertised before it is served.
 */
export const IMPLEMENTED_OPERATIONS: readonly string[] = [
  'ApplicationServices.documentManager.mdiActiveDocument',
  'ApplicationServices.documentManager.documents',
  'DatabaseServices.database.symbolTables',
  'DatabaseServices.database.revision',
  'DatabaseServices.database.getObjectId',
  'DatabaseServices.transaction.getObject',
  'DatabaseServices.transaction.commit',
  'EditorInput.editor.getPoint',
  'EditorInput.editor.getEntity',
  'EditorInput.editor.getSelection',
  'EditorInput.editor.getString',
  'EditorInput.editor.getDouble',
  'EditorInput.editor.getInteger',
  'EditorInput.editor.getKeyword',
  'EditorInput.editor.selectAll',
  'EditorInput.editor.currentSelection',
];

/** First poll delay; doubles per attempt. */
const DEFAULT_POLL_DELAY_MS = 8;
/** Ceiling for the doubling delay. */
const MAX_POLL_DELAY_MS = 32;
/** Enough polls to cover the budget below at the capped delay. */
const DEFAULT_MAX_POLLS = 160;
/** How long a non-prompt submission may stay unobserved before `api.timeout: <id>`. */
const TOTAL_TIMEOUT_MS = 5000;
/** How long a prompt may wait for the user (ten minutes). */
const PROMPT_TIMEOUT_MS = 10 * 60 * 1000;

/**
 * The editor operations that wait for a human.
 *
 * `selectAll` is immediate on the host, but it is answered on the same lane as the
 * prompts and costs nothing to give the same budget; every other operation is a
 * machine answer that must arrive within {@link TOTAL_TIMEOUT_MS}.
 */
const PROMPT_OPERATIONS: readonly string[] = [
  'editor.getPoint',
  'editor.getEntity',
  'editor.getSelection',
  'editor.getString',
  'editor.getDouble',
  'editor.getInteger',
  'editor.getKeyword',
  'editor.selectAll',
];

function timeoutFor(operation: string, deps: CadBridgeDeps): number {
  if (!PROMPT_OPERATIONS.includes(operation)) return TOTAL_TIMEOUT_MS;
  return deps.promptTimeoutMs ?? PROMPT_TIMEOUT_MS;
}

/** The one local refusal code for a request this adapter cannot serve. */
function adapterMissing(namespace: string, operation: string): CadApiError {
  return new CadApiError(
    `api.adapter-missing: ${namespace}.${operation}`,
    `no host adapter implements ${namespace}.${operation}`,
  );
}

/** The one local refusal code for a request that is not even well formed. */
function invalidRequest(): CadApiError {
  return new CadApiError('api.invalid_argument: request', 'invalid argument: request');
}

/** The one refusal code for an answer the adapter cannot understand. */
function malformedAnswer(): CadApiError {
  return new CadApiError('api.invalid_argument: bridge.answer', 'invalid argument: bridge.answer');
}

/** Plain-object narrowing for untrusted invoke replies. */
function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Validate the request envelope before touching the operation name.
 *
 * A malformed request is refused locally; nothing is sent to the host, and the
 * refusal never claims an operation was attempted.
 */
function requireWellFormedRequest(request: CadApiRequest): void {
  const candidate: unknown = request;
  if (!isRecord(candidate)) throw invalidRequest();
  const namespace = candidate['namespace'];
  if (typeof namespace !== 'string' || !API_NAMESPACES.includes(namespace as CadApiNamespace)) {
    throw invalidRequest();
  }
  const operation = candidate['operation'];
  if (typeof operation !== 'string' || operation.length === 0) throw invalidRequest();
}

/**
 * Read one `{ ok, value?, error? }` envelope (the read reply or a poll outcome).
 *
 * Exactly one half must be present. `ok:false` carries the host's stable marker,
 * which becomes the `CadApiError` code verbatim — no prefix rewriting, no
 * wrapping — so `cad_waiting`, `api.revision-conflict` and
 * `unsupported-runtime: cad_api.…` reach the plugin unchanged. `ok:true` hands back
 * `value` untouched for the client to decode. A contradictory or incomplete
 * envelope is `api.invalid_argument: bridge.answer`.
 */
function outcomeValue(answer: unknown): unknown {
  if (!isRecord(answer)) throw malformedAnswer();
  const ok = answer['ok'];
  if (ok === false) {
    const error = answer['error'];
    if (typeof error !== 'string' || error.length === 0) throw malformedAnswer();
    if (answer['value'] !== undefined) throw malformedAnswer();
    throw new CadApiError(error, error);
  }
  if (ok !== true) throw malformedAnswer();
  if (answer['error'] !== undefined) throw malformedAnswer();
  if (answer['value'] === undefined) throw malformedAnswer();
  return answer['value'];
}

/** One correlation id per submission: `apir-<counter>-<random>`. */
let requestCounter = 0;

/**
 * The next submission id.
 *
 * The host's grammar is `[A-Za-z0-9._-]{1,64}` and anything else is refused as
 * `cad_api.invalid_request: requestId`, so the id is built only from those
 * characters. It is an opaque correlation token: compared, never parsed.
 */
function nextRequestId(): string {
  requestCounter += 1;
  const random = Math.floor(Math.random() * 0x1_0000_0000).toString(36);
  return `apir-${requestCounter.toString(36)}-${random}`;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/**
 * One invoke, with a host refusal kept as the stable code.
 *
 * `invoke` already normalises a Tauri/wasm rejection to `Error(message)`; the
 * message *is* the marker on this wire, so it becomes the `CadApiError` code. A
 * poll refusal therefore propagates verbatim instead of decaying into a generic
 * bridge failure.
 */
async function invokeHost(
  deps: CadBridgeDeps,
  command: string,
  args?: Record<string, unknown>,
): Promise<unknown> {
  try {
    return await deps.invoke<unknown>(command, args);
  } catch (failure) {
    const message = failure instanceof Error ? failure.message : String(failure);
    if (message.length === 0) throw malformedAnswer();
    throw new CadApiError(message, message);
  }
}

/** `cad_api_submit` answers `{ accepted: true }`; anything else is malformed. */
function requireAccepted(answer: unknown): void {
  if (!isRecord(answer) || answer['accepted'] !== true) throw malformedAnswer();
}

/** The `cad_api_poll` answer: `{ results: [ { requestId, ok, … } ] }`, oldest first. */
function pollResults(answer: unknown): readonly Readonly<Record<string, unknown>>[] {
  if (!isRecord(answer)) throw malformedAnswer();
  const results = answer['results'];
  if (!Array.isArray(results)) throw malformedAnswer();
  return results.map((entry) => {
    if (!isRecord(entry)) throw malformedAnswer();
    return entry;
  });
}

/**
 * Submit one mutation and wait for its outcome.
 *
 * The poll drains the host's completed-outcome FIFO, so this matches by
 * `requestId` and drops anything else it drains (the single WebView client has one
 * mutation in flight; a straggler is an earlier timed-out request's answer, which
 * no one is waiting for any more). Polls are spaced by a doubling delay capped at
 * 32 ms, and the whole wait is bounded: an unanswered submission is
 * `api.timeout: <requestId>`, never a fabricated result.
 */
async function submitAndWait(deps: CadBridgeDeps, request: CadApiRequest): Promise<unknown> {
  const requestId = nextRequestId();
  const ack = await invokeHost(deps, 'cad_api_submit', {
    requestId,
    namespace: request.namespace,
    operation: request.operation,
    parameters: request.parameters,
  });
  requireAccepted(ack);

  const baseDelay = deps.pollDelayMs ?? DEFAULT_POLL_DELAY_MS;
  const timeoutMs = timeoutFor(request.operation, deps);
  const maxPolls =
    deps.maxPolls ?? Math.max(DEFAULT_MAX_POLLS, Math.ceil(timeoutMs / MAX_POLL_DELAY_MS) + 8);
  const deadline = Date.now() + timeoutMs;
  let delay = baseDelay;

  for (let poll = 0; poll < maxPolls; poll += 1) {
    const outcomes = pollResults(await invokeHost(deps, 'cad_api_poll'));
    const outcome = outcomes.find((entry) => entry['requestId'] === requestId);
    if (outcome !== undefined) return outcomeValue(outcome);
    if (poll > 0 && Date.now() >= deadline) break;
    await sleep(delay);
    delay = Math.min(delay * 2, Math.max(MAX_POLL_DELAY_MS, baseDelay));
  }
  throw new CadApiError(
    `api.timeout: ${requestId}`,
    `the host did not report the outcome of ${requestId}`,
  );
}

/**
 * Build the host bridge for this WebView.
 *
 * `hostInfo()` lets a host refusal propagate verbatim: `invoke` already
 * normalises a Tauri rejection to `Error(message)`, and this adapter neither
 * wraps it nor invents a payload. A malformed answer throws
 * `cad_api_status: <detail>` from the decoder.
 */
export function createCadHostBridge(deps: CadBridgeDeps): CadHostBridge {
  return {
    async hostInfo(): Promise<CadApiHostInfo> {
      const answer = await deps.invoke<unknown>('cad_api_status');
      return hostInfoFrom(parseApiStatusPayload(answer), deps.status());
    },
    async request<Result>(request: CadApiRequest): Promise<Result> {
      requireWellFormedRequest(request);
      // The label is `namespace.operation`, exactly the descriptor's spelling: a
      // request that names another namespace is not this operation, and the host
      // must not be asked to sort that out.
      switch (`${request.namespace}.${request.operation}`) {
        case 'ApplicationServices.documentManager.mdiActiveDocument':
        case 'ApplicationServices.documentManager.documents':
        case 'DatabaseServices.database.symbolTables':
        case 'DatabaseServices.database.revision':
        case 'EditorInput.editor.currentSelection':
          return (await callHost(deps, request)) as Result;
        default:
          throw adapterMissing(request.namespace, request.operation);
      }
    },
    async mutate<Result>(request: CadApiRequest): Promise<Result> {
      requireWellFormedRequest(request);
      // Reads of the live session (`getObjectId`, `getObject`) travel this lane
      // too: the host serves them from the session, so they are queued and polled
      // exactly like a commit. The label check is the same list, so a supported row
      // the adapter has not wired still refuses locally.
      if (!IMPLEMENTED_OPERATIONS.includes(`${request.namespace}.${request.operation}`)) {
        throw adapterMissing(request.namespace, request.operation);
      }
      return (await submitAndWait(deps, request)) as Result;
    },
  };
}

/** Forward one well-formed read to the shared `cad_api_call` command. */
async function callHost(deps: CadBridgeDeps, request: CadApiRequest): Promise<unknown> {
  const answer = await deps.invoke<unknown>('cad_api_call', {
    namespace: request.namespace,
    operation: request.operation,
    parameters: request.parameters,
  });
  return outcomeValue(answer);
}
