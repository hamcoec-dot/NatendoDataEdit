import { describe, expect, it } from "vitest";
import type { DocInfo } from "./backend";
import { raggedAtOpen, raggedNotice, raggedResult } from "./ragged";

const info = (p: Partial<DocInfo>) => ({ usual_fields: 14, ragged_rows: 212, first_row_ragged: false, column_changes: false, ...p }) as DocInfo;

describe("ragged rows", () => {
  it("counts the file's rows, leaving out a title row of another width", () => {
    expect(raggedAtOpen(info({}), true)).toBe(212);
    expect(raggedAtOpen(info({ first_row_ragged: true }), true)).toBe(211);
    expect(raggedAtOpen(info({ first_row_ragged: true }), false)).toBe(212);
    expect(raggedAtOpen({} as DocInfo, true)).toBe(0);
  });
  it("says the count and the usual width, and nothing after a column change", () => {
    expect(raggedNotice(info({}), true)).toBe("212 rows have a different number of fields than most (usual: 14).");
    expect(raggedNotice(info({ ragged_rows: 1 }), true)).toContain("1 row has");
    expect(raggedNotice(info({ ragged_rows: 0 }), true)).toBe("");
    expect(raggedNotice(info({ column_changes: true }), true)).toBe("");
  });
  it("names short and long rows apart", () => {
    expect(raggedResult("all", 5, 2)).toContain("(2 short, 3 long)");
    expect(raggedResult("short", 2, 2)).toContain("too few fields");
    expect(raggedResult("long", 0, 0)).toBe("No row has too many fields.");
  });
});
