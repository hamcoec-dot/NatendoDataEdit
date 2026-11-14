import { describe, expect, it } from "vitest";
import { guessIndexCols, streetSummary } from "./streets";

describe("street lookup", () => {
  it("guesses the index's columns", () => {
    expect(guessIndexCols(["Street Name", "Low", "High", "Side", "Zip", "Precinct", "Notes", "District"])).toEqual({
      street: 0, low: 1, high: 2, side: 3, area: 4, bring: [5, 7], header: true,
    });
  });
  it("says what happened", () => {
    expect(streetSummary({ found: 1, no_street: 2, no_range: 0, overlaps: 0, no_number: 0, bad_ranges: 1 })).toBe(
      "Found 1 address. 2 streets not in the index. 1 index row could not be read and was skipped.",
    );
  });
});
