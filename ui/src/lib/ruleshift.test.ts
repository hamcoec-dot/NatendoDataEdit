import { describe, expect, it } from "vitest";
import { Tab } from "./tab.svelte";
import type { Check, DocInfo } from "./backend";

const info = (pos: number, seq: number, cols: number): DocInfo =>
  ({ id: 1, rows: 10, cols, total_rows: 10, history_pos: pos, has_filter: false, filtered: false, seq }) as unknown as DocInfo;

const key: Check = { col: 2, kind: { type: "unique", match_case: false, also: [4] } };
const req: Check = { col: 0, kind: { type: "required" } };

/** A tab at undo position 0 with the two rules. */
function tab(): Tab {
  const t = new Tab(info(0, 1, 6), 100);
  t.checks = [key, req];
  return t;
}

describe("data check rules follow columns put in and taken out, through Undo and Redo", () => {
  it("a split column left of a rule moves it (and its key column)", () => {
    const t = tab();
    t.update(info(1, 2, 8), true); // the split made 2 more columns after column 1
    t.shiftCols(2, 2);
    expect(t.checks).toEqual([{ col: 4, kind: { type: "unique", match_case: false, also: [6] } }, req]);
  });

  it("insert, Undo, Redo", () => {
    const t = tab();
    t.update(info(1, 2, 7), true);
    t.shiftCols(0, 1);
    expect(t.checks.map((c) => c.col)).toEqual([3, 1]);
    t.update(info(0, 3, 6), true); // Undo
    expect(t.checks).toEqual([key, req]);
    t.redoing++;
    t.update(info(1, 4, 7), true); // Redo
    t.redoing--;
    expect(t.checks.map((c) => c.col)).toEqual([3, 1]);
    expect(t.checks[0]!.kind).toEqual({ type: "unique", match_case: false, also: [5] });
  });

  it("deleting a key column drops the rule, and Undo brings it back", () => {
    const t = tab();
    t.update(info(1, 2, 5), true);
    expect(t.shiftCols(4, -1)).toBe(1);
    expect(t.checks).toEqual([req]);
    t.update(info(0, 3, 6), true); // Undo
    expect(t.checks).toEqual([key, req]);
    t.redoing++;
    t.update(info(1, 4, 5), true); // Redo
    t.redoing--;
    expect(t.checks).toEqual([req]);
  });

  it("a new change after Undo forgets the shift that could have been redone", () => {
    const t = tab();
    t.update(info(1, 2, 7), true);
    t.shiftCols(0, 1);
    t.update(info(0, 3, 6), true); // Undo
    t.update(info(1, 4, 6), true); // another change, not a redo
    expect(t.checks).toEqual([key, req]);
    t.update(info(0, 5, 6), true); // Undo of that change: the rules stay
    expect(t.checks).toEqual([key, req]);
  });
});
