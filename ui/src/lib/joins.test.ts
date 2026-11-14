import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { compareOf, defaultSpec, guessTargets, joinCounts, joinWarnings, keyPart, previewOf, simulateJoin, withCompare } from "./joins";

const fin = { format: null, part: 0, raw_dates: false, record: "" };

describe("joins", () => {
  it("compares keys like the engine", () => {
    const loose = { loose: true, match_case: false, trim: true };
    expect(keyPart("00123", loose)).toBe(keyPart("123", loose));
    expect(keyPart("1.0", loose)).toBe(keyPart("1", loose));
    expect(keyPart("AB-012", loose)).toBe(keyPart("ab 12", loose));
    expect(keyPart("555-0123", loose)).not.toBe(keyPart("5550123", loose));
    expect(keyPart("12-3", loose)).not.toBe(keyPart("1-23", loose));
    expect(keyPart("1,234.50", loose)).toBe(keyPart("1234.5", loose));
    expect(keyPart("-", loose)).toBe("");
    expect(keyPart("123", loose)).not.toBe(keyPart("124", loose));
    expect(keyPart(" A ", { loose: false, match_case: false, trim: true })).toBe("a");
    expect(compareOf(withCompare("loose"))).toBe("loose");
    expect(compareOf(withCompare("below"))).toBe("below");
  });

  it("simulates repeats, keep rows and not-found text", () => {
    const mine = [["id", "n"], ["1", "a"], ["2", "b"], ["3", "c"]];
    const other = [["id", "v"], ["1", "X"], ["1", "Y"], ["3", "Z"]];
    const every = simulateJoin(mine, other, { ...defaultSpec(), repeats: "every", keep_rows: "matched" }, null);
    expect(every.report).toMatchObject({ matched: 2, unmatched: 1, removed: 1, extra_rows: 1 });
    expect(every.rows[0]!.extra).toEqual([["1", "a", "Y"]]);
    const joined = simulateJoin(mine, other, { ...defaultSpec(), repeats: "joined", not_found: "-", place: "after_key", prefix: "B " }, null);
    expect(joined.titles).toEqual(["id", "B v", "n"]);
    expect(joined.rows[0]!.set).toEqual([[1, "X; Y"]]);
    expect(joined.rows[1]!.set).toEqual([[1, "-"]]);
    const p = previewOf(mine, joined, { ...defaultSpec(), place: "after_key" });
    expect(p.rows[0]!.values).toEqual(["1", "X; Y", "a"]);
    expect(joinCounts(joined.report, { repeats: "joined", keep_rows: "all", key_match: "exact" })).toContain("2 rows matched");
  });

  it("finds the nearest smaller key", () => {
    const mine = [["age"], ["5"], ["40"], ["70"]];
    const bands = [["from", "band"], ["0", "Child"], ["18", "Adult"], ["65", "Senior"]];
    const r = simulateJoin(mine, bands, { ...defaultSpec(), key_match: "below" }, null);
    expect(r.rows.map((x) => x.set[0]?.[1])).toEqual(["Child", "Adult", "Senior"]);
  });

  it("guesses update targets by name and warns", () => {
    expect(guessTargets(["id", "City", "zip"], ["id", "city", "other"], [1, 2], [0])).toEqual([1, -1]);
    expect(joinWarnings({ matched: 0, unmatched: 3, added: 0, repeated_keys: 0, cols: 1, dropped_cols: 0, blank_keys: 0, changed: 0, removed: 0, extra_rows: 0, bad_keys: 0 }, 3)).toHaveLength(1);
  });

  it("keeps rows on a pasted list in the stub backend", async () => {
    const b = new MockBackend();
    const a = await b.importFinish((await b.importText("id,n\n001,a\n2,b\n3,c")).id, fin);
    const spec = { ...defaultSpec(), write: "nothing" as const, keep_rows: "matched" as const, loose: true };
    const pv = await b.joinPreview(a.id, null, "1\n3", spec);
    expect(pv.report.removed).toBe(1);
    const r = await b.dataOp(a.id, { op: "join", other: null, list: "1\n3", spec });
    expect(r.join?.removed).toBe(1);
    expect((await b.cellText(a.id, 2, 0)).text).toBe("3");
  });

  it("updates columns in the stub backend", async () => {
    const b = new MockBackend();
    const a = await b.importFinish((await b.importText("id,city\n1,\n2,Old")).id, fin);
    const o = await b.importFinish((await b.importText("id,city\n1,Oslo\n2,Rome")).id, fin);
    const spec = { ...defaultSpec(), write: "fill_blanks" as const, targets: [1] };
    const r = await b.dataOp(a.id, { op: "join", other: o.id, spec });
    expect(r.join?.changed).toBe(1);
    expect((await b.cellText(a.id, 1, 1)).text).toBe("Oslo");
    expect((await b.cellText(a.id, 2, 1)).text).toBe("Old");
  });
});
