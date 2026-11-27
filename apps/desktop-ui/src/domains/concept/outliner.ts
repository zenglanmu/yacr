// The Outliner tray wire: normalisation, the one read, and the local
// expand/collapse projection.
//
// The host owns the tree (`cad_outliner` → `kind: "outliner"`): a flat pre-order
// list of groups, block references, model-space entities and block members, each
// carrying `depth` and exactly the entity ids a click should select. This module
// validates the answer and computes which rows are visible under the user's
// collapse state; it never re-orders, invents or drops rows.
//
// Selection is not this read: the panel dispatches the existing
// `cad_select_entities` wire command with a row's `selectIds`.
//
// Wire shape (frozen):
//
// ```jsonc
// { "revision": 3,
//   "rows": [{ "kind": "group", "id": 1, "label": "walls", "depth": 0,
//              "hasChildren": true, "selectable": true, "selectIds": [12, 13] }],
//   "total": 1, "truncated": false }
// ```

import type { Invoke } from '../../host/invoke.ts';

/** The one read command (`kind: "outliner"`). */
export const OUTLINER_COMMAND = 'cad_outliner';

/** One Outliner row. */
export interface OutlinerRow {
  /** `"group"`, `"insert"`, `"entity"` or `"member"`. */
  readonly kind: string;
  /** Entity id, or the group sequence for a group row. */
  readonly id: number;
  /** Display label. */
  readonly label: string;
  /** Containing group name for a group member row, else `null`. */
  readonly parentGroup: string | null;
  /** Nesting depth (top level is `0`). */
  readonly depth: number;
  /** Whether the row is temporarily hidden in this session. */
  readonly hidden: boolean;
  /** Whether the row has children. */
  readonly hasChildren: boolean;
  /** Whether a click can select this row. */
  readonly selectable: boolean;
  /** The entity ids one click sends. */
  readonly selectIds: readonly number[];
}

