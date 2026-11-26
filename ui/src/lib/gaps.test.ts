import { describe, expect, it } from "vitest";
import { copyText, counted, fileRows, grouped, rangeText, rowsText, type GapReport } from "./gaps";

const range = { series: [], from: "INV-01005", to: "INV-01009", count: "5" };

describe("gap report words", () => {
  it("matches the engine's range lines", () => {
    expect(rangeText(range)).toBe("INV-01005 to INV-01009 (5 missing)");
    expect(rangeText({ series: ["12", "A"], from: "7", to: "7", count: "1" })).toBe("12 / A: 7 (1 missing)");
  });

  it("copies range lines only and says when some are left out", () => {
    const r = { ranges: [range], range_total: 3 } as unknown as GapReport;
    expect(copyText(r)).toBe("INV-01005 to INV-01009 (5 missing)\n2 more ranges not copied\n");
  });

  it("keeps huge counts exact and names hand-added rows", () => {
    expect(grouped("9223372036854775807").replace(/\D/g, "")).toBe("9223372036854775807");
    expect(rowsText([5, null])).toBe("5, added by hand");
    expect([counted(1, "row"), counted(0, "row"), fileRows([7]), fileRows([7, 9]), fileRows([])]).toEqual(["1 row", "0 rows", " (file row 7)", " (file rows 7, 9)", ""]);
  });
});
