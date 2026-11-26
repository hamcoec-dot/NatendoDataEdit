import { describe, expect, it } from "vitest";
import type { NextRow, RowText } from "./backend";
import { groups, joinPreview, nextNotes, nextPreview, rowHeading, stepRow } from "./repair";

describe("repair bar", () => {
  const fields = ["1", "Smith", " John", "Rome"];
  it("groups neighbouring fields from the boundaries clicked", () => {
    expect(groups(4, new Set())).toEqual([1, 1, 1, 1]);
    expect(groups(4, new Set([1]))).toEqual([1, 2, 1]);
    expect(groups(4, new Set([0, 1, 2]))).toEqual([4]);
    expect(joinPreview(fields, new Set([1]), ",")).toEqual(["1", "Smith, John", "Rome"]);
  });
  it("joins the next row with a space, a line break or nothing", () => {
    expect(nextPreview(["1", "Os"], ["lo", "7"], "space")).toEqual(["1", "Os lo", "7"]);
    expect(nextPreview(["1", "Os"], ["lo", "7"], "line_break")).toEqual(["1", "Os\nlo", "7"]);
    expect(nextPreview(["1", "2", ""], ["3", "4"], "nothing")).toEqual(["1", "2", "3", "4"]);
  });
  it("says what the next row takes with it", () => {
    const next = { id: 3, file_row: 4, edited: true, hidden: true, noted: true, fields: [] } as NextRow;
    expect(nextNotes(next)).toMatch(/row 4 of the file.*hidden; it will be joined.*changed.*note/);
  });
  it("names the row", () => {
    expect(rowHeading({ file_row: 2, row: 1, width: 4, usual: 3 } as RowText)).toBe("Row 2 of the file: 4 fields, usual 3.");
    expect(rowHeading({ file_row: 0, row: null, width: 1, usual: 3 } as RowText)).toContain("added by hand (not on screen)");
  });
  it("steps over rows by id and stays on the next one after a repair", () => {
    expect(stepRow([5, 8, 9], 8, 0, 1)).toBe(9);
    expect(stepRow([5, 8, 9], 5, 0, -1)).toBe(9);
    // Row 8 was repaired: the list found again no longer has it; its place now holds the next one.
    expect(stepRow([5, 9], 8, 1, 1)).toBe(9);
    expect(stepRow([], 8, 1, 1)).toBeNull();
  });
});
