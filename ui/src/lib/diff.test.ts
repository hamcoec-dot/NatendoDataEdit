import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import fx from "../../../crates/engine/src/testdata/diff.json";

const TAB = String.fromCharCode(9);
const CRLF = String.fromCharCode(13, 10);
const grid = (csv: string) => csv.split(CRLF).filter(Boolean).map((l) => l.split(",").join(TAB)).join(CRLF);

async function open(m: MockBackend, csv: string) {
  const d = await m.newDocument();
  await m.dataOp(d.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text: grid(csv) });
  return d.id;
}
const spec = { keys: fx.keys, keep_first: true, match_case: false, trim: false };
const text = async (m: MockBackend, doc: number) => {
  const v = await m.viewport(doc, 0, 20, 0, 12, 200);
  const rows: string[] = [];
  for (let r = 0; r < v.rows; r++) {
    const cells = v.cells.slice(r * v.cols, (r + 1) * v.cols);
    while (cells.length && cells[cells.length - 1] === "") cells.pop();
    if (cells.length) rows.push(cells.join(","));
  }
  return rows;
};

describe("the browser stand-in's diff gives what the engine's does (shared fixture)", () => {
  it("counts, columns and result rows", async () => {
    const m = new MockBackend();
    const a = await open(m, fx.a);
    const b = await open(m, fx.b);
    const o = await m.diffFiles(a, b, spec);
    expect([o.report.same, o.report.changed, o.report.added, o.report.removed, o.report.cells]).toEqual([fx.report.same, fx.report.changed, fx.report.added, fx.report.removed, fx.report.cells]);
    expect(o.report.moved).toBe(fx.report.moved);
    expect(o.report.only_b).toEqual(fx.report.only_b);
    const lines = (await text(m, o.info.id)).map((l) => l.replace(/,+$/, ""));
    expect(lines).toEqual(fx.lines.map((l) => l.replace(/,+$/, "")));
  });
});

describe("marks are looked up by key, so they follow a sort; an edit makes them out of date", () => {
  it("sorts, edits, and the result tab", async () => {
    const m = new MockBackend();
    const a = await open(m, fx.a);
    const b = await open(m, fx.b);
    const o = await m.diffFiles(a, b, spec);
    const ids = async (doc: number) => (await m.diffMarks(doc, 0, 20, 0, 20)).marks.map((x) => [x.row, x.col, x.kind]);
    expect(await ids(a)).toEqual([[2, 2, 0], [3, 1, 0], [4, null, 2]]);
    expect(await ids(b)).toEqual([[2, 0, 0], [3, 2, 0], [4, null, 1]]);
    expect(await ids(o.info.id)).toEqual([[1, null, 2], [2, 5, 0], [3, 4, 0], [4, null, 1]]);
    // sort A by name, descending: Di, Cy, Bo, Ann
    await m.sortRows(a, { keys: [{ col: 1, descending: true, kind: "text", match_case: false }], keep_first: true });
    expect(await ids(a)).toEqual([[1, null, 2], [2, 1, 0], [3, 2, 0]]);
    // sort the result tab too (by its first column, descending): the marks stay on their rows
    await m.sortRows(o.info.id, { keys: [{ col: 0, descending: false, kind: "text", match_case: false }], keep_first: true });
    const r = await m.diffMarks(o.info.id, 0, 20, 0, 20);
    expect(r.out_of_date).toBe(false);
    expect(r.marks.length).toBe(4);
    // an edit: out of date, nothing drawn
    await m.setCells(a, [[1, 1, "Zed"]]);
    const e = await m.diffMarks(a, 0, 20, 0, 20);
    expect(e.out_of_date).toBe(true);
    expect(e.marks).toEqual([]);
  });

  it("next difference walks the marked rows both ways and wraps; a tab in no diff has no marks", async () => {
    const m = new MockBackend();
    const a = await open(m, fx.a);
    const b = await open(m, fx.b);
    await m.diffFiles(a, b, spec);
    const rows: number[] = [];
    let at = 0;
    for (let i = 0; i < 4; i++) {
      const j = (await m.diffNext(a, at, false))!;
      rows.push(j.row + (j.wrapped ? 100 : 0));
      at = j.row;
    }
    expect(rows).toEqual([2, 3, 4, 102]);
    const back = (await m.diffNext(a, 2, true))!;
    expect([back.row, back.wrapped]).toEqual([4, true]);
    const other = await open(m, "x,y\r\n1,2\r\n");
    expect((await m.diffMarks(other, 0, 5, 0, 5)).diff).toBeNull();
    await m.diffClear(a);
    expect((await m.diffMarks(a, 0, 20, 0, 20)).diff).toBeNull();
  });

  it("comparing a tab with itself gives all the same, no marks", async () => {
    const m = new MockBackend();
    const a = await open(m, fx.a);
    const o = await m.diffFiles(a, a, spec);
    expect([o.report.same, o.report.changed, o.report.added, o.report.removed]).toEqual([4, 0, 0, 0]);
    expect((await m.diffMarks(a, 0, 20, 0, 20)).marks).toEqual([]);
  });

  it("empty keys are counted and never paired; duplicates are reported", async () => {
    const m = new MockBackend();
    const a = await open(m, "id,v\r\n1,a\r\n1,b\r\n2,c\r\n,x\r\n,y\r\n");
    const b = await open(m, "id,v\r\n1,a\r\n1,B\r\n2,c\r\n,x\r\n");
    const o = await m.diffFiles(a, b, spec);
    expect([o.report.empty_a, o.report.empty_b]).toEqual([2, 1]);
    expect(o.report.dup_a).toEqual(["1"]);
    expect(o.report.dup_b).toEqual(["1"]);
    expect(o.report.changed).toBe(1);
  });
});
