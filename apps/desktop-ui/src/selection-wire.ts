// Structural validation for the frozen selection block
// (`cad-app::selection::SelectionProperties`, projected by
// `cad-ui-core::session::status::CadSelection`).
//
// The block is always present and structurally exact: `empty == (count == 0)`,
// `mixedKeys` mirrors the rows marked `mixed`, and a mixed row carries no value.
// A block that breaks any invariant is a contract violation, never something to
// coerce into a fabricated empty state.

/** The closed set of editing-control machine keys. */
export type PropertyRowKind = 'text' | 'number' | 'enum' | 'bool';

/** One validated property row of the selection block. */
export interface SelectionPropertyRow {
  readonly key: string;
  readonly value: string | null;
  readonly group: string;
  readonly mixed: boolean;
  readonly editable: boolean;
  readonly kind: PropertyRowKind;
  readonly options: readonly string[];
}

/** The validated `cad_status.selection` block. */
export interface CadSelection {
  readonly count: number;
  readonly empty: boolean;
  readonly rows: readonly SelectionPropertyRow[];
  readonly mixedKeys: readonly string[];
}

// The closed set of editing-control machine keys. A value outside it is a
// contract violation because a panel would not know how to edit the row.
const ROW_KINDS: ReadonlySet<string> = new Set(['text', 'number', 'enum', 'bool']);

/**
 * The selected-object count any consumer may read off a selection slice.
 *
 * Structural on purpose: the shortcut menu and the command-admission rules both
 * ask the same question and must answer it the same way. Only a positive integer
 * counts as a selection — `true`, `"2"` or a fractional number are not evidence
 * of selected geometry, so they read as zero (the honest "nothing selected").
 */
export interface SelectionCountSlice {
  readonly count?: unknown;
}

/** The selected-object count of a selection slice, or `0` when absent/malformed. */
export function selectionCount(selection: SelectionCountSlice | null | undefined): number {
  const count = selection?.count;
  return typeof count === 'number' && Number.isSafeInteger(count) && count > 0 ? count : 0;
}

/** Validate (and return) a selection block; a broken one throws `CAD_STATUS_INVALID`. */
export function parseSelection(selection: unknown): CadSelection {
  if (!selection || typeof selection !== 'object' || Array.isArray(selection)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const block = selection as {
    count?: unknown;
    empty?: unknown;
    rows?: unknown;
    mixedKeys?: unknown;
  };
  const { count, empty, rows, mixedKeys } = block;
  if (typeof count !== 'number' || !Number.isSafeInteger(count) || count < 0
    || typeof empty !== 'boolean' || empty !== (count === 0)
    || !Array.isArray(rows) || !Array.isArray(mixedKeys)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const rowList = rows as unknown[];
  const mixedList = mixedKeys as unknown[];
  // Rows describe the selected objects: a single object gets its full set, a
  // multi-selection the intersection of the common keys. Rows without a
  // selection are malformed, not a shortcut we may silently truncate.
  if (rowList.length > 0 && count < 1) throw Error('CAD_STATUS_INVALID');
  // Row keys/groups are machine keys the catalog may not know; that is
  // tolerated (the UI shows the raw key). Missing or mistyped metadata is not.
  // `mixed` and `value` are one fact in two fields: a mixed row has no value,
  // and an unmixed row must have one (never a fabricated empty string).
  const validRow = (row: unknown): boolean => {
    if (!row || typeof row !== 'object' || Array.isArray(row)) return false;
    const candidate = row as {
      key?: unknown;
      value?: unknown;
      group?: unknown;
      mixed?: unknown;
      editable?: unknown;
      kind?: unknown;
      options?: unknown;
    };
    return typeof candidate.key === 'string' && candidate.key.length > 0
      && (candidate.value === null || typeof candidate.value === 'string')
      && typeof candidate.group === 'string' && candidate.group.length > 0
      && typeof candidate.mixed === 'boolean'
      && candidate.mixed === (candidate.value === null)
      && typeof candidate.editable === 'boolean'
      && typeof candidate.kind === 'string' && ROW_KINDS.has(candidate.kind)
      && Array.isArray(candidate.options)
      && (candidate.options as unknown[]).every(option => typeof option === 'string');
  };
  if (!rowList.every(validRow)) throw Error('CAD_STATUS_INVALID');
  if (mixedList.length > 0 && count <= 1) throw Error('CAD_STATUS_INVALID');
  if (!mixedList.every(key => typeof key === 'string' && key.length > 0)) {
    throw Error('CAD_STATUS_INVALID');
  }
  // One source of truth: `mixedKeys` is exactly the keys of the mixed rows, in
  // row order. Two drifting lists would let the panel show the wrong summary.
  const mixedFromRows = (rowList as SelectionPropertyRow[])
    .filter(row => row.mixed)
    .map(row => row.key);
  if (mixedFromRows.length !== mixedList.length
    || mixedFromRows.some((key, index) => key !== mixedList[index])) {
    throw Error('CAD_STATUS_INVALID');
  }
  return selection as CadSelection;
}
