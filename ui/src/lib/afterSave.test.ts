import { describe, expect, it } from "vitest";
import { Tab } from "./tab.svelte";
import type { DocInfo, HistoryKept } from "./backend";

const info = (pos: number, can_undo = pos > 0, can_redo = false): DocInfo =>
  ({ id: 1, rows: 10, cols: 3, total_rows: 10, history_pos: pos, has_filter: false, filtered: false, can_undo, can_redo }) as unknown as DocInfo;
const kept = (p: Partial<HistoryKept>): HistoryKept => ({ undo: 0, redo: 0, undo_cut_columns: 0, redo_cut_columns: 0, undo_cut_memory: 0, redo_cut_memory: 0, cleared: false, ...p });

/** A tab with engine steps at positions 1..n. */
function tabWithSteps(n: number): Tab {
  const t = new Tab(info(0), 100);
  t.recordView();
  for (let p = 1; p <= n; p++) {
    t.update(info(p), true);
    t.recordView();
  }
  return t;
}

describe("undo after save: the looks kept with engine steps", () => {
  it("keeps the looks of the steps the save kept and drops the ones it let go", () => {
    const t = tabWithSteps(4);
    expect([...t.stepWhat.keys()]).toEqual([1, 2, 3, 4]);
    // The save kept the 2 newest undo steps (positions 3 and 4).
    t.afterSave(kept({ undo: 2, undo_cut_columns: 2 }));
    expect([...t.stepWhat.keys()]).toEqual([3, 4]);
    expect([...t.stepViews.keys()].every((k) => k >= 3)).toBe(true);
  });

  it("drops the redo looks past a column step the save let go (B3)", () => {
    const t = tabWithSteps(3);
    // Undo to position 2: the step at 3 waits on the redo side, with a look of its own.
    t.redoing = 0;
    t.update(info(2, true, true), true);
    t.lookRedo.push({ pos: 3, before: t.snapView().look, after: t.snapView().look });
    expect(t.canRedo).toBe(true);
    // The save let that redo step go (it changed columns): no redo look is left, and the engine has none.
    t.afterSave(kept({ undo: 2, redo: 0, redo_cut_columns: 1 }));
    t.update(info(2, true, false), false);
    expect(t.lookRedo).toEqual([]);
    expect([...t.stepWhat.keys()]).toEqual([1, 2]);
    expect(t.canRedo).toBe(false);
  });

  it("drops every engine look when nothing was kept", () => {
    const t = tabWithSteps(2);
    t.afterSave(null);
    expect(t.stepWhat.size + t.stepViews.size).toBe(0);
    t.afterSave(kept({ cleared: true, undo_cut_columns: 2 }));
    expect(t.stepWhat.size).toBe(0);
  });
});
