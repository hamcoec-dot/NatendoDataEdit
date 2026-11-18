import { describe, expect, it } from "vitest";
import type { Histogram, Insight, Profile } from "./backend";
import { barEdges, blankRule, pickedRange, rangeRule, toggleValue, FACET_BINS } from "./facets";
import { insightText, shortCount } from "./insights";
import { binExpr, binLabel, outlierExpr, shapeExpr, shapeOf } from "./profile";

describe("column profile helpers", () => {
  it("shapes values as the engine does, cut at 40 characters", () => {
    expect(shapeOf("37421-0042")).toBe("99999-9999");
    expect(shapeOf("Ab 1")).toBe("AA 9");
    const long = "x".repeat(45);
    expect(shapeOf(long)).toBe("A".repeat(40) + "…");
    expect(shapeExpr("[zip]", "A".repeat(40) + "…")).toBe(`LEFT(SHAPE([zip]), 40) = "${"A".repeat(40)}"`);
  });

  it("quotes a shape holding a quote", () => {
    expect(shapeExpr("[x]", 'A"9')).toBe('SHAPE([x]) = "A""9"');
  });

  it("gives the first and last bars open ends", () => {
    const h: Histogram = { kind: "numbers", low: 0, high: 10, counts: [1, 2, 3, 4, 5] };
    expect(binLabel(h, 0)).toBe("up to 2");
    expect(binLabel(h, 4)).toBe("8 and over");
    expect(binExpr("[n]", h, 0)).toBe("VALUE([n]) < 2");
    expect(binExpr("[n]", h, 4)).toBe("VALUE([n]) >= 8");
    expect(binExpr("[n]", h, 2)).toBe("AND(VALUE([n]) >= 4, VALUE([n]) < 6)");
    const d: Histogram = { kind: "dates", low: 19_723, high: 19_724, counts: [1] };
    expect(binExpr("[d]", d, 0)).toBe('AND(DATEVALUE([d]) >= DATEVALUE("2024-01-01"), DATEVALUE([d]) <= DATEVALUE("2024-01-02"))');
  });

  it("writes a negative mean so the expression still reads", () => {
    const p = { mean: -5, sd: 2 } as Profile;
    expect(outlierExpr("[t]", p)).toBe("ABS(VALUE([t]) - (-5)) > 8");
    expect(outlierExpr("[t]", { mean: 3, sd: 0 } as Profile)).toBeNull();
  });
});

describe("filter panel helpers", () => {
  it("ticks and unticks values, blanks included", () => {
    let r = toggleValue(null, "D", true);
    expect(r).toEqual({ type: "values", values: ["D"], blanks: false });
    r = toggleValue(r, "", true);
    expect(r).toEqual({ type: "values", values: ["D"], blanks: true });
    r = toggleValue(toggleValue(r, "D", false), "", false);
    expect(r).toBeNull();
  });

  it("splits a range into bars, the last one closed", () => {
    expect(barEdges([0, 24], 0)).toEqual([0, 1]);
    expect(barEdges([0, 24], FACET_BINS - 1)).toEqual([23, 24]);
    const r = rangeRule("numbers", 1.23456789, 2);
    expect(pickedRange(r, "numbers")).toEqual(["1.234568", "2"]);
    expect(pickedRange(rangeRule("dates", 19_723, 19_724), "dates")).toEqual(["2024-01-01", "2024-01-02"]);
    expect(blankRule(true)).toEqual({ type: "conditions", first: { type: "blank" } });
  });
});

describe("insight strip text", () => {
  it("shortens counts and says when every value differs", () => {
    expect([shortCount(950), shortCount(1200), shortCount(34_000), shortCount(1_500_000)]).toEqual(["950", "1.2k", "34k", "1.5M"]);
    const i: Insight = { col: 0, filled: 0.996, distinct: 3, unique: false, numbers: false, bars: [1] };
    expect(insightText(i, 100, 1000)).toBe("99% · 3+");
    expect(insightText({ ...i, filled: 1, unique: true }, 100, 100)).toBe("100% · all different");
    expect(insightText({ ...i, filled: 0.001 }, 100, 100)).toBe("1% · 3");
  });
});
