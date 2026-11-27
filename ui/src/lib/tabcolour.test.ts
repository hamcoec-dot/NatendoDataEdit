import { describe, expect, it } from "vitest";
import { cleanColour, copyName, namedView, rememberable, tabLabel, TAB_COLOURS } from "./tabcolour";
import { sessionView, type View } from "./views";

const view = (colour?: number): View => ({
  version: 1,
  name: "v",
  header: true,
  columns: [],
  freeze_rows: 0,
  freeze_cols: 0,
  sort: null,
  filter: null,
  highlights: [],
  tab_colour: colour,
});

describe("tab colour", () => {
  it("takes only a known colour number", () => {
    expect(cleanColour(3)).toBe(3);
    for (const bad of [-1, TAB_COLOURS.length, 1.5, "2", null, undefined]) expect(cleanColour(bad)).toBeNull();
  });
  it("is said to screen readers", () => {
    expect(tabLabel("march.csv", 3)).toBe("march.csv, tab colour green");
    expect(tabLabel("march.csv", null)).toBeUndefined();
  });
  it("is kept with the session view but left out of named views", () => {
    expect(sessionView(view(2)).tab_colour).toBe(2);
    expect("tab_colour" in namedView(view(2))).toBe(false);
  });
  it("of a private tab never reaches the remembered session", () => {
    const tab = (path: string | null, file: boolean, folder: boolean) => ({ info: { path }, private: { file, folder } });
    expect(rememberable(tab("C:\d\a.csv", false, false))).toBe(true);
    expect(rememberable(tab("C:\d\a.csv", true, false))).toBe(false);
    expect(rememberable(tab("C:\d\a.csv", false, true))).toBe(false);
    // A Duplicate has no file until Save As.
    expect(rememberable(tab(null, false, false))).toBe(false);
  });
});

describe("the copy's name", () => {
  it("counts up past names in use", () => {
    expect(copyName("v.csv", ["v.csv"])).toBe("v.csv (copy)");
    expect(copyName("v.csv", ["v.csv", "V.CSV (copy)"])).toBe("v.csv (copy 2)");
    expect(copyName("v.csv (copy)", ["v.csv", "v.csv (copy)"])).toBe("v.csv (copy 2)");
    expect(copyName("v.csv (copy 2)", ["v.csv (copy)", "v.csv (copy 2)", "v.csv (copy 3)"])).toBe("v.csv (copy 4)");
  });
});
