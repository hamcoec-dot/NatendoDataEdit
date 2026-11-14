import { describe, expect, it } from "vitest";
import { insertCount, MAX_INSERT_COLS, MAX_INSERT_ROWS } from "./insert";

describe("insert count", () => {
  it("takes whole numbers within the limits", () => {
    expect(insertCount("10", "rows")).toBe(10);
    expect(insertCount(" 1,000 ", "rows")).toBe(1000);
    expect(insertCount(String(MAX_INSERT_ROWS), "rows")).toBe(MAX_INSERT_ROWS);
    expect(insertCount(String(MAX_INSERT_COLS), "cols")).toBe(MAX_INSERT_COLS);
  });

  it("refuses anything else with a message", () => {
    for (const bad of ["", "0", "-5", "2.5", "1e9", "abc", "99999999999999999999", "NaN", "Infinity"]) {
      expect(typeof insertCount(bad, "rows")).toBe("string");
    }
    expect(insertCount(String(MAX_INSERT_ROWS + 1), "rows")).toMatch(/At most 100,000 rows/);
    expect(insertCount(String(MAX_INSERT_COLS + 1), "cols")).toMatch(/At most 1,000 columns/);
  });
});
