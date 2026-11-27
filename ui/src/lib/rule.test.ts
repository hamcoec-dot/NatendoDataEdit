import { describe, expect, it } from "vitest";
import { describe as words, mockFormula, mockLearn, oddWords, piecePattern, ruleProblem, runParts, tokens, type Program } from "./rule";

const names = (c: number) => ["first", "last", "short"][c] ?? `column ${c + 1}`;

describe("the rule card's rule", () => {
  it("cuts values as the engine does", () => {
    expect(tokens("Ann  Lee", "part")).toEqual(["Ann", "Lee"]);
    expect(tokens("ab12-cd", "word")).toEqual(["ab", "cd"]);
    expect(tokens("ab12-cd34", "number")).toEqual(["12", "34"]);
    expect(tokens("no dash", { split: { by: "-" } })).toEqual([]);
    expect(tokens("", "whole")).toEqual([]);
  });

  it("learns initials and says them in words", () => {
    const p = mockLearn(
      [
        { row: ["Ann", "Lee", ""], out: "A.Lee" },
        { row: ["Bo", "Kim", ""], out: "B.Kim" },
      ],
      [0, 1],
    );
    expect(p).not.toBeNull();
    expect(runParts(p!, (c) => ["Cy", "Hart"][c] ?? null)).toEqual(["C.Hart", false]);
    expect(words(p!, names)).toBe('first letter of first + "." + last');
  });

  it("refuses a rule that cannot run", () => {
    const p: Program = { pieces: [{ kind: "from", col: 2, cut: "whole", index: 0, first: false, case: "keep" }] };
    expect(ruleProblem(p, 3, 2)).toMatch(/column it fills/);
    expect(ruleProblem(p, 2, 1)).toMatch(/no longer has/);
    expect(ruleProblem({ pieces: [] }, 3, 2)).toMatch(/no pieces/);
    expect(ruleProblem({ pieces: [{ kind: "text", text: "" }] }, 3, 2)).toMatch(/empty/);
  });

  it("writes the formula the engine writes, and says why there is none", () => {
    expect(piecePattern({ split: { by: "." } }, 1, true)).toBe("^(?:[^\\.]*\\.){1}([^\\.]+)");
    expect(piecePattern("word", -1, false)).toBe("([\\p{Alphabetic}]+)[^\\p{Alphabetic}]*$");
    const p: Program = {
      pieces: [
        { kind: "from", col: 0, cut: "whole", index: 0, first: true, case: "upper" },
        { kind: "text", text: '"' },
      ],
    };
    expect(mockFormula(p, names, () => false)).toEqual({ formula: 'IFERROR(UPPER(LEFT(REGEXEXTRACT([first], "^(.+)$", 1), 1)) & """", "")' });
    expect(mockFormula(p, names, () => true)).toEqual({ why: "first is read as numbers or dates, so a formula would change its text" });
  });

  it("counts the odd results in plain words", () => {
    const none = { fills: 3, missed: 0, empty_piece: 0, odd_shape: 0, odd_length: 0, rows: [], odd_rows: 0 };
    expect(oddWords(none)).toBe("None of the results look odd.");
    expect(oddWords({ ...none, odd_shape: 2, odd_length: 1, odd_rows: 2 })).toBe("2 results look odd (2 shaped unlike the examples, 1 far longer or shorter).");
  });
});
