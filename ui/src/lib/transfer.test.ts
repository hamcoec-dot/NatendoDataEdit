import { describe, expect, it } from "vitest";
import { MockBackend } from "./mock";
import { exportDefaults, joinSummary, keepsBytes, plainNumber, saveAsDefaults, writeRow } from "./transfer";
import { defaultSpec } from "./joins";

describe("save as options", () => {
  it("start from the file and say when bytes are kept", async () => {
    const b = new MockBackend();
    const d = (await b.openFile())!.doc!;
    const o = saveAsDefaults(d);
    expect(o).toMatchObject({ delimiter: ",", quote: '"', quoting: "keep", encoding: "UTF-8", line_break: "CRLF" });
    expect(keepsBytes(o, d)).toBe(true);
    expect(keepsBytes({ ...o, quoting: "all" }, d)).toBe(false);
    expect(keepsBytes({ ...o, delimiter: ";" }, d)).toBe(false);
  });

  it("quote as the engine does", () => {
    const o = { delimiter: ",", quote: '"', quoting: "needed" as const, encoding: "UTF-8", bom: false, line_break: "CRLF" as const, convert_line_breaks: false };
    const row = ["Ann", "1.5", "", 'say "hi"', "a,b"];
    expect(writeRow(row, o)).toBe('Ann,1.5,,"say ""hi""","a,b"');
    expect(writeRow(row, { ...o, quoting: "all" })).toBe('"Ann","1.5","","say ""hi""","a,b"');
    expect(writeRow(row, { ...o, quoting: "text" })).toBe('"Ann",1.5,,"say ""hi""","a,b"');
    expect(writeRow(["2,5"], { ...o, quoting: "text" })).toBe('"2,5"');
    expect(writeRow(["2,5"], { ...o, delimiter: ";", quoting: "text" })).toBe("2,5");
    for (const [v, num] of [["12", true], ["-0.5", true], ["1e9", true], ["1e", false], ["1.2.3", false], ["", false], ["12 ", false]] as const)
      expect(plainNumber(v)).toBe(num);
  });
});

describe("import, export, join and append in the browser stand-in", () => {
  it("imports clipboard JSON and exports Markdown", async () => {
    const b = new MockBackend();
    const s = await b.importText('[{"a":1,"b":"x"},{"a":2}]');
    expect(s.info.format).toBe("json");
    expect(await b.importPreview(s.id, { format: null, part: 0, raw_dates: false, record: "" })).toEqual([["a", "b"], ["1", "x"], ["2", ""]]);
    const d = await b.importFinish(s.id, { format: null, part: 0, raw_dates: false, record: "" });
    expect(d.rows).toBe(3);
    const md = await b.exportPreview(d.id, { ...exportDefaults(true), format: "markdown" });
    expect(md.split("\n")[0]).toBe("| a | b |");
    await expect(b.exportText(d.id, { ...exportDefaults(true), format: "template", row: "{nope}" })).rejects.toBeTruthy();
    await b.importClose(s.id);
    await expect(b.importPreview(s.id, { format: null, part: 0, raw_dates: false, record: "" })).rejects.toBeTruthy();
  });

  it("joins by key and appends by name", async () => {
    const b = new MockBackend();
    const a = await b.importFinish((await b.importText("id,name\n1,Ann\n2,Bob")).id, { format: null, part: 0, raw_dates: false, record: "" });
    const o = await b.importFinish((await b.importText("id,city\n2,Rome\n9,Lima")).id, { format: null, part: 0, raw_dates: false, record: "" });
    const r = await b.dataOp(a.id, {
      op: "join",
      other: o.id,
      spec: { ...defaultSpec(), match_case: true, trim: false, add_unmatched: true },
    });
    expect(r.join).toMatchObject({ matched: 1, unmatched: 1, added: 1 });
    expect((await b.cellText(a.id, 2, 2)).text).toBe("Rome");
    expect((await b.cellText(a.id, 3, 0)).text).toBe("9");
    expect(joinSummary("join", r.join!)).toContain("1 row matched");
    const ap = await b.dataOp(a.id, { op: "append", other: o.id, spec: { by_name: true, keep_first: true, other_header: true, add_columns: true } });
    expect(ap.join?.added).toBe(2);
    expect(joinSummary("append", ap.join!)).toBe("Added 2 rows.");
  });
});
