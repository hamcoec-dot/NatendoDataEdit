import { describe, expect, it } from "vitest";
import { blockTarget, dropIndex, gapAt, onOutline } from "./dragmove";
import { Columns } from "./columns";

describe("drag to move", () => {
  it("picks the nearer gap of an item", () => {
    expect(gapAt(3, 10, 100)).toBe(3);
    expect(gapAt(3, 49, 100)).toBe(3);
    expect(gapAt(3, 50, 100)).toBe(4);
    expect(gapAt(3, 99, 100)).toBe(4);
  });

  it("turns a gap into the new start, or null for no move", () => {
    // Two items at 2-3 of 10.
    expect(dropIndex(2, 2, 0, 10)).toBe(0);
    expect(dropIndex(2, 2, 1, 10)).toBe(1);
    expect(dropIndex(2, 2, 2, 10)).toBeNull();
    expect(dropIndex(2, 2, 3, 10)).toBeNull();
    expect(dropIndex(2, 2, 4, 10)).toBeNull();
    expect(dropIndex(2, 2, 5, 10)).toBe(3);
    expect(dropIndex(2, 2, 10, 10)).toBe(8);
    // Past the end is the end.
    expect(dropIndex(2, 2, 99, 10)).toBe(8);
    // Not before the header row.
    expect(dropIndex(3, 1, 0, 10, 1)).toBe(1);
    expect(dropIndex(0, 1, 5, 10, 1)).toBeNull();
    // Out of range or empty.
    expect(dropIndex(9, 2, 0, 10)).toBeNull();
    expect(dropIndex(0, 0, 5, 10)).toBeNull();
  });

  it("keeps the grabbed cell under the pointer, inside the table", () => {
    const rect = { row: 2, col: 1, rows: 3, cols: 2 };
    expect(blockTarget(rect, { row: 3, col: 1 }, { row: 6, col: 4 }, 100, 10, 0)).toEqual({ row: 5, col: 4 });
    // Clamped at the edges.
    expect(blockTarget(rect, { row: 3, col: 1 }, { row: 99, col: 9 }, 100, 10, 0)).toEqual({ row: 97, col: 8 });
    expect(blockTarget(rect, { row: 4, col: 2 }, { row: 0, col: 0 }, 100, 10, 1)).toEqual({ row: 1, col: 0 });
    // A block as big as the table stays where it is.
    expect(blockTarget({ row: 0, col: 0, rows: 5, cols: 3 }, { row: 0, col: 0 }, { row: 4, col: 2 }, 5, 3, 0)).toEqual({ row: 0, col: 0 });
  });

  it("finds the outline, not the inside", () => {
    const box = { x: 100, y: 50, w: 80, h: 40 };
    expect(onOutline(100, 70, box)).toBe(true);
    expect(onOutline(97, 70, box)).toBe(true);
    expect(onOutline(180, 60, box)).toBe(true);
    expect(onOutline(140, 50, box)).toBe(true);
    expect(onOutline(140, 92, box)).toBe(true);
    expect(onOutline(140, 70, box)).toBe(false);
    expect(onOutline(90, 70, box)).toBe(false);
    expect(onOutline(140, 100, box)).toBe(false);
  });

  it("carries widths, formats and wrapping with moved columns", () => {
    const c = new Columns(6, 100);
    c.setWidth(1, 150);
    c.setWidth(4, 0);
    c.wraps.add(1);
    c.formats.set(2, { kind: "number", decimals: 2 } as never);
    // Columns 1-2 to start at 3: 0 3 4 1 2 5.
    c.move(1, 2, 3);
    expect([0, 1, 2, 3, 4, 5].map((i) => c.width(i))).toEqual([100, 100, 0, 150, 100, 100]);
    expect([...c.wraps]).toEqual([3]);
    expect([...c.formats.keys()]).toEqual([4]);
    expect(c.x(6)).toBe(550);
    // And back.
    c.move(3, 2, 1);
    expect([0, 1, 2, 3, 4, 5].map((i) => c.width(i))).toEqual([100, 150, 100, 100, 0, 100]);
    expect([...c.wraps]).toEqual([1]);
    expect([...c.formats.keys()]).toEqual([2]);
  });

  it("sizes a run of columns at once and shows hidden ones", () => {
    const c = new Columns(6, 100);
    c.setWidth(1, 0);
    c.setWidth(2, 0);
    c.setWidth(4, 0);
    // Hidden columns in the run stay hidden.
    c.setRange(3, 2, 120);
    expect([0, 1, 2, 3, 4, 5].map((i) => c.width(i))).toEqual([100, 0, 0, 120, 0, 100]);
    expect(c.hiddenIn(0, 1)).toBe(false);
    expect(c.hiddenIn(2, 4)).toBe(true);
    c.showRange(1, 3);
    expect([0, 1, 2, 3, 4, 5].map((i) => c.width(i))).toEqual([100, 100, 100, 120, 0, 100]);
    expect(c.hiddenIn(0, 4)).toBe(false);
    // Every column: a new default, hidden ones stay hidden.
    c.setWidth(5, 0);
    c.setRange(0, 6, 80);
    expect([0, 1, 2, 3, 4, 5].map((i) => c.width(i))).toEqual([80, 80, 80, 80, 0, 0]);
    expect(c.x(6)).toBe(320);
  });

  it("gives hidden columns their width back when shown", () => {
    const c = new Columns(5, 100);
    c.setWidth(1, 170);
    c.setWidth(1, 0);
    c.setWidth(2, 0);
    // Moved and shifted while hidden, the old width goes along.
    c.move(1, 1, 3);
    c.shift(0, 1);
    expect(c.isHidden(4)).toBe(true);
    c.showAll();
    expect([0, 1, 2, 3, 4, 5].map((i) => c.width(i))).toEqual([100, 100, 100, 100, 170, 100]);
    c.setWidth(0, 0);
    c.showRange(0, 1);
    expect(c.width(0)).toBe(100);
  });
});
