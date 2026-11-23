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

// ---- several blocks (Ctrl+click) -------------------------------------------
// A selection with more than one block: `Tab.extra` holds the earlier blocks
// and the Selection above is the last (primary) one.

/** The most blocks one selection may hold. */
export const MAX_AREAS = 10_000;

export function sameRect(a: Rect, b: Rect): boolean {
  return a.row === b.row && a.rows === b.rows && a.col === b.col && a.cols === b.cols;
}

/** `inner` lies wholly inside `outer`. */
export function rectInside(inner: Rect, outer: Rect): boolean {
  return inner.row >= outer.row && inner.col >= outer.col && inner.row + inner.rows <= outer.row + outer.rows && inner.col + inner.cols <= outer.col + outer.cols;
}

/** Whether a cell is in any of the blocks. */
export function containsAny(areas: readonly Rect[], row: number, col: number): boolean {
  for (const r of areas) if (row >= r.row && row < r.row + r.rows && col >= r.col && col < r.col + r.cols) return true;
  return false;
}

/** The blocks that touch a window of rows and columns (half-open ends). */
export function touching(areas: readonly Rect[], row0: number, row1: number, col0: number, col1: number): Rect[] {
  return areas.filter((r) => r.row < row1 && r.row + r.rows > row0 && r.col < col1 && r.col + r.cols > col0);
}

/** The same cells as non-overlapping blocks (one cell is in one block only), so
 *  counts and writes never double up. Sweeps down the rows: each band of rows
 *  between two block edges merges the column ranges of the blocks it crosses,
 *  and bands that give the same ranges one below the other are joined. */
export function disjoint(areas: readonly Rect[]): Rect[] {
  if (areas.length <= 1) return areas.map((r) => ({ ...r }));
  const starts = new Map<number, Rect[]>();
  const ends = new Map<number, Rect[]>();
  const ys = new Set<number>();
  for (const r of areas) {
    if (r.rows <= 0 || r.cols <= 0) continue;
    let s = starts.get(r.row);
    if (!s) starts.set(r.row, (s = []));
    s.push(r);
    const e = r.row + r.rows;
    let n = ends.get(e);
    if (!n) ends.set(e, (n = []));
    n.push(r);
    ys.add(r.row);
    ys.add(e);
  }
  const bounds = [...ys].sort((a, b) => a - b);
  const active = new Set<Rect>();
  const out: Rect[] = [];
  let open = new Map<string, Rect>();
  for (let i = 0; i < bounds.length - 1; i++) {
    const y = bounds[i]!;
    for (const r of ends.get(y) ?? []) active.delete(r);
    for (const r of starts.get(y) ?? []) active.add(r);
    const height = bounds[i + 1]! - y;
    const next = new Map<string, Rect>();
    if (active.size) {
      const spans = [...active].map((r) => [r.col, r.col + r.cols] as [number, number]).sort((p, q) => p[0] - q[0] || p[1] - q[1]);
      const emit = (a: number, b: number): void => {
        const key = `${a}:${b}`;
        const above = open.get(key);
        if (above) {
          above.rows += height;
          next.set(key, above);
        } else {
          const r: Rect = { row: y, rows: height, col: a, cols: b - a };
          out.push(r);
          next.set(key, r);
        }
      };
      let [c0, c1] = spans[0]!;
      for (let k = 1; k < spans.length; k++) {
        const [a, b] = spans[k]!;
        if (a <= c1) c1 = Math.max(c1, b);
        else {
          emit(c0, c1);
          c0 = a;
          c1 = b;
        }
      }
      emit(c0, c1);
    }
    open = next;
  }
  return out;
}

/** How many cells the blocks cover, overlaps counted once. */
export function cellCountAreas(areas: readonly Rect[]): number {
  let n = 0;
  for (const r of disjoint(areas)) n += r.rows * r.cols;
  return n;
}

/** The columns any of the blocks touch, in order, each once. */
export function colsOfAreas(areas: readonly Rect[]): number[] {
  const set = new Set<number>();
  for (const r of areas) for (let c = r.col; c < r.col + r.cols; c++) set.add(c);
  return [...set].sort((a, b) => a - b);
}
