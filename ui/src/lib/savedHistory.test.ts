import { describe, expect, it } from "vitest";
import { savedHistoryNote } from "./savedHistory";
import type { HistoryKept } from "./backend";

const h = (p: Partial<HistoryKept>): HistoryKept => ({
  undo: 0,
  redo: 0,
  undo_cut_columns: 0,
  redo_cut_columns: 0,
  undo_cut_memory: 0,
  redo_cut_memory: 0,
  cleared: false,
  ...p,
});

describe("what a save says about the undo history", () => {
  it("says nothing when the whole history was kept", () => {
    expect(savedHistoryNote(null)).toBe("");
    expect(savedHistoryNote(h({ undo: 5, redo: 2 }))).toBe("");
  });
  it("says where undo stops after a column change, and what redo lost", () => {
    expect(savedHistoryNote(h({ undo: 2, undo_cut_columns: 3 }))).toBe("Undo goes back to the column change before it; earlier steps were let go.");
    expect(savedHistoryNote(h({ redo_cut_columns: 1, cleared: true }))).toBe("Redo past a column change was let go (1 step).");
  });
  it("says how many steps memory let go, once", () => {
    expect(savedHistoryNote(h({ undo: 1, undo_cut_memory: 120 }))).toBe("The 120 steps furthest back could not be kept (not enough memory for the undo history).");
  });
  it("says when nothing could be kept for another reason", () => {
    expect(savedHistoryNote(h({ cleared: true }))).toBe("The undo history could not be kept across the save and starts again here.");
  });
});
