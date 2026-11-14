import { describe as group, expect, it } from "vitest";
import { TOOLS, describe, localNow, runCleaner, stepProblem, toolOf, type Step } from "./clean";

group("cleaning tools", () => {
  it("has a working default for every tool", () => {
    const ids = new Set<string>();
    for (const t of TOOLS) {
      const s = t.make();
      expect(s.tool).toBe(t.id);
      expect(ids.has(t.id)).toBe(false);
      ids.add(t.id);
      expect(describe(s).length).toBeGreaterThan(3);
      expect(toolOf(t.id)).toBe(t);
    }
  });

  it("names problems before asking the engine", () => {
    expect(stepProblem({ tool: "replace", find: { text: "", kind: "plain", match_case: false, whole_cell: false }, with: "x" })).toMatch(/Type the text/);
    expect(stepProblem({ tool: "pad", width: 5, with: "ab", right: false })).toMatch(/one character/);
    expect(stepProblem({ tool: "pad", width: 20_000, with: "0", right: false })).toMatch(/between 1 and/);
    expect(stepProblem({ tool: "dates", format: "--" })).toMatch(/format needs/);
    expect(stepProblem({ tool: "number", start: Infinity, step: 1, digits: 0, at_end: false, separator: "", replace: true })).toMatch(/10\^15/);
    expect(stepProblem({ tool: "case", to: "upper" })).toBe("");
  });

  it("gives the local time as days since 1970", () => {
    const d = new Date(2026, 10, 12, 14, 5, 0);
    const days = localNow(d);
    const back = new Date(Math.round(days * 86_400_000));
    expect([back.getUTCFullYear(), back.getUTCMonth(), back.getUTCDate(), back.getUTCHours(), back.getUTCMinutes()]).toEqual([2026, 10, 12, 14, 5]);
  });

  it("runs steps in order (browser stand-in)", () => {
    const steps: Step[] = [
      { tool: "spaces", how: "collapse" },
      { tool: "case", to: "title" },
      { tool: "name", part: "last_first" },
    ];
    expect(runCleaner({ version: 1, name: "", steps }, "  mary  ann lee ", 0)).toBe("Lee, Mary Ann");
    expect(runCleaner({ version: 1, name: "", steps: [{ tool: "number", start: 1, step: 1, digits: 3, at_end: false, separator: "-", replace: false }] }, "x", 4)).toBe("005-x");
  });
});
