import { describe, expect, it } from "vitest";
import { colLetters, lettersCol, parseGoto } from "./goto";

describe("column letters", () => {
  it("count like a spreadsheet", () => {
    expect([0, 1, 25, 26, 27, 51, 52, 701, 702].map(colLetters)).toEqual(["A", "B", "Z", "AA", "AB", "AZ", "BA", "ZZ", "AAA"]);
    for (const c of [0, 5, 25, 26, 700, 702, 16383]) expect(lettersCol(colLetters(c))).toBe(c);
    expect(lettersCol("abcd")).toBeNull();
  });
});

describe("parseGoto", () => {
  const titles = ["id", "First name", "Last name", "city", "C2"];
  it("reads row numbers", () => {
    expect(parseGoto("250", titles)).toEqual({ row: 250, col: null });
    expect(parseGoto("250, 3", titles)).toEqual({ row: 250, col: 2 });
  });
  it("reads the name box's own form", () => {
    expect(parseGoto("R12 C3", titles)).toEqual({ row: 12, col: 2 });
    expect(parseGoto("r12c3", titles)).toEqual({ row: 12, col: 2 });
    expect(parseGoto("R12", titles)).toEqual({ row: 12, col: null });
    expect(parseGoto("C4", titles)).toEqual({ row: 4, col: 2 });
  });
  it("prefers a column title", () => {
    expect(parseGoto("C2", titles)).toEqual({ row: null, col: 4 });
    expect(parseGoto("last name", titles)).toEqual({ row: null, col: 2 });
    expect(parseGoto("Last name 200", titles)).toEqual({ row: 200, col: 2 });
    expect(parseGoto("city: 7", titles)).toEqual({ row: 7, col: 3 });
  });
  it("reads Excel letters", () => {
    expect(parseGoto("F200", titles)).toEqual({ row: 200, col: 5 });
    expect(parseGoto("aa3", titles)).toEqual({ row: 3, col: 26 });
    expect(parseGoto("D", titles)).toEqual({ row: null, col: 3 });
  });
  it("gives up on nonsense", () => {
    expect(parseGoto("", titles)).toBeNull();
    expect(parseGoto("what?", titles)).toBeNull();
  });
});
