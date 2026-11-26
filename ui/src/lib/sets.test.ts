import { describe, expect, it } from "vitest";
import { countsText, listsIn, sliceName, type SetsRun } from "./sets";

const names = ["A", "B", "C"];

describe("compare lists words", () => {
  it("names slices as the engine does", () => {
    expect([1, 3, 7, 4].map((m) => sliceName(m, names))).toEqual(["A only", "A and B, not C", "A, B and C", "C only"]);
    expect(sliceName(3, ["A", "B"])).toBe("A and B");
    expect(sliceName(3, ["A", "B", "C", "D"])).toBe("A and B, not C or D");
    expect(listsIn(5, 3)).toEqual([0, 2]);
  });

  it("copies the counts, empty slices only when asked", () => {
    const run: SetsRun = {
      id: 1,
      names: ["A", "B\tx"],
      report: {
        lists: [
          { rows: 3, keys: 2, blank: 1 },
          { rows: 1, keys: 1, blank: 0 },
        ],
        slices: [
          { mask: 1, keys: 2, rows: [2, 0] },
          { mask: 2, keys: 1, rows: [0, 1] },
          { mask: 3, keys: 0, rows: [0, 0] },
        ],
      },
    };
    expect(countsText(run, false)).toBe("Slice\tDifferent keys\tRows in A\tRows in B x\nA only\t2\t2\t0\nB x only\t1\t0\t1\nA: 3 rows, 2 different keys, 1 with a blank key\nB x: 1 rows, 1 different keys, 0 with a blank key\n");
    expect(countsText(run, true)).toContain("A and B x\t0\t0\t0\n");
  });
});
