import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { addAsColumn, compactPlan, landed, leftOut, mergePlan, sameTitles, setSource, sourceOf } from "./combineMap";
import type { FileHead } from "./backend";

describe("combine with a column map", () => {
  it("a choice by hand changes the preview; merge and reset", async () => {
    const m = new MockBackend();
    const start = (await m.combinePick())!;
    const p = start.plan;
    expect(landed(p, start.files, 1)[0]).toEqual(["", "Cy Made-up", "", "103", "P3", "new"]);
    // Merge the suggestion: CustID moves onto Cust_ID.
    const s = start.suggestions.find((x) => p.titles[x.keep] === "Cust_ID" && p.titles[x.merge] === "CustID")!;
    expect(s.keep_samples.length).toBeGreaterThan(0);
    const merged = mergePlan(p, s.keep, s.merge)!;
    expect(merged.titles).toEqual(["Cust_ID", "Name", "Precinct 1", "Precinct 2", "Notes"]);
    expect(landed(merged, start.files, 1)[0]).toEqual(["103", "Cy Made-up", "", "P3", "new"]);
    // Precinct 1 and 2 are never suggested.
    expect(start.suggestions.every((x) => !p.titles[x.keep]!.startsWith("Precinct"))).toBe(true);
    // A dropdown: February's Precinct 2 goes to Precinct 1.
    const chosen = setSource(merged, 1, 2, 2);
    expect(sourceOf(chosen, 1, 2)).toBe(2);
    const checked = await m.combineCheck(1, chosen);
    expect(checked.plan.titles).toEqual(["Cust_ID", "Name", "Precinct 1", "Notes"]);
    // A left-out column can come back as its own.
    const none = setSource(checked.plan, 1, 3, null);
    expect(leftOut(none, 1)).toEqual([3]);
    expect(compactPlan(addAsColumn(none, 1, 3, "Notes")).titles.at(-1)).toBe("Notes");
  });
  it("two columns of one file never land together", async () => {
    const m = new MockBackend();
    const start = (await m.combinePick())!;
    expect(mergePlan(start.plan, 1, 1)).toBeNull();
    const both = { titles: ["A", "B"], files: [[0, 1]] };
    expect(mergePlan(both, 0, 1)).toBeNull();
    await expect(m.combineCheck(1, { titles: ["A"], files: [[0, 0]] })).rejects.toThrow(/overwrite/);
  });
  it("save then load gives the same map; combine makes a tab", async () => {
    const m = new MockBackend();
    const start = (await m.combinePick())!;
    const merged = mergePlan(start.plan, 0, 3)!;
    expect(await m.combineSaveMapping(1, merged, "add")).toBe(true);
    const fit = await m.combineLoadMapping();
    expect(fit?.plan).toEqual(merged);
    const d = await m.combineRun(1, merged, true);
    expect(d.rows).toBe(4);
    expect(d.cols).toBe(merged.titles.length + 1);
  });

  it("keeps a hand-made map after a file changed only when every file's titles are the same", () => {
    const head = (name: string, titles: string[]): FileHead => ({ name, titles, cols: titles.length, rows_est: 2, whole: true, preview: [] });
    const before = [head("jan.csv", ["id", "City"]), head("feb.csv", ["ID", "Town"])];
    expect(sameTitles(before, [head("jan.csv", ["id", "City"]), head("feb.csv", ["ID", "Town"])])).toBe(true);
    expect(sameTitles(before, [head("jan.csv", ["id", "City"]), head("feb.csv", ["ID", "Town", "Note"])])).toBe(false);
    expect(sameTitles(before, [head("jan.csv", ["id", "City"]), head("feb.csv", ["Town", "ID"])])).toBe(false);
    expect(sameTitles(before, [head("jan.csv", ["id", "City"])])).toBe(false);
  });
});
