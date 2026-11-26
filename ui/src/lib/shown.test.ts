import { describe, expect, it } from "vitest";
import { hiddenWords, kindOfOp, sortWords } from "./shown.svelte";

describe("what is said about hidden rows", () => {
  it("says nothing while every row shows", () => {
    expect(hiddenWords("all", 0, 10)).toBe("");
    expect(hiddenWords("empty", 0, 10)).toBe("");
  });
  it("names the counts", () => {
    expect(hiddenWords("all", 3, 10)).toBe("This changes all 10 rows, including 3 hidden ones, because the source column is changed too.");
    expect(hiddenWords("empty", 1, 10)).toBe("1 hidden row left empty.");
    expect(sortWords(3, 10)).toBe("Sorting the 7 shown rows; 3 hidden rows stay where they are.");
    expect(sortWords(1, 2)).toBe("Sorting the 1 shown row; 1 hidden row stays where it is.");
  });
  it("knows which operations reach hidden rows", () => {
    for (const op of ["split_column", "merge_columns", "split_name", "extract"]) expect(kindOfOp(op)).toBe("all");
    for (const op of ["count_repeats", "group_calc", "check_column", "compare_column"]) expect(kindOfOp(op)).toBe("empty");
    expect(kindOfOp("clean")).toBeNull();
  });
});
