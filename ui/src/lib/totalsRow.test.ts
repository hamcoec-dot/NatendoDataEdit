import { afterEach, describe, expect, it, vi } from "vitest";
import type { ColumnTotal, ColumnTotals } from "./backend";
import { Columns } from "./columns";
import { EDIT_DELAY, FILTER_DELAY, TotalsCache, totalCopy, totalParts, totalsLabel } from "./totalsRow";

const total = (col: number, over: Partial<ColumnTotal> = {}): ColumnTotal => ({ col, cells: 4, filled: 3, numbers: 3, blank: 1, sum: 1234.5, average: 411.5, min: 1, max: 1000, too_large: false, suggested: "sum", ...over });
const reply = (cols: number[]): ColumnTotals => ({ revision: 1, rows: 4, filtered: true, columns: cols.map((c) => total(c)) });

describe("totals row cells", () => {
  it("shows the suggested total until a choice is made", () => {
    expect(totalParts(total(0), undefined).name).toBe("Sum");
    expect(totalParts(total(0, { suggested: "count" }), undefined)).toEqual({ name: "Count", value: "3" });
    expect(totalParts(total(0), "blank")).toEqual({ name: "Blank", value: "1" });
    expect(totalParts(total(0), "none")).toEqual({ name: "", value: "" });
    expect(totalParts(total(0, { sum: null, numbers: 0 }), "sum").value).toBe("—");
    expect(totalParts(total(0, { sum: null, too_large: true }), "sum").value).toBe("too large");
  });
  it("copies plain numbers", () => {
    expect(totalCopy(total(0), undefined)).toBe("1234.5");
    expect(totalCopy(total(0, { sum: 0.1 + 0.2 }), "sum")).toBe("0.3");
    expect(totalCopy(total(0), "none")).toBe("");
    expect(totalCopy(undefined, "sum")).toBe("");
  });
  it("says how many rows when some are hidden", () => {
    expect(totalsLabel(142, true)).toBe("Total of 142 shown rows");
    expect(totalsLabel(1, true)).toBe("Total of 1 shown row");
    expect(totalsLabel(142, false)).toBe("Total");
  });
  it("a choice moves with its column", () => {
    const c = new Columns(5, 100);
    c.totals.set(2, "max");
    c.shift(0, 1);
    expect([...c.totals]).toEqual([[3, "max"]]);
    c.move(3, 1, 0);
    expect([...c.totals]).toEqual([[0, "max"]]);
    const kept = c.snapshot();
    c.shift(0, -1);
    expect(c.totals.size).toBe(0);
    c.restore(kept);
    expect([...c.totals]).toEqual([[0, "max"]]);
  });
});

describe("totals cache", () => {
  afterEach(() => vi.useRealTimers());
  const settle = () => vi.advanceTimersByTimeAsync(0);

  it("asks only for missing columns, waits after a change and fades old numbers", async () => {
    vi.useFakeTimers();
    const cache = new TotalsCache();
    const ask = vi.fn(async (cols: number[]) => reply(cols));
    const done = vi.fn();
    cache.want("k1", "d1", [0, 1], ask, done);
    await vi.advanceTimersByTimeAsync(FILTER_DELAY);
    expect(ask).toHaveBeenLastCalledWith([0, 1]);
    expect(cache.get(1)?.stale).toBe(false);
    expect(cache.label()).toBe("Total of 4 shown rows");
    // Scrolling: only the new column, at once.
    cache.want("k1", "d1", [1, 2], ask, done);
    await settle();
    expect(ask).toHaveBeenLastCalledWith([2]);
    // A filter: soon; the old number stays, faded.
    cache.want("k2", "d1", [1], ask, done);
    expect(cache.get(1)?.stale).toBe(true);
    await vi.advanceTimersByTimeAsync(FILTER_DELAY);
    expect(ask).toHaveBeenCalledTimes(3);
    // An edit: after the longer pause.
    cache.want("k3", "d2", [1], ask, done);
    await vi.advanceTimersByTimeAsync(EDIT_DELAY - 1);
    expect(ask).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(1);
    expect(ask).toHaveBeenCalledTimes(4);
  });

  it("drops a reply for an older state and asks nothing when stopped", async () => {
    vi.useFakeTimers();
    const cache = new TotalsCache();
    let release: (r: ColumnTotals) => void = () => {};
    const slow = () => new Promise<ColumnTotals>((r) => (release = r));
    cache.want("k1", "d1", [0], slow, () => {});
    await vi.advanceTimersByTimeAsync(FILTER_DELAY);
    cache.want("k2", "d1", [0], async (c) => reply(c), () => {});
    release(reply([0]));
    await settle();
    expect(cache.get(0)).toBeNull();
    cache.stop();
    const ask = vi.fn(async (c: number[]) => reply(c));
    await vi.advanceTimersByTimeAsync(5000);
    expect(ask).not.toHaveBeenCalled();
    expect(cache.get(0)).toBeNull();
  });
});
