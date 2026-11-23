import { describe, expect, it } from "vitest";
import { Tab, exportHandTab, importHandTab } from "./tab.svelte";
import type { DocInfo } from "./backend";
import { single } from "./selection";

const info = (id: number, pos = 0): DocInfo => ({ id, rows: 1000, cols: 6, total_rows: 1000, history_pos: pos, has_filter: false, filtered: false }) as unknown as DocInfo;

describe("a tab's look and place move to another window", () => {
  it("keeps widths, hidden columns, panes, header, selection, scroll, split, sort and filter", () => {
    const a = new Tab(info(7), 100);
    a.columns.setWidth(1, 180);
    a.columns.setWidth(3, 0);
    a.frozenRows = 2;
    a.frozenCols = 1;
    a.header = true;
    a.headerTouched = true;
    a.banded = true;
    a.rowLines = 2;
    a.scrollX = 40;
    a.scrollY = 1234;
    a.sel = { ...single({ row: 20, col: 2 }), focus: { row: 25, col: 3 } };
    a.split = { dir: "cols", at: 0.4 };
    a.splitScroll = { scrollX: 5, scrollY: 600 };
    a.sort = { keys: [{ col: 2, descending: true }] } as never;
    a.sortAt = 3;
    a.filter = { columns: [{ col: 1, rule: { type: "values", values: ["x"], blanks: false } }] } as never;
    a.highlights = [{ col: 1, rule: { op: "blank" } as never, color: 1, whole_row: false }];
    a.facetCards = [{ col: 2, kind: "values", range: null }];
    // Through JSON, as the hand-over goes.
    const sent = JSON.parse(JSON.stringify(exportHandTab(a)));
    const b = new Tab(info(7), 100);
    importHandTab(b, sent);
    expect(b.columns.width(1)).toBe(180);
    expect(b.columns.isHidden(3)).toBe(true);
    expect(b.frozenRows).toBe(2);
    expect(b.frozenCols).toBe(1);
    expect(b.header).toBe(true);
    expect(b.headerTouched).toBe(true);
    expect(b.banded).toBe(true);
    expect(b.rowLines).toBe(2);
    expect([b.scrollX, b.scrollY]).toEqual([40, 1234]);
    expect(b.sel.focus).toEqual({ row: 25, col: 3 });
    expect(b.sel.anchor).toEqual({ row: 20, col: 2 });
    expect(b.split).toEqual({ dir: "cols", at: 0.4 });
    expect(b.splitScroll).toEqual({ scrollX: 5, scrollY: 600 });
    expect(b.sort).toEqual(a.sort);
    expect(b.sortAt).toBe(3);
    expect(b.filter).toEqual(a.filter);
    expect(b.highlights).toEqual(a.highlights);
    expect(b.facetCards).toEqual([{ col: 2, kind: "values", range: null }]);
  });

  it("is plain data: nothing from the file's cells is in it", () => {
    const a = new Tab(info(1), 100);
    const h = exportHandTab(a) as unknown as Record<string, unknown>;
    expect(Object.keys(h).sort()).not.toContain("cache");
    expect(JSON.stringify(h)).not.toContain("cells");
  });
});

const sources = import.meta.glob(["../**/*.ts", "../**/*.svelte", "!../**/*.test.ts"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

describe("one way to listen for app events", () => {
  it("no page imports the default event listener (it hears other windows' events)", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20);
    const bad = Object.entries(sources)
      .filter(([, text]) => /@tauri-apps\/api\/event/.test(text))
      .map(([name]) => name);
    expect(bad).toEqual([]);
  });
});
