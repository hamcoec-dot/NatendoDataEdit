import { describe, expect, it } from "vitest";
import { Tab } from "./tab.svelte";
import type { DocInfo } from "./backend";

const info = (pos: number): DocInfo => ({ id: 1, rows: 10, cols: 5, total_rows: 10, history_pos: pos, has_filter: false, filtered: false }) as unknown as DocInfo;

describe("column moves follow undo and redo", () => {
  it("replays by undo position, not by which call is running", () => {
    const t = new Tab(info(0), 100);
    t.columns.setWidth(1, 150);
    t.scales = [{ col: 1, kind: "bars" } as never];
    t.highlights = [{ col: 1, rule: { op: "blank" } as never, color: 1, whole_row: false }];
    // Move column 1 to 3 (undo position 1), then column 0 to 4 (position 2).
    t.update(info(1), true);
    t.noteColMove(1, 1, 3);
    t.update(info(2), true);
    t.noteColMove(0, 1, 4);
    expect(t.columns.width(2)).toBe(150);
    expect(t.scales[0]!.col).toBe(2);
    expect(t.highlights[0]!.col).toBe(2);
    // Two undos land in one reply (a held Ctrl+Z): both moves go back.
    t.update(info(0), true);
    expect(t.columns.width(1)).toBe(150);
    expect(t.scales[0]!.col).toBe(1);
    // A redo replays the first move only.
    t.redoing++;
    t.update(info(1), true);
    t.redoing--;
    expect(t.columns.width(3)).toBe(150);
    expect(t.highlights[0]!.col).toBe(3);
    // A new change instead of the second redo forgets that move: undoing the
    // change moves nothing, undoing the first move takes it back.
    t.update(info(2), true);
    t.update(info(1), true);
    expect(t.columns.width(3)).toBe(150);
    t.update(info(0), true);
    expect(t.columns.width(1)).toBe(150);
    expect(t.columns.width(3)).toBe(100);
    t.redoing++;
    t.update(info(1), true);
    t.update(info(2), true);
    t.redoing--;
    expect(t.columns.width(3)).toBe(150);
    expect(t.columns.width(4)).toBe(100);
  });
});
