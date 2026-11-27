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

  /** Best of seven after a warm-up. */
  function best(f: () => unknown): number {
    f();
    let ms = Infinity;
    for (let i = 0; i < 7; i++) {
      const t0 = performance.now();
      f();
      ms = Math.min(ms, performance.now() - t0);
    }
    return ms;
  }

  /** The cap's worth of areas must not cost much more than ten times a
   *  tenth of them (linear-ish, not quadratic: that would be a hundred
   *  times). A ratio, both measured together, so a loaded machine slows
   *  both alike; the absolute ceiling is only a backstop, and perf.rs keeps
   *  the strict gate. */
  function scalesWell(make: (n: number) => Rect[]): void {
    const small = make(MAX_AREAS / 10);
    const full = make(MAX_AREAS);
    // The small run is timed on a tenth of the work: floor it so timer
    // granularity cannot inflate the ratio.
    const a = Math.max(best(() => disjoint(small)), 1);
    const b = best(() => disjoint(full));
    expect(b / a, `${b.toFixed(2)} ms for ${MAX_AREAS}, ${a.toFixed(2)} ms for a tenth`).toBeLessThan(40);
    expect(b).toBeLessThan(500);
  }

  it("splits the cap's worth of single cells quickly (it scales, not squares)", () => {
    const r = rnd(11);
    const areas: Rect[] = Array.from({ length: MAX_AREAS }, () => ({ row: Math.floor(r() * 1_000_000), rows: 1, col: Math.floor(r() * 20), cols: 1 }));
    expect(disjoint(areas).length).toBeGreaterThan(0);
    scalesWell((n) => areas.slice(0, n));
  });

  it("splits the cap's worth of cells in one row and in a staircase quickly too", () => {
    const row: Rect[] = Array.from({ length: MAX_AREAS }, (_, i) => ({ row: 5, rows: 1, col: i * 2, cols: 1 }));
    const stairs: Rect[] = Array.from({ length: MAX_AREAS }, (_, i) => ({ row: i, rows: 1, col: i, cols: 1 }));
    for (const a of [row, stairs]) {
      expect(disjoint(a).reduce((n, r) => n + r.rows * r.cols, 0)).toBe(MAX_AREAS);
      scalesWell((n) => a.slice(0, n));
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
