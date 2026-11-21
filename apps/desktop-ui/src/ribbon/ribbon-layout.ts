// Pure column-major layout math for the ribbon's small-item grids.
//
// Split verbatim out of `ribbon-model.ts` (which stays under the code-size gate)
// when the View tab's toggle batch pushed it past the ceiling. A plain sibling,
// not a barrel: the model and the tests name this file directly.

/** One cell of a column-major panel grid. */
export interface RibbonCell<T> {
  readonly item: T;
  readonly column: number;
  readonly row: number;
}

/**
 * Cell coordinates for a column-major panel grid with `rows` rows.
 *
 * This mirrors the layout `.ribbon-panel__group.is-small` actually produces
 * (`display: grid; grid-auto-flow: column; grid-template-rows: repeat(rows, …)`),
 * so a test can assert what a reader will see from the model alone: item 1 and
 * item 2 are neighbours *only* because they share a column.
 */
export function columnMajorCells<T>(items: readonly T[], rows: number): RibbonCell<T>[] {
  const height = Math.max(1, Math.trunc(rows) || 1);
  return items.map((item, index) => ({ item, column: Math.floor(index / height), row: index % height }));
}

/**
 * Whether every pair in `pairs` lands in two cells that share an edge
 * (vertically adjacent in one column, or horizontally adjacent in one row).
 *
 * `key` extracts the comparison token from an item, so the same helper works for
 * `standardView` names and for any other pairwise grouping.
 */
export function pairsShareEdge<T extends { readonly id: string }>(
  items: readonly T[],
  rows: number,
  pairs: readonly (readonly [string, string])[],
  key: (item: T) => string = item => item.id,
): boolean {
  const cells = new Map(columnMajorCells(items, rows).map(cell => [key(cell.item), cell]));
  return pairs.every(([a, b]) => {
    const first = cells.get(a);
    const second = cells.get(b);
    if (!first || !second) return false;
    if (first.column === second.column) return Math.abs(first.row - second.row) === 1;
    return first.row === second.row && Math.abs(first.column - second.column) === 1;
  });
}
