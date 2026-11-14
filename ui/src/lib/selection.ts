// Selection: an anchor cell and a focus cell; the selected block is the
// rectangle between them. Whole rows or columns are selected by spanning
// the full width or height.

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
