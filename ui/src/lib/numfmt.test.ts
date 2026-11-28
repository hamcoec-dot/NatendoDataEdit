// The display format codes: the cases shared with the engine
// (crates/engine/tests/data/numfmt-cases.json, run there by cargo test too),
// the hard limits, random and runaway codes, and the compile cache.
import { describe, expect, it } from "vitest";
import cases from "../../../crates/engine/tests/data/numfmt-cases.json";
import { cacheSize, Code, CodeError, codeProblem, Display, displayOf, readDecimal, roundDigits } from "./numfmt";

interface Case {
  code: string;
  date?: string;
  time?: string;
  cells?: [string, string | null][];
  color?: string;
  error?: string;
}

describe("display format codes", () => {
  it("give the shared cases exactly what the engine gives", () => {
    const all = cases as Case[];
    expect(all.length).toBeGreaterThanOrEqual(100);
    let cells = 0;
    for (const c of all) {
      let d: Display | null = null;
      let err: string | null = null;
      try {
        d = Display.make(c.code, c.date ?? "", c.time ?? "");
      } catch (e) {
        expect(e).toBeInstanceOf(CodeError);
        err = (e as Error).message;
      }
      if (c.error !== undefined) {
        expect(err, `code ${JSON.stringify(c.code)}`).toBe(c.error);
        continue;
      }
      expect(err, `code ${JSON.stringify(c.code)}`).toBeNull();
      for (const [cell, want] of c.cells ?? []) {
        cells++;
        const got = d!.render(cell);
        expect(got?.text ?? null, `code ${JSON.stringify(c.code)} cell ${JSON.stringify(cell)}`).toBe(want);
        if (c.color !== undefined && got) expect(got.color ?? "", `colour of ${c.code} ${cell}`).toBe(c.color);
      }
    }
    expect(cells).toBeGreaterThanOrEqual(300);
  });

  it("round exact ties half away from zero, on the written digits", () => {
    const r = (cell: string, frac: number) => {
      const [i, f] = roundDigits(readDecimal(cell)!, 0, frac)!;
      return f ? `${i}.${f}` : i;
    };
    expect(r("0.125", 2)).toBe(".13");
    expect(r("0.375", 2)).toBe(".38");
    expect(r("2.5", 0)).toBe("3");
    expect(r("-2.5", 0)).toBe("3");
    expect(r("1.005", 2)).toBe("1.01");
    expect(r("0.0000001", 2)).toBe(".00");
    expect(r("9.995", 2)).toBe("10.00");
    // toFixed says 1.00 for 1.005 (binary rounding): the rule is not toFixed's.
    expect((1.005).toFixed(2)).toBe("1.00");
  });

  it("never throw on random codes and long runs of one token, and keep the output small", () => {
    const alphabet = ["0", "#", "?", ".", ",", "%", "@", ";", '"', "\\", "[", "]", "[Red]", "[>1]", "[<=-2]", "y", "m", "d", "h", "s", "AM/PM", "A/P", "-", "(", ")", " ", "$", "E+", "*", "_", "/", "x", "€", "0.00", "#,##0", "yyyy", "mmmm"];
    const cells = ["", "0", "-0", "1234.5", "-1234.5", "0.125", "1e20", "1e-30", "99999999999999.9", "abc", "2026-03-14", "2026-03-14 14:30", "14:30:59.9", "2:30 PM", "(5)", "12%", "$1,234.56"];
    let x = 0x2545f491;
    const next = () => {
      x ^= x << 13;
      x ^= x >>> 17;
      x ^= x << 5;
      return x >>> 0;
    };
    const codes: string[] = [];
    for (let k = 0; k < 2000; k++) {
      const n = (next() % 24) + 1;
      let c = "";
      for (let j = 0; j < n; j++) c += alphabet[next() % alphabet.length];
      codes.push(c);
    }
    for (const t of ["0", ",", "%", '"', "\\", "#", "?", "[", "]", ";", "y", "m", "0.", ".0", "[Red]"]) {
      codes.push(t.repeat(200), t.repeat(201), `0${t.repeat(199)}`);
    }
    codes.push('"abc', "0[Red", "x".repeat(10_000));
    for (const code of codes) {
      let k: Code | null = null;
      try {
        k = Code.compile(code);
      } catch (e) {
        expect(e).toBeInstanceOf(CodeError);
      }
      if (!k) continue;
      for (const cell of cells) {
        const s = k.render(cell);
        if (s) expect(s.text.length).toBeLessThanOrEqual(4 * cell.length + 4 * code.length + 64);
      }
    }
  });

  it("are compiled once per text, capped, and a bad code compiles to nothing", () => {
    const a = displayOf("0.00", "", "");
    expect(displayOf("0.00", "", "")).toBe(a);
    expect(displayOf("0.0E+0", "", "")).toBeNull();
    for (let i = 0; i < 300; i++) displayOf(`0;"${i}"`, "", "");
    expect(cacheSize()).toBeLessThanOrEqual(256);
    // A new text is a new compile: an edit in place can never draw with the old one.
    expect(displayOf("0.000", "", "")!.render("1")!.text).toBe("1.000");
    expect(codeProblem("0.00")).toBeNull();
    expect(codeProblem("0.0E+0")).toBe("Not supported at character 4: scientific notation (E+).");
  });
});
