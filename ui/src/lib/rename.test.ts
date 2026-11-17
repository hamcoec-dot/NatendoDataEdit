import { describe, expect, it } from "vitest";
import { columnRef, renameInChecks, renameInExpr, resolveColumn } from "./rename";
import type { Check } from "./backend";

const titles = ["id", "first_name", "Age", "first_name", "a]b"];

describe("resolveColumn", () => {
  it("finds columns as the engine does", () => {
    expect(resolveColumn("first_name", titles)).toBe(1);
    expect(resolveColumn("AGE", titles)).toBe(2);
    expect(resolveColumn("#4", titles)).toBe(3);
    expect(resolveColumn("#9", titles)).toBeNull();
    expect(resolveColumn("nope", titles)).toBeNull();
  });
});

describe("renameInExpr", () => {
  it("renames references to the column, case-insensitive ones too", () => {
    expect(renameInExpr("[first_name] <> \"\" AND [age] > 1", titles, 1, "given")).toBe('[given] <> "" AND [age] > 1');
    expect(renameInExpr("LEN([FIRST_NAME])", titles, 1, "given")).toBe("LEN([given])");
    expect(renameInExpr("[age] > 18", titles, 2, "years")).toBe("[years] > 18");
  });
  it("leaves other columns, text, #n and broken expressions alone", () => {
    expect(renameInExpr('"[first_name]" = [id]', titles, 1, "given")).toBe('"[first_name]" = [id]');
    expect(renameInExpr("[#2] = 1", titles, 1, "given")).toBe("[#2] = 1");
    expect(renameInExpr("[first_name", titles, 1, "given")).toBe("[first_name");
    expect(renameInExpr('"open', titles, 1, "given")).toBe('"open');
    // The second first_name column is not what [first_name] means.
    expect(renameInExpr("[first_name]", titles, 3, "x")).toBe("[first_name]");
  });
  it("reads and writes ] doubled", () => {
    expect(renameInExpr("[a]]b] = 1", titles, 4, "c]d")).toBe("[c]]d] = 1");
    expect(columnRef("x]")).toBe("[x]]]");
  });
});

describe("renameInChecks", () => {
  it("changes only row rules that use the column", () => {
    const checks: Check[] = [
      { col: 1, kind: { type: "not_blank" } } as unknown as Check,
      { col: null, kind: { type: "expression", expr: "[first_name] <> [id]" } },
      { col: null, kind: { type: "expression", expr: "[id] > 0" } },
    ];
    const r = renameInChecks(checks, titles, 1, "given name");
    expect(r.changed).toBe(1);
    expect(r.checks[1]).toEqual({ col: null, kind: { type: "expression", expr: "[given name] <> [id]" } });
    expect(r.checks[0]).toBe(checks[0]);
    expect(r.checks[2]).toBe(checks[2]);
  });
});
