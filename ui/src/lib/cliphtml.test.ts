import { describe, expect, it } from "vitest";
import { buildHtml, escapeHtml, htmlCell, excelKeepsAsNumber } from "./cliphtml";
import { defaultFormat, formatCell, type DisplayFormat } from "./m9";

/** Undo escapeHtml (for the round trip): the entities it writes, and <br> back to a line break. */
const unescape = (s: string) =>
  s
    .replace(/<br>/g, "\n")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&");

/** A small seeded random source, so a failure can be run again. */
function rng(seed: number) {
  let x = seed >>> 0;
  return () => {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    return (x >>> 0) / 2 ** 32;
  };
}
const ALPHABET = ["<", ">", "&", '"', "'", "a", "Z", "0", " ", "\t", "\n", "\r", "\u0000", "\u0007", "\u007f", "\u0085", "é", "日", "😀", ";", "=", "-", "@", "+", "/", "\\"];

describe("escaping (review note 2)", () => {
  it("round-trips any value, with the control characters dropped and no < of its own left", () => {
    const r = rng(20261128);
    for (let n = 0; n < 2000; n++) {
      const len = Math.floor(r() * 24);
      let s = "";
      for (let k = 0; k < len; k++) s += ALPHABET[Math.floor(r() * ALPHABET.length)];
      const e = escapeHtml(s);
      // Only our own <br> tags.
      expect(e.replace(/<br>/g, "")).not.toMatch(/[<>"']/);
      const kept = s.replace(/\r/g, "").replace(/[\u0000-\u0008\u000b-\u001f\u007f\u0080-\u009f]/g, "");
      expect(unescape(e)).toBe(kept);
    }
  });
  it("escapes titles too", () => {
    const h = buildHtml({ titles: ["<b>name</b>", "a&b"], rows: [], formats: [], text: [] });
    expect(h).toContain("&lt;b&gt;name&lt;/b&gt;");
    expect(h).toContain("a&amp;b");
    expect(h).toMatch(/^<!DOCTYPE html><html><head><meta charset="utf-8"><\/head>/);
  });
});

describe("Excel keeps what it is given (review note 1)", () => {
  it("only plain numbers may be numbers", () => {
    for (const s of ["0", "12", "-5", "10.50", "0.25", "123456789012345"]) expect(excelKeepsAsNumber(s)).toBe(true);
    for (const s of ["01234", "1234567890123456", "1-2", "=1+1", "+1", "@a", "-", "1e5", "1,234", " 1", "", "3/4", "-0012"]) expect(excelKeepsAsNumber(s)).toBe(false);
  });
  it("marks every other cell as text, and text or Id columns always", () => {
    const h = buildHtml({ rows: [["01234", "=1+1", "1-2", "42", "9876543210987654", "+7", "@x"]], formats: [], text: [false, false, false, false, false, false, false] });
    const cells = [...h.matchAll(/<td style="([^"]*)">([^<]*)<\/td>/g)].map((m) => [m[2], (m[1] ?? "").includes("mso-number-format:'\\@'")]);
    expect(cells).toEqual([
      ["01234", true],
      ["=1+1", true],
      ["1-2", true],
      ["42", false],
      ["9876543210987654", true],
      ["+7", true],
      ["@x", true],
    ]);
    const id = buildHtml({ rows: [["42"]], formats: [], text: [true] });
    expect(id).toContain("mso-number-format:'\\@'");
  });
  it("a date shown under a date format is text, never re-read by Excel", () => {
    const f: DisplayFormat = { ...defaultFormat("date"), date: "medium" };
    expect(htmlCell("2026-03-14", f, false)).toEqual({ shown: "Mar 14, 2026", number: false });
  });
});

describe("display formats as the grid shows them (review note 8)", () => {
  it("dates, numbers and currency give the grid's words", () => {
    const cases: [string, DisplayFormat][] = [
      ["2026-03-14", { ...defaultFormat("date"), date: "mdy" }],
      ["2026-03-14", { ...defaultFormat("date"), date: "long" }],
      ["1234.5", defaultFormat("number")],
      ["1234.5", defaultFormat("currency")],
      ["0.125", defaultFormat("percent")],
      ["not a number", defaultFormat("currency")],
    ];
    for (const [v, f] of cases) expect(htmlCell(v, f, false).shown).toBe(formatCell(v, f));
    expect(htmlCell("1234.5", defaultFormat("currency"), false).number).toBe(true);
  });
});
