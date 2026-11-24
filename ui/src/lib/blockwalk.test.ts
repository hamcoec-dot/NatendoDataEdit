import { describe, expect, it } from "vitest";
import { entryCell, stepAcrossBlocks, stepInBlock, type Cell, type Rect } from "./selection";

function rnd(seed: number): () => number {
  let x = seed >>> 0;
  return () => {
    x = (x * 1664525 + 1013904223) >>> 0;
    return x / 2 ** 32;
  };
}

/** The slow obvious walk: every shown cell of each block in order, blocks in pick
 *  order and round again. Tab goes along each row, Enter down each column. */
function cellsOf(b: Rect, by: "row" | "col", hidden: Set<number>): Cell[] {
  const out: Cell[] = [];
  if (by === "row") {
    for (let r = b.row; r < b.row + b.rows; r++) for (let c = b.col; c < b.col + b.cols; c++) if (!hidden.has(c)) out.push({ row: r, col: c });
  } else {
    for (let c = b.col; c < b.col + b.cols; c++) if (!hidden.has(c)) for (let r = b.row; r < b.row + b.rows; r++) out.push({ row: r, col: c });
  }
  return out;
}

describe("Enter and Tab through several blocks, in the order the blocks were picked", () => {
  it("matches a cell-by-cell walk on 2,000 random block sets (both ways, hidden columns, mixed keys)", () => {
    const r = rnd(2024);
    for (let t = 0; t < 2_000; t++) {
      const n = 1 + Math.floor(r() * 5);
      const hidden = new Set<number>();
      for (let c = 0; c < 8; c++) if (r() < 0.2) hidden.add(c);
      const picked: Rect[] = Array.from({ length: n }, () => ({
        row: Math.floor(r() * 12),
        rows: 1 + Math.floor(r() * 4),
        col: Math.floor(r() * 6),
        cols: 1 + Math.floor(r() * 3),
      }));
      const isHidden = (c: number) => hidden.has(c);
      // The model's place: which pick (the last one first) and where in its cells.
      let idx = n - 1;
      let by: "row" | "col" = r() < 0.5 ? "row" : "col";
      let blocks = picked.map((b) => ({ ...b }));
      const first = cellsOf(picked[idx]!, by, hidden);
      if (!first.length) continue;
      let pos = Math.floor(r() * first.length);
      let cell = first[pos]!;
      for (let step = 0; step < 40; step++) {
        const back = r() < 0.3;
        const nextBy: "row" | "col" = r() < 0.5 ? "row" : "col";
        const dr = nextBy === "col" ? (back ? -1 : 1) : 0;
        const dc = nextBy === "row" ? (back ? -1 : 1) : 0;
        // Model: walk the block's list for this key; past its end, on to the next non-empty block.
        const list = cellsOf(picked[idx]!, nextBy, hidden);
        const at = list.findIndex((c) => c.row === cell.row && c.col === cell.col);
        let want: Cell | null = null;
        let wantIdx = idx;
        if (at >= 0 && at + (back ? -1 : 1) >= 0 && at + (back ? -1 : 1) < list.length) want = list[at + (back ? -1 : 1)]!;
        else if (n === 1) want = list.length ? (back ? list[list.length - 1]! : list[0]!) : null;
        else {
          for (let k = 1; k <= n && !want; k++) {
            const j = (((idx + (back ? -k : k)) % n) + n) % n;
            const l = cellsOf(picked[j]!, nextBy, hidden);
            if (l.length) {
              want = back ? l[l.length - 1]! : l[0]!;
              wantIdx = j;
            }
          }
        }
        const got = stepAcrossBlocks(blocks, cell, dr, dc, isHidden);
        if (!want) {
          expect(got).toBeNull();
          break;
        }
        expect(got, `case ${t} step ${step}`).not.toBeNull();
        expect(got!.cell, `case ${t} step ${step}: cell`).toEqual(want);
        // The active block (the last) is the model's block, and the picks are only turned round.
        expect(got!.blocks[n - 1], `case ${t} step ${step}: active block`).toEqual(picked[wantIdx]);
        expect(
          [...got!.blocks].sort((a, b) => a.row - b.row || a.col - b.col || a.rows - b.rows || a.cols - b.cols),
          `case ${t}: same blocks`,
        ).toEqual([...picked].sort((a, b) => a.row - b.row || a.col - b.col || a.rows - b.rows || a.cols - b.cols));
        // The order is the pick order, turned so the active one is last.
        for (let k = 0; k < n; k++) expect(got!.blocks[k]).toEqual(picked[(wantIdx - (n - 1) + k + n * 2) % n]);
        blocks = got!.blocks;
        cell = got!.cell;
        idx = wantIdx;
        pos = 0;
        by = nextBy;
      }
      void pos;
      void by;
    }
  });

  it("Enter through a block thousands of rows tall with hidden columns inside never stops dead", () => {
    const r = rnd(77);
    for (let t = 0; t < 12; t++) {
      const rows = 1_001 + Math.floor(r() * 4_000);
      const cols = 2 + Math.floor(r() * 4);
      const b: Rect = { row: Math.floor(r() * 10), rows, col: Math.floor(r() * 3), cols };
      const hidden = new Set<number>();
      for (let c = b.col; c < b.col + cols; c++) if (r() < 0.4) hidden.add(c);
      const shown = [...Array(cols).keys()].map((i) => b.col + i).filter((c) => !hidden.has(c));
      if (!shown.length) continue;
      const isHidden = (c: number) => hidden.has(c);
      // Walk the whole block down (Enter) and up (Shift+Enter): the cell-by-cell order, with the wrap.
      for (const dir of [1, -1]) {
        const order: Cell[] = [];
        for (const c of shown) for (let i = 0; i < rows; i++) order.push({ row: b.row + i, col: c });
        if (dir < 0) order.reverse();
        let cur = order[0]!;
        let bad = "";
        for (let i = 1; i <= order.length && !bad; i++) {
          const got = stepInBlock(b, cur, dir, 0, isHidden);
          const want = order[i % order.length]!;
          if (!got || got.cell.row !== want.row || got.cell.col !== want.col || got.wrapped !== (i === order.length)) bad = `case ${t} dir ${dir} step ${i}: ${JSON.stringify(got)} vs ${JSON.stringify(want)}`;
          else cur = got.cell;
        }
        expect(bad).toBe("");
      }
    }
    // The case reported: Enter at the bottom of column A with column B hidden goes to column C.
    const b: Rect = { row: 0, rows: 5_000, col: 0, cols: 3 };
    expect(stepInBlock(b, { row: 4_999, col: 0 }, 1, 0, (c) => c === 1)).toEqual({ cell: { row: 0, col: 2 }, wrapped: false });
    expect(stepInBlock(b, { row: 0, col: 2 }, -1, 0, (c) => c === 1)).toEqual({ cell: { row: 4_999, col: 0 }, wrapped: false });
  });

  it("one block still wraps inside itself (the old rule)", () => {
    const b: Rect = { row: 2, rows: 2, col: 1, cols: 2 };
    const none = () => false;
    expect(stepInBlock(b, { row: 3, col: 2 }, 0, 1, none)).toEqual({ cell: { row: 2, col: 1 }, wrapped: true });
    expect(stepInBlock(b, { row: 2, col: 1 }, 0, -1, none)).toEqual({ cell: { row: 3, col: 2 }, wrapped: true });
    expect(stepInBlock(b, { row: 2, col: 2 }, 1, 0, none)).toEqual({ cell: { row: 3, col: 2 }, wrapped: false });
    expect(stepInBlock(b, { row: 3, col: 1 }, 1, 0, none)).toEqual({ cell: { row: 2, col: 2 }, wrapped: false });
    // A block that shows no column: nothing moves.
    expect(stepInBlock(b, { row: 2, col: 1 }, 0, 1, () => true)).toBeNull();
    expect(entryCell(b, false, () => true)).toBeNull();
  });

  it("goes block to block in the order picked (not top to bottom), and back", () => {
    const low: Rect = { row: 10, rows: 1, col: 0, cols: 2 };
    const high: Rect = { row: 1, rows: 1, col: 3, cols: 1 };
    const none = () => false;
    // Picked: low first, high last (active). Tab from high's only cell goes to low's first cell.
    const a = stepAcrossBlocks([low, high], { row: 1, col: 3 }, 0, 1, none)!;
    expect(a.cell).toEqual({ row: 10, col: 0 });
    expect(a.blocks[1]).toEqual(low);
    // Tab along low, then on to high again (the first pick comes round).
    const b = stepAcrossBlocks(a.blocks, a.cell, 0, 1, none)!;
    expect(b.cell).toEqual({ row: 10, col: 1 });
    const c = stepAcrossBlocks(b.blocks, b.cell, 0, 1, none)!;
    expect(c.cell).toEqual({ row: 1, col: 3 });
    // Shift+Tab goes back the same way.
    const d = stepAcrossBlocks(c.blocks, c.cell, 0, -1, none)!;
    expect(d.cell).toEqual({ row: 10, col: 1 });
  });
});
