// Keyboard geometry for the compact grid chooser (MTEXT's nine attachment codes).
//
// The chooser is a 3×3 grid because the codes are a 3×3 position grid: row is
// `(code - 1) / 3`, column is `(code - 1) % 3`. Arrow keys must therefore move in
// **space**, not in list order — Left at the first column stays put instead of
// jumping to the previous row's last cell, and Down in the last row stays put
// instead of wrapping to the first row. Home/End still jump to the first/last
// code, which is the one linear habit the row keeps.
//
// This module is the pure half so the rule is testable without a mounted DOM; the
// component only moves focus to whatever index it returns.

/** The column count of the one grid picker (the attachment grid is 3×3). */
export const CHOICE_GRID_COLUMNS = 3;

/** The keys the grid chooser interprets itself (everything else is native). */
export type ChoiceGridKey = 'ArrowLeft' | 'ArrowRight' | 'ArrowUp' | 'ArrowDown' | 'Home' | 'End';

/** Whether `key` is one the grid chooser owns. */
export function isChoiceGridKey(key: string): key is ChoiceGridKey {
  return key === 'ArrowLeft' || key === 'ArrowRight' || key === 'ArrowUp' || key === 'ArrowDown'
    || key === 'Home' || key === 'End';
}

/**
 * The index to focus for one key press, or `-1` when nothing can be focused.
 *
 * `from` is the currently focused index (a non-integer, negative or out-of-range
 * value starts from the first cell). Movement is geometric: Left/Right stay inside
 * the current row, Up/Down stay inside the grid, and a move off the edge is a
 * no-op rather than a wrap. `count` is the number of codes (nine for the
 * attachment grid) and `columns` defaults to [`CHOICE_GRID_COLUMNS`].
 */
export function choiceGridFocus(
  from: number,
  key: ChoiceGridKey,
  count: number,
  columns: number = CHOICE_GRID_COLUMNS,
): number {
  const total = Number.isSafeInteger(count) && count > 0 ? count : 0;
  if (total === 0) return -1;
  const width = Number.isSafeInteger(columns) && columns > 0 ? columns : CHOICE_GRID_COLUMNS;
  const start = Number.isSafeInteger(from) && from >= 0 && from < total ? from : 0;
  if (key === 'Home') return 0;
  if (key === 'End') return total - 1;
  const column = start % width;
  if (key === 'ArrowLeft') return column > 0 ? start - 1 : start;
  // The last row may be partial, so `+1` also has to stay inside the count.
  if (key === 'ArrowRight') return column < width - 1 && start + 1 < total ? start + 1 : start;
  if (key === 'ArrowUp') return start - width >= 0 ? start - width : start;
  return start + width < total ? start + width : start;
}
