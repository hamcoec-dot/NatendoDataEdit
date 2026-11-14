import { describe, expect, it } from "vitest";
import { tableLabel, tableName } from "./sqlite";

describe("tableName", () => {
  it("makes a plain name from the tab's name", () => {
    expect(tableName("Voters 2026.csv")).toBe("Voters_2026");
    expect(tableName("--.csv")).toBe("data");
    expect(tableName("sqlite_master.csv")).toBe("t_sqlite_master");
  });
});

describe("tableLabel", () => {
  it("shows counts for tables only", () => {
    expect(tableLabel({ name: "a", rows: 1200, view: false })).toBe("a (1,200 rows)");
    expect(tableLabel({ name: "v", rows: null, view: true })).toBe("v (view)");
    expect(tableLabel({ name: "b", rows: null, view: false })).toBe("b");
  });
});
