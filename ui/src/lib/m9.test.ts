import { describe, expect, it } from "vitest";
import { autosaveDue, cleanFormat, defaultFormat, describeSuggestion, formatCell, keyGroups, linkOf, pickMatches, readDate, readNumber, tickOf, toggled, totalText, totalValue, wrapLines } from "./m9";
import { parsePrefs } from "./prefs.svelte";

describe("links", () => {
  it("opens only web and e-mail addresses", () => {
    expect(linkOf("https://example.com/a?b=1")).toBe("https://example.com/a?b=1");
    expect(linkOf("  http://example.com  ")).toBe("http://example.com/");
    expect(linkOf("www.example.com/x")).toBe("https://www.example.com/x");
    expect(linkOf("someone@example.org")).toBe("mailto:someone@example.org");
    expect(linkOf("mailto:a.b@example.co.uk")).toBe("mailto:a.b@example.co.uk");
    for (const bad of [
      "",
      "file:///C:/Windows/notepad.exe",
      "C:\\Windows\\notepad.exe",
      "\\\\server\\share",
      "javascript:alert(1)",
      "ms-settings:display",
      "https://user:pw@example.com",
      "https://exa mple.com",
      "example.com",
      "not an @ address",
      "mailto:nobody",
      `https://e.com/${"a".repeat(3000)}`,
    ])
      expect(linkOf(bad), bad).toBeNull();
  });
});

describe("display formats", () => {
  it("reads numbers in common shapes", () => {
    expect(readNumber("1,234.5")).toBe(1234.5);
    expect(readNumber("$12")).toBe(12);
    expect(readNumber("(3)")).toBe(-3);
    expect(readNumber("-0.5")).toBe(-0.5);
    expect(readNumber("45%")).toBeCloseTo(0.45);
    expect(readNumber("12abc")).toBeNull();
    expect(readNumber("1,23")).toBeNull();
  });
  it("formats without changing text that does not fit", () => {
    const n = { ...defaultFormat("number"), decimals: 1 };
    expect(formatCell("1234.56", n)).toBe("1,234.6");
    expect(formatCell("hello", n)).toBe("hello");
    expect(formatCell("-0.04", n)).toBe("0.0");
    expect(formatCell("0.256", defaultFormat("percent"))).toBe("26%");
    expect(formatCell("-5", { ...defaultFormat("currency"), symbol: "€" })).toBe("-€5.00");
    expect(formatCell("2026-03-14", { ...defaultFormat("date"), date: "medium" })).toBe("Mar 14, 2026");
    expect(formatCell("3/14/2026", { ...defaultFormat("date"), date: "iso" })).toBe("2026-03-14");
    expect(formatCell("2026-02-30", defaultFormat("date"))).toBe("2026-02-30");
    expect(readDate("14/03/2026", true)?.toISOString().slice(0, 10)).toBe("2026-03-14");
    // Long ID numbers show as written (a number would change digits past 15).
    expect(formatCell("12345678901234567890", n)).toBe("12345678901234567890");
    expect(formatCell("123456789012345", n)).toBe("123,456,789,012,345.0");
    expect(formatCell("1.50000000000000000000", n)).toBe("1.5");
    // Years before 100 stay as they are (not 1900-1999).
    expect(formatCell("0012-03-04", defaultFormat("date"))).toBe("0012-03-04");
    expect(readDate("0012-03-04")?.getUTCFullYear()).toBe(12);
  });
  it("cleans formats from views", () => {
    expect(cleanFormat({ kind: "number", decimals: 99, symbol: "toolong" })).toMatchObject({ decimals: 10, symbol: "tool" });
    expect(cleanFormat({ kind: "bogus" })).toBeNull();
    expect(cleanFormat(null)).toBeNull();
  });
});

describe("tick boxes", () => {
  it("reads and toggles in the cell's own style", () => {
    expect(tickOf("TRUE")).toBe(true);
    expect(tickOf("no")).toBe(false);
    expect(tickOf("")).toBe(false);
    expect(tickOf("maybe")).toBeNull();
    expect(toggled("TRUE")).toBe("FALSE");
    expect(toggled("Yes")).toBe("No");
    expect(toggled("n")).toBe("y");
    expect(toggled("1")).toBe("0");
    expect(toggled("x")).toBe("");
    expect(toggled("", ["Yes", "No"])).toBe("Yes");
    expect(toggled("", [])).toBe("TRUE");
    expect(toggled("maybe")).toBeNull();
  });
});

