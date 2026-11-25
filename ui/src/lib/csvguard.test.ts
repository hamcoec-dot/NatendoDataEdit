import { describe, expect, it } from "vitest";
import { guardNote } from "./csvguard";
import { toolsRequest } from "./m10";
import { parsePrefs } from "./prefs.svelte";

const read = (p: object) => parsePrefs(JSON.stringify(p));

describe("CSV formula guard", () => {
  it("says nothing when no cell changed, and counts the ones that did", () => {
    expect(guardNote()).toBe("");
    expect(guardNote(0)).toBe("");
    expect(guardNote(1)).toContain("1 cell that");
    expect(guardNote(3)).toContain("3 cells that");
  });

  it("is on for reports and safe copies and off for split files by default", () => {
    expect(read({}).csvFormulaGuard).toBe(true);
    expect(read({}).csvFormulaGuardSplit).toBe(false);
    expect(read({ csvFormulaGuard: false, csvFormulaGuardSplit: true })).toMatchObject({ csvFormulaGuard: false, csvFormulaGuardSplit: true });
    expect(read({ csvFormulaGuard: "no", csvFormulaGuardSplit: "yes" })).toMatchObject({ csvFormulaGuard: true, csvFormulaGuardSplit: false });
  });

  it("sends the split tick, off unless ticked", () => {
    const base = { cols: [], col: 0, header: true, byCol: false, every: 10 } as unknown as Parameters<typeof toolsRequest>[1];
    expect(toolsRequest("split", base)).toMatchObject({ split: { guard: false } });
    expect(toolsRequest("split", { ...base, guard: true })).toMatchObject({ split: { guard: true } });
  });
});