/** One normalised answer. */
export interface OutlinerAnswer {
  readonly revision: number;
  readonly rows: readonly OutlinerRow[];
  /** The host's own total (may exceed `rows.length`). */
  readonly total: number;
  readonly truncated: boolean;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function idOf(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/**
 * The `GROUPEDIT "<name>" RENAME "<new>"` line for a group row.
 *
 * Group names are addressed with double quotes in the typed grammar; a name
 * that itself contains a quote (or a line break) cannot be expressed by that
 * grammar and is refused here instead of being escaped into a different name.
 */
export function groupRenameCommand(from: unknown, to: unknown): string | null {
  const name = typeof from === 'string' ? from.trim() : '';
  const next = typeof to === 'string' ? to.trim() : '';
  const unsafe = (value: string) => value === '' || value.includes('"')
    || value.includes('\n') || value.includes('\r');
  if (unsafe(name) || unsafe(next)) return null;
  return `GROUPEDIT "${name}" RENAME "${next}"`;
}

/** The entity ids of the selected rows, in row order (duplicates dropped). */
export function selectedOutlinerIds(
  rows: readonly OutlinerRow[],
  selectedKeys: ReadonlySet<string>,
): readonly number[] {
  const ids: number[] = [];
  const seen = new Set<number>();
  for (const row of rows) {
    if (!selectedKeys.has(outlinerRowKey(row)) || !row.selectable) continue;
    for (const id of row.selectIds) {
      if (!seen.has(id)) {
        seen.add(id);
        ids.push(id);
      }
    }
  }
  return ids;
}

/**
 * `GROUP CREATE "<name>"` for the selected Outliner rows.
 *
 * The name grammar quotes names and has no escape; a name containing a quote or
 * a line break is refused here rather than silently becoming a different name.
 */
export function groupCreateCommand(name: unknown): string | null {
  const trimmed = typeof name === 'string' ? name.trim() : '';
  if (trimmed === '' || trimmed.includes('"')
    || trimmed.includes('\n') || trimmed.includes('\r')) {
    return null;
  }
  return `GROUP CREATE "${trimmed}"`;
}

/**
 * `GROUPEDIT "<group>" ADD|REMOVE <ids>` for a drag/regroup action.
 *
 * The group name uses the typed grammar's double quotes (no escape); ids are
 * positive safe integers joined by commas. An unsafe name or an empty/invalid
 * id list yields `null`.
 */
export function groupEditCommand(
  group: unknown,
  verb: 'ADD' | 'REMOVE',
  ids: readonly unknown[],
): string | null {
  const name = typeof group === 'string' ? group.trim() : '';
  if (name === '' || name.includes('"') || name.includes('\n') || name.includes('\r')) {
    return null;
  }
  const entityIds: number[] = [];
  for (const raw of ids) {
    if (typeof raw !== 'number' || !Number.isSafeInteger(raw) || raw < 0
      || entityIds.includes(raw)) {
      continue;
    }
    entityIds.push(raw);
  }
  if (entityIds.length === 0) return null;
  return `GROUPEDIT "${name}" ${verb} ${entityIds.join(',')}`;
}

/**
 * `GROUPEDIT "<parent>" REMOVE <ids>` commands for the selected rows that
 * currently belong to a group.
 *
 * Rows are grouped by their `parentGroup`, so one undo step covers each old
 * group; already-ungrouped rows contribute nothing.
 */
export function ungroupCommands(
  rows: readonly OutlinerRow[],
  selectedKeys: ReadonlySet<string>,
): readonly string[] {
  const byParent = new Map<string, number[]>();
  for (const row of rows) {
    if (!selectedKeys.has(outlinerRowKey(row)) || !row.selectable
      || row.parentGroup === null) {
      continue;
    }
    const ids = byParent.get(row.parentGroup) ?? [];
    for (const id of row.selectIds) {
      if (!ids.includes(id)) ids.push(id);
    }
    byParent.set(row.parentGroup, ids);
  }
  const commands: string[] = [];
  for (const [parent, ids] of byParent) {
    const command = groupEditCommand(parent, 'REMOVE', ids);
    if (command !== null) commands.push(command);
  }
  return commands;
}

/** The stable key for one row (its kind and id are unique in one tree). */
export function outlinerRowKey(row: Pick<OutlinerRow, 'kind' | 'id'>): string {
  return `${row.kind}:${row.id}`;
}

/** Normalise one `cad_outliner` answer; a row without a label or id is dropped. */
export function parseOutlinerAnswer(answer: unknown): OutlinerAnswer {
  const source = recordOf(answer);
  if (source === null) throw new Error('outliner.invalid_answer: expected an object');
  const rawRows: unknown[] = Array.isArray(source.rows) ? source.rows : [];
  const rows: OutlinerRow[] = [];
  for (const raw of rawRows) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const id = idOf(fields.id);
    const label = typeof fields.label === 'string' ? fields.label : null;
    const kind = typeof fields.kind === 'string' && fields.kind !== '' ? fields.kind : null;
    if (id === null || label === null || kind === null) continue;
    const parentGroup = typeof fields.parentGroup === 'string' && fields.parentGroup !== ''
      ? fields.parentGroup
      : null;
    const depth = idOf(fields.depth) ?? 0;
    const selectIds: number[] = [];
    for (const candidate of Array.isArray(fields.selectIds) ? fields.selectIds : []) {
      const value = idOf(candidate);
      if (value !== null && !selectIds.includes(value)) selectIds.push(value);
    }
    rows.push({
      kind,
      id,
      label,
      parentGroup,
      depth,
      hidden: fields.hidden === true,
      hasChildren: fields.hasChildren === true,
      selectable: fields.selectable === true && selectIds.length > 0,
      selectIds,
    });
  }
  const reported = source.total;
  return {
    revision: idOf(source.revision) ?? 0,
    rows,
    total: typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= 0
      ? reported
      : rows.length,
    truncated: source.truncated === true,
  };
}

/**
 * The rows visible under `collapsed`, in wire order.
 *
 * A row is hidden when any ancestor is collapsed. Rows are a pre-order walk with
 * an explicit `depth`, so one pass with the depth of the last hidden container is
 * enough; a collapsed row's whole subtree is the contiguous run deeper than it.
 */
export function visibleOutlinerRows(
  rows: readonly OutlinerRow[],
  collapsed: ReadonlySet<string>,
): readonly OutlinerRow[] {
  const visible: OutlinerRow[] = [];
  let hiddenDepth: number | null = null;
  for (const row of rows) {
    if (hiddenDepth !== null) {
      if (row.depth > hiddenDepth) continue;
      hiddenDepth = null;
    }
    visible.push(row);
    if (row.hasChildren && collapsed.has(outlinerRowKey(row))) hiddenDepth = row.depth;
  }
  return visible;
}

/** The one read. */
export async function readOutliner(invoke: Invoke): Promise<OutlinerAnswer> {
  return parseOutlinerAnswer(await invoke(OUTLINER_COMMAND));
}
