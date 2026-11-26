import { describe, expect, it } from "vitest";
import { columnRef, defaultPage, describeCheck, expandTabs, guessStarts, newCheck, placeCheck, printHtml, shiftChecks, splitFixed, toggleStart, toJsPattern, LOG_PRESETS } from "./m8";
import { mockEval } from "./mockexpr";

describe("fixed width", () => {
  const lines = ["Name       City        Amount", "Ada        London         120", "Grace                    1500", "Ken        New York        33"];
  it("guesses column starts, blank cells included", () => {
    expect(guessStarts(lines)).toEqual([0, 11, 23]);
    expect(splitFixed(lines[2]!, [0, 11, 23])).toEqual(["Grace", "", "1500"]);
    expect(splitFixed(lines[3]!, [0, 11, 23])).toEqual(["Ken", "New York", "33"]);
    expect(guessStarts([])).toEqual([0]);
  });
  it("counts characters, not bytes, and expands tabs", () => {
    expect(splitFixed("日本語abc", [0, 3])).toEqual(["日本語", "abc"]);
    expect(expandTabs("a\tb")).toBe("a       b");
    expect(splitFixed(" a  b ", [3], true)).toEqual([" a ", " b "]);
  });
  it("adds and removes edges", () => {
    expect(toggleStart([0, 5], 3)).toEqual([0, 3, 5]);
    expect(toggleStart([0, 3, 5], 3)).toEqual([0, 5]);
    expect(toggleStart([0], 0)).toEqual([0]);
  });
});

describe("log presets", () => {
  it("compile in the browser's syntax too", () => {
    for (const p of LOG_PRESETS) expect(() => new RegExp(toJsPattern(p.pattern))).not.toThrow();
    const re = new RegExp(toJsPattern(LOG_PRESETS[2]!.pattern));
    expect(re.exec("2024-05-01 10:00:01 INFO started")?.groups?.level).toBe("INFO");
  });
});

describe("checks and expressions", () => {
  it("describes checks in words", () => {
    const t = ["Age", "Email"];
    expect(describeCheck({ col: 0, kind: { type: "number", min: 0, max: 120, whole: true } }, t)).toBe("Age: whole number from 0 to 120");
    expect(describeCheck(newCheck("required", 1), t)).toBe("Email: must not be blank");
    expect(describeCheck({ col: null, kind: { type: "expression", expr: "[a] < [b]" } }, t)).toBe("row rule: [a] < [b]");
    expect(newCheck("expression", 3).col).toBeNull();
  });
  it("writes column references the engine reads", () => {
    expect(columnRef("Price", 0)).toBe("[Price]");
    expect(columnRef("", 2)).toBe("[#3]");
    expect(columnRef("a]b", 1)).toBe("[#2]");
    expect(columnRef(" padded ", 4)).toBe("[#5]");
  });
  it("the preview stand-in evaluates simple expressions", () => {
    const names = ["Price", "Qty", "Name"];
    expect(mockEval("[Price] * [Qty]", names, ["2", "3", "x"])).toBe("6");
    expect(mockEval('UPPER([Name]) & "!"', names, ["", "", "ada"])).toBe("ADA!");
    expect(mockEval("[Price] / [Qty]", names, ["1", "0", ""])).toBe("#DIV/0!");
    expect(mockEval('IF([Qty] > 2, "many", "few")', names, ["", "3", ""])).toBe("many");
    expect(() => mockEval("[Nope]", names, [])).toThrow(/no column/);
  });
});

describe("printing", () => {
  it("escapes values and sizes the page", () => {
    const p = { ...defaultPage('My "report"'), landscape: true, paper: "a4" as const };
    const html = printHtml(["<b>", "n"], [["a&b", "1,200.50"]], p, "Arial");
    expect(html).toContain("&lt;b&gt;");
    expect(html).toContain("a&amp;b");
    expect(html).toContain('class="n"');
    expect(html).toContain("size: 297mm 210mm");
    expect(html).toContain("counter(pages)");
    expect(html).not.toContain('"report"";');
    expect(html).toContain('content: "My \\22 report\\22 "');
    const evil = printHtml(null, [], { ...p, title: '"; } </style><script>x</script>' }, "Arial");
    expect(evil).not.toContain("</style><script>");
    expect(printHtml(null, [], { ...p, page_numbers: false, title: "" }, "Arial")).not.toContain("<thead>");
  });
});

describe("unique rules over several columns", () => {
  const rule = { col: 0, kind: { type: "unique" as const, match_case: false, also: [2, 3] } };
  it("names every key column", () => {
    expect(describeCheck(rule, ["First", "Last", "City", "Zip"])).toBe("First: no repeated values together with City, Zip");
    expect(describeCheck(newCheck("unique", 1), ["First", "Last"])).toBe("Last: no repeated values");
  });
  it("moves every key column with a column move, and drops the rule when one is gone", () => {
    const swap = (k: number): number => [3, 2, 1, 0][k]!;
    expect(placeCheck(rule, swap)).toEqual({ col: 3, kind: { type: "unique", match_case: false, also: [1, 0] } });
    expect(placeCheck(rule, (k) => (k === 3 ? null : k))).toBeNull();
    expect(placeCheck({ col: null, kind: { type: "expression", expr: "[a]" } }, () => null)).toEqual({ col: null, kind: { type: "expression", expr: "[a]" } });
    expect(placeCheck(newCheck("required", 1), (k) => k + 1)).toEqual({ col: 2, kind: { type: "required" } });
  });
});
describe("rules follow inserted and deleted columns", () => {
  const key = { col: 2, kind: { type: "unique" as const, match_case: false, also: [4] } };
  const req = newCheck("required", 0);
  const row = { col: null, kind: { type: "expression" as const, expr: "[a] > 1" } };
  it("moves rules right of columns put in", () => {
    const r = shiftChecks([key, req, row], 1, 2);
    expect(r.dropped).toBe(0);
    expect(r.checks).toEqual([{ col: 4, kind: { type: "unique", match_case: false, also: [6] } }, req, row]);
  });
  it("moves rules left of columns taken out, and drops a rule that used one", () => {
    const left = shiftChecks([key, req, row], 0, -1);
    expect(left.dropped).toBe(1);
    expect(left.checks).toEqual([{ col: 1, kind: { type: "unique", match_case: false, also: [3] } }, row]);
    const keyCol = shiftChecks([key, req], 3, -2);
    expect(keyCol.dropped).toBe(1);
    expect(keyCol.checks).toEqual([req]);
    const after = shiftChecks([key], 5, -1);
    expect(after).toEqual({ checks: [key], dropped: 0 });
  });
});