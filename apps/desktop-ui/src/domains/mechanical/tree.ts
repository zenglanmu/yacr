// The mechanical FeatureManager tree wire: its normalisation and the one read.
//
// The host owns the projection (`cad_mechanical_tree` → `mechanicalTree`): the
// feature table in evaluation order, then the body rows, already bounded. This
// module only validates the answer and maps its stable machine tokens onto the
// catalog. It never re-orders, filters for looks, or turns a feature into a
// claim that geometry exists.
//
// Wire shape (frozen):
//
// ```jsonc
// { "revision": 42,
//   "rows": [{ "kind": "feature", "id": 7, "token": "extrude",
//              "status": "schema_only", "order": 0, "inputs": 1, "parameters": 2 },
//            { "kind": "body", "id": 9, "token": "solid",
//              "status": "ready", "order": null, "inputs": 0, "parameters": 0 }],
//   "total": 2, "truncated": false }
// ```
//
// A row without a usable `kind`/`id`/`token`/`status` is dropped (it cannot be
// shown or addressed), and an unknown token renders raw instead of being mapped
// to a guess.

import type { Invoke } from '../../host/invoke.ts';

/** The one read command (`kind: "mechanicalTree"`). */
export const MECHANICAL_TREE_COMMAND = 'cad_mechanical_tree';

/** A machine description of one row. */
export interface MechanicalRow {
  /** `feature` | `body`, or the host's raw token when unknown. */
  readonly kind: string;
  /** The element row the node projects. */
  readonly id: number;
  /** `extrude`/… for a feature; `solid`/`sheet`/`wire`/`unknown` for a body. */
  readonly token: string;
  /** `schema_only` (feature), `ready` or `unavailable` (body). */
  readonly status: string;
  /** Evaluation order for a feature; `null` for a body. */
  readonly order: number | null;
  /** Input count for a feature; `0` for a body. */
  readonly inputs: number;
  /** Bound parameter-slot count on the projecting element. */
  readonly parameters: number;
}

/** One normalised answer. */
export interface MechanicalTreeAnswer {
  readonly revision: number;
  readonly rows: readonly MechanicalRow[];
  /** The host's own total for the whole tree (may exceed `rows.length`). */
  readonly total: number;
  /** Whether the host bounded the page. */
  readonly truncated: boolean;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function countOf(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

/**
 * Normalise one `cad_mechanical_tree` answer.
 *
 * The host's order is kept verbatim. Only structurally usable rows survive; the
 * reported `total` is authoritative even when the page is short, so a bounded
 * answer cannot be mistaken for a complete one.
 */
export function parseMechanicalTreeAnswer(answer: unknown): MechanicalTreeAnswer {
  const source = recordOf(answer);
  const rawRows: unknown[] = Array.isArray(source?.rows) ? source.rows : [];
  const rows: MechanicalRow[] = [];
  for (const raw of rawRows) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const kind = typeof fields.kind === 'string' && fields.kind !== '' ? fields.kind : null;
    const id = typeof fields.id === 'number' && Number.isSafeInteger(fields.id) && fields.id >= 0
      ? fields.id
      : null;
    const token = typeof fields.token === 'string' && fields.token !== '' ? fields.token : null;
    const status = typeof fields.status === 'string' && fields.status !== '' ? fields.status : null;
    if (kind === null || id === null || token === null || status === null) continue;
    const order = typeof fields.order === 'number' && Number.isSafeInteger(fields.order) && fields.order >= 0
      ? fields.order
      : null;
    rows.push({
      kind,
      id,
      token,
      status,
      order,
      inputs: countOf(fields.inputs),
      parameters: countOf(fields.parameters),
    });
  }
  const reported = source?.total;
  const total = typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= 0
    ? reported
    : rows.length;
  return {
    revision: countOf(source?.revision),
    rows,
    total,
    truncated: source?.truncated === true,
  };
}

/** The catalog key naming a feature operation, or `null` for a token we cannot name. */
export function featureOperationKey(token: string): string | null {
  switch (token) {
    case 'extrude': return 'mechanical.op.extrude';
    case 'revolve': return 'mechanical.op.revolve';
    case 'loft': return 'mechanical.op.loft';
    case 'sweep': return 'mechanical.op.sweep';
    case 'boolean_union': return 'mechanical.op.boolean_union';
    case 'boolean_subtract': return 'mechanical.op.boolean_subtract';
    case 'boolean_intersect': return 'mechanical.op.boolean_intersect';
    case 'fillet': return 'mechanical.op.fillet';
    case 'shell': return 'mechanical.op.shell';
    case 'hole': return 'mechanical.op.hole';
    case 'pattern': return 'mechanical.op.pattern';
    default: return null;
  }
}

/** The catalog key naming a body's topology token, or `null`. */
export function bodyKindKey(token: string): string | null {
  switch (token) {
    case 'solid': return 'mechanical.body.solid';
    case 'sheet': return 'mechanical.body.sheet';
    case 'wire': return 'mechanical.body.wire';
    case 'unknown': return 'mechanical.body.unknown';
    default: return null;
  }
}

/** The catalog key naming a row's status, or `null`. */
export function mechanicalStatusKey(status: string): string | null {
  switch (status) {
    case 'schema_only': return 'mechanical.status.schema_only';
    case 'ready': return 'mechanical.status.ready';
    case 'unavailable': return 'mechanical.status.unavailable';
    default: return null;
  }
}

/** Read the tree once through the one IPC seam. */
export async function readMechanicalTree(invoke: Invoke): Promise<MechanicalTreeAnswer> {
  return parseMechanicalTreeAnswer(await invoke(MECHANICAL_TREE_COMMAND));
}
