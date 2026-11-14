import { describe, expect, it } from "vitest";
import { needsPhrase, withPhrase, type Recipe } from "./recipe";

const base: Recipe = { version: 1, name: "", columns: [], header: true, steps: [], left_out: [] };

describe("recipe phrase", () => {
  it("finds scramble steps and fills a copy", () => {
    const r: Recipe = {
      ...base,
      steps: [
        { op: "delete_cols", at: 0, count: 1 },
        { op: "clean", spec: { cleaner: { version: 1, name: "", steps: [{ tool: "trim" }, { tool: "scramble", loose: true }] } } },
      ],
    };
    expect(needsPhrase(r)).toBe(true);
    expect(needsPhrase({ ...base, steps: [{ op: "sort" }] })).toBe(false);
    const filled = withPhrase(r, "a long secret phrase here");
    const steps = (filled.steps[1]!.spec as { cleaner: { steps: { phrase?: string }[] } }).cleaner.steps;
    expect(steps[1]!.phrase).toBe("a long secret phrase here");
    expect(steps[0]!.phrase).toBeUndefined();
    // The original is left without it.
    expect(JSON.stringify(r)).not.toContain("secret");
  });
});
