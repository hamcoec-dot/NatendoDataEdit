import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { codeOf, fromCode, GROUPS } from "./SymbolPicker.svelte";

async function sample(m: MockBackend) {
  const d = await m.newDocument();
  // id, value, tag
  const rows = [
    ["id", "v", "tag"],
    ["1", "10", "a;b"],
    ["2", "", "a"],
    ["1", "10", "c"],
    ["", "", ""],
  ];
  await m.dataOp(d.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: rows.map((r) => r.join("\t")).join("\r\n") });
  return d.id;
}

async function column(m: MockBackend, doc: number, col: number): Promise<string[]> {
  const info = await m.docInfo(doc);
  const out: string[] = [];
  for (let r = 0; r < info.rows; r++) out.push((await m.cellText(doc, r, col)).text);
  return out;
}

describe("data operations (browser stand-in)", () => {
  it("pastes with arithmetic and fills a series", async () => {
    const m = new MockBackend();
    const doc = await sample(m);
    const rep = await m.dataOp(doc, { op: "paste", spec: { row: 1, col: 1, op: "add", skip_blanks: false, transpose: false }, text: "5\r\n5\r\nx" });
    expect([rep.cells, rep.skipped]).toEqual([2, 1]);
    expect(await column(m, doc, 1)).toEqual(["v", "15", "5", "10", ""]);
    await m.dataOp(doc, { op: "fill", spec: { row: 1, col: 0, rows: 2, cols: 1, dir: "down", count: 2, mode: { kind: "auto" } } });
    expect(await column(m, doc, 0)).toEqual(["id", "1", "2", "3", "4"]);
  });

  it("finds and removes duplicates, blanks, and splits rows", async () => {
    const m = new MockBackend();
    const doc = await sample(m);
    const spec = { cols: [0, 1], match_case: false, trim: true, keep_first: true };
    const shown = await m.dataOp(doc, { op: "show_duplicates", spec });
    expect(shown.info?.filtered).toBe(true);
    expect(shown.groups).toBe(1);
    await m.clearFilter(doc);
    const removed = await m.dataOp(doc, { op: "remove_duplicates", spec });
    expect(removed.rows).toBe(1);
    const blanks = await m.dataOp(doc, { op: "remove_blank_rows", spec: { spaces: true, keep_first: true } });
    expect(blanks.rows).toBe(1);
    await m.dataOp(doc, { op: "split_rows", spec: { col: 2, separator: ";", trim: true, skip_empty: true, keep_first: true } });
    expect(await column(m, doc, 2)).toEqual(["tag", "a", "b", "a"]);
  });

  it("builds a pivot as a new, unsaved document", async () => {
    const m = new MockBackend();
    const doc = await sample(m);
    const t = await m.newTable(doc, { op: "pivot", spec: { rows: [0], col: 2, value: 1, aggregate: "count", separator: ", ", keep_first: true } });
    expect(t.modified).toBe(true);
    expect(t.id).not.toBe(doc);
    expect((await m.cellText(t.id, 0, 0)).text).toBe("id");
  });

  it("jumps to data edges like Excel", async () => {
    const m = new MockBackend();
    const doc = await sample(m);
    expect(await m.dataEdge(doc, 0, 1, "down")).toEqual([1, 1]);
    expect(await m.dataEdge(doc, 1, 1, "down")).toEqual([3, 1]);
  });
});

describe("symbol picker codes", () => {
  it("reads and writes character codes, refusing control characters", () => {
    expect(fromCode("U+00E9")).toBe("é");
    expect(fromCode("0x1F600")).toBe("😀");
    expect(fromCode("e9")).toBe("é");
    expect(fromCode("0")).toBeNull();
    expect(fromCode("d800")).toBeNull();
    expect(fromCode("110000")).toBeNull();
    expect(fromCode("hello")).toBeNull();
    expect(codeOf("é")).toBe("U+00E9");
    for (const g of GROUPS) for (const ch of g.chars) expect([...ch].length).toBe(1);
  });
});
