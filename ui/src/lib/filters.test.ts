import { describe, expect, it } from "vitest";
import { FIND_DEFAULTS, conditionOf, describeRule, emptyForm, findSpecOf, formOf, ruleFor, withRule } from "./filters";
import { decodeViewport, encodeViewport } from "./binary";
import { MockBackend } from "./mock";
import type { Condition } from "./backend";

describe("filter rules", () => {
  it("adds, replaces and removes a column's rule", () => {
    let s = withRule(null, 2, { type: "values", values: ["a"], blanks: false }, true);
    s = withRule(s, 0, { type: "conditions", first: { type: "blank" } }, true);
    expect(s.columns.map((c) => c.col)).toEqual([0, 2]);
    expect(ruleFor(s, 2)).toEqual({ type: "values", values: ["a"], blanks: false });
    s = withRule(s, 2, null, false);
    expect(s.columns.map((c) => c.col)).toEqual([0]);
    expect(s.keep_first).toBe(false);
  });

  it("builds conditions from the form and back", () => {
    const f = emptyForm();
    expect(conditionOf({ ...f, kind: "number", op: "gt", a: "abc" })).toBe("Enter a number.");
    expect(conditionOf({ ...f, kind: "number", op: "between", a: "1,000", b: "5" })).toEqual({ type: "number_between", low: 1000, high: 5 });
    const conds: Condition[] = [
      { type: "text", op: "begins_with", text: "ab", match_case: true },
      { type: "number", op: "le", value: 3 },
      { type: "date_between", low: "2026-01-01", high: "2026-12-31" },
      { type: "not_blank" },
      { type: "pattern", pattern: { text: "^a", kind: "regex", match_case: false, whole_cell: false }, negate: true },
    ];
    for (const c of conds) expect(conditionOf(formOf(c))).toEqual(c);
    expect(describeRule({ type: "conditions", first: conds[1]!, second: conds[3]!, or: true })).toBe("≤ 3 or is not empty");
  });

  it("turns find options into a search", () => {
    const sel = { row: 2, rows: 3, col: 1, cols: 2 };
    const o = { ...FIND_DEFAULTS, text: "x", kind: "wildcard" as const, matchCase: true, wholeWord: true, scope: "column" as const };
    expect(findSpecOf(o, sel, true, [4])).toEqual({
      pattern: { text: "x", kind: "wildcard", match_case: true, whole_cell: false, whole_word: true, ignore_accents: false, escapes: false },
      scope: { kind: "columns", cols: [1, 2] },
      skip_first: true,
      skip_cols: [4],
    });
    expect(findSpecOf({ ...o, includeHidden: true }, sel, true, [4]).skip_cols).toEqual([]);
    // Include hidden: hidden rows too, and the hidden columns are named so a
    // replace can say how many hidden cells it changes.
    const h = findSpecOf({ ...o, includeHidden: true, byColumns: true, preserveCase: true }, sel, true, [4]);
    expect([h.hidden_rows, h.hidden_cols, h.by_columns, h.pattern.preserve_case]).toEqual([true, [4], true, true]);
    expect(findSpecOf({ ...o, kind: "number", text: "5" }, sel, true).pattern.kind).toBe("number");
    expect(findSpecOf({ ...o, scope: "selection" }, sel, false).scope).toEqual({ kind: "range", ...sel });
  });
});

describe("viewport row numbers", () => {
  it("round-trip with and without a filter", () => {
    const v = { firstRow: 0, firstCol: 0, rows: 2, cols: 1, cells: ["a", "b"], truncated: new Uint8Array(2) };
    expect(decodeViewport(encodeViewport(v)).rowNumbers).toBeUndefined();
    const f = decodeViewport(encodeViewport({ ...v, rowNumbers: [0, 7_000_000_000] }));
    expect(f.rowNumbers).toEqual([0, 7_000_000_000]);
  });
});

describe("browser stand-in", () => {
  it("filters, sorts and replaces at the command level", async () => {
    const b = new MockBackend();
    const d = (await b.openFile())!.doc!;
    const id = d!.id;
    const spec = { columns: [{ col: 3, rule: { type: "values" as const, values: ["Osaka"], blanks: false } }], keep_first: true };
    const info = await b.setFilter(id, spec);
    expect(info.filtered).toBe(true);
    expect(info.rows).toBeLessThan(info.total_rows);
    const v = await b.viewport(id, 0, 3, 3, 1, 50);
    expect(v.cells.slice(1)).toEqual(["Osaka", "Osaka"]);
    expect(v.rowNumbers?.[0]).toBe(0);
    // Moving shown rows while filtered: they gather before the next shown row
    // and report it; a move made from an older count of shown rows is refused.
    const [p1, p2, p3] = await b.docRows(id, 1, 3);
    const mv = await b.change(id, { op: "move_rows", at: 1, count: 1, to: 2, shown: info.rows });
    expect(mv.rows_change).toEqual({ kind: "move", runs: [[p1, 1]], to: p3! - 1 });
    expect(await b.docRows(id, 1, 3)).toEqual([p2! - 1, p3! - 1, p3]);
    await expect(b.change(id, { op: "move_rows", at: 1, count: 1, to: 2, shown: info.rows + 1 })).rejects.toMatchObject({ kind: "invalid" });
    await b.change(id, { op: "undo" });
    expect(await b.docRows(id, 1, 3)).toEqual([p1, p2, p3]);
    // Deleting a shown row reports the document row it took.
    const at = (await b.docRows(id, 1, 1))[0]!;
    expect(at).toBeGreaterThan(1);
    const del = await b.change(id, { op: "delete_rows", at: 1, count: 1 });
    expect(del.rows_change).toEqual({ kind: "delete", runs: [[at, 1]] });
    await b.change(id, { op: "undo" });
    const cleared = await b.clearFilter(id);
    expect(cleared.filtered).toBe(false);
    const found = await b.findNext(id, { pattern: { text: "Quito", kind: "plain", match_case: false, whole_cell: true }, scope: { kind: "all" }, skip_first: true }, [0, 0], false);
    expect(found).toMatchObject({ col: 3, wrapped: false });
  });
});
