import { describe, expect, it } from "vitest";
import { pinColumn, recordList, togglePin } from "./record";

const titles = ["Name", "City", "Notes", "Name", "Secret"];
const values = ["Ann", "Oslo", "likes tea", "Bo", "x"];
const base = { n: 5, titles, values, hidden: (c: number) => c === 4, showHidden: false, query: "", pins: [] as string[] };

describe("record list", () => {
  it("lists columns in order and leaves hidden ones out unless asked", () => {
    expect(recordList(base).order).toEqual([0, 1, 2, 3]);
    expect(recordList({ ...base, showHidden: true }).order).toEqual([0, 1, 2, 3, 4]);
  });
  it("filters by title or value, ignoring case", () => {
    expect(recordList({ ...base, query: "CITY" }).order).toEqual([1]);
    expect(recordList({ ...base, query: "tea" }).order).toEqual([2]);
    expect(recordList({ ...base, query: "zzz" }).order).toEqual([]);
  });
  it("puts pinned fields first in pin order and pins the first of two equal titles", () => {
    const l = recordList({ ...base, pins: ["Notes", "Name"] });
    expect(l.order).toEqual([2, 0, 1, 3]);
    expect([...l.pinned].sort()).toEqual([0, 2]);
    expect(pinColumn(titles, 5, "Name")).toBe(0);
    expect(pinColumn(titles, 5, "Gone")).toBeNull();
  });
  it("a pin whose title is gone shows nothing and a pinned field still obeys the search", () => {
    expect(recordList({ ...base, pins: ["Gone"] }).order).toEqual([0, 1, 2, 3]);
    expect(recordList({ ...base, pins: ["Notes"], query: "oslo" }).order).toEqual([1]);
  });
  it("toggles pins by title", () => {
    expect(togglePin([], titles, 1)).toEqual(["City"]);
    expect(togglePin(["City", "Notes"], titles, 1)).toEqual(["Notes"]);
  });
  it("names an untitled column the way the grid does", () => {
    expect(recordList({ ...base, titles: ["", "B"], n: 2, hidden: () => false, values: ["a", "b"], query: "column 1" }).order).toEqual([0]);
  });
});
