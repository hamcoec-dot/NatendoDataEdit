import { describe, expect, it } from "vitest";
import { barSpan, BUILTIN_LISTS, completion, dividerAfter, parseList, scaleColour, scalePos, toolsRequest, unchangedTabs, valueType, type ToolChoices } from "./m10";
import { describe as describeStep, runStep, stepProblem, TOOLS } from "./clean";

const base: ToolChoices = {
  cols: [1],
  col: 0,
  header: true,
  from: "above",
  text: "",
  spaces: true,
  matchCase: false,
  trim: true,
  byCol: true,
  every: 10,
  blankRows: 1,
  perRow: 3,
  down: false,
  skipBlanks: true,
  sizeKind: "rows",
  size: 5,
  seed: 7,
  other: 2,
  wholeRow: false,
  showSame: false,
  byTitle: true,
  source: true,
};

describe("tools requests", () => {
  it("builds each tool's request", () => {
    expect(toolsRequest("fill_blanks", base)).toEqual({ data: { op: "fill_blanks", spec: { cols: [1], from: { kind: "above" }, spaces: true, keep_first: true } } });
    expect(toolsRequest("fill_blanks", { ...base, from: "value", text: "" })).toBe("Type the text to put in blank cells.");
    expect(toolsRequest("fill_blanks", { ...base, from: "value", text: "n/a" })).toMatchObject({ data: { spec: { from: { kind: "value", text: "n/a" } } } });
    expect(toolsRequest("count_repeats", { ...base, cols: [] })).toBe("Tick at least one column.");
    expect(toolsRequest("breaks", { ...base, byCol: false, every: 0 })).toBe("Choose how many rows go between breaks.");
    expect(toolsRequest("breaks", base)).toMatchObject({ data: { op: "breaks", spec: { col: 0, every: 0, rows: 1 } } });
    expect(toolsRequest("reshape", { ...base, perRow: 0 })).toMatch(/between 1/);
    expect(toolsRequest("sample", { ...base, sizeKind: "percent", size: 120 })).toMatch(/percent/);
    expect(toolsRequest("sample", { ...base, sizeKind: "percent", size: 12.5 })).toMatchObject({ table: { spec: { size: { kind: "percent", p: 12.5 } } } });
    expect(toolsRequest("sample", { ...base, sizeKind: "every", size: 0 })).toMatch(/every/i);
    expect(toolsRequest("compare", { ...base, other: -1 })).toMatch(/Open the other/);
    expect(toolsRequest("compare", { ...base, wholeRow: true, cols: [] })).toMatchObject({ table: { op: "compare", other: 2, spec: { keys: [] } } });
    expect(toolsRequest("split", { ...base, byCol: false, every: 1000 })).toEqual({ split: { col: null, every: 1000, keep_first: true, guard: false } });
    expect(toolsRequest("combine", { ...base, header: false })).toEqual({ combine: { header: false, by_title: false, source: true } });
  });
});

describe("grid helpers", () => {
  it("tells value types apart", () => {
    expect(valueType(" ")).toBe("blank");
    expect(valueType("1,234.5")).toBe("number");
    expect(valueType("2024-02-29")).toBe("date");
    expect(valueType("May")).toBe("text");
    expect(valueType("abc")).toBe("text");
  });
  it("places colours and bars", () => {
    expect(scalePos(5, 0, 10)).toBe(0.5);
    expect(scalePos(-1, 0, 10)).toBe(0);
    expect(scalePos(3, 3, 3)).toBe(0.5);
    expect(scaleColour("scale3", 0)).toBe("rgba(248, 105, 107, 0.85)");
    expect(scaleColour("scale3", 1)).toBe("rgba(99, 190, 123, 0.85)");
    expect(scaleColour("scale2", 0)).toBe("rgba(255, 255, 255, 0.85)");
    expect(barSpan(5, 0, 10)).toEqual({ from: 0, to: 0.5 });
    expect(barSpan(-5, -10, 10)).toEqual({ from: 0.25, to: 0.5 });
    expect(barSpan(1, 1, 1)).toEqual({ from: 0, to: 1 });
  });
  it("finds divider rows", () => {
    expect(dividerAfter(["a", "a", "b", "c", "c"])).toEqual([1, 2]);
    expect(dividerAfter([])).toEqual([]);
  });
  it("completes typing", () => {
    expect(completion("ro", "Rome")).toEqual({ text: "rome", from: 2 });
    expect(completion("Rome", "Rome")).toBeNull();
    expect(completion("x", "Rome")).toBeNull();
    expect(completion("", "Rome")).toBeNull();
  });
  it("reads custom lists", () => {
    expect(parseList("Low\nmedium, High\n\nlow")).toEqual(["Low", "medium", "High"]);
    expect(BUILTIN_LISTS[0]!.items).toEqual(["Low", "Medium", "High"]);
  });
  it("finds unchanged tabs", () => {
    expect(unchangedTabs([{ info: { modified: true } }, { info: { modified: false } }]).length).toBe(1);
  });
});

describe("new cleaning steps", () => {
  it("are listed with problems and stand-ins", () => {
    for (const id of ["accents", "fix_number", "calc", "sort_parts", "url_decode", "url_encode"]) expect(TOOLS.some((t) => t.id === id)).toBe(true);
    expect(runStep({ tool: "accents" }, "Crème", 0)).toBe("Creme");
    expect(runStep({ tool: "fix_number", how: "trailing_minus" }, "50-", 0)).toBe("-50");
    expect(runStep({ tool: "calc", op: "multiply", value: 2 }, "2.5", 0)).toBe("5");
    expect(runStep({ tool: "sort_parts", separator: ", ", descending: false, distinct: true, trim: true }, "b, a, b", 0)).toBe("a, b");
    expect(runStep({ tool: "url_decode" }, "a%20b", 0)).toBe("a b");
    expect(runStep({ tool: "url_encode" }, "a b", 0)).toBe("a%20b");
    expect(stepProblem({ tool: "calc", op: "divide", value: 0 })).toMatch(/divide/);
    expect(stepProblem({ tool: "calc", op: "round", value: 1.5 })).toMatch(/decimals/);
    expect(stepProblem({ tool: "sort_parts", separator: "", descending: false, distinct: false, trim: false })).toMatch(/separator/);
    expect(describeStep({ tool: "fix_number", how: "percent" })).toBe("Percent to number");
  });
});

import { conditionOf, describeRule, emptyForm, formOf } from "./filters";

describe("largest and smallest filter", () => {
  it("builds, reads back and describes the rule", () => {
    const f = { ...emptyForm(), kind: "top" as const, op: "bottom_percent", a: "10" };
    const c = conditionOf(f);
    expect(c).toEqual({ type: "top", count: 10, percent: true, bottom: true });
    expect(conditionOf({ ...f, a: "120" })).toMatch(/percent/);
    expect(conditionOf({ ...f, op: "top", a: "2.5" })).toMatch(/whole/);
    if (typeof c !== "string") {
      expect(formOf(c)).toMatchObject({ kind: "top", op: "bottom_percent", a: "10" });
      expect(describeRule({ type: "conditions", first: c })).toBe("smallest 10 %");
    }
  });
});
