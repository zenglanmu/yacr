// The WebView's implementation of the package's `CadHostBridge`.
//
// This is the seam between the frozen contract package and this host's wire:
// `hostInfo()` reads the `cad_api_status` snapshot (validated by
// `plugin-api/descriptor.ts`), and `request()` serves the wire operations this
// build actually performs. It owns no semantics of its own - every capability
// answer comes from the host's own table, and every refusal is the host's or the
// adapter's name for a call it cannot serve.
//
// # The implemented operations
//
// Four reflected reads are wired (M2.2/M2.3):
// `ApplicationServices.documentManager.mdiActiveDocument`,
// `ApplicationServices.documentManager.documents`,
// `DatabaseServices.database.symbolTables` and `DatabaseServices.database.revision`.
// Each forwards to the shared `cad_api_call` command and hands the reply's `value`
// back **unshaped**: the client package decodes it, so this layer never invents a
// document, a table or a revision and never rewrites a field name.
//
// Every other request - including a `supported` row this adapter has not wired -
// is refused locally as `api.adapter-missing: <namespace>.<operation>` and the
// host is **never** called for it. That is the honest answer while the adapter is
// partial: letting a `supported` row through and answering something cheap would
// fake a capability the host never actually served. Adding one operation means
// adding its `case` below, not loosening the `default`.
//
// `IMPLEMENTED_OPERATIONS` is exported for the tests: it is the single list of
// what this build can answer, and every descriptor row that flips to `supported`
// must appear here.

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
}

/**
 * The wire operations this build actually implements, as
 * `<namespace>.<operation>` (the same label the descriptor uses).
 *
 * Every name listed here must have a `case` in `request()` that calls the host for
 * real, and `test/plugin-api/bridge.test.ts` refuses a descriptor row that is
 * `supported` without a matching entry — so a row can never be advertised before
 * it is served.
 */
export const IMPLEMENTED_OPERATIONS: readonly string[] = [
  'ApplicationServices.documentManager.mdiActiveDocument',
  'ApplicationServices.documentManager.documents',
  'DatabaseServices.database.symbolTables',
  'DatabaseServices.database.revision',
];

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

/**
 * Validate the request envelope before touching the operation name.
 *
 * A malformed request is refused locally; nothing is sent to the host, and the
 * refusal never claims an operation was attempted.
 */
function requireWellFormedRequest(request: CadApiRequest): void {
  const candidate: unknown = request;
  if (typeof candidate !== 'object' || candidate === null) throw invalidRequest();
  const record = candidate as { readonly namespace?: unknown; readonly operation?: unknown };
  const namespace = record.namespace;
  if (typeof namespace !== 'string' || !API_NAMESPACES.includes(namespace as CadApiNamespace)) {
    throw invalidRequest();
  }
  const operation = record.operation;
  if (typeof operation !== 'string' || operation.length === 0) throw invalidRequest();
}

/**
 * Read the `cad_api_call` reply envelope `{ ok, value?, error? }`.
 *
 * Exactly one half must be present. `ok:false` carries the host's stable marker,
 * which becomes the `CadApiError` code verbatim — no prefix rewriting, no
 * wrapping — so `cad_waiting` and `unsupported-runtime: cad_api.<ns>.<op>` reach the
 * plugin unchanged. `ok:true` hands back `value` untouched for the client to
 * decode. A contradictory or incomplete envelope is `api.invalid_argument:
 * bridge.answer`.
 */
function outcomeValue(answer: unknown): unknown {
  if (typeof answer !== 'object' || answer === null || Array.isArray(answer)) {
    throw malformedAnswer();
  }
  const record = answer as {
    readonly ok?: unknown;
    readonly value?: unknown;
    readonly error?: unknown;
  };
  if (record.ok === false) {
    const error = record.error;
    if (typeof error !== 'string' || error.length === 0) throw malformedAnswer();
    if (record.value !== undefined) throw malformedAnswer();
    throw new CadApiError(error, error);
  }
  if (record.ok !== true) throw malformedAnswer();
  if (record.error !== undefined) throw malformedAnswer();
  if (record.value === undefined) throw malformedAnswer();
  return record.value;
}

/** Forward one well-formed request to the shared `cad_api_call` command. */
async function callHost(deps: CadBridgeDeps, request: CadApiRequest): Promise<unknown> {
  const answer = await deps.invoke<unknown>('cad_api_call', {
    namespace: request.namespace,
    operation: request.operation,
    parameters: request.parameters,
  });
  return outcomeValue(answer);
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
          return (await callHost(deps, request)) as Result;
        default:
          throw adapterMissing(request.namespace, request.operation);
      }
    },
  };
}
