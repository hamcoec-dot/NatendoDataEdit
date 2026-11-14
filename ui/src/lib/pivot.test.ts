import { describe, expect, it } from "vitest";
import { allOpen, fieldName, insertValue, moveValue, removeValue, looksNumeric, mockDrill, mockItems, mockPivot, newField, newLayout, newSpec, newValue, scanKey, toggled, valueName } from "./pivot";

const data = [
  ["N", "Jan", "10"],
  ["S", "Jan", "5"],
  ["N", "Feb", "7"],
  ["n", "Jan", "1"],
];
const titles = ["region", "month", "sales"];

describe("pivot helpers", () => {
  it("guesses Sum for number columns", () => {
    expect(looksNumeric(["1", "2.5", "$3", ""])).toBe(true);
    expect(looksNumeric(["a", "2"])).toBe(false);
    expect(looksNumeric([])).toBe(false);
  });

  it("names fields and values like the engine", () => {
    expect(valueName(newValue(2, "sum"), titles)).toBe("Sum of sales");
    expect(valueName({ ...newValue(2, "count_distinct"), name: "Reps" }, titles)).toBe("Reps");
    const f = { ...newField(1), group: { kind: "month" as const }, top: { n: 5, value: 0, bottom: false } };
    expect(fieldName(f, titles)).toBe("month (Months) · top 5");
  });

  it("only fields, groups, picks and summaries need the file read again", () => {
    const a = { ...newSpec(true), rows: [newField(0)], values: [newValue(2, "sum")] };
    const b = structuredClone(a);
    b.values[0]!.show_as = "pct_of_grand";
    b.rows[0]!.sort = { by: "value", value: 0, desc: true };
    b.layout.grand_row = false;
    expect(scanKey(b)).toBe(scanKey(a));
    b.values[0]!.agg = "count";
    expect(scanKey(b)).not.toBe(scanKey(a));
  });

  it("expands and collapses groups", () => {
    let l = newLayout();
    l = toggled(l, true, ["n"], false);
    expect(l.collapsed_rows).toEqual([["n"]]);
    l = toggled(l, true, ["n"], true);
    expect(l.collapsed_rows).toEqual([]);
    l = allOpen(l, true, false);
    expect(l.row_depth).toBe(1);
    l = toggled(l, true, ["s"], true);
    expect(l.expanded_rows).toEqual([["s"]]);
    expect(allOpen(l, true, true)).toMatchObject({ row_depth: 0, expanded_rows: [], collapsed_rows: [] });
  });

  it("browser stand-in pivots, lists items and drills down", () => {
    const s = { ...newSpec(true), rows: [newField(0)], cols: [newField(1)], values: [newValue(2, "sum")] };
    const g = mockPivot(data, titles, s, null);
    expect(g.rows.map((r) => [...r.labels, ...r.cells])).toEqual([
      ["N", "7", "11", "18"],
      ["S", "", "5", "5"],
      ["Grand Total", "7", "16", "23"],
    ]);
    expect(mockItems(data, 0, s).items.map((i) => [i.label, i.rows])).toEqual([
      ["N", 3],
      ["S", 1],
    ]);
    expect(mockDrill(data, s, ["n"], ["jan"]).length).toBe(2);
    expect(mockPivot(data, titles, s, 2).sampled).toBe(true);
  });
});

describe("value field order", () => {
  const spec = () => {
    const s = { ...newSpec(true), rows: [newField(0)], values: [newValue(2, "sum"), newValue(2, "count"), newValue(2, "max")] };
    s.rows[0]!.sort = { by: "value", value: 1, desc: true };
    s.rows[0]!.top = { n: 3, value: 2, bottom: false };
    s.rows[0]!.value_filter = { value: 0, op: "greater", a: 1, b: 0 };
    return s;
  };

  it("moving a value keeps what points at each one", () => {
    const s = spec();
    moveValue(s, 2, 0); // [max, sum, count]
    expect(s.values.map((v) => v.agg)).toEqual(["max", "sum", "count"]);
    expect(s.rows[0]!.sort.value).toBe(2);
    expect(s.rows[0]!.top?.value).toBe(0);
    expect(s.rows[0]!.value_filter?.value).toBe(1);
    moveValue(s, 0, 2); // back: [sum, count, max]
    expect(s.rows[0]).toMatchObject({ sort: { value: 1 }, top: { value: 2 }, value_filter: { value: 0 } });
  });

  it("inserting shifts later references; removing drops its own", () => {
    const s = spec();
    insertValue(s, 1, newValue(2, "average")); // [sum, avg, count, max]
    expect(s.rows[0]!.sort.value).toBe(2);
    expect(s.rows[0]!.top?.value).toBe(3);
    expect(s.rows[0]!.value_filter?.value).toBe(0);
    removeValue(s, 3); // max gone: top N goes
    expect(s.rows[0]!.top).toBeNull();
    removeValue(s, 2); // count gone: sort back to labels
    expect(s.rows[0]!.sort).toEqual({ by: "label", value: 0, desc: false });
    removeValue(s, 0); // sum gone: value filter goes
    expect(s.rows[0]!.value_filter).toBeNull();
  });
});
