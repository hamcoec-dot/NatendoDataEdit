import { describe, expect, it } from "vitest";
import { acceptedPairs, guessFields, guessKind, type Candidate } from "./match";

describe("match people", () => {
  it("guesses how to compare from titles", () => {
    expect(guessKind("Last Name")).toBe("name");
    expect(guessKind("DOB")).toBe("exact");
    expect(guessKind("Zip")).toBe("exact");
    expect(guessKind("City")).toBe("text");
    expect(guessFields(["First", "Last", "City", "DOB"], ["dob", "last", "first"])).toEqual([
      { col: 0, other_col: 2, kind: "name", weight: 1 },
      { col: 1, other_col: 1, kind: "name", weight: 1 },
      { col: 3, other_col: 0, kind: "exact", weight: 1 },
    ]);
  });
  it("lists accepted pairs", () => {
    const c = (id: number, other: number, score: number): Candidate => ({ row: id, id, other_row: other, score, parts: [], here: [], there: [] });
    expect(acceptedPairs([c(1, 4, 97), c(2, 5, 80), c(3, 6, 90)], new Set([0, 2]))).toEqual([
      [1, 4, 97],
      [3, 6, 90],
    ]);
  });
});
