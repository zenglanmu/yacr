// The WebView's implementation of the package's `CadHostBridge`.
//
// This is the seam between the frozen contract package and this host's wire:
// `hostInfo()` reads the `cad_api_status` snapshot (validated by
// `plugin-api/descriptor.ts`), and `request()` is where a future wire operation
// would be served. It owns no semantics of its own - every capability answer
// comes from the host's own table, and every refusal is the host's or the
// adapter's name for a call it cannot serve.
//
// # Zero implemented operations, on purpose
//
// This build serves no wire operation yet. Every well-formed request is refused
// by name as `api.adapter-missing: <namespace>.<operation>`, and the host is
// **never** called for it. That is the honest answer while the adapter is empty:
// the alternative - letting a `supported` row through and answering something
// cheap - would fake a capability the host never actually served. Adding one
// operation means adding its `case` below, not loosening the `default`.
//
// `IMPLEMENTED_OPERATIONS` is exported for the tests: it is the single list of
// what this build can answer, and it must stay empty until a case exists.

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
 * Empty until the first adapter lands; see the module header. Every name listed
 * here must have a `case` in `request()` that calls the host for real, and
 * `test/plugin-api/bridge.test.ts` refuses a descriptor row that is `supported`
 * without a matching entry — so a row can never be advertised before it is served.
 */
export const IMPLEMENTED_OPERATIONS: readonly string[] = [];

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
      // One `case` per implemented wire operation would live here. Until then the
      // only answer is the named refusal: "the host says supported but this
      // adapter has no implementation" must fail loud, never fake a success.
      switch (request.operation) {
        default:
          throw adapterMissing(request.namespace, request.operation);
      }
    },
  };
}
