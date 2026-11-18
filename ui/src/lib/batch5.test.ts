import { describe, expect, it } from "vitest";
import { toolsRequest, type ToolChoices } from "./m10";
import { exportDefaults } from "./transfer";

const base: ToolChoices = {
  cols: [],
  col: 0,
  header: true,
  from: "above",
  text: "",
  spaces: true,
  matchCase: false,
  trim: true,
  byCol: true,
  every: 100,
  blankRows: 1,
  perRow: 3,
  down: false,
  skipBlanks: true,
  sizeKind: "rows",
  size: 10,
  seed: 2024,
  other: -1,
  wholeRow: false,
  showSame: false,
  byTitle: true,
  source: true,
};

describe("audit samples", () => {
  it("draws per group with the seed, row numbers and order", () => {
    const r = toolsRequest("sample", { ...base, per: 1, rowNumbers: true, drawnOrder: false });
    expect(r).toEqual({
      table: { op: "sample", spec: { size: { kind: "rows", n: 10 }, seed: 2024, keep_first: true, per: 1, drawn_order: false, row_numbers: true } },
    });
    const all = toolsRequest("sample", { ...base, per: -1 });
    expect(typeof all !== "string" && all.table?.op === "sample" && all.table.spec.per).toBeNull();
  });

  it("refuses every nth row per group", () => {
    expect(toolsRequest("sample", { ...base, sizeKind: "every", size: 5, per: 2 })).toMatch(/per group/);
  });
});

describe("Excel export defaults", () => {
  it("are made for Excel users, with parts as big as a sheet", () => {
    const o = exportDefaults(true, "voters");
    expect(o.format).toBe("xlsx");
    expect(o.look).toBe(true);
    expect(o.part_rows).toBe(0);
  });
});
