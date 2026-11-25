import { describe, expect, it } from "vitest";
import GREMLINS from "../../../crates/engine/src/testdata/gremlins.json";
import { sanitizeText, scanGremlins, USUAL_SANITIZE, xrayNote, xrayRuns } from "./invisible";

// The shared corpus: the engine (xray.rs) runs the same file, so the two copies cannot drift.
describe("X-ray gremlins: the shared corpus", () => {
  for (const c of GREMLINS.corpus as { text: string; flags: string[][]; sanitized: string }[]) {
    it(JSON.stringify(c.text), () => {
      const found = scanGremlins(c.text);
      expect(found.map((g) => [g.kind, g.ch, g.twin])).toEqual(c.flags.map((f) => [f[0], f[1], f[2]]));
      const out = sanitizeText(c.text, USUAL_SANITIZE) ?? c.text;
      expect(out).toBe(c.sanitized);
      // Idempotent.
      expect(sanitizeText(out, USUAL_SANITIZE)).toBeNull();
    });
  }
});

describe("X-ray gremlins: the screen", () => {
  it("never flags a word of one script, a hyphenated pair of scripts, or a letter with no twin", () => {
    expect(scanGremlins("Москва Αθήνα")).toEqual([]);
    expect(scanGremlins("Smith-Иванова")).toEqual([]);
    expect(scanGremlins("xбy")).toEqual([]);
    expect(scanGremlins("A1б")).toEqual([]);
  });

  it("names what it finds and glows it", () => {
    expect(xrayNote("Smаrt")).toBe("Cyrillic а (U+0430) inside a Latin word");
    expect(xrayNote("“x”")).toBe("2 smart quotes");
    expect(xrayNote("plain")).toBe("");
    const runs = xrayRuns("Smаrt a\tb");
    expect(runs.some((r) => r.glow && r.text === "а")).toBe(true);
    expect(runs.some((r) => r.glow && r.text === "→")).toBe(true);
    expect(runs.map((r) => r.text).join("")).toBe("Smаrt·a→b");
  });

  it("tabs and line breaks are fixed only when asked", () => {
    expect(sanitizeText("a\tb\r\nc", { ...USUAL_SANITIZE, tabs_breaks: true })).toBe("a b c");
    expect(sanitizeText("a\tb", USUAL_SANITIZE)).toBeNull();
  });
});

describe("review fixes (B1)", () => {
  const withBreaks = { ...USUAL_SANITIZE, tabs_breaks: true };
  it("a CRLF pair is one space and the user's own double spaces stay", () => {
    expect(sanitizeText("a  b\r\nc", withBreaks)).toBe("a  b c");
    expect(sanitizeText("a\r\n\r\nb", withBreaks)).toBe("a  b");
    expect(sanitizeText("a\nb\rc", withBreaks)).toBe("a b c");
    expect(sanitizeText("a  b", withBreaks)).toBeNull();
  });
  it("a word with a letter that has no Latin twin is never changed", () => {
    expect(sanitizeText("Иванoв", USUAL_SANITIZE)).toBeNull();
    expect(sanitizeText("Αθηνa", USUAL_SANITIZE)).toBeNull();
    expect(sanitizeText("O’Иванов", USUAL_SANITIZE)).toBe("O'Иванов");
    expect(sanitizeText("Smаrt", USUAL_SANITIZE)).toBe("Smart");
  });
});
