// The frozen plugin-API contract: one version number, the namespace list, and
// the per-operation capability table.
//
// `API_OPERATIONS` is a **mirror** of `crates/cad-ui-core/api/plugin-api.json`
// (owned by the host lane). `test/contract.test.ts` reads the shipped JSON and
// compares every row, so drift fails a test instead of letting a plugin call an
// operation the host refuses.
//
// # What gets a row
//
// A row describes a **host-served operation**: something a carrier answers over
// the wire (or reserves for a later round). Pure package features — the
// `Geometry`/`Colors` value helpers and the manifest parser — never cross the
// wire, so they have no row and are available unconditionally. Advertising them
// in the descriptor would claim host state that does not exist.
//
// Four rows are `supported` in this build — the reflected document reads
// (`documentManager.mdiActiveDocument`, `documentManager.documents`), the database
// revision and the symbol-table projection, all served over `cad_api_call` — and the
// other 17 are `pending`: the table is an honest reservation list, not an
// availability list. It is deliberately complete rather than aspirational — an
// operation that exists here but is `pending` is refused by name, an operation the
// host marks `unsupported` is refused by name, and an operation the host does not
// name at all is refused as `api.descriptor_missing`. There is no silent "maybe"
// state and no empty-success path.

/** Wire/API generation. A host and this package must agree exactly. */
export const API_VERSION = 1;

/**
 * The AutoCAD .NET namespaces this contract mirrors.
 *
 * All six are part of the API surface, but only four currently have descriptor
 * rows: `Geometry` and `Colors` are pure package features (see the header).
 */
export type CadApiNamespace =
  | 'ApplicationServices'
  | 'DatabaseServices'
  | 'EditorInput'
  | 'Runtime'
  | 'Geometry'
  | 'Colors';

/**
 * `supported`: the carrier serves the operation and the package may call it.
 * `pending`:     the operation is named in the contract but the host does not
 *                serve it yet; calls are refused with `api.pending: <ns>.<op>`.
 * `unsupported`: the carrier can never serve it in this generation; calls are
 *                refused with `unsupported-runtime: cad_api.<ns>.<op>`.
 */
export type CadApiOperationStatus = 'supported' | 'pending' | 'unsupported';

/** One capability row. Pending/unsupported rows must carry an exact `reason`. */
export interface CadApiOperation {
  readonly namespace: CadApiNamespace;
  readonly operation: string;
  readonly status: CadApiOperationStatus;
  readonly reason?: string;
}

/** Namespace order of the contract table, mirrored from the shared JSON. */
export const API_NAMESPACES: readonly CadApiNamespace[] = [
  'ApplicationServices',
  'DatabaseServices',
  'EditorInput',
  'Runtime',
  'Geometry',
  'Colors',
];

/**
 * The v1 capability table: 21 host operations, 17 `pending` and four `supported`.
 *
 * Do not flip a row to `supported` here without the host side landing first (and
 * a `case` in the WebView adapter) — the contract test will catch the JSON drift,
 * but the runtime behaviour is what plugins see. Do not add a row for a package
 * feature either: see the header.
 */
export const API_OPERATIONS: readonly CadApiOperation[] = [
  // The first real read: served by `cad_api_call` (docs/commands.md), decoded by
  // `createDocumentManager`. It is `supported` with no `reason`, exactly like the
  // shared JSON row.
  {
    namespace: 'ApplicationServices',
    operation: 'documentManager.mdiActiveDocument',
    status: 'supported',
  },
  // The reflected document list: the same descriptor the active read reports, as
  // `{ documents: [...] }` (0 or 1 today). Served, so no `reason`.
  {
    namespace: 'ApplicationServices',
    operation: 'documentManager.documents',
    status: 'supported',
  },
  {
    namespace: 'ApplicationServices',
    operation: 'document.lock',
    status: 'pending',
    reason: pendingReason('ApplicationServices', 'document.lock'),
  },
  {
    namespace: 'DatabaseServices',
    operation: 'database.getObjectId',
    status: 'pending',
    reason: pendingReason('DatabaseServices', 'database.getObjectId'),
  },
  // The block/layer projection and the active layer name, as one read. Served, so
  // no `reason`; `Database.symbolTables` and `Database.clayer` share one request.
  {
    namespace: 'DatabaseServices',
    operation: 'database.symbolTables',
    status: 'supported',
  },
  // The revision a commit's `expectedRevision` is compared against. Read from the
  // host every call (`{ database: id }` -> `{ revision }`), never cached.
  {
    namespace: 'DatabaseServices',
    operation: 'database.revision',
    status: 'supported',
  },
  {
    namespace: 'DatabaseServices',
    operation: 'transaction.start',
    status: 'pending',
    reason: pendingReason('DatabaseServices', 'transaction.start'),
  },
  {
    namespace: 'DatabaseServices',
    operation: 'transaction.getObject',
    status: 'pending',
    reason: pendingReason('DatabaseServices', 'transaction.getObject'),
  },
  {
    namespace: 'DatabaseServices',
    operation: 'transaction.addNewlyCreated',
    status: 'pending',
    reason: pendingReason('DatabaseServices', 'transaction.addNewlyCreated'),
  },
  {
    namespace: 'DatabaseServices',
    operation: 'transaction.commit',
    status: 'pending',
    reason: pendingReason('DatabaseServices', 'transaction.commit'),
  },
  {
    namespace: 'DatabaseServices',
    operation: 'transaction.abort',
    status: 'pending',
    reason: pendingReason('DatabaseServices', 'transaction.abort'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getPoint',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getPoint'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getEntity',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getEntity'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getSelection',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getSelection'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getString',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getString'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getDouble',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getDouble'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getInteger',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getInteger'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.getKeyword',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.getKeyword'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.writeMessage',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.writeMessage'),
  },
  {
    namespace: 'EditorInput',
    operation: 'editor.selectAll',
    status: 'pending',
    reason: pendingReason('EditorInput', 'editor.selectAll'),
  },
  {
    namespace: 'Runtime',
    operation: 'runtime.registerCommand',
    status: 'pending',
    reason: pendingReason('Runtime', 'runtime.registerCommand'),
  },
];

/** The row for one operation, or `undefined` when the contract does not name it. */
export function operationOf(
  namespace: CadApiNamespace,
  operation: string,
): CadApiOperation | undefined {
  for (const row of API_OPERATIONS) {
    if (row.namespace === namespace && row.operation === operation) return row;
  }
  return undefined;
}

/** Refusal code for an operation the contract names but the host does not serve. */
export function pendingReason(namespace: CadApiNamespace, operation: string): string {
  return `api.pending: ${namespace}.${operation}`;
}

/**
 * Refusal code for an operation the carrier explicitly marks `unsupported`.
 *
 * Only an explicit `unsupported` row produces this. A row the host does not name
 * at all is a different failure (`descriptorMissingReason`): the package and the
 * host are not talking about the same contract, which is not the same claim as
 * "this carrier cannot serve it".
 */
export function unsupportedReason(namespace: CadApiNamespace, operation: string): string {
  return `unsupported-runtime: cad_api.${namespace}.${operation}`;
}

/**
 * Refusal code for an operation the host's descriptor does not name at all.
 *
 * Distinct from `unsupportedReason` on purpose: a missing row means the
 * descriptor and this package disagree about the contract (a version skew or a
 * dropped row), while `unsupported` is the host saying "not on this carrier".
 */
export function descriptorMissingReason(namespace: CadApiNamespace, operation: string): string {
  return `api.descriptor_missing: ${namespace}.${operation}`;
}
