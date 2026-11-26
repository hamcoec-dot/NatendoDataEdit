import { describe, expect, it } from "vitest";
import { titleLines } from "./header";

describe("report title lines above the column titles", () => {
  it("finds a title, a date and a blank line above the titles", () => {
    const rows = [
      ["Sales by shop", "", ""],
      ["Printed 2026-05-01", "", ""],
      ["", "", ""],
      ["Shop", "Month", "Amount"],
      ["North", "May", "120"],
      ["South", "May", "95"],
    ];
    expect(titleLines(rows)).toBe(3);
  });
  it("is 0 for a plain table and for data with a blank first cell", () => {
    expect(titleLines([["Shop", "Amount"], ["North", "120"], ["South", "95"]])).toBe(0);
    expect(titleLines([["a", "1", "x"], ["", "2", "y"], ["c", "3", "z"]])).toBe(0);
  });
});
