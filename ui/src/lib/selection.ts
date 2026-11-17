// Selection: an anchor cell and a focus cell; the selected block is the
// rectangle between them. Whole rows or columns are selected by spanning
// the full width or height. As in a spreadsheet, the active cell (where
// typing goes, and where a plain arrow key moves from) is where the
// selection started: the anchor, or `active` when that is elsewhere in the
// block (whole rows or columns picked with Shift+Space or Ctrl+Space).
// Shift+arrows and Shift+click move the focus, the far corner.

export interface Cell {
  row: number;
  col: number;
}

export interface Rect {
  row: number;
  rows: number;
  col: number;
  cols: number;
}

export interface Selection {
  anchor: Cell;
  focus: Cell;
  /** The active cell when it is not the anchor (inside the block). */
  active?: Cell;
}

/** The active cell of a selection. */
export function activeOf(s: Selection): Cell {
  return s.active ?? s.anchor;
}

export function rectOf(s: Selection): Rect {
  const row = Math.min(s.anchor.row, s.focus.row);
  const col = Math.min(s.anchor.col, s.focus.col);
  return {
    row,
    col,
    rows: Math.abs(s.anchor.row - s.focus.row) + 1,
    cols: Math.abs(s.anchor.col - s.focus.col) + 1,
  };
}

export function contains(r: Rect, row: number, col: number): boolean {
  return row >= r.row && row < r.row + r.rows && col >= r.col && col < r.col + r.cols;
}

export function cellCount(r: Rect): number {
  return r.rows * r.cols;
}

export function clampCell(c: Cell, rows: number, cols: number): Cell {
  return {
    row: Math.max(0, Math.min(Math.max(rows - 1, 0), c.row)),
    col: Math.max(0, Math.min(Math.max(cols - 1, 0), c.col)),
  };
}

export function single(c: Cell): Selection {
  return { anchor: { ...c }, focus: { ...c } };
}
