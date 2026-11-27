import { describe, expect, it } from "vitest";
import type { ColumnTotal } from "./backend";
import { autoSumPlan, autoSumWords, isAutoSumKey, nameList, settlePending } from "./autosum";
import type { TotalChoice } from "./totalsRow";

const total = (col: number, numbers: number): ColumnTotal => ({
  col,
  cells: 10,
  filled: 10,
  numbers,
  blank: 0,
  sum: numbers ? 5 : null,
  average: null,
  min: null,
  max: null,
  too_large: false,
  suggested: numbers ? "sum" : "count",
});
const key = (o: Partial<{ key: string; code: string; altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }>) => ({
  key: "",
  code: "",
  altKey: true,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...o,
});

describe("Alt+= key", () => {
  it("matches = on any layout and the = place, never AltGr or Ctrl", () => {
    expect(isAutoSumKey(key({ key: "=", code: "Equal" }))).toBe(true);
    // German layout: "=" is Shift+0.
    expect(isAutoSumKey(key({ key: "=", code: "Digit0", shiftKey: true }))).toBe(true);
    // A layout where the "=" place types something else.
    expect(isAutoSumKey(key({ key: "´", code: "Equal" }))).toBe(true);
    expect(isAutoSumKey(key({ key: "+", code: "Equal", shiftKey: true }))).toBe(false);
    expect(isAutoSumKey(key({ key: "=", code: "Equal", ctrlKey: true }))).toBe(false);
    expect(isAutoSumKey(key({ key: "=", code: "Equal", altKey: false }))).toBe(false);
  });
});

describe("which columns get Sum", () => {
  const none = new Map<number, TotalChoice>();
  it("uses the column types and the totals already known, and waits for the rest", () => {
    const typed = [
      [0, "number"],
      [1, "text"],
      [2, "id"],
      [3, { date: null }],
    ] as const;
    const known = new Map([
      [4, total(4, 3)],
      [5, total(5, 0)],
    ]);
    const p = autoSumPlan([0, 1, 2, 3, 4, 5, 6], typed, (c) => known.get(c) ?? null, none, false, false);
    expect(p).toEqual({ hide: false, sum: [0, 4], text: [1, 2, 3, 5], pending: [6] });
  });
  it("a typed number column with no totals yet still gets Sum (nothing is read)", () => {
    let asked = 0;
    const p = autoSumPlan([0], [[0, "number"]], () => (asked++, null), none, false, false);
    expect(p.sum).toEqual([0]);
    expect(asked).toBe(0);
  });
});

describe("the second press", () => {
  const known = (c: number) => total(c, 5);
  it("hides only when every chosen column shows Sum already", () => {
    const sums = new Map<number, TotalChoice>([
      [0, "sum"],
      [1, "sum"],
    ]);
    expect(autoSumPlan([0, 1], undefined, known, sums, true, true).hide).toBe(true);
    // Average set by hand on one: Sum there instead of hiding.
    const avg = new Map<number, TotalChoice>([
      [0, "sum"],
      [1, "average"],
    ]);
    const p = autoSumPlan([0, 1], undefined, known, avg, true, true);
    expect(p.hide).toBe(false);
    expect(p.sum).toEqual([0, 1]);
  });
  it("with only text columns it hides when this key turned the row on", () => {
    const text = (c: number) => total(c, 0);
    expect(autoSumPlan([2], undefined, text, new Map(), true, true).hide).toBe(true);
    expect(autoSumPlan([2], undefined, text, new Map(), true, false).hide).toBe(false);
  });
  it("never hides a row that is off", () => {
    expect(autoSumPlan([0], undefined, known, new Map([[0, "sum"]]), false, true).hide).toBe(false);
  });
});

describe("pending columns and words", () => {
  it("settle by the totals' number count", () => {
    const m = new Map([
      [1, total(1, 2)],
      [2, total(2, 0)],
    ]);
    expect(settlePending([1, 2, 3], (c) => m.get(c) ?? null)).toEqual({ sum: [1], text: [2], still: [3] });
  });
  it("say what happened", () => {
    expect(nameList(["a", "b", "c", "d"])).toBe("a, b and 2 more");
    expect(autoSumWords(["amount", "zip"], ["city"], 0)).toBe("Totals row on: Sum of amount and zip. city holds no numbers, so it shows a count. Alt+= again hides it.");
    expect(autoSumWords([], [], 2)).toBe("Working out 2 columns… Alt+= again hides it.");
  });
});
