// Gap #7: a workbook's own row heights are set as its tab opens: not an undo
// step, not a change to the file, the title row's left out with titles on.
import { afterEach, describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { Tab } from "./tab.svelte";
import { applyWorkbookHeights } from "./workbookheights";
import type { Imported } from "./backend";

const opts = { format: "spreadsheet" as const, part: 0, raw_dates: false, record: "" };

async function opened(heights: [number, number][]): Promise<{ m: MockBackend; d: Imported }> {
  MockBackend.workbookHeights = heights;
  const m = new MockBackend();
  const s = await m.importText("name,town\nAnn,Oslo\nBob,Rome\nCy,Lima\n");
  return { m, d: await m.importFinish(s.id, { ...opts }) };
}

describe("a workbook's own row heights", () => {
  afterEach(() => {
    MockBackend.workbookHeights = [];
  });

  it("are set on the rows they belong to, and the status says how many", async () => {
    const { d } = await opened([[1, 30], [3, 200]]);
    const t = new Tab(d, 100);
    const note = applyWorkbookHeights(t, d, true, 24);
    expect(t.rowHeights.toView()).toEqual([[1, 30], [3, 200]]);
    expect(note).toBe(" Row heights from the workbook kept for 2 rows.");
  });

  it("are not an undo step and change nothing in the file", async () => {
    const { m, d } = await opened([[2, 60]]);
    const before = await m.docInfo(d.id);
    const t = new Tab(d, 100);
    applyWorkbookHeights(t, d, true, 24);
    // No look step to undo (Ctrl+Z right after opening does nothing to the heights).
    expect(t.lookSteps).toEqual([]);
    expect(t.rowHeights.toView()).toEqual([[2, 60]]);
    // The document is exactly as the import made it: same changed mark, same undo position.
    const after = await m.docInfo(d.id);
    expect([after.modified, after.history_pos, after.can_undo]).toEqual([before.modified, before.history_pos, before.can_undo]);
  });

  it("leave out the title row's height with titles on, and heights equal to the plain one", async () => {
    const { d } = await opened([[0, 50], [1, 24], [2, 40]]);
    const on = new Tab(d, 100);
    applyWorkbookHeights(on, d, true, 24);
    expect(on.rowHeights.toView()).toEqual([[2, 40]]);
    // Titles off: the first row is a row of data and keeps its height.
    const off = new Tab(d, 100);
    applyWorkbookHeights(off, d, false, 24);
    expect(off.rowHeights.toView()).toEqual([[0, 50], [2, 40]]);
  });

  it("say nothing when the workbook has none (and a text import never has any)", async () => {
    const { d } = await opened([]);
    const t = new Tab(d, 100);
    expect(applyWorkbookHeights(t, d, true, 24)).toBe("");
    expect(t.rowHeights.empty).toBe(true);
    MockBackend.workbookHeights = [[1, 30]];
    const m = new MockBackend();
    const s = await m.importText("a,b\n1,2\n");
    const text = await m.importFinish(s.id, { ...opts, format: "delimited" });
    expect(text.row_heights).toBeUndefined();
  });

  it("say how many were left out past the cap", async () => {
    const { d } = await opened([[1, 30]]);
    const note = applyWorkbookHeights(new Tab(d, 100), { ...d, row_heights_over: 5 }, true, 24);
    expect(note).toBe(" Row heights from the workbook kept for 1 row (the first 100,000 that have one; 5 more rows left at the plain height).");
  });
});

describe("a PDF's missing characters", () => {
  it("are counted, never quoted, and CJK named only when no font for it was usable", async () => {
    const { missingNote } = await import("./backend");
    expect(missingNote({ missing_chars: 0 })).toBe("");
    expect(missingNote({})).toBe("");
    expect(missingNote({ missing_chars: 1, missing_cjk: false })).toBe(" 1 character could not be printed: no installed font has it.");
    expect(missingNote({ missing_chars: 1234, missing_cjk: true })).toBe(" 1,234 characters could not be printed: no installed font has them (no font for Chinese, Japanese or Korean is installed).");
  });
});
