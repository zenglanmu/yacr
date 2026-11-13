// Capability gating and bridge-answer validation.
//
// Everything the package does over the wire goes through `CadApiWire`. It is the
// single place that decides "may this call happen at all?" — before any bridge
// call — so no other module needs to remember the rule, and a refusal never
// reaches the host (which is what the capability suite counts).
//
// The wire has two lanes over the same gate: `call`/`callVoid` ask the read
// bridge, `submit` asks the mutating bridge (queue + reported outcome). A read
// operation names one row; which lane it uses is the client's decision, not the
// descriptor's.
//
// This module is internal: it is imported by concrete modules and by the tests,
// never re-exported from `src/index.ts` (see `test/surface.test.ts`).
//
// Rules, in order:
//   * a row whose status is `pending` -> `api.pending: <ns>.<op>`, no call;
//   * a row the host marks `unsupported` -> `unsupported-runtime: cad_api.<ns>.<op>`,
//     no call;
//   * an operation the host's table does not name at all ->
//     `api.descriptor_missing: <ns>.<op>`, no call (a different failure: the two
//     sides disagree about the contract, rather than this carrier lacking it);
//   * `supported` -> ask the bridge, and map any rejection to `CadApiError`;
//   * a `null`/`undefined` answer to a value-returning call ->
//     `api.invalid_argument: bridge.answer` — never a placeholder;
//   * a void call must answer `{ accepted: true }`; anything else is the same
//     `api.invalid_argument: bridge.answer` (an ack is a real answer).

import { descriptorMissingReason, pendingReason, unsupportedReason } from '../abi.ts';
import type { CadApiNamespace, CadApiOperation } from '../abi.ts';
import { CadApiError, invalidArgument } from '../errors.ts';
import type { CadApiHostInfo, CadHostBridge } from '../host-bridge.ts';

/**
 * The gated transport used by every client proxy.
 *
 * `assertSupported` is the synchronous half (used by getters and by
 * `startTransaction`, which must not await); `call`/`callVoid` are the async
 * half and always run the same check first.
 */
export interface CadApiWire {
  /** The host snapshot this wire was built from. */
  readonly host: CadApiHostInfo;
  /** Throws the pending / unsupported / missing refusal; never calls out. */
  assertSupported(namespace: CadApiNamespace, operation: string): void;
  /** Gate, call, and require a non-null answer (still unvalidated on purpose). */
  call(
    namespace: CadApiNamespace,
    operation: string,
    parameters?: Readonly<Record<string, unknown>>,
  ): Promise<unknown>;
  /** Gate, call, and require the host's `{ accepted: true }` ack. */
  callVoid(
    namespace: CadApiNamespace,
    operation: string,
    parameters?: Readonly<Record<string, unknown>>,
  ): Promise<void>;
  /**
   * Gate, **submit**, and require a non-null answer (the mutating lane).
   *
   * Same gate, different lane: the carrier queues the request and resolves the
   * host's reported outcome, so a mutation never resolves before it happened.
   */
  submit(
    namespace: CadApiNamespace,
    operation: string,
    parameters?: Readonly<Record<string, unknown>>,
  ): Promise<unknown>;
}

/** Plain (non-array) object narrowing: the one gate every decoded answer passes. */
export function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function findOperation(
  host: CadApiHostInfo,
  namespace: CadApiNamespace,
  operation: string,
): CadApiOperation | undefined {
  for (const row of host.operations) {
    if (row.namespace === namespace && row.operation === operation) return row;
  }
  return undefined;
}

function refusal(namespace: CadApiNamespace, operation: string, code: string): CadApiError {
  return new CadApiError(code, undefined, { operation: `${namespace}.${operation}` });
}

/**
 * Reject unless the host advertises `supported` for this operation.
 *
 * The three refusals are deliberately distinct: `pending` is "reserved, not yet",
 * `unsupported` is "never on this carrier" and `descriptor_missing` is "the host
 * does not know this operation", which is a contract mismatch rather than a
 * capability answer.
 */
