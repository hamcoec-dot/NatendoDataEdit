import { describe, expect, it } from "vitest";
import type { DateScan } from "./backend";
import { datePill, dateSummary, dateTip, dateWhy } from "./dates";

const scan = (p: Partial<DateScan>) =>
  ({ looked: 10, rows: 10, blank: 0, iso: 0, written: 0, month_first_only: 0, day_first_only: 0, ambiguous: 0, same_either: 0, compact: 0, compact_numbers: 0, not_date: 0, two_digit_year: 0, set_order: null, other_order: 0, ...p }) as DateScan;

describe("date radar", () => {
  it("says both readings of an ambiguous date", () => {
    expect(dateTip("03/04/2026")).toBe("03/04/2026 could be 4 March or 3 April");
    expect(dateTip(" 4-7-26 14:00")).toContain("could be 7 April or 4 July");
    expect(dateTip("25/04/2026")).toContain("other way round");
  });
  it("greys Text and Id columns with the reason", () => {
    expect(dateWhy("text")).toContain("Text");
    expect(dateWhy("id")).toContain("Id");
    expect(dateWhy("auto")).toBe("");
  });
  it("warns about mixed clues, and about none", () => {
    expect(dateSummary(scan({ ambiguous: 3, month_first_only: 2, day_first_only: 1 }), "born")).toContain("mixes both orders");
    expect(dateSummary(scan({ ambiguous: 3 }), "born")).toContain("Nothing in this column tells the order");
    expect(dateSummary(scan({ ambiguous: 1, compact_numbers: 8 }), "born")).toContain("8 eight-digit values could be dates and are taken as numbers");
    expect(dateSummary(scan({ looked: 200000, rows: 500000, ambiguous: 1 }), "born")).toContain("from the first 200,000 of 500,000 rows");
  });
  it("answers a column that is already set to an order", () => {
    const s = scan({ set_order: "dmy", other_order: 12 });
    expect(dateSummary(s, "born")).toBe("born is set to day first. 12 cells can only be read month first.");
    expect(datePill(s)).toBe("Dates the other way round: 12");
    expect(datePill(scan({ ambiguous: 340 }))).toBe("Ambiguous dates: 340");
  });
});
