import { describe, expect, it } from "vitest";
import { colLetters, rewriteA1, typedCell } from "./cellformula";

describe("cell formulas", () => {
  it("tells formulas from text", () => {
    expect(typedCell("=A1*2")).toEqual({ formula: "=A1*2" });
    expect(typedCell("'=A1*2")).toEqual({ text: "=A1*2" });
    expect(typedCell("=")).toEqual({ text: "=" });
    expect(typedCell("'quoted")).toEqual({ text: "'quoted" });
    expect(typedCell("plain")).toEqual({ text: "plain" });
  });

  it("names columns with letters", () => {
    expect([0, 25, 26, 701, 702].map(colLetters)).toEqual(["A", "Z", "AA", "ZZ", "AAA"]);
  });

  it("moves references unless fixed, and reads ranges", () => {
    const grid = [
      ["Item", "Price"],
      ["Pen", "2"],
      ["Ink", "3"],
    ];
    const at = (r: number, c: number) => grid[r]?.[c] ?? "";
    const w = rewriteA1("B2*2 + $B$2 + SUM(B1:B3)", [1, 2], [2, 2], [5, 3], at);
    expect(w?.keep).toBe("B3*2 + $B$2 + SUM(B2:B4)");
    expect(w?.run).toBe('"3"*2 + "2" + SUM("2","3")');
    // B2:B4 is off the table (three rows): null.
    expect(rewriteA1("SUM(B1:B3)", [1, 2], [2, 2], [3, 3], at)).toBeNull();
    const ok = rewriteA1("SUM(B1:B3) & \"B2\" & LOG10(1)", [1, 2], [1, 2], [3, 3], at);
    expect(ok?.run).toBe('SUM("2","3") & "B2" & LOG10(1)');
    expect(rewriteA1("COUNTA(A1:B1)", [0, 0], [0, 0], [3, 3], at)?.run).toBe('COUNTA("Item","Price")');
  });
});

import { formulaHelp } from "./cellformula";
import { FUNCTIONS } from "./m8";

describe("formula helper", () => {
  it("suggests functions and columns, and says how the open function is used", () => {
    const h = formulaHelp("=ROUND(NETW", 11, FUNCTIONS);
    expect(h.suggest.map((s) => s.label)).toEqual(["NETWORKDAYS"]);
    expect(h.from).toBe(7);
    expect(h.hint).toMatch(/^ROUND\(/);
    const inside = formulaHelp("=TEXTAFTER([Email], ", 20, FUNCTIONS);
    expect(inside.suggest).toEqual([]);
    expect(inside.hint).toMatch(/^TEXTAFTER/);
    const c = formulaHelp("=[pri", 5, FUNCTIONS, ["Item", "Price", "Unit price"]);
    expect(c.suggest.map((s) => s.label)).toEqual(["[Price]", "[Unit price]"]);
    expect(formulaHelp('="DA', 4, FUNCTIONS).suggest).toEqual([]);
    // A finished call: the hint is for the one outside it.
    expect(formulaHelp("=IF(LEN([a]) > 2, ", 18, FUNCTIONS).hint).toMatch(/^IF\(/);
  });
});