describe("wrapped text", () => {
  const m = (s: string) => s.length;
  it("breaks by words and cuts long words", () => {
    expect(wrapLines("the quick brown fox", 10, m, 5)).toEqual(["the quick", "brown fox"]);
    expect(wrapLines("abcdefghijkl", 5, m, 5)).toEqual(["abcde", "fghij", "kl"]);
    expect(wrapLines("a\nb", 10, m, 5)).toEqual(["a", "b"]);
  });
  it("ends in … when lines run out", () => {
    const out = wrapLines("one two three four five six", 8, m, 2);
    expect(out.length).toBe(2);
    expect(out[1]!.endsWith("…")).toBe(true);
    expect(wrapLines("a\nb\nc", 10, m, 2)).toEqual(["a", "b…"]);
  });
});

describe("pick lists and totals", () => {
  it("offers values starting with the text first", () => {
    expect(pickMatches(["Banana", "Apple", "Pineapple"], "app")).toEqual(["Apple", "Pineapple"]);
    expect(pickMatches(["a", "b"], "")).toEqual(["a", "b"]);
    expect(pickMatches(Array.from({ length: 500 }, (_, i) => `v${i}`), "", 10).length).toBe(10);
  });
  it("picks the chosen total", () => {
    const t = { cells: 4, filled: 3, numbers: 2, sum: 5, average: 2.5, min: 1, max: 4, too_large: false, chars: 9, distinct: 3, distinct_exact: true };
    expect(totalValue(t, "count")).toBe(3);
    expect(totalValue(t, "numbers")).toBe(2);
    expect(totalValue(t, "average")).toBe(2.5);
    const f = (n: number | null) => String(n);
    const big = { ...t, sum: null, average: null, too_large: true };
    expect(totalText(big, "sum", f)).toBe("too large");
    expect(totalText(big, "max", f)).toBe("4");
    expect(totalValue(t, "chars")).toBe(9);
    expect(totalText(t, "distinct", f)).toBe("3");
    expect(totalText({ ...t, distinct: 2_000_000, distinct_exact: false }, "distinct", f)).toBe("at least 2000000");
    expect(totalText({ ...t, distinct: null }, "distinct", f)).toBe("…");
  });
  it("lists shortcuts by menu", () => {
    const g = keyGroups(
      [{ label: "&File", items: ["a", "-", "b"] }, { label: "&Edit", items: ["a", "c"] }],
      (id) => ({ a: { label: "&Save", keys: ["Ctrl+S"] }, b: { label: "Nothing", keys: [] }, c: { label: "Undo…", keys: ["Ctrl+Z"] } })[id] ?? null,
    );
    expect(g).toEqual([
      { name: "File", items: [{ label: "Save", keys: ["Ctrl+S"] }] },
      { name: "Edit", items: [{ label: "Undo", keys: ["Ctrl+Z"] }] },
    ]);
  });
});

describe("earlier saves setting", () => {
  it("is off unless chosen, also for settings saved by older builds", () => {
    expect(parsePrefs(null).keepEarlierSaves).toBe(false);
    expect(parsePrefs(JSON.stringify({ keepVersions: true })).keepEarlierSaves).toBe(false);
    expect(parsePrefs(JSON.stringify({ keepEarlierSaves: true })).keepEarlierSaves).toBe(true);
    expect(parsePrefs(JSON.stringify({ keepEarlierSaves: "yes" })).keepEarlierSaves).toBe(false);
  });
});

describe("autosave and suggestions", () => {
  it("is due only with unsaved changes, after the wait", () => {
    expect(autosaveDue(10 * 60_000, 0, 2, true, true)).toBe(true);
    expect(autosaveDue(60_000, 0, 2, true, true)).toBe(false);
    expect(autosaveDue(10 * 60_000, 0, 2, false, true)).toBe(false);
    expect(autosaveDue(10 * 60_000, 0, 2, true, false)).toBe(false);
    expect(autosaveDue(5, null, 2, true, true)).toBe(true);
  });
  it("words suggestions", () => {
    const d = describeSuggestion({ kind: "spaces", col: 1, count: 3, examples: [" a"], fix: { fix: "clean", cols: [1], step: { tool: "spaces", how: "trim" } } }, () => "Name");
    expect(d.found).toBe("3 cells in “Name” have extra spaces");
    expect(d.fix).toBe("Remove spaces at the start and end");
    expect(describeSuggestion({ kind: "duplicate_rows", col: null, count: 1, examples: [], fix: { fix: "remove_duplicates" } }, () => "").found).toBe("1 row repeats an earlier row exactly");
  });
});
