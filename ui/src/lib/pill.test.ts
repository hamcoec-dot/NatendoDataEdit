import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { pillParts, shapeMatches, shapeOf, stubInsights } from "./insights";
import type { Insight } from "./backend";
import shapes from "../../../crates/engine/src/testdata/shapes.json";

describe("shapeOf agrees with the engine's shape_of (shared fixture)", () => {
  for (const c of shapes.cases) {
    it(JSON.stringify(c.value), () => {
      expect(shapeOf(c.value)).toBe(c.shape);
      expect(shapeMatches(c.value, c.shape)).toBe(true);
      expect(shapeMatches(c.value, c.shape + "x")).toBe(false);
    });
  }
});

const ins = (over: Partial<Insight>): Insight => ({ col: 0, filled: 1, distinct: 5, unique: false, numbers: false, bars: [1], ...over });

describe("the pill's text", () => {
  it("shows the shape with its share and the leftover; a ~ while it is from a sample", () => {
    const p = pillParts(ins({ top_shape: "999-9999", top_share: 0.994 }), 1000, 1_000_000)!;
    expect(p.shape).toBe("~99.4% 999-9999");
    expect(p.rest).toBe(" · 0.6% differ");
    const exact = pillParts(ins({ top_shape: "999-9999", top_share: 0.994 }), 1000, 1000)!;
    expect(exact.shape).toBe("99.4% 999-9999");
  });
  it("never says 100% or 0%, and is absent without a dominant shape", () => {
    const p = pillParts(ins({ top_shape: "A", top_share: 0.99999 }), 10, 10)!;
    expect(p.shape).toBe("99.9% A");
    expect(p.rest).toBe(" · 0.1% differ");
    expect(pillParts(ins({ top_shape: null, top_share: 0 }), 10, 10)).toBeNull();
    expect(pillParts(ins({}), 10, 10)).toBeNull();
  });
});

describe("the browser stand-in's pill figures", () => {
  it("names a dominant shape (>= 90%) while another exists, and nothing otherwise", () => {
    const phones = Array.from({ length: 99 }, (_, i) => `555-${String(1000 + i)}`).concat(["(555) 123-4567"]);
    const r = stubInsights([0, 1, 2], (c) => (c === 0 ? phones : c === 1 ? phones.slice(0, 99) : phones.slice(0, 80).concat(Array.from({ length: 20 }, (_, i) => `x${i}`))), 7);
    expect(r.cols[0]!.top_shape).toBe("999-9999");
    expect(r.cols[0]!.top_share).toBeCloseTo(0.99);
    expect(r.cols[1]!.top_shape).toBeNull(); // every value the same shape: nothing differs
    expect(r.cols[2]!.top_shape).toBeNull(); // 80%: not dominant
    expect(r.revision).toBe(7);
  });
});

describe("the browser stand-in's show_shape", () => {
  const fin = { format: null, part: 0, raw_dates: false, record: "" };
  async function sample(m: MockBackend, rows: string[]) {
    // (Rows pasted into a new document are new rows and always stay visible in the stand-in, so the file is imported.)
    const d = await m.importFinish((await m.importText(["v,w", ...rows.map((r) => `${r},x`)].join(String.fromCharCode(10)))).id, fin);
    return d.id;
  }
  const shown = async (m: MockBackend, doc: number) => {
    const info = await m.docInfo(doc);
    const out: string[] = [];
    for (let r = 1; r < info.rows; r++) out.push((await m.cellText(doc, r, 0)).text);
    return out;
  };
  const rows = ["555-1234", "555-9876", "oops", "", "555-0000", "12345", ""];
  const spec = (over = {}) => ({ col: 0, keep_first: true, shape: "999-9999", negate: true, include_blanks: false, ...over });

  it("shows the rows that differ, or match, with blanks only when asked", async () => {
    const m = new MockBackend();
    const doc = await sample(m, rows);
    await m.dataOp(doc, { op: "show_shape", spec: spec() });
    expect(await shown(m, doc)).toEqual(["oops", "12345"]);
    await m.clearFilter(doc);
    await m.dataOp(doc, { op: "show_shape", spec: spec({ include_blanks: true }) });
    expect(await shown(m, doc)).toEqual(["oops", "", "12345", ""]);
    await m.clearFilter(doc);
    await m.dataOp(doc, { op: "show_shape", spec: spec({ negate: false }) });
    expect(await shown(m, doc)).toEqual(["555-1234", "555-9876", "555-0000"]);
    await m.clearFilter(doc);
    expect(await shown(m, doc)).toEqual(rows);
  });

  it("narrows a filter that is already on, and refuses a changed table", async () => {
    const m = new MockBackend();
    const doc = await sample(m, rows);
    await m.dataOp(doc, { op: "show_shape", spec: spec({ negate: false }) }); // the three phones
    await m.dataOp(doc, { op: "show_shape", spec: spec({ negate: true }) }); // odd ones among the rows shown now: none
    expect(await shown(m, doc)).toEqual([]);
    await m.clearFilter(doc);
    const rev = (await m.docInfo(doc)).data_revision;
    await m.setCells(doc, [[1, 0, "edited"]]);
    await expect(m.dataOp(doc, { op: "show_shape", spec: spec({ revision: rev }) })).rejects.toMatchObject({ kind: "changed" });
  });
});
