import { describe, expect, it } from "vitest";
import { Columns } from "./columns";
import { RowHeights } from "./rowheights";
import { snapForward, snapMax, type Axis } from "./scrollsnap";

const cols = (c: Columns): Axis => ({ start: (i) => c.x(i), at: (x) => c.at(x) });
const rows = (h: RowHeights, base: number): Axis => ({ start: (r) => h.offsetOf(r, base), at: (y) => h.rowAt(y, base) });

/** The scroll a reveal used before this change: the item's far edge at the view's far edge. */
const plainReveal = (a: number, size: number, vis: number): number => Math.min(a, a + size - vis);
/** The max before this change. */
const plainMax = (content: number, vis: number, slack: number): number => Math.max(0, content - vis + slack);

describe("snapping a reveal to whole columns", () => {
  it("the reported case: four 130 px columns in 491 px land on column 2, not 29 px in", () => {
    const c = new Columns(4, 130);
    expect(plainReveal(390, 130, 491)).toBe(29);
    expect(snapForward(cols(c), 0, 3, 130, 491)).toBe(130);
    // ...and the max lets it stay there (today's max was 69).
    expect(plainMax(520, 491, 40)).toBe(69);
    expect(snapMax(cols(c), 0, 4, 491, 40)).toBe(130);
  });

  it("does not scroll a column that already fits", () => {
    const c = new Columns(4, 130);
    expect(snapForward(cols(c), 0, 2, 130, 491)).toBe(0);
    expect(snapForward(cols(c), 0, 3, 130, 600)).toBe(0);
    expect(snapMax(cols(c), 0, 4, 600, 40)).toBe(0);
  });

  it("lands every step of a walk to the right on a boundary, the cell whole on screen", () => {
    const c = new Columns(30, 130);
    for (const [i, w] of [[3, 60], [7, 300], [12, 45], [20, 220]] as const) c.setWidth(i, w);
    const vis = 700;
    const bounds = new Set(Array.from({ length: 31 }, (_, i) => c.x(i)));
    let scroll = 0;
    for (let i = 0; i < 30; i++) {
      const a = c.x(i);
      const w = c.width(i);
      if (a < scroll) scroll = a;
      else if (a + w > scroll + vis) scroll = snapForward(cols(c), 0, i, w, vis);
      expect(bounds.has(scroll), `column ${i}`).toBe(true);
      expect(a >= scroll && a + w <= scroll + vis, `column ${i}`).toBe(true);
      expect(scroll).toBeLessThanOrEqual(snapMax(cols(c), 0, 30, vis, 40));
    }
  });

  it("puts a column wider than the view at its own left edge", () => {
    const c = new Columns(5, 130);
    c.setWidth(3, 900);
    expect(snapForward(cols(c), 0, 3, 900, 491)).toBe(390);
  });

  it("measures from the first column that scrolls (frozen columns)", () => {
    const c = new Columns(8, 100);
    c.setWidth(0, 70);
    // Two frozen columns: the body starts at x(2) = 170; column 6 starts 400 in.
    expect(snapForward(cols(c), 2, 6, 100, 450)).toBe(100);
    expect(snapForward(cols(c), 2, 7, 100, 450)).toBe(200);
    expect(snapMax(cols(c), 2, 8, 450, 40)).toBe(200);
  });

  it("passes over hidden columns", () => {
    const c = new Columns(6, 100);
    c.setWidth(1, 0);
    // Shown: 0, 2, 3, 4, 5 at 0, 100, 200, 300, 400. Column 5 in 330 px needs 170 → 200.
    expect(snapForward(cols(c), 0, 5, 100, 330)).toBe(200);
    // The boundary shared by the hidden column and the next is fine as a stop.
    expect(snapForward(cols(c), 0, 3, 100, 250)).toBe(100);
  });

  it("does not leave a screen of blank space after a very wide last column (S2)", () => {
    const c = new Columns(4, 130);
    c.setWidth(3, 2000);
    const content = c.total();
    const vis = 491;
    const max = snapMax(cols(c), 0, 4, vis, 40);
    expect(max).toBe(plainMax(content, vis, 40));
    // The wheel and the scrollbar still reach the end, with 40 px after it.
    expect(content - max).toBe(vis - 40);
  });

  it("leaves at most one column (or 40 px) of blank space after a normal last column (S2)", () => {
    const c = new Columns(4, 130);
    const max = snapMax(cols(c), 0, 4, 491, 40);
    const blank = max - (c.total() - 491);
    expect(blank).toBeLessThanOrEqual(Math.max(130, 40));
    // A table that ends exactly on a boundary keeps today's max.
    const d = new Columns(10, 100);
    expect(snapMax(cols(d), 0, 10, 700, 40)).toBe(plainMax(1000, 700, 40));
  });

  it("keeps a wheel or scrollbar position that is not on a boundary (clamp leaves it)", () => {
    const c = new Columns(30, 130);
    const max = snapMax(cols(c), 0, 30, 700, 40);
    for (const v of [1, 29, 333.5, 1001, max - 1]) expect(Math.min(Math.max(0, v), max)).toBe(v);
  });
});

describe("snapping a reveal to whole rows", () => {
  it("plain rows: moving down stops with the top row whole", () => {
    const h = new RowHeights();
    // 24 px rows, 500 px of rows on screen: row 20 (480..504) needs 4 px; the snap takes a whole row.
    expect(plainReveal(480, 24, 500)).toBe(4);
    expect(snapForward(rows(h, 24), 0, 20, 24, 500)).toBe(24);
    expect(snapForward(rows(h, 24), 0, 19, 24, 500)).toBe(0);
  });

  it("custom and wrapped heights", () => {
    const h = new RowHeights();
    h.set([2], 100);
    h.set([5], 60);
    const ax = rows(h, 24);
    // Tops: 0,24,48,148,172,196,256,280 … row 7 (280..304) in 200 px needs 104 → row 3 at 148.
    expect(snapForward(ax, 0, 7, 24, 200)).toBe(148);
    // A row taller than the view lands at its own top.
    h.set([9], 400);
    expect(snapForward(ax, 0, 9, 400, 200)).toBe(h.offsetOf(9, 24));
  });

  it("frozen rows: measured from the first body row", () => {
    const h = new RowHeights();
    // Two frozen rows; row 22 is 20 rows (480 px) below the first body row.
    expect(snapForward(rows(h, 24), 2, 22, 24, 500)).toBe(24);
  });

  it("a page down, Ctrl+End and the max agree", () => {
    const h = new RowHeights();
    const n = 1_000_001;
    const ax = rows(h, 24);
    const vis = 500;
    const max = snapMax(ax, 0, n, vis, 24);
    const last = snapForward(ax, 0, n - 1, 24, vis);
    expect(last % 24).toBe(0);
    expect(last).toBeLessThanOrEqual(max);
    // A page of 19 rows from the top: row 19 fits, row 39 lands on a boundary.
    expect(snapForward(ax, 0, 39, 24, vis) % 24).toBe(0);
  });
});
