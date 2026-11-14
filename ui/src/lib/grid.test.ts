import tauriConf from "../../../src-tauri/tauri.conf.json?raw";
import { describe, expect, it, vi } from "vitest";
import { decodeViewport, encodeViewport } from "./binary";
import { Columns } from "./columns";
import { BLOCK_ROWS, CellCache } from "./cache";
import { parseTsv, toTsv } from "./tsv";
import { rectOf, contains, clampCell } from "./selection";
import { MockBackend } from "./mock";
import { looksLikeHeader } from "./header";

describe("binary viewport", () => {
  it("round-trips text, empty cells and truncation flags", () => {
    const v = {
      firstRow: 5_000_000_000,
      firstCol: 3,
      rows: 2,
      cols: 2,
      cells: ["a", "", "Ünïcødé 😀", "x\ny"],
      truncated: new Uint8Array([0, 0, 1, 0]),
    };
    const back = decodeViewport(encodeViewport(v));
    expect(back).toEqual(v);
  });

  it("reads a reply sent as a list of numbers (Tauri's fallback channel)", () => {
    const v = {
      firstRow: 7, firstCol: 1, rows: 1, cols: 2, cells: ["Email", "Ünï"], truncated: new Uint8Array(2),
    };
    const list = Array.from(new Uint8Array(encodeViewport(v)));
    expect(decodeViewport(list)).toEqual(v);
  });

  it("lets the page reach Tauri's fast channel (the grid was blank in the installed app)", () => {
    const conf = JSON.parse(tauriConf);
    const csp: string = conf.app.security.csp;
    const connect = csp.split(";").map((d) => d.trim()).find((d) => d.startsWith("connect-src"));
    expect(connect).toBeDefined();
    expect(connect).toContain("ipc:");
    expect(connect).toContain("http://ipc.localhost");
  });

  it("refuses a damaged header before reserving room for its cells", () => {
    const buf = new ArrayBuffer(24);
    const view = new DataView(buf);
    view.setUint32(12, 65_536, true);
    view.setUint32(16, 16_384, true);
    expect(() => decodeViewport(buf)).toThrow(/cut short/);
  });

  it("rejects a cut-short reply instead of reading past it", () => {
    const buf = encodeViewport({
      firstRow: 0, firstCol: 0, rows: 1, cols: 1, cells: ["hello"], truncated: new Uint8Array(1),
    });
    expect(() => decodeViewport(buf.slice(0, buf.byteLength - 2))).toThrow();
    expect(() => decodeViewport(new ArrayBuffer(4))).toThrow();
  });
});

describe("Columns", () => {
  it("handles millions of columns with few overrides", () => {
    const c = new Columns(5_000_000, 100);
    c.setWidth(2, 50);
    c.setWidth(4_000_000, 300);
    expect(c.x(0)).toBe(0);
    expect(c.x(3)).toBe(250);
    expect(c.x(4_000_001)).toBe(4_000_001 * 100 - 50 + 200);
    expect(c.at(260)).toBe(3);
    expect(c.at(c.x(4_000_000) + 250)).toBe(4_000_000);
    expect(c.at(-5)).toBe(0);
    expect(c.at(1e12)).toBe(4_999_999);
  });

  it("hides and skips columns", () => {
    const c = new Columns(5, 10);
    c.setWidth(1, 0);
    c.setWidth(2, 0);
    expect(c.step(0, 1)).toBe(3);
    expect(c.step(3, -1)).toBe(0);
    expect(c.at(10)).toBe(3);
    expect(c.hiddenCount()).toBe(2);
    c.showAll();
    expect(c.hiddenCount()).toBe(0);
    expect(c.x(3)).toBe(30);
  });

  it("shifts widths when columns are inserted or deleted", () => {
    const c = new Columns(5, 10);
    c.setWidth(3, 40);
    c.shift(1, 2);
    expect(c.width(5)).toBe(40);
    c.shift(0, -2);
    expect(c.width(3)).toBe(40);
    c.shift(3, -1);
    expect(c.width(3)).toBe(10);
  });
});

