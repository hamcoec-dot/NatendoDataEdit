import { describe, expect, it } from "vitest";
import { cellCountAreas, colsOfAreas, containsAny, disjoint, MAX_AREAS, rectInside, touching, type Rect } from "./selection";

/** The cells of some blocks, the slow obvious way. */
function cells(areas: Rect[]): Set<string> {
  const s = new Set<string>();
  for (const r of areas) for (let i = 0; i < r.rows; i++) for (let j = 0; j < r.cols; j++) s.add(`${r.row + i}:${r.col + j}`);
  return s;
}

function rnd(seed: number): () => number {
  let x = seed;
  return () => {
    x = (x * 1664525 + 1013904223) >>> 0;
    return x / 2 ** 32;
  };
}

describe("disjoint", () => {
  it("keeps one block and empties none", () => {
    expect(disjoint([{ row: 2, rows: 3, col: 1, cols: 2 }])).toEqual([{ row: 2, rows: 3, col: 1, cols: 2 }]);
    expect(disjoint([])).toEqual([]);
  });

  it("splits overlapping, nested, touching and identical blocks into the same cells with no cell twice", () => {
    const cases: Rect[][] = [
      [
        { row: 0, rows: 4, col: 0, cols: 4 },
        { row: 2, rows: 4, col: 2, cols: 4 },
      ],
      [
        { row: 0, rows: 10, col: 0, cols: 10 },
        { row: 3, rows: 2, col: 3, cols: 2 },
      ],
      [
        { row: 0, rows: 2, col: 0, cols: 2 },
        { row: 2, rows: 2, col: 0, cols: 2 },
      ],
      [
        { row: 1, rows: 2, col: 1, cols: 2 },
        { row: 1, rows: 2, col: 1, cols: 2 },
      ],
      [
        { row: 5, rows: 1, col: 5, cols: 1 },
        { row: 5, rows: 1, col: 6, cols: 1 },
        { row: 5, rows: 1, col: 5, cols: 2 },
      ],
    ];
    for (const c of cases) {
      const d = disjoint(c);
      const want = cells(c);
      expect(cells(d)).toEqual(want);
      expect(d.reduce((n, r) => n + r.rows * r.cols, 0)).toBe(want.size);
    }
  });

  it("agrees with a brute-force cell set on random small cases", () => {
    const r = rnd(7);
    for (let t = 0; t < 400; t++) {
      const n = 1 + Math.floor(r() * 8);
      const areas: Rect[] = Array.from({ length: n }, () => ({ row: Math.floor(r() * 8), rows: 1 + Math.floor(r() * 5), col: Math.floor(r() * 8), cols: 1 + Math.floor(r() * 5) }));
      const d = disjoint(areas);
      const want = cells(areas);
      expect(cells(d)).toEqual(want);
      expect(d.reduce((a, x) => a + x.rows * x.cols, 0)).toBe(want.size);
      expect(cellCountAreas(areas)).toBe(want.size);
    }
  });

  it("splits the cap's worth of single cells in well under 50 ms", () => {
    const r = rnd(11);
    const areas: Rect[] = Array.from({ length: MAX_AREAS }, () => ({ row: Math.floor(r() * 1_000_000), rows: 1, col: Math.floor(r() * 20), cols: 1 }));
    const t0 = performance.now();
    const d = disjoint(areas);
    const ms = performance.now() - t0;
    expect(d.length).toBeGreaterThan(0);
    expect(ms).toBeLessThan(50);
  });

  it("splits the cap's worth of cells in one row and in a staircase quickly too", () => {
    const row: Rect[] = Array.from({ length: MAX_AREAS }, (_, i) => ({ row: 5, rows: 1, col: i * 2, cols: 1 }));
    const stairs: Rect[] = Array.from({ length: MAX_AREAS }, (_, i) => ({ row: i, rows: 1, col: i, cols: 1 }));
    for (const a of [row, stairs]) {
      const t0 = performance.now();
      const d = disjoint(a);
      expect(performance.now() - t0).toBeLessThan(50);
      expect(d.reduce((n, r) => n + r.rows * r.cols, 0)).toBe(MAX_AREAS);
    }
  });
});

describe("helpers", () => {
  it("containsAny, touching, rectInside, colsOfAreas", () => {
    const a: Rect[] = [
      { row: 0, rows: 2, col: 0, cols: 2 },
      { row: 10, rows: 1, col: 5, cols: 1 },
    ];
    expect(containsAny(a, 1, 1)).toBe(true);
    expect(containsAny(a, 5, 5)).toBe(false);
    expect(touching(a, 9, 20, 0, 10)).toEqual([a[1]]);
    expect(rectInside({ row: 0, rows: 1, col: 1, cols: 1 }, a[0]!)).toBe(true);
    expect(rectInside({ row: 0, rows: 3, col: 1, cols: 1 }, a[0]!)).toBe(false);
    expect(colsOfAreas(a)).toEqual([0, 1, 5]);
  });
});
