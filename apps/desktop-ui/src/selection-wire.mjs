// Structural validation for the frozen selection block
// (`cad-app::selection::SelectionProperties`, projected by
// `cad-ui-core::session::status::CadSelection`).
//
// The block is always present and structurally exact: `empty == (count == 0)`,
// `mixedKeys` mirrors the rows marked `mixed`, and a mixed row carries no value.
// A block that breaks any invariant is a contract violation, never something to
// coerce into a fabricated empty state.

// The closed set of editing-control machine keys. A value outside it is a
// contract violation because a panel would not know how to edit the row.
const ROW_KINDS = new Set(['text', 'number', 'enum', 'bool']);

export function parseSelection(selection) {
  if (!selection || typeof selection !== 'object' || Array.isArray(selection)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const { count, empty, rows, mixedKeys } = selection;
  if (!Number.isSafeInteger(count) || count < 0
    || typeof empty !== 'boolean' || empty !== (count === 0)
    || !Array.isArray(rows) || !Array.isArray(mixedKeys)) {
    throw Error('CAD_STATUS_INVALID');
  }
  // Rows describe the selected objects: a single object gets its full set, a
  // multi-selection the intersection of the common keys. Rows without a
  // selection are malformed, not a shortcut we may silently truncate.
  if (rows.length > 0 && count < 1) throw Error('CAD_STATUS_INVALID');
  // Row keys/groups are machine keys the catalog may not know; that is
  // tolerated (the UI shows the raw key). Missing or mistyped metadata is not.
  // `mixed` and `value` are one fact in two fields: a mixed row has no value,
  // and an unmixed row must have one (never a fabricated empty string).
  const validRow = row => row && typeof row === 'object' && !Array.isArray(row)
    && typeof row.key === 'string' && row.key.length > 0
    && (row.value === null || typeof row.value === 'string')
    && typeof row.group === 'string' && row.group.length > 0
    && typeof row.mixed === 'boolean'
    && row.mixed === (row.value === null)
    && typeof row.editable === 'boolean'
    && ROW_KINDS.has(row.kind)
    && Array.isArray(row.options) && row.options.every(option => typeof option === 'string');
  if (!rows.every(validRow)) throw Error('CAD_STATUS_INVALID');
  if (mixedKeys.length > 0 && count <= 1) throw Error('CAD_STATUS_INVALID');
  if (!mixedKeys.every(key => typeof key === 'string' && key.length > 0)) {
    throw Error('CAD_STATUS_INVALID');
  }
  // One source of truth: `mixedKeys` is exactly the keys of the mixed rows, in
  // row order. Two drifting lists would let the panel show the wrong summary.
  const mixedFromRows = rows.filter(row => row.mixed).map(row => row.key);
  if (mixedFromRows.length !== mixedKeys.length
    || mixedFromRows.some((key, index) => key !== mixedKeys[index])) {
    throw Error('CAD_STATUS_INVALID');
  }
  return selection;
}
