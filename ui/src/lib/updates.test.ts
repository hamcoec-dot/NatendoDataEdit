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

describe("ticks with hidden rows", () => {
  const hidden = (i: number) => i === 0;
  it("holds hidden rows back from take-all until each is ticked", () => {
    const t = { all: true, skipHidden: true, flips: new Set<number>() };
    expect([ticked(t, 0, true), ticked(t, 1, false)]).toEqual([false, true]);
    expect(tickedCount(t, 3, 1, hidden)).toBe(2);
    const one = { ...t, flips: new Set([0, 1]) };
    expect([ticked(one, 0, true), ticked(one, 1, false)]).toEqual([true, false]);
    expect(tickedCount(one, 3, 1, hidden)).toBe(2);
  });
  it("takes them like any other once they are not held back", () => {
    const t = { all: true, skipHidden: false, flips: new Set<number>([0]) };
    expect(ticked(t, 0, true)).toBe(false);
    expect(tickedCount(t, 3, 1, hidden)).toBe(2);
  });
});

import { ticked, tickedCount } from "./updates";
