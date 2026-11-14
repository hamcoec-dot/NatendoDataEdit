import { describe, expect, it } from "vitest";
import { CASE_CHOICES, KIND_CHOICES, SPACES_CHOICES, TOOLS, describe as describeStep, runCleaner, stepProblem } from "./clean";
import { dateLike, isoOf } from "./editing";
import { defaultFormat, formatSteps } from "./m9";

const run = (steps: ReturnType<typeof formatSteps>, t: string) => runCleaner({ version: 1, name: "", steps }, t, 0);

describe("ASAP clean-up check (M12)", () => {
  it("offers the new tools and choices", () => {
    for (const id of ["words", "random_number"]) expect(TOOLS.some((t) => t.id === id)).toBe(true);
    expect(Object.keys(CASE_CHOICES)).toEqual(expect.arrayContaining(["first", "title_small", "names"]));
    expect(KIND_CHOICES.invisible).toMatch(/zero-width/);
    expect(SPACES_CHOICES.plain).toMatch(/no-break/);
    expect(describeStep({ tool: "dates", format: "yyyy-mm-dd", from: "yyyymmdd" })).toBe("Dates from yyyymmdd as yyyy-mm-dd");
    expect(stepProblem({ tool: "dates", format: "yyyy", from: "--" })).toMatch(/pattern/);
    expect(stepProblem({ tool: "random_number", min: 5, max: 1, unique: false })).toMatch(/above/);
    expect(stepProblem({ tool: "random_number", min: 1, max: 5, unique: true })).toBe("");
  });

  it("writes display formats into the cells", () => {
    const f = defaultFormat("percent");
    expect(run(formatSteps({ ...f, decimals: 1 }), "0.125")).toBe("12.5%");
    const c = { ...defaultFormat("currency"), symbol: "$", decimals: 2, thousands: true };
    expect(run(formatSteps(c), "-5")).toBe("-$5.00");
    expect(formatSteps({ ...defaultFormat("date"), date: "mdy" })).toEqual([{ tool: "dates", format: "mm/dd/yyyy" }]);
    expect(formatSteps(defaultFormat("checkbox"))).toEqual([]);
  });

  it("picks dates in the cell's own style", () => {
    expect(isoOf("03/14/2026")).toBe("2026-03-14");
    expect(isoOf("14.03.2026")).toBe("2026-03-14");
    expect(isoOf("2026/3/4")).toBe("2026-03-04");
    expect(isoOf("02/30/2026")).toBeNull();
    expect(isoOf("soon")).toBeNull();
    expect(isoOf("", new Date(2026, 10, 13))).toBe("2026-11-13");
    expect(dateLike("03/14/2026", "2026-11-05")).toBe("11/05/2026");
    expect(dateLike("3/4/2026", "2026-11-05")).toBe("11/5/2026");
    expect(dateLike("14.03.2026", "2026-11-05")).toBe("05.11.2026");
    expect(dateLike("2026/03/14", "2026-11-05")).toBe("2026/11/05");
    expect(dateLike("", "2026-11-05")).toBe("2026-11-05");
  });
});
