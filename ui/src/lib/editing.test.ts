import { describe, expect, it } from "vitest";
import { dictFromEdits, editPairs, editsFromDict, finalOf, mergeInto, parsePairs, parsePositions, quote, rulesExpr, templateExpr } from "./editing";
import { MockBackend } from "./mock";

const fin = { format: null, part: 0, raw_dates: false, record: "" };

describe("editing tools", () => {
  it("writes templates and rules as expressions", () => {
    const titles = ["First", "Last", "Age", "my]col"];
    expect(templateExpr("{First} {Last}", titles)).toEqual({ expr: '[First] & " " & [Last]', unknown: [] });
    expect(templateExpr('{{x}} "{#4}"', titles).expr).toBe('"{x} """ & [#4] & """"');
    expect(templateExpr("{Nope}", titles).unknown).toEqual(["Nope"]);
    expect(templateExpr("", titles).expr).toBe('""');
    const r = rulesExpr(
      [
        { col: 2, op: "ge", value: "65", result: "Senior" },
        { col: 1, op: "starts", value: "Mc", result: "{Last}!" },
        { col: 0, op: "blank", value: "", result: "?" },
      ],
      "Other",
      titles,
    );
    expect(r.expr).toBe('IFS([Age] >= 65, "Senior", LOWER(LEFT([Last], 2)) = LOWER("Mc"), [Last] & "!", ISBLANK([First]), "?", TRUE, "Other")');
    expect(quote('a"b')).toBe('"a""b"');
  });

  it("reads translation lists and positions", () => {
    expect(parsePairs("NY\tNew York\nCA = California\n\nTX,Texas\nFL → Florida")).toEqual([
      ["NY", "New York"],
      ["CA", "California"],
      ["TX", "Texas"],
      ["FL", "Florida"],
    ]);
    expect(editPairs(new Map([["a", "b"], ["c", "c"]]))).toEqual([["a", "b"]]);
    expect(parsePositions("7, 3 3 x 0")).toEqual([3, 7]);
  });

  it("runs the value editor, group figures and extract in the stub backend", async () => {
    const b = new MockBackend();
    const a = await b.importFinish((await b.importText("house,name,code\nA,Ann Lee,ny\nB,Bob Ray,NY\nA,Cy Fox,ca")).id, fin);
    const t = await b.valueTally(a.id, { col: 2, keep_first: true });
    expect(t.values.map((v) => v.value).sort()).toEqual(["NY", "ca", "ny"]);
    expect(t.look_alikes).toEqual([{ members: [1, 2] }]);
    let r = await b.dataOp(a.id, { op: "map_values", spec: { cols: [2], pairs: [["ny", "NY"]], match_case: true, trim: false, keep_first: true } });
    expect(r.cells).toBe(1);
    r = await b.dataOp(a.id, { op: "group_calc", spec: { keys: [0], value: null, func: "count", title: "Size", at: 1, match_case: false, trim: true, keep_first: true } });
    expect(r.groups).toBe(2);
    expect((await b.cellText(a.id, 1, 1)).text).toBe("2");
    await b.dataOp(a.id, { op: "extract", spec: { col: 2, what: "last_word", title: "Last", remove: true, keep_first: true } });
    expect((await b.cellText(a.id, 1, 2)).text).toBe("Ann");
    expect((await b.cellText(a.id, 1, 3)).text).toBe("Lee");
  });
});

describe("merging values (C6)", () => {
  const m = (o: Record<string, string>) => new Map(Object.entries(o));
  it("merges into a target's own final value, so chains never form", () => {
    let e = mergeInto(new Map(), ["NY", "N.Y."], "New York");
    expect(Object.fromEntries(e)).toEqual({ NY: "New York", "N.Y.": "New York" });
    // New York merged on: the others follow it (no chain New York -> NYC left behind them).
    e = mergeInto(e, ["New York"], "NYC");
    expect(Object.fromEntries(e)).toEqual({ NY: "NYC", "N.Y.": "NYC", "New York": "NYC" });
    // Dropping on a value that is itself merged goes to its final value.
    e = mergeInto(e, ["ny"], "N.Y.");
    expect(e.get("ny")).toBe("NYC");
    expect(finalOf(e, "N.Y.")).toBe("NYC");
  });
  it("a value dropped on itself (or on what it already becomes) does nothing", () => {
    const e = m({ A: "B" });
    expect(mergeInto(e, ["A"], "A")).toEqual(m({ A: "B" }));
    // A already becomes B: B dropped on A is B on itself.
    expect(mergeInto(e, ["B"], "A")).toEqual(m({ A: "B" }));
  });
  it("keeps the old map for undo", () => {
    const e = m({ A: "B" });
    const n = mergeInto(e, ["C"], "B");
    expect(e).toEqual(m({ A: "B" }));
    expect(n.get("C")).toBe("B");
  });
  it("puts a dictionary in, listing (not overriding) the person's own different merges", () => {
    const mine = m({ "Ham Co": "Hamilton" });
    const r = editsFromDict(mine, [["ham co", "Hamilton County"], ["Knox Co", "Knox"], ["Ham", "Hamilton County"]], (s) => s.trim().toLowerCase());
    expect(r.conflicts).toEqual([{ value: "Ham Co", mine: "Hamilton", theirs: "Hamilton County" }]);
    expect(r.added).toBe(2);
    expect(Object.fromEntries(r.edits)).toEqual({ "Ham Co": "Hamilton", "Knox Co": "Knox", Ham: "Hamilton County" });
    // The same merge as the dictionary is no conflict.
    expect(editsFromDict(m({ X: "Y" }), [["x", "y"]], (s) => s.toLowerCase()).conflicts).toEqual([]);
  });
  it("turns pending merges into dictionary pairs", () => {
    expect(dictFromEdits(m({ A: "B", C: "C", D: "B" }))).toEqual([["A", "B"], ["D", "B"]]);
  });
});
