import { describe, expect, it } from "vitest";
import { columnRef, defaultPage, fitNote, fitScale, measureWidths, MIN_FONT, PRINT_CSP, printBlock, printFit, describeCheck, expandTabs, guessStarts, newCheck, placeCheck, printHtml, shiftChecks, splitFixed, toggleStart, toJsPattern, LOG_PRESETS } from "./m8";
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

  it("locks the page: the content policy first, and values can never load anything", () => {
    const p = defaultPage("t");
    const html = printHtml(["a"], [["<img src=https://x>"], ["</style><link rel=stylesheet href=//x>"]], p, "Arial");
    expect(PRINT_CSP).toBe("default-src 'none'; style-src 'unsafe-inline'");
    expect(html.startsWith(`<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${PRINT_CSP}">`)).toBe(true);
    expect(html).not.toMatch(/<img|<link|<\/style><link/);
    expect(html).toContain("&lt;img src=https://x&gt;");
    expect(html).toContain("&lt;/style&gt;&lt;link rel=stylesheet href=//x&gt;");
    // The preview too.
    expect(printHtml(["a"], [], p, "Arial", true).startsWith('<!doctype html><html><head><meta http-equiv="Content-Security-Policy"')).toBe(true);
  });

  it("prints the selected block: its shown columns and its rows", () => {
    const shown = [0, 1, 3, 4, 5];
    // A block over columns 1-4 with column 2 hidden, rows 3-40.
    expect(printBlock({ row: 3, rows: 38, col: 1, cols: 4 }, shown, 1, 100, 6)).toEqual({ first: 3, rows: 38, cols: [1, 3, 4] });
    // Whole rows: every shown column.
    expect(printBlock({ row: 5, rows: 2, col: 0, cols: 6 }, shown, 1, 100, 6)).toEqual({ first: 5, rows: 2, cols: shown });
    // A block that starts on the title row prints from the first data row.
    expect(printBlock({ row: 0, rows: 4, col: 0, cols: 1 }, shown, 1, 100, 6)).toEqual({ first: 1, rows: 3, cols: [0] });
    // Only hidden columns: nothing to print.
    expect(printBlock({ row: 3, rows: 2, col: 2, cols: 1 }, shown, 1, 100, 6)?.rows).toBe(0);
    // Several blocks.
    expect(printBlock(null, shown, 1, 100, 6)).toBeNull();
  });

  it("fitScale shrinks to the content and stops at 4 pt", () => {
    expect(fitScale(500, 540, 9)).toEqual({ size: 9, fits: true });
    expect(fitScale(1080, 540, 9)).toEqual({ size: 4.5, fits: true });
    expect(fitScale(2000, 540, 9)).toEqual({ size: MIN_FONT, fits: false });
  });

  it("measures each column's widest line, titles bold, without measuring short ones", () => {
    const seen: string[] = [];
    const m = (s: string, bold: boolean): number => {
      seen.push(s);
      return s.length * (bold ? 0.6 : 0.5);
    };
    const w = measureWidths(["Name", "N"], [["Avery Ashdown", "1"], ["Al", "22"], ["x\nlonger line here", "3"]], m);
    expect(w).toEqual([16 * 0.5, 2 * 0.5]);
    expect(seen).not.toContain("Al");
    expect(measureWidths(null, [["a", "bb"]], m)).toEqual([0.5, 1]);
  });

  it("fits like the PDF: to the width, then on one page", () => {
    const p = { ...defaultPage(""), page_numbers: false };
    // 30 columns of 10 em at 9 pt: far wider than Letter portrait.
    const wide = new Array(30).fill(10);
    const f = printFit(wide, p, 10, true, true);
    expect(f.size).toBe(MIN_FONT);
    expect(f.fitsWidth).toBe(false);
    expect(f.pages).toBeGreaterThan(1);
    expect(fitNote(f, p, false)).toContain("Too many columns for one page width even at 4 pt");
    // Six columns fit at the size asked: nothing changes.
    const narrow = new Array(6).fill(5);
    const n = printFit(narrow, p, 10, true);
    expect(n).toMatchObject({ size: 9, widths: null, fitsWidth: true, pages: 1 });
    expect(fitNote(n, p, false)).toBe("");
    // Twelve columns shrink a little and fit.
    const mid = printFit(new Array(12).fill(6), p, 10, true, true);
    expect(mid.size).toBeLessThan(9);
    expect(mid.size).toBeGreaterThan(5);
    expect(mid.widths!.reduce((a, b) => a + b, 0)).toBeLessThanOrEqual(540 + 0.01);
    expect(fitNote(mid, p, true)).toMatch(/^Text will print at \d+(\.\d)? pt\.$/);
    // Off: no shrinking.
    expect(printFit(new Array(12).fill(6), { ...p, fit_width: false }, 10, true).size).toBe(9);
    // One page: 40 rows at 24 pt shrink onto one page; 2,000 rows cannot.
    const big = { ...p, font_size: 24, fit_page: true };
    const one = printFit(narrow, big, 40, true);
    expect(one.size).toBeLessThan(24);
    expect(one.pages).toBe(1);
    for (const pdf of [false, true]) {
      const many = printFit(narrow, { ...big, title: "List" }, 2000, true, pdf);
      expect(many.size).toBe(MIN_FONT);
      expect(many.pages).toBeGreaterThan(1);
      expect(fitNote(many, big, pdf)).toContain(`This needs ${many.pages.toLocaleString()} pages even at 4 pt`);
    }
    // Small text says so.
    expect(fitNote({ size: 5.5, asked: 9, widths: [], fitsWidth: true, pages: 1 }, p, false)).toBe("Text will print at 5.5 pt (small: try landscape).");
    expect(fitNote({ size: 5.5, asked: 9, widths: [], fitsWidth: true, pages: 1 }, { ...p, landscape: true }, false)).toBe("Text will print at 5.5 pt (small).");
  });

  it("prints shrunk text with fixed column widths", () => {
    const p = defaultPage("");
    const fit = printFit(new Array(12).fill(6), p, 3, true);
    const html = printHtml(new Array(12).fill("t"), [], p, "Arial", false, [], fit);
    expect(html).toContain("table-layout: fixed");
    expect(html).toContain(`font: ${+fit.size.toFixed(3)}pt/1.25 Arial`);
    expect((html.match(/<col style="width:/g) ?? []).length).toBe(12);
    expect(html).not.toContain("text-overflow: ellipsis");
    // On one page with nothing to shrink, the widths are still fixed (G2).
    const roomy = { ...p, fit_page: true };
    const f0 = printFit([5, 5, 5], roomy, 10, true);
    expect(f0.size).toBe(9);
    expect(f0.widths).toHaveLength(3);
    const h0 = printHtml(["a", "b", "c"], [], roomy, "Arial", false, [], f0);
    expect(h0).toContain("table-layout: fixed");
    expect((h0.match(/<col style="width:/g) ?? []).length).toBe(3);
    // Fit to width alone, nothing to shrink: laid out as before.
    expect(printFit([5, 5, 5], p, 10, true).widths).toBeNull();
    // On one page, long values are cut (as in the PDF), never wrapped onto more lines.
    const one = { ...p, fit_page: true };
    expect(printHtml(["t"], [["x"]], one, "Arial", false, [], printFit([6], one, 1, true))).toContain("th, td { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }");
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

describe("value must exist in another file", () => {
  it("is described with the file and column", () => {
    const c = newCheck("foreign_key", 1);
    expect(c.kind).toEqual({ type: "foreign_key", table: "", column: "", match_case: false, trim: true, allow_blank: true });
    expect(describeCheck({ ...c, kind: { ...c.kind, table: "precincts.csv", column: "Code" } as typeof c.kind }, ["Name", "Precinct"])).toBe("Precinct: must exist in precincts.csv > Code");
  });
});
