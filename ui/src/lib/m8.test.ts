import { describe, expect, it } from "vitest";
import { columnRef, defaultPage, describeCheck, expandTabs, guessStarts, newCheck, printHtml, splitFixed, toggleStart, toJsPattern, LOG_PRESETS } from "./m8";
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
