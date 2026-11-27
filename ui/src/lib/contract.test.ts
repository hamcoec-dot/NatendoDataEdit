import { describe, expect, it } from "vitest";
import type { Contract, DriftReport } from "./backend";
import { acceptValues, addAlias, driftSummary, movesFor, updateBaseline } from "./contractText";
import { drift, nameProblem, pair, patternMatches, recognise } from "./contractMock";

const contract = (): Contract => ({
  contract: 1,
  name: "Monthly voters",
  columns: [
    { title: "id", blank_rate: 0 },
    { title: "name", blank_rate: 0 },
    { title: "party", allowed: { values: ["DEM", "REP"], match_case: true }, blank_rate: 0 },
    { title: "phone", blank_rate: 0.25 },
  ],
  rows: { min: 2, max: 4 },
  recognise: { pattern: "voters_*.csv", titles: true },
  blank_points: 5,
});

describe("file contracts", () => {
  it("pairs renamed, moved, missing and added columns", () => {
    const p = pair(contract(), ["id", "full_name", "phone", "party", "email"]);
    expect(p.map).toEqual([0, 1, 3, 2]);
    expect(p.renamed).toEqual([1]);
    expect(p.added).toEqual([4]);
    expect(p.moved.length).toBe(1);
    expect(pair(contract(), ["id", "a", "b", "party", "phone"]).missing).toEqual([1]);
  });

  it("recognises by pattern and titles", () => {
    expect(patternMatches("voters_*.csv", "VOTERS_feb.CSV")).toBe(true);
    expect(patternMatches("voters_?.csv", "voters_12.csv")).toBe(false);
    const c = contract();
    expect(recognise([c], "x.csv", ["id", "name", "party", "phone"])[0]?.by_pattern).toBe(false);
    expect(recognise([c], "x.csv", ["id", "name", "party"])).toEqual([]);
    expect(recognise([c], "voters_1.csv", null)[0]?.by_pattern).toBe(true);
  });

  it("summarises a drift report", () => {
    const c = contract();
    const titles = ["id", "full_name", "party", "phone"];
    const rows = [
      ["1", "Ann", "DEM", ""],
      ["2", "Bo", "GRN", ""],
      ["3", "Cy", "LIB", "5"],
      ["4", "Di", "GRN", ""],
      ["5", "Ed", "REP", "7"],
    ];
    const r = drift(c, pair(c, titles), titles, rows);
    expect(r.drifted).toBe(true);
    expect(r.columns[2]!.first_new).toEqual(["GRN", "LIB"]);
    expect(driftSummary(r)).toBe("1 renamed column, 3 new values in party, blanks in phone up 35 points, 5 rows (expected 2 to 4)");
    const ok = drift(c, pair(c, ["id", "name", "party", "phone"]), ["id", "name", "party", "phone"], [["1", "A", "DEM", "1"], ["2", "B", "REP", ""], ["3", "C", "DEM", "3"], ["4", "D", "REP", "4"]]);
    expect(driftSummary(ok)).toBe("nothing drifted");
  });

  it("accepts values, updates the baseline and keeps an alias", () => {
    const c = contract();
    const titles = ["id", "full_name", "party", "phone"];
    const r: DriftReport = drift(c, pair(c, titles), titles, [["1", "A", "GRN", ""]]);
    expect(acceptValues(c, r, [2]).contract.columns[2]!.allowed!.values).toEqual(["DEM", "REP", "GRN"]);
    const b = updateBaseline(c, r);
    expect(b.columns[3]!.blank_rate).toBe(1);
    expect(b.rows).toEqual({ min: 0, max: 2 });
    expect(addAlias(c, 1, "full_name").columns[1]!.aliases).toEqual(["full_name"]);
    expect(c.columns[1]!.aliases).toBeUndefined();
  });

  it("turns an order into single moves", () => {
    for (const order of [[0, 1, 3, 2, 4], [3, 0, 1, 2], [2, 1, 0], [0, 1, 2]]) {
      const cols = order.map((_, i) => i);
      for (const m of movesFor(order)) cols.splice(m.to, 0, ...cols.splice(m.at, 1));
      expect(cols).toEqual(order);
    }
  });

  it("refuses names that cannot be file names", () => {
    for (const bad of ["a/b", "a:b", "CON", "nul.txt", "x.", "x ", "", "a?"]) expect(nameProblem(bad)).not.toBeNull();
    expect(nameProblem("Monthly voters")).toBeNull();
  });
});