describe("CellCache", () => {
  const fake = (calls: number[][]) => async (row: number, rows: number, col: number, cols: number) => {
    calls.push([row, rows, col, cols]);
    const cells: string[] = [];
    for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) cells.push(`${row + r},${col + c}`);
    return { firstRow: row, firstCol: col, rows, cols, cells, truncated: new Uint8Array(rows * cols) };
  };

  it("fetches each block once and serves cells from it", async () => {
    const calls: number[][] = [];
    const onLoad = vi.fn();
    const cache = new CellCache(fake(calls), onLoad);
    cache.request(0, 10, 0, 5);
    cache.request(0, 10, 0, 5);
    expect(calls.length).toBe(1);
    await Promise.resolve();
    await Promise.resolve();
    expect(cache.get(3, 4)).toBe("3,4");
    expect(cache.get(BLOCK_ROWS + 1, 0)).toBeUndefined();
    expect(onLoad).toHaveBeenCalled();
  });

  it("keeps stale values after invalidate until fresh ones arrive", async () => {
    const calls: number[][] = [];
    const cache = new CellCache(fake(calls), () => {});
    cache.request(0, 1, 0, 1);
    await new Promise((r) => setTimeout(r));
    cache.invalidate();
    expect(cache.get(0, 0)).toBe("0,0");
    cache.request(0, 1, 0, 1);
    expect(calls.length).toBe(2);
  });

  it("evicts least recently used blocks", async () => {
    const cache = new CellCache(fake([]), () => {}, 2);
    for (let b = 0; b < 4; b++) {
      cache.request(b * BLOCK_ROWS, b * BLOCK_ROWS + 1, 0, 1);
      await new Promise((r) => setTimeout(r));
    }
    expect(cache.size).toBe(2);
    expect(cache.get(3 * BLOCK_ROWS, 0)).toBe(`${3 * BLOCK_ROWS},0`);
  });
});

describe("TSV", () => {
  it("round-trips tabs, quotes and line breaks", () => {
    const rows = [["a", "b\tc", 'say "hi"'], ["multi\nline", "", "x"]];
    expect(parseTsv(toTsv(rows))).toEqual(rows);
  });

  it("reads plain spreadsheet text", () => {
    expect(parseTsv("1\t2\r\n3\t4\r\n")).toEqual([["1", "2"], ["3", "4"]]);
    expect(parseTsv("one")).toEqual([["one"]]);
    expect(parseTsv("a\t\n")).toEqual([["a", ""]]);
    expect(parseTsv('"unclosed\tx')).toEqual([['"unclosed', "x"]]);
    expect(parseTsv("")).toEqual([]);
  });
});

describe("selection", () => {
  it("normalises any drag direction", () => {
    const r = rectOf({ anchor: { row: 5, col: 3 }, focus: { row: 2, col: 1 } });
    expect(r).toEqual({ row: 2, col: 1, rows: 4, cols: 3 });
    expect(contains(r, 5, 3)).toBe(true);
    expect(contains(r, 6, 3)).toBe(false);
    expect(clampCell({ row: 99, col: -1 }, 10, 4)).toEqual({ row: 9, col: 0 });
  });
});

describe("browser stub backend", () => {
  it("edits, moves columns and undoes", async () => {
    const b = new MockBackend();
    const d = (await b.openFile())!.doc!;
    await b.setCells(d.id, [[1, 1, "Zed"]]);
    let v = await b.viewport(d.id, 1, 1, 0, 3, 100);
    expect(v.cells).toEqual(["1", "Zed", "Stone"]);
    await b.change(d.id, { op: "move_cols", at: 2, count: 1, to: 0 });
    v = await b.viewport(d.id, 1, 1, 0, 3, 100);
    expect(v.cells).toEqual(["Stone", "1", "Zed"]);
    await b.change(d.id, { op: "undo" });
    const info = await b.change(d.id, { op: "undo" });
    expect(info.modified).toBe(false);
    v = await b.viewport(d.id, 1, 1, 0, 3, 100);
    expect(v.cells).toEqual(["1", "Ada", "Stone"]);
  });
});


describe("header guess", () => {
  it("spots titles over typed data", () => {
    expect(looksLikeHeader([["id", "name", "amount"], ["1", "Ada", "3.50"], ["2", "Bo", "1,200.00"]])).toBe(true);
    expect(looksLikeHeader([["Name", "Joined"], ["Ada", "2021-03-04"], ["Bo", "2020-01-02"]])).toBe(true);
  });

  it("treats a first row of data as data", () => {
    expect(looksLikeHeader([["1", "Ada", "3.50"], ["2", "Bo", "1.25"]])).toBe(false);
    expect(looksLikeHeader([["Ada", "Stone"], ["Bo", "Wells"], ["Cy", "Varga"]])).toBe(false);
    expect(looksLikeHeader([["a", ""], ["1", "2"]])).toBe(false);
    expect(looksLikeHeader([["x", "x"], ["1", "2"]])).toBe(false);
    expect(looksLikeHeader([["only one row"]])).toBe(false);
    expect(looksLikeHeader([[], []])).toBe(false);
  });
});
