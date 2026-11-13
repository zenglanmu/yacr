// Strict decoding of the `cad_api_status` wire answer.
//
// The host owns the capability table (`crates/cad-ui-core/api/plugin-api.json`);
// this module is the only place the WebView turns that snapshot into typed rows.
// It is strict on purpose: a payload the chrome cannot fully understand throws
// `Error('cad_api_status: <detail>')` instead of being coerced, because a granted
// capability the host never advertised is exactly the lie this adapter exists to
// prevent. There is no "best effort" branch and no placeholder row.
//
// `hostInfoFrom` then maps the payload plus the current `cad_status` snapshot to
// the package's `CadApiHostInfo`. The carrier/document-owner fields are
// informational (the package never routes on them), but they must still be the
// carrier the repo rule assigns: only `dom-canvas` is special, every other
// observed surface mode is `native-below`, and a missing status is `unknown`.

// The host depends on the published contract package by relative path (with the
// `.ts` extension), not by the `@yacr/plugin-api` specifier: this repo has no npm
// workspace and no checked-in `node_modules`, so a bare specifier would resolve
// neither in `tsc` nor in `node --test`. The path is the one dependency edge the
// UI package has on `packages/cad-plugin-api`, and it is deliberate.
import { API_NAMESPACES, pendingReason } from '../../../../packages/cad-plugin-api/src/index.ts';
import type {
  CadApiHostInfo,
  CadApiNamespace,
  CadApiOperation,
  CadApiOperationStatus,
} from '../../../../packages/cad-plugin-api/src/index.ts';
import type { CadStatus } from '../cad/cad-status.ts';

/** The validated shape of the `cad_api_status` answer. */
export interface ApiStatusPayload {
  readonly apiVersion: number;
  readonly namespaces: readonly CadApiNamespace[];
  readonly operations: readonly CadApiOperation[];
}

/** `<namespace>.<operation>`, at least two dot-separated camelCase segments. */
const OPERATION_PATTERN = /^[a-z][a-zA-Z0-9]*(\.[a-z][a-zA-Z0-9]*)+$/;

/** The closed status set; anything else is a host contract violation. */
const STATUSES: readonly CadApiOperationStatus[] = ['supported', 'pending', 'unsupported'];

/** The stable prefix the Rust host uses for a capability this carrier lacks. */
const UNSUPPORTED_PREFIX = 'unsupported-runtime: ';

/** Throw the one error shape this module produces; never returns. */
function fail(detail: string): never {
  throw new Error(`cad_api_status: ${detail}`);
}

/** Whether `value` is a plain (non-array) object. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Read the namespace list: non-empty, known names only, no duplicates. */
function parseNamespaces(value: unknown): readonly CadApiNamespace[] {
  if (!Array.isArray(value) || value.length === 0) {
    return fail('namespaces must be a non-empty array');
  }
  const names: CadApiNamespace[] = [];
  for (const entry of value) {
    if (typeof entry !== 'string' || !API_NAMESPACES.includes(entry as CadApiNamespace)) {
      fail(`unknown namespace: ${JSON.stringify(entry)}`);
    }
    const namespace = entry as CadApiNamespace;
    if (names.includes(namespace)) fail(`duplicate namespace: ${namespace}`);
    names.push(namespace);
  }
  return names;
}

/** Read one capability row, enforcing the status/reason relationship exactly. */
function parseOperation(value: unknown, index: number): CadApiOperation {
  if (!isRecord(value)) fail(`operations[${index}] is not an object`);

  const namespace = value['namespace'];
  if (typeof namespace !== 'string' || !API_NAMESPACES.includes(namespace as CadApiNamespace)) {
    fail(`operations[${index}] has an unknown namespace: ${JSON.stringify(namespace)}`);
  }
  const ns = namespace as CadApiNamespace;

  const operation = value['operation'];
  if (typeof operation !== 'string' || !OPERATION_PATTERN.test(operation)) {
    fail(`operations[${index}] has a malformed operation name: ${JSON.stringify(operation)}`);
  }
  const label = `${ns}.${operation}`;

  const status = value['status'];
  if (typeof status !== 'string' || !STATUSES.includes(status as CadApiOperationStatus)) {
    fail(`operation ${label} has an unknown status: ${JSON.stringify(status)}`);
  }
  const kind = status as CadApiOperationStatus;

  const reason = value['reason'];
  if (kind === 'supported') {
    if (reason !== undefined) fail(`operation ${label} is supported but carries a reason`);
    return { namespace: ns, operation, status: kind };
  }
  // Every non-supported row must say why, in the host's own words.
  if (typeof reason !== 'string' || reason.length === 0) {
    fail(`operation ${label} is ${kind} but has no reason`);
  }
  if (kind === 'pending') {
    const expected = pendingReason(ns, operation);
    if (reason !== expected) fail(`operation ${label} has a wrong pending reason: ${reason}`);
  } else if (!reason.startsWith(UNSUPPORTED_PREFIX)) {
    fail(`operation ${label} has an unsupported reason without the stable prefix: ${reason}`);
  }
  return { namespace: ns, operation, status: kind, reason };
}

/**
 * Validate the raw `cad_api_status` answer.
 *
 * Every violation throws `cad_api_status: <detail>`; the caller (the bridge)
 * lets that reject, so a malformed host answer never becomes a usable client.
 */
export function parseApiStatusPayload(value: unknown): ApiStatusPayload {
  if (!isRecord(value)) fail('payload is not an object');

  const apiVersion = value['apiVersion'];
  if (typeof apiVersion !== 'number' || !Number.isInteger(apiVersion) || apiVersion < 1) {
    fail(`apiVersion must be an integer >= 1: ${JSON.stringify(apiVersion)}`);
  }

  const namespaces = parseNamespaces(value['namespaces']);

  const rawOperations = value['operations'];
  if (!Array.isArray(rawOperations)) fail('operations must be an array');

  const operations: CadApiOperation[] = [];
  const seen = new Set<string>();
  rawOperations.forEach((row, index) => {
    const parsed = parseOperation(row, index);
    const key = `${parsed.namespace}.${parsed.operation}`;
    if (seen.has(key)) fail(`duplicate operation row: ${key}`);
    seen.add(key);
    operations.push(parsed);
  });

  return { apiVersion, namespaces, operations };
}

/**
 * Map a validated payload plus the current status snapshot to host info.
 *
 * `status === null` means the chrome has not observed a host yet: carrier
 * `unknown`, document owner `none`. Otherwise only `dom-canvas` is special
 * (repo rule: every other observed surface mode is `native-below`), and the
 * owner is `wasm` only when the host says so. Operations are the payload rows
 * verbatim - no local row is ever added.
 */
export function hostInfoFrom(payload: ApiStatusPayload, status: CadStatus | null): CadApiHostInfo {
  const surfaceMode = status?.hostProfile?.surfaceMode;
  const observedOwner = status === null ? undefined : status.hostProfile?.documentOwner;
  return Object.freeze({
    apiVersion: payload.apiVersion,
    carrier:
      surfaceMode === 'dom-canvas' ? 'dom-canvas' : status === null ? 'unknown' : 'native-below',
    documentOwner: observedOwner === 'wasm' ? 'wasm' : status === null ? 'none' : 'native',
    operations: payload.operations,
  });
}
