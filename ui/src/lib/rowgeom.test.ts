import { describe, expect, it } from "vitest";

const sources = import.meta.glob(["../**/*.ts", "../**/*.svelte", "!../**/*.test.ts"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

const START = "// ---- row geometry";
const END = "// ---- end of row geometry";

describe("how tall a row is is decided in one place", () => {
  it("the old row-height constant is gone from every file", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20);
    for (const [f, t] of Object.entries(sources)) expect(/\browH\b/.test(t), `${f} still uses rowH`).toBe(false);
  });

  it("the plain row height (uniformRowH) is used only inside the geometry layer of the grid", () => {
    let inGrid = 0;
    for (const [f, t] of Object.entries(sources)) {
      if (/\/Grid\.svelte$/.test(f)) {
        const a = t.indexOf(START);
        const b = t.indexOf(END);
        expect(a, "geometry layer start marker").toBeGreaterThan(-1);
        expect(b, "geometry layer end marker").toBeGreaterThan(a);
        const outside = t.slice(0, a) + t.slice(b);
        expect(/\buniformRowH\b/.test(outside), "Grid.svelte uses uniformRowH outside its geometry layer").toBe(false);
        inGrid = (t.slice(a, b).match(/\buniformRowH\b/g) ?? []).length;
      } else {
        expect(/\buniformRowH\b/.test(t), `${f} uses uniformRowH`).toBe(false);
      }
    }
    expect(inGrid).toBeGreaterThan(5);
  });

  it("no other file keeps its own row-height constant (the find panel's result list is its own list)", () => {
    for (const [f, t] of Object.entries(sources)) {
      if (/\/FindPanel\.svelte$/.test(f)) continue;
      expect(/\b(ROW_H|ROW_HEIGHT|rowHeightPx|ROW_PX)\b/.test(t), `${f} has a row-height constant`).toBe(false);
    }
  });
});