export function assertOperationSupported(
  host: CadApiHostInfo,
  namespace: CadApiNamespace,
  operation: string,
): void {
  const row = findOperation(host, namespace, operation);
  if (row === undefined) {
    throw refusal(namespace, operation, descriptorMissingReason(namespace, operation));
  }
  if (row.status === 'unsupported') {
    throw refusal(namespace, operation, unsupportedReason(namespace, operation));
  }
  if (row.status === 'pending') {
    throw refusal(namespace, operation, pendingReason(namespace, operation));
  }
  if (row.status !== 'supported') {
    // A status this package cannot classify is treated as a broken descriptor:
    // refusing is the only honest answer, and it must never fall through to a call.
    throw refusal(
      namespace,
      operation,
      `api.descriptor_invalid: operations.${namespace}.${operation}.status`,
    );
  }
}

/**
 * Map a bridge rejection to `CadApiError`.
 *
 * A Rust host rejects with a shaped payload (`{ code, message, details }`); a
 * JS host may reject with a `CadApiError`. Anything else becomes
 * `api.invalid_argument: bridge.failure` rather than an untyped crash, and the
 * original value is preserved as `cause`.
 */
export function errorFromBridgeFailure(failure: unknown): CadApiError {
  if (failure instanceof CadApiError) return failure;
  if (isRecord(failure)) {
    const code = failure['code'];
    if (typeof code === 'string' && code.length > 0) {
      const message = failure['message'];
      const details = failure['details'];
      return new CadApiError(
        code,
        typeof message === 'string' && message.length > 0 ? message : code,
        {
          details: isRecord(details) ? details : undefined,
          cause: failure,
        },
      );
    }
  }
  return new CadApiError(
    'api.invalid_argument: bridge.failure',
    'the host bridge rejected without a stable code',
    { cause: failure },
  );
}

/**
 * Require a plain object answer. Arrays and primitives are refused: a proxy that
 * expected `{ object }` must not read fields off a number.
 */
export function requireAnswerObject(answer: unknown, field: string): Readonly<Record<string, unknown>> {
  if (!isRecord(answer)) throw invalidArgument(field);
  return answer;
}

/** Build the gated transport for one host snapshot. */
export function createWire(host: CadApiHostInfo, bridge: CadHostBridge): CadApiWire {
  return {
    host,
    assertSupported(namespace, operation) {
      assertOperationSupported(host, namespace, operation);
    },
    async call(
      namespace: CadApiNamespace,
      operation: string,
      parameters: Readonly<Record<string, unknown>> = {},
    ): Promise<unknown> {
      assertOperationSupported(host, namespace, operation);
      let answer: unknown;
      try {
        answer = await bridge.request<unknown>({ namespace, operation, parameters });
      } catch (failure) {
        throw errorFromBridgeFailure(failure);
      }
      if (answer === undefined || answer === null) throw invalidArgument('bridge.answer');
      return answer;
    },
    async submit(
      namespace: CadApiNamespace,
      operation: string,
      parameters: Readonly<Record<string, unknown>> = {},
    ): Promise<unknown> {
      assertOperationSupported(host, namespace, operation);
      let answer: unknown;
      try {
        answer = await bridge.mutate<unknown>({ namespace, operation, parameters });
      } catch (failure) {
        throw errorFromBridgeFailure(failure);
      }
      if (answer === undefined || answer === null) throw invalidArgument('bridge.answer');
      return answer;
    },
    async callVoid(
      namespace: CadApiNamespace,
      operation: string,
      parameters: Readonly<Record<string, unknown>> = {},
    ): Promise<void> {
      assertOperationSupported(host, namespace, operation);
      let answer: unknown;
      try {
        answer = await bridge.request<unknown>({ namespace, operation, parameters });
      } catch (failure) {
        throw errorFromBridgeFailure(failure);
      }
      // A void operation still has to say it did the thing. `{ accepted: false }`,
      // `{ ok: 1 }`, `undefined` and an empty object are all "no usable answer".
      if (!isRecord(answer) || answer['accepted'] !== true) {
        throw invalidArgument('bridge.answer');
      }
    },
  };
}
