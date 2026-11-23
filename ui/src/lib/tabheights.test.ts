import { describe, expect, it } from "vitest";
import { Tab, exportHandTab, importHandTab } from "./tab.svelte";
import type { DocInfo } from "./backend";
import { MAX_CUSTOM_ROWS, RowHeights } from "./rowheights";

const info = (over: Partial<DocInfo> = {}): DocInfo => ({ id: 1, rows: 1000, cols: 6, total_rows: 1000, history_pos: 0, has_filter: false, filtered: false, hidden_rows: 0, can_undo: false, can_redo: false, ...over }) as unknown as DocInfo;

function tabWith(rows: number[], px = 60): Tab {
  const t = new Tab(info(), 100);
  t.rowHeights.set(rows, px);
  return t;
}

describe("a tab's custom row heights follow the rows", () => {
  it("an insert above a tall row moves its height down with it", () => {
    const t = tabWith([5]);
    t.pendingRows = { kind: "insert", at: 2, count: 3 };
    t.update(info({ rows: 1003, total_rows: 1003, history_pos: 1 }), true);
    expect(t.rowHeights.toView()).toEqual([[8, 60]]);
    expect(t.rowHeightsNote).toBe("");
  });

  it("deleting the tall row removes its height; deleting rows above moves it up", () => {
    const a = tabWith([5, 9]);
    a.pendingRows = { kind: "delete", at: 5, count: 1 };
    a.update(info({ rows: 999, total_rows: 999, history_pos: 1 }), true);
    expect(a.rowHeights.toView()).toEqual([[8, 60]]);
  });

  it("moving rows carries their heights", () => {
    const t = tabWith([5]);
    t.pendingRows = { kind: "move", at: 5, count: 2, to: 20 };
    t.update(info({ history_pos: 1 }), true);
    expect(t.rowHeights.toView()).toEqual([[20, 60]]);
  });

  it("a row-count change nothing told the tab about clears the heights with a note", () => {
    const t = tabWith([5]);
    t.update(info({ rows: 900, total_rows: 900, history_pos: 1 }), true);
    expect(t.rowHeights.empty).toBe(true);
    expect(t.rowHeightsNote).toMatch(/custom row heights were cleared/i);
  });

  it("while rows are hidden a row change clears them (the grid cannot place the rows)", () => {
    const t = tabWith([5]);
    t.pendingRows = { kind: "reset" };
    t.update(info({ history_pos: 1 }), true);
    expect(t.rowHeights.empty).toBe(true);
    expect(t.rowHeightsNote).not.toBe("");
  });

  it("changes that leave the rows alone (an edit, a sort) leave the heights alone", () => {
    const t = tabWith([5]);
    t.update(info({ history_pos: 1 }), true);
    t.update(info({ history_pos: 2 }), true);
    expect(t.rowHeights.toView()).toEqual([[5, 60]]);
    expect(t.rowHeightsNote).toBe("");
  });

  it("Undo does not touch them here (the look put back with the step does)", () => {
    const t = tabWith([5]);
    t.update(info({ history_pos: 3 }), true);
    t.update(info({ rows: 1010, total_rows: 1010, history_pos: 2 }), true);
    expect(t.rowHeights.toView()).toEqual([[5, 60]]);
    expect(t.rowHeightsNote).toBe("");
  });
});

describe("Undo and Redo of a row change put the height on the same data row", () => {
  it("insert, undo, redo through the look kept with each step", () => {
    const t = tabWith([5]);
    t.recordView();
    // Insert 3 rows at 2: the step's look before and after.
    t.pendingRows = { kind: "insert", at: 2, count: 3 };
    t.update(info({ rows: 1003, total_rows: 1003, history_pos: 1 }), true);
    const after = t.snapView();
    expect(t.rowHeights.toView()).toEqual([[8, 60]]);
    // Undo: the look from before the step comes back.
    t.restoreView({ look: { ...after.look, rows: new RowHeights().snapshot() }, sort: null }, true);
    expect(t.rowHeights.empty).toBe(true);
    t.restoreView(after, true);
    expect(t.rowHeights.toView()).toEqual([[8, 60]]);
  });
});

describe("the look kept for Undo shares an unchanged set of heights", () => {
  it("50 data edits with 100,000 heights keep one snapshot and cost the heights once", () => {
    const t = new Tab(info({ rows: 5_000_000, total_rows: 5_000_000 }), 100);
    t.rowHeights.set(
      Array.from({ length: MAX_CUSTOM_ROWS }, (_, i) => i * 2),
      40,
    );
    t.recordView();
    const first = t.snapView();
    for (let i = 1; i <= 50; i++) {
      t.update(info({ rows: 5_000_000, total_rows: 5_000_000, history_pos: i }), true);
      t.recordView();
    }
    const last = t.snapView();
    expect(last.look.rows).toBe(first.look.rows);
    expect(last.look.rows.rows).toBe(first.look.rows.rows);
    // Only the heights as a whole count once against the undo memory.
    const bytes = (t as unknown as { lookBytes: number }).lookBytes;
    expect(bytes).toBeLessThan(MAX_CUSTOM_ROWS * 16 * 2);
  });

  it("a changed set is a new snapshot, and the change is an undo step of its own", () => {
    const t = tabWith([3]);
    t.recordView();
    const was = t.snapView();
    t.rowHeights.set([4], 80);
    t.recordView();
    const now = t.snapView();
    expect(now.look.rows).not.toBe(was.look.rows);
    expect(t.lookSteps.at(-1)?.before.rows).toBe(was.look.rows);
  });
});

describe("heights move with a tab to another window", () => {
  it("through the hand-over JSON", () => {
    const a = tabWith([3, 9], 70);
    const sent = JSON.parse(JSON.stringify(exportHandTab(a)));
    const b = new Tab(info(), 100);
    importHandTab(b, sent);
    expect(b.rowHeights.toView()).toEqual([
      [3, 70],
      [9, 70],
    ]);
  });
});
