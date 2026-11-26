import { describe, expect, it } from "vitest";
import { buildMatrix, emptyPick, stepsFor, TICKS } from "./matrix";

const read = { decimal_comma: false, date_order: "mdy" as const };
const saved = { file: "names.ndclean", cleaner: { version: 1, name: "", steps: [{ tool: "accents" as const }] } };

describe("clean matrix ticks", () => {
  it("run in the fixed order however they were ticked", () => {
    const steps = stepsFor({ ticks: ["digits", "collapse", "trim", "breaks", "invisible", "sanitize"], case: "upper", saved });
    expect(steps.map((s) => (s.tool === "spaces" ? s.how : s.tool))).toEqual(["sanitize", "remove", "line_breaks", "trim", "collapse", "accents", "digits", "case"]);
    expect(TICKS.map((t) => t.id)).toEqual(["sanitize", "invisible", "breaks", "trim", "collapse", "digits"]);
  });

  it("puts a saved cleaner in its place without digits ticked", () => {
    expect(stepsFor({ ticks: [], case: "", saved })).toEqual([{ tool: "accents" }]);
  });

  it("leaves out columns with nothing to do and records titles and the file", () => {
    const picks = { 2: { ...emptyPick(), saved }, 0: { ...emptyPick(), ticks: ["trim" as const] }, 1: emptyPick() };
    const m = buildMatrix([2, 1, 0], picks, ["Name", "Town", "Zip"], true, read);
    expect(m.entries.map((e) => [e.col, e.title, e.from_file])).toEqual([
      [0, "Name", undefined],
      [2, "Zip", "names.ndclean"],
    ]);
    expect(buildMatrix([0], picks, ["Name"], false, read).entries[0]!.title).toBe("");
  });
});
