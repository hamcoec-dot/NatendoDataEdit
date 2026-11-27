import { describe, expect, it, vi } from "vitest";
import { Tab, exportHandTab, importHandTab } from "./tab.svelte";
import { backend, type DocInfo } from "./backend";
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

describe("Undo or Redo of a row change whose kept look was let go", () => {
  it("clears the heights with a note when the history moved and nothing was restored (an insert)", () => {
    const t = tabWith([5]);
    t.pendingRows = { kind: "insert", at: 2, count: 3 };
    t.update(info({ rows: 1003, total_rows: 1003, history_pos: 1 }), true);
    const posBefore = t.info.history_pos;
    t.update(info({ rows: 1000, total_rows: 1000, history_pos: 0 }), true); // Undo
    expect(t.clearHeightsIfLookLost(posBefore, false)).toBe(true);
    expect(t.rowHeights.empty).toBe(true);
    expect(t.rowHeightsNote).toMatch(/Undo or Redo/);
  });

  it("does the same for the Undo of a row MOVE (the row count does not change)", () => {
    const t = tabWith([5]);
    t.pendingRows = { kind: "move", at: 5, count: 1, to: 20 };
    t.update(info({ history_pos: 1 }), true);
    expect(t.rowHeights.toView()).toEqual([[20, 60]]);
    const posBefore = t.info.history_pos;
    t.update(info({ history_pos: 0 }), true); // Undo of the move
    expect(t.clearHeightsIfLookLost(posBefore, false)).toBe(true);
    expect(t.rowHeights.empty).toBe(true);
  });

  it("leaves them when the look was restored, or the history did not move", () => {
    const t = tabWith([5]);
    t.update(info({ history_pos: 3 }), true);
    expect(t.clearHeightsIfLookLost(4, true)).toBe(false);
    expect(t.clearHeightsIfLookLost(3, false)).toBe(false);
    expect(t.rowHeights.toView()).toEqual([[5, 60]]);
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

describe("heights while rows are hidden (filter on)", () => {
  // Timers and promises settle; the ask waits SHOWN_SETTLE_MS (0 here) first.
  const flush = async () => {
    for (let i = 0; i < 3; i++) await new Promise((r) => setTimeout(r, 0));
  };
  Tab.SHOWN_SETTLE_MS = 0;

  it("draws plain until the engine says where the rows are, then by shown row", async () => {
    const spy = vi.spyOn(backend(), "shownRowsAt").mockImplementation(async (_d, places) => places.map((p) => (p === 5 ? null : p / 2)));
    const t = tabWith([4, 5, 8]);
    t.update(info({ filtered: true, rows: 500, has_filter: true }), false);
    expect(t.heightsInForce()).toBeNull();
    await flush();
    expect(t.heightsInForce()?.toView()).toEqual([
      [2, 60],
      [4, 60],
    ]);
    // Filter off: by document row again, no call.
    t.update(info(), false);
    expect(t.heightsInForce()?.toView()).toEqual([
      [4, 60],
      [5, 60],
      [8, 60],
    ]);
    spy.mockRestore();
  });

  it("asks one at a time: typing a 10-character filter sends at most 3 calls", async () => {
    let calls = 0;
    let release: (() => void) | null = null;
    const spy = vi.spyOn(backend(), "shownRowsAt").mockImplementation((_d, places) => {
      calls++;
      return new Promise((ok) => (release = () => ok(places.map(() => null))));
    });
    const t = tabWith([1, 2, 3]);
    for (let k = 0; k < 10; k++) {
      t.update(info({ filtered: true, rows: 900 - k, has_filter: true }), false);
      t.heightsInForce(); // a redraw per key
    }
    // Nothing is asked while the rows keep changing; once they settle, one call.
    expect(calls).toBe(0);
    await flush();
    expect(calls).toBe(1);
    release!();
    await flush();
    // The answer is for the current rows: used, and no more calls.
    expect(t.heightsInForce()).not.toBeNull();
    await flush();
    expect(calls).toBe(1);
    // Keys typed while a call is out: one more call once it is back and the rows settle.
    t.update(info({ filtered: true, rows: 10, has_filter: true }), false);
    t.heightsInForce();
    await flush();
    t.update(info({ filtered: true, rows: 9, has_filter: true }), false);
    t.heightsInForce();
    release!();
    await flush();
    t.heightsInForce();
    await flush();
    expect(calls).toBeLessThanOrEqual(3);
    spy.mockRestore();
  });

  it("an answer for older rows is never used", async () => {
    let release: (() => void) | null = null;
    const spy = vi.spyOn(backend(), "shownRowsAt").mockImplementation((_d, places) => new Promise((ok) => (release = () => ok(places.map((p) => p)))));
    const t = tabWith([3]);
    t.update(info({ filtered: true, rows: 800, has_filter: true }), false);
    t.heightsInForce();
    await flush();
    t.update(info({ filtered: true, rows: 700, has_filter: true }), false);
    release!();
    await flush();
    expect(t.heightsInForce()).toBeNull();
    spy.mockRestore();
  });

  it("setting a shown row's height while filtered updates both, without a call", async () => {
    const spy = vi.spyOn(backend(), "shownRowsAt").mockImplementation(async (_d, places) => places.map((p) => p - 10));
    const t = tabWith([20]);
    t.update(info({ filtered: true, rows: 500, has_filter: true }), false);
    t.heightsInForce();
    await flush();
    const before = spy.mock.calls.length;
    expect(t.applyHeights([3], [13], 80)).toBe(true);
    expect(t.rowHeights.toView()).toEqual([
      [13, 80],
      [20, 60],
    ]);
    expect(t.heightsInForce()?.toView()).toEqual([
      [3, 80],
      [10, 60],
    ]);
    expect(t.applyHeights([10], [20], null)).toBe(true);
    expect(t.heightsInForce()?.toView()).toEqual([[3, 80]]);
    expect(spy.mock.calls.length).toBe(before);
    spy.mockRestore();
  });

  it("an insert or delete while filtered follows the positions the engine reports", () => {
    const t = tabWith([4, 9, 15]);
    t.update(info({ filtered: true, rows: 500, has_filter: true }), false);
    t.pendingRows = { kind: "report" };
    t.update(info({ filtered: true, rows: 502, total_rows: 1002, history_pos: 1, has_filter: true, rows_change: { kind: "insert", at: 9, count: 2 } }), true);
    expect(t.rowHeights.toView()).toEqual([
      [4, 60],
      [11, 60],
      [17, 60],
    ]);
    t.pendingRows = { kind: "report" };
    t.update(
      info({ filtered: true, rows: 499, total_rows: 999, history_pos: 2, has_filter: true, rows_change: { kind: "delete", runs: [[2, 1], [11, 1], [13, 1]] } }),
      true,
    );
    expect(t.rowHeights.toView()).toEqual([
      [3, 60],
      [14, 60],
    ]);
    expect(t.rowHeightsNote).toBe("");
    // No report (past the cap): cleared with the note.
    t.pendingRows = { kind: "report" };
    t.update(info({ filtered: true, rows: 300, total_rows: 800, history_pos: 3, has_filter: true }), true);
    expect(t.rowHeights.empty).toBe(true);
    expect(t.rowHeightsNote).toMatch(/cleared/);
  });

  it("sorting while filtered leaves each height on its document row", () => {
    const t = tabWith([4, 9]);
    t.update(info({ filtered: true, rows: 500, has_filter: true }), false);
    t.update(info({ filtered: true, rows: 500, has_filter: true, history_pos: 1 }), true);
    expect(t.rowHeights.toView()).toEqual([
      [4, 60],
      [9, 60],
    ]);
  });
});
