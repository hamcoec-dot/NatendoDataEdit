import { describe, expect, it } from "vitest";
import { choiceOf, columnNotes } from "./updates";

describe("changes from a newer file", () => {
  it("sends take-all plus exceptions, sorted", () => {
    expect(choiceOf(true, new Set([5, 2]))).toEqual({ all: true, picked: [2, 5] });
    expect(choiceOf(false, new Set())).toEqual({ all: false, picked: [] });
  });
});

describe("column notes", () => {
  it("names columns not compared", () => {
    const n = columnNotes({ here_only: ["phone"], newer_only: ["precinct", "email"], moved: 1 }, true);
    expect(n).toHaveLength(3);
    expect(n[0]).toContain("phone");
    expect(n[1]).toContain("precinct, email");
    expect(columnNotes({ here_only: [], newer_only: [], moved: 2 }, false)).toEqual([]);
  });
});
