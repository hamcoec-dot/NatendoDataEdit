import { describe, expect, it } from "vitest";
import { breaks, cautions, copyText, findingText, percentText, type Finding, type RelationsReport } from "./relations";

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
