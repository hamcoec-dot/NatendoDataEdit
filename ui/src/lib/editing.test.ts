import { describe, expect, it } from "vitest";
import { editPairs, parsePairs, parsePositions, quote, rulesExpr, templateExpr } from "./editing";
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
