import { describe, expect, it } from "vitest";
import { addChecks, addedText, breaks, brokenRows, cautions, checksFor, copyText, findingText, percentText, type Finding, type RelationsReport } from "./relations";
import { placeCheck, shiftChecks, describeCheck } from "./m8";

const title = (c: number) => ["ZIP", "State", "Phone"][c]!;
const holds = { coverage: 1000, agreeing: 997, values: 50, broken_values: 2, percent: 99.7 };
const f: Finding = { kind: "depends", a: 0, b: 1, there: holds, back: null, exact: true, few_rows: false, few_per_value: false };

describe("relationship words", () => {
  it("never rounds up to 100", () => {
    expect([percentText(99.96), percentText(100), percentText(99.7)]).toEqual(["99.9%", "100.0%", "99.7%"]);
  });

  it("says what each kind found, with its coverage", () => {
    expect(findingText(f, title)).toBe("ZIP -> State: holds in 99.7% (997 of 1,000 rows whose ZIP repeats; 2 values break it)");
    expect(findingText({ ...f, kind: "one_to_one", back: { ...holds, broken_values: 1 } }, title)).toContain("ZIP <-> State (one to one): ZIP settles State in 99.7%");
    expect(findingText({ ...f, kind: "co_blank", a: 2, b: 1 }, title)).toBe("Phone and State are blank together in 99.7% of the rows where either is blank (997 of 1,000)");
  });

  it("carries its cautions into the copy", () => {
    const weak = { ...f, exact: false, few_rows: true, few_per_value: true };
    expect(cautions(weak)).toEqual(["estimated from a sample", "based on few rows", "few rows per value"]);
    expect(copyText({ findings: [weak] } as unknown as RelationsReport, title)).toContain("it) [estimated from a sample] [based on few rows] [few rows per value]\n");
    expect(breaks(f)).toBe(true);
    expect(breaks({ ...f, there: { ...holds, agreeing: 1000 } })).toBe(false);
  });
});

describe("make this a check", () => {
  const titles = ["ZIP", "State", "Phone"];
  it("makes one rule for a dependency, two for one to one", () => {
    expect(checksFor(f, titles, true)).toEqual([{ col: 1, name: "State settled by ZIP", kind: { type: "settled_by", by: 0, match_case: true } }]);
    const both = checksFor({ ...f, kind: "one_to_one", back: holds }, titles, true);
    expect(both.map((c) => [c.col, c.kind.type === "settled_by" ? c.kind.by : -1])).toEqual([[1, 0], [0, 1]]);
  });

  it("makes blank together a row rule with the scan's blank (spaces only is blank)", () => {
    const [c] = checksFor({ ...f, kind: "co_blank", a: 2, b: 1 }, titles, true);
    expect(c).toEqual({ col: null, name: "Phone and State blank together", kind: { type: "expression", expr: "ISBLANK(TRIM([Phone])) = ISBLANK(TRIM([State]))" } });
    // Titles with brackets are escaped; a repeated title or no title row goes by number.
    expect(checksFor({ ...f, kind: "co_blank" }, ["a]b", "x", "x"], true)[0]!.kind).toEqual({ type: "expression", expr: "ISBLANK(TRIM([a]]b])) = ISBLANK(TRIM([#2]))" });
    expect(checksFor({ ...f, kind: "co_blank", a: 1, b: 2 }, ["a]b", "x", "x"], true)[0]!.kind).toEqual({ type: "expression", expr: "ISBLANK(TRIM([#2])) = ISBLANK(TRIM([#3]))" });
    expect(checksFor({ ...f, kind: "co_blank" }, titles, false)[0]!.kind).toEqual({ type: "expression", expr: "ISBLANK(TRIM([#1])) = ISBLANK(TRIM([#2]))" });
  });

  it("does not add a rule twice, whatever it is called", () => {
    const made = checksFor(f, titles, true);
    const first = addChecks([], made);
    expect([first.added.length, first.already]).toEqual([1, 0]);
    const again = addChecks([{ ...made[0]!, name: "renamed" }], made);
    expect([again.checks.length, again.added.length, again.already]).toEqual([1, 0, 1]);
  });

  it("says the count, and only matches the scan when it counted every row", () => {
    expect(brokenRows(f)).toBe(3);
    expect(addedText(f, ["State settled by ZIP"], 0, 3)).toBe('Added the check "State settled by ZIP": 3 rows break it. Data › Data checks lists them; F7 goes to the next problem.');
    expect(addedText({ ...f, exact: false }, ["State settled by ZIP"], 0, 4)).toContain(": 4 rows break it (the scan estimated 3).");
    expect(addedText(f, [], 1, null)).toBe("That check is already in Data checks.");
  });

  it("moves with its columns and goes with the settling column", () => {
    const [c] = checksFor(f, titles, true);
    expect(placeCheck(c!, (k) => k + 2)).toEqual({ ...c, col: 3, kind: { type: "settled_by", by: 2, match_case: true } });
    expect(shiftChecks([c!], 0, -1)).toEqual({ checks: [], dropped: 1 });
    expect(describeCheck({ ...c!, name: null }, titles)).toBe("State: the same for each ZIP");
  });
});
