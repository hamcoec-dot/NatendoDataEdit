import { describe, expect, it } from "vitest";
import { clampRowPx, MAX_CUSTOM_ROWS, MAX_ROW_PX, MIN_ROW_PX, RowHeights } from "./rowheights";

function rnd(seed: number): () => number {
  let x = seed >>> 0;
  return () => {
    x = (x * 1664525 + 1013904223) >>> 0;
    return x / 2 ** 32;
  };
}

/** The slow obvious model: one height per row. */
class Model {
  h: (number | null)[];
  constructor(public n: number) {
    this.h = Array.from({ length: n }, () => null);
  }
  height(r: number, base: number): number {
    return this.h[r] ?? base;
  }
  offset(r: number, base: number): number {
    let y = 0;
    for (let i = 0; i < r; i++) y += this.height(i, base);
    return y;
  }
  rowAt(y: number, base: number): number {
    let top = 0;
    for (let r = 0; r < this.n + 1000; r++) {
      const h = this.height(r, base);
      if (y < top + h) return r;
      top += h;
    }
    return this.n + 1000;
  }
}

function sameCustom(a: RowHeights, m: Model): void {
  const want: [number, number][] = [];
  m.h.forEach((v, i) => {
    if (v !== null) want.push([i, v]);
  });
  expect(a.toView()).toEqual(want);
}

describe("RowHeights lookups", () => {
  it("agrees with a row-by-row walk on 10,000 random cases", () => {
    const r = rnd(42);
    for (let t = 0; t < 10_000; t++) {
      const n = 1 + Math.floor(r() * 40);
      const base = 14 + Math.floor(r() * 30);
      const m = new Model(n);
      const h = new RowHeights();
      const picks = Math.floor(r() * 8);
      for (let i = 0; i < picks; i++) {
        const row = Math.floor(r() * n);
        const px = MIN_ROW_PX + Math.floor(r() * 120);
        m.h[row] = px;
        h.set([row], px);
      }
      for (let k = 0; k < 6; k++) {
        const row = Math.floor(r() * (n + 3));
        expect(h.offsetOf(row, base)).toBe(m.offset(row, base));
        expect(h.heightOf(row, base)).toBe(m.height(row, base));
        const y = Math.floor(r() * (m.offset(n, base) + 3 * base));
        expect(h.rowAt(y, base)).toBe(m.rowAt(y, base));
      }
      expect(h.totalHeight(n, base)).toBe(m.offset(n, base));
      // rowAt and offsetOf are inverses on every row boundary.
      for (let row = 0; row < Math.min(n, 6); row++) expect(h.rowAt(h.offsetOf(row, base), base)).toBe(row);
    }
  });

  it("is the plain arithmetic with no custom rows", () => {
    const h = new RowHeights();
    expect(h.empty).toBe(true);
    expect(h.offsetOf(1000, 24)).toBe(24_000);
    expect(h.rowAt(24_000, 24)).toBe(1000);
    expect(h.rowAt(24_000 + 23, 24)).toBe(1000);
    expect(h.heightOf(7, 24)).toBe(24);
  });

  it("clamps heights and refuses to pass the cap", () => {
    const h = new RowHeights();
    h.set([1], 3);
    h.set([2], 9999);
    expect(h.custom(1)).toBe(MIN_ROW_PX);
    expect(h.custom(2)).toBe(MAX_ROW_PX);
    expect(clampRowPx(20.4)).toBe(20);
    const many = Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => i + 10);
    const g = new RowHeights();
    expect(g.set(many, 30)).toBe(true);
    expect(g.set([5_000_000], 30)).toBe(false);
    expect(g.count).toBe(MAX_CUSTOM_ROWS);
    expect(g.custom(5_000_000)).toBe(null);
  });
});

describe("RowHeights follow the rows", () => {
  function build(seed: number) {
    const r = rnd(seed);
    const n = 5 + Math.floor(r() * 25);
    const m = new Model(n);
    const h = new RowHeights();
    for (let i = 0; i < 6; i++) {
      const row = Math.floor(r() * n);
      const px = MIN_ROW_PX + Math.floor(r() * 100);
      m.h[row] = px;
      h.set([row], px);
    }
    return { r, n, m, h };
  }

  it("insert, delete and move match the same change made to the model", () => {
    for (let t = 0; t < 3_000; t++) {
      const { r, n, m, h } = build(t + 1);
      const op = Math.floor(r() * 3);
      if (op === 0) {
        const at = Math.floor(r() * (n + 1));
        const count = 1 + Math.floor(r() * 4);
        m.h.splice(at, 0, ...Array.from({ length: count }, () => null));
        h.insertRows(at, count);
      } else if (op === 1) {
        const at = Math.floor(r() * n);
        const count = 1 + Math.floor(r() * Math.min(4, n - at));
        m.h.splice(at, count);
        h.deleteRows(at, count);
      } else {
        const at = Math.floor(r() * n);
        const count = 1 + Math.floor(r() * Math.min(4, n - at));
        // `to` as dropIndex gives it: where the block starts afterwards.
        const rest = n - count;
        const to = Math.floor(r() * (rest + 1));
        const block = m.h.splice(at, count);
        m.h.splice(to, 0, ...block);
        h.moveRows(at, count, to);
      }
      sameCustom(h, m);
    }
  });

  it("reset puts rows back to the uniform height", () => {
    const h = new RowHeights();
    h.set([2, 3, 9], 50);
    h.reset([3, 4]);
    expect(h.toView()).toEqual([
      [2, 50],
      [9, 50],
    ]);
    h.clear();
    expect(h.empty).toBe(true);
  });
});

