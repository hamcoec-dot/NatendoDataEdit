import { describe, expect, it } from "vitest";
import type { Check } from "./backend";
import { patternChecks, withPatterns } from "./fieldcheck";

const pat = (col: number, text: string): Check => ({ col, kind: { type: "pattern", pattern: { text, kind: "regex", match_case: false, whole_cell: true } } });

describe("field checks", () => {
  it("reads and writes one box per column, keeping other checks", () => {
    const checks: Check[] = [{ col: 0, kind: { type: "required" } }, pat(1, "\\d{5}"), pat(1, "x")];
    expect(patternChecks(checks, 3)).toEqual(["", "\\d{5}", ""]);
    const out = withPatterns(checks, ["", "\\d{5}(-\\d{4})?", "[a-z]+"], false);
    expect(out[0]).toEqual({ col: 0, kind: { type: "required" } });
    expect(out[1]!.kind.type === "pattern" && out[1]!.kind.pattern.text).toBe("\\d{5}(-\\d{4})?");
    expect(out[1]!.kind.type === "pattern" && out[1]!.kind.pattern.whole_cell).toBe(false);
    // The second check on column 1 is not a box: it stays.
    expect(out[2]).toEqual(pat(1, "x"));
    expect(out[3]).toEqual({ col: 2, kind: { type: "pattern", pattern: { text: "[a-z]+", kind: "regex", match_case: false, whole_cell: false } } });
    // An emptied box removes its check.
    expect(withPatterns([pat(1, "a")], ["", ""], true)).toEqual([]);
  });
});
