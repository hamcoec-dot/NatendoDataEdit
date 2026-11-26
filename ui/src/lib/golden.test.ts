import { describe, expect, it } from "vitest";
import { guessIdColumn, makeRule, notes, type GoldenSummary } from "./golden";

const summary = (over: Partial<GoldenSummary["counts"]>, never = 0): GoldenSummary => ({
  counts: { clusters: 1, folded: 1, hub_only: 0, skipped: 0, review: 0, unmerged_pairs: 0, flagged: 0, rows: 5, hidden: 0, ...over },
  listed: 1,
  never_offered: never,
  review: [],
});

describe("golden record words", () => {
  it("says nothing when there is nothing to know", () => {
    expect(notes(summary({}))).toEqual([]);
  });
  it("warns about hidden rows and pairs nobody saw", () => {
    const said = notes(summary({ hidden: 3, flagged: 1, hub_only: 2 }, 1200)).join(" ");
    expect(said).toContain("3 hidden rows were not matched and are kept as they are");
    expect(said).toContain("1,200 pairs above the score were never listed");
    expect(said).toContain("nobody has checked those people");
    expect(said).toContain("highlighted");
    expect(said).toContain("joined through one row only");
  });
  it("makes the rules the app takes", () => {
    expect(makeRule("union", " | ", false)).toEqual({ rule: "union", separator: " | " });
    expect(makeRule("newest", "", true)).toEqual({ rule: "newest", blank_counts: true });
    expect(makeRule("sum", "", true)).toEqual({ rule: "sum" });
  });
  it("guesses an identifier column", () => {
    expect(guessIdColumn(["Name", "Member Id"])).toBe(1);
    expect(guessIdColumn(["voter_id", "Name"])).toBe(0);
    expect(guessIdColumn(["Name", "Paid"])).toBe(null);
  });
});
