import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { describeCheck, isIsoDate, jumpText, shapeRegex } from "./lint";
import type { Check, CheckSpec } from "./backend";
import rules from "../../../crates/engine/src/testdata/shape_rules.json";

describe("shapeRegex gives the engine's pattern text (shared fixture)", () => {
  for (const c of rules.cases) {
    it(JSON.stringify(c.shape), () => {
      expect(shapeRegex(c.shape)).toBe(c.regex);
    });
  }
  it("takes any rest after a shape cut at 40 characters", () => {
    expect(shapeRegex("9".repeat(40) + "…")).toBe("\\p{N}".repeat(40) + "(?s:.+)");
  });
});

describe("rules say what they are", () => {
  const c = (kind: Check["kind"], name?: string): Check => ({ col: 0, kind, name });
  it("uses the name, else a sentence from the kind (as the engine's describe)", () => {
    expect(describeCheck(c({ type: "required" }, "5-digit ZIP"))).toBe("5-digit ZIP");
    expect(describeCheck(c({ type: "required" }))).toBe("Required (not blank)");
    expect(describeCheck(c({ type: "number", min: 1, max: 9, whole: true }))).toBe("Whole number from 1 to 9");
    expect(describeCheck(c({ type: "date_iso" }))).toBe("Date written YYYY-MM-DD");
    expect(describeCheck(c({ type: "unique", match_case: false }))).toBe("Unique (no repeats)");
    expect(describeCheck(c({ type: "required" }, "  "))).toBe("Required (not blank)");
  });
  it("reads dates written year-month-day as the engine does", () => {
    for (const ok of ["2024-02-29", "2000-02-29", "1999-12-31", "2023-02-28", "0001-01-01"]) expect(isIsoDate(ok), ok).toBe(true);
    for (const bad of ["2023-02-29", "1900-02-29", "02/03/2024", "2024-2-3", "2024-13-01", "2024-00-10", "2024-04-31", "2024-01-32", "20240131", " 2024-01-31", "２０２４-01-31", "", "2024/01/31"]) expect(isIsoDate(bad), bad).toBe(false);
  });
});

describe("jump to the next problem", () => {
  const read = { decimal_comma: false, date_order: "mdy" as const };
  async function table() {
    const m = new MockBackend();
    const d = await m.newDocument();
    await m.dataOp(d.id, {
      op: "paste",
      spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false },
      text: ["a\tb\tc", "x\t1\t", "\t2\tz", "y\t3\tz", "\t\t"].join("\r\n"),
    });
    const spec: CheckSpec = {
      checks: [
        { col: 0, kind: { type: "required" } },
        { col: 2, kind: { type: "required" } },
      ],
      keep_first: true,
      read,
      now: 0,
    };
    return { m, doc: d.id, spec };
  }
  it("walks the problem cells in reading order, wraps, and goes both ways", async () => {
    const { m, doc, spec } = await table();
    // Problems: (1,2) (2,0) (4,0) (4,2)
    const seq: [number, number][] = [];
    let at: [number, number] = [0, 0];
    for (let i = 0; i < 5; i++) {
      const r = await m.nextProblem(doc, at[0], at[1], false, [], spec);
      expect(r.hit).not.toBeNull();
      at = [r.hit!.row, r.hit!.col];
      seq.push(at);
      if (i === 4) expect(r.wrapped).toBe(true);
    }
    expect(seq).toEqual([[1, 2], [2, 0], [4, 0], [4, 2], [1, 2]]);
    const back = await m.nextProblem(doc, 1, 2, true, [], spec);
    expect([back.hit!.row, back.hit!.col, back.wrapped]).toEqual([4, 2, true]);
  });
  it("passes over problems in hidden columns and counts them", async () => {
    const { m, doc, spec } = await table();
    const r = await m.nextProblem(doc, 0, 0, false, [2], spec);
    expect([r.hit!.row, r.hit!.col]).toEqual([2, 0]);
    expect(r.hidden_skipped).toBe(2);
    const none = await m.nextProblem(doc, 0, 0, false, [0, 2], spec);
    expect(none.hit).toBeNull();
    expect(jumpText(none, null)).toBe("No problems in shown columns (4 in hidden columns)");
  });
  it("words the message", () => {
    const hit = { row: 1, col: 0, check: 0, row_rule: false };
    expect(jumpText({ hit, wrapped: true, hidden_skipped: 1 }, "Required (not blank)")).toBe("Required (not blank) · back at the top · 1 problem in hidden columns passed");
    expect(jumpText({ hit: null, wrapped: false, hidden_skipped: 0 }, null)).toBe("No problems found");
  });
});