describe("RowHeights set on one row (a drag)", () => {
  it("gives the same result as the general path on 3,000 random one-row changes", () => {
    const r = rnd(7);
    for (let t = 0; t < 3_000; t++) {
      const a = new RowHeights();
      const b = new RowHeights();
      for (let i = 0; i < 12; i++) {
        const row = Math.floor(r() * 30);
        const px = MIN_ROW_PX + Math.floor(r() * 200);
        a.set([row], px); // one row: the fast path
        b.set([row, row], px); // two entries: the general path
        expect(a.toView()).toEqual(b.toView());
        const q = Math.floor(r() * 40);
        expect(a.offsetOf(q, 24)).toBe(b.offsetOf(q, 24));
        expect(a.totalHeight(40, 24)).toBe(b.totalHeight(40, 24));
      }
    }
  });

  it("changes nothing for the same height and refuses past the cap", () => {
    const h = new RowHeights();
    h.set([4], 50);
    const s = h.snapshot();
    expect(h.set([4], 50)).toBe(true);
    expect(h.sameAs(s)).toBe(true);
    const many = new RowHeights();
    many.set(Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => i), 30);
    expect(many.set([MAX_CUSTOM_ROWS + 5], 30)).toBe(false);
    expect(many.set([7], 31)).toBe(true);
    expect(many.custom(7)).toBe(31);
  });

  it("a drag of one row (60 moves) with 100,000 heights is fast: each move well under 5 ms (median)", () => {
    const h = new RowHeights();
    h.set(Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => i * 3), 40);
    const times: number[] = [];
    for (let i = 0; i < 60; i++) {
      const t0 = performance.now();
      h.set([150_000], 30 + i);
      times.push(performance.now() - t0);
    }
    times.sort((x, y) => x - y);
    expect(times[30]!).toBeLessThan(5);
  });
});

describe("RowHeights speed", () => {
  it("insert one row above 100,000 custom rows in well under 100 ms (best of 3)", () => {
    const rows = Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => i * 3);
    const h = new RowHeights();
    h.set(rows, 40);
    h.insertRows(0, 1);
    let best = Infinity;
    for (let i = 0; i < 3; i++) {
      const t0 = performance.now();
      h.insertRows(0, 1);
      best = Math.min(best, performance.now() - t0);
    }
    expect(best).toBeLessThan(100);
    // A lookup is a binary search: a million of them cost next to nothing.
    const t1 = performance.now();
    let x = 0;
    for (let i = 0; i < 1_000_000; i++) x += h.offsetOf((i * 7919) % 300_000, 24);
    expect(performance.now() - t1).toBeLessThan(1500);
    expect(x).toBeGreaterThan(0);
  });
});

describe("RowHeights snapshots (Undo)", () => {
  it("restore puts the heights back, and an unchanged set is the same snapshot every time", () => {
    const h = new RowHeights();
    h.set([1, 5], 60);
    const a = h.snapshot();
    const again = h.snapshot();
    expect(again.rows).toBe(a.rows);
    expect(again.px).toBe(a.px);
    h.set([5], 20);
    expect(h.sameAs(a)).toBe(false);
    h.restore(a);
    expect(h.toView()).toEqual([
      [1, 60],
      [5, 60],
    ]);
    expect(h.sameAs(a)).toBe(true);
  });

  it("50 data edits with 100,000 custom rows keep ONE snapshot (nothing is copied per step)", () => {
    const h = new RowHeights();
    h.set(
      Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => i * 2),
      40,
    );
    const snaps = Array.from({ length: 50 }, () => h.snapshot());
    for (const s of snaps) {
      expect(s.rows).toBe(snaps[0]!.rows);
      expect(s.px).toBe(snaps[0]!.px);
    }
    expect(h.bytes).toBe(MAX_CUSTOM_ROWS * 16);
  });
});

describe("RowHeights from a view file", () => {
  it("round-trips", () => {
    const h = new RowHeights();
    h.set([3, 10, 200], 77);
    const { heights, dropped } = RowHeights.fromView(h.toView(), 1000);
    expect(dropped).toBe(0);
    expect(heights.toView()).toEqual(h.toView());
  });

  it("drops what is wrong without failing the rest", () => {
    const raw = [
      [5, 30],
      [5, 40],
      [-1, 30],
      [2.5, 30],
      [999_999, 30],
      [7, 3],
      [8, 9000],
      ["x", 30],
      [9],
      [10, Number.NaN],
      [1, 20],
    ];
    const { heights, dropped } = RowHeights.fromView(raw, 100);
    expect(heights.toView()).toEqual([
      [1, 20],
      [5, 40],
      [7, MIN_ROW_PX],
      [8, MAX_ROW_PX],
    ]);
    expect(dropped).toBe(7);
    expect(RowHeights.fromView("nope", 10).heights.empty).toBe(true);
    expect(RowHeights.fromView(undefined, 10).dropped).toBe(0);
  });

  it("stops at the cap", () => {
    const raw = Array.from({ length: MAX_CUSTOM_ROWS + 50 }, (_, i) => [i, 30]);
    const { heights, dropped } = RowHeights.fromView(raw, 10_000_000);
    expect(heights.count).toBe(MAX_CUSTOM_ROWS);
    expect(dropped).toBe(50);
  });

  it("loads 100,000 heights in well under 100 ms (best of 3)", () => {
    const raw = Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => [i * 3, 30 + (i % 50)]);
    RowHeights.fromView(raw, 10_000_000);
    let best = Infinity;
    for (let i = 0; i < 3; i++) {
      const t0 = performance.now();
      RowHeights.fromView(raw, 10_000_000);
      best = Math.min(best, performance.now() - t0);
    }
    expect(best).toBeLessThan(100);
  });
});
