import { describe, expect, it } from "vitest";
import { columnRef, engineTitles, remapChecks, remapExpr, renameInChecks, renameInExpr, resolveColumn } from "./rename";
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
  it("names untitled columns Column N", () => {
    expect(engineTitles(["", "id"])).toEqual(["Column 1", "id"]);
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
  it("pins to a number when the new name is another column's too", () => {
    // Old, id, City; rename Old to City: [City] meant column 3, [Old] column 1.
    const t = ["Old", "id", "City"];
    expect(renameInExpr('[City] <> "" AND [Old] > 1', t, 0, "City")).toBe('[#3] <> "" AND [City] > 1');
    // The other order: the renamed column comes after the one it now matches.
    const u = ["City", "id", "Old"];
    expect(renameInExpr("[Old] > 1 AND [City] = 2", u, 2, "City")).toBe("[#3] > 1 AND [City] = 2");
    // A clash only in case: exact names win, so each still finds its own.
    expect(renameInExpr("[Old] = [City]", u, 2, "city")).toBe("[city] = [City]");
    expect(renameInExpr("[city]", ["Old", "id", "City"], 0, "city")).toBe("[City]");
  });
  it("follows an untitled column (Column N) when it gets a name", () => {
    expect(renameInExpr("[Column 1] = 1", ["", "id"], 0, "Age")).toBe("[Age] = 1");
    // Its name taken away: back to Column N.
    expect(renameInExpr("[Age] = 1", ["Age", "id"], 0, "")).toBe("[Column 1] = 1");
  });
});

describe("remapExpr (saved views by place)", () => {
  it("rewrites a view's names for the columns it now uses", () => {
    const view = ["id", "first_name", "city"];
    const file = ["id", "given name", "town"];
    const r = remapExpr("[first_name] <> [city] AND [id] > 0", view, file, (k) => k);
    expect(r).toEqual({ expr: "[given name] <> [town] AND [id] > 0", pinned: 0 });
    // A column left out: its references are left as they are.
    expect(remapExpr("[city] = 1", view, file, (k) => (k === 2 ? null : k)).expr).toBe("[city] = 1");
  });
});

describe("renameInChecks / remapChecks", () => {
  it("changes only row rules that use the column", () => {
    const checks: Check[] = [
      { col: 1, kind: { type: "not_blank" } } as unknown as Check,
      { col: null, kind: { type: "expression", expr: "[first_name] <> [id]" } },
      { col: null, kind: { type: "expression", expr: "[id] > 0" } },
    ];
    const r = renameInChecks(checks, titles, 1, "given name");
    expect(r.changed).toBe(1);
    expect(r.pinned).toBe(0);
    expect(r.checks[1]).toEqual({ col: null, kind: { type: "expression", expr: "[given name] <> [id]" } });
    expect(r.checks[0]).toBe(checks[0]);
    expect(r.checks[2]).toBe(checks[2]);
    const pin = remapChecks([{ col: null, kind: { type: "expression", expr: "[City] = 1" } }], ["Old", "City"], ["City", "City"], (k) => k);
    expect(pin.pinned).toBe(1);
  });
});
