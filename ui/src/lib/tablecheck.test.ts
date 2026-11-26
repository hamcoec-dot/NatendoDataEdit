import { describe, expect, it } from "vitest";
import { Tab } from "./tab.svelte";
import type { DocInfo, TableCheck } from "./backend";
import { describeTableCheck, newTableCheck, placeTableCheck, shiftTableChecks, tableColRef } from "./m8";
import { MockBackend } from "./mock";

const info = (pos: number, seq: number, cols: number): DocInfo =>
  ({ id: 1, rows: 10, cols, total_rows: 10, history_pos: pos, has_filter: false, filtered: false, seq }) as unknown as DocInfo;

const blanks: TableCheck = { kind: { type: "no_blanks", col: 3 } };
const sums: TableCheck = { name: "Totals", kind: { type: "aggregate", expr: "SUM([A]) = SUM([B])" } };

describe("table check helpers", () => {
  it("writes a column into a formula so its title cannot break out", () => {
    expect(tableColRef("Votes", 0)).toBe("[Votes]");
    expect(tableColRef("a]b", 1)).toBe("[a]]b]");
    expect(tableColRef("x), SUM([y", 2)).toBe("[x), SUM([y]");
    expect(tableColRef('say "hi", ok', 3)).toBe('[say "hi", ok]');
    expect(tableColRef(" padded", 4)).toBe("[#5]");
    expect(tableColRef("", 5)).toBe("[#6]");
    expect(tableColRef("#3", 6)).toBe("[#7]");
  });
  it("moves and drops checks that read a column; formulas stay", () => {
    expect(shiftTableChecks([blanks, sums], 1, 2)).toEqual({ checks: [{ kind: { type: "no_blanks", col: 5 } }, sums], dropped: 0 });
    expect(shiftTableChecks([blanks, sums], 3, -1)).toEqual({ checks: [sums], dropped: 1 });
    expect(placeTableCheck(blanks, (c) => (c === 3 ? 0 : c))).toEqual({ kind: { type: "no_blanks", col: 0 } });
  });
  it("describes checks in words", () => {
    const t = ["A", "B", "C", "Party"];
    expect(describeTableCheck(blanks, t)).toBe("No blanks in Party");
    expect(describeTableCheck(sums, t)).toBe("Totals");
    expect(describeTableCheck({ kind: { type: "aggregate", expr: "SUM([A]) = 1", tolerance: { kind: "relative", value: 0.01 } } }, t)).toBe("SUM([A]) = 1 (within 1%)");
    expect(describeTableCheck(newTableCheck("distinct", 3), t)).toBe("at most 10 different values in Party");
    expect(describeTableCheck(newTableCheck("row_count", 0), t)).toBe("Rows: at least 1");
  });
});

describe("table checks follow columns put in and taken out, through Undo and Redo", () => {
  it("delete the column, Undo brings the check back; Redo drops it again", () => {
    const t = new Tab(info(0, 1, 6), 100);
    t.tableChecks = [blanks, sums];
    t.update(info(1, 2, 5), true);
    expect(t.shiftCols(3, -1)).toBe(1);
    expect(t.tableChecks).toEqual([sums]);
    t.update(info(0, 3, 6), true); // Undo
    expect(t.tableChecks).toEqual([blanks, sums]);
    t.redoing++;
    t.update(info(1, 4, 5), true); // Redo
    t.redoing--;
    expect(t.tableChecks).toEqual([sums]);
  });
  it("insert left of it, add a table check, Undo: both stay, moved back", () => {
    const t = new Tab(info(0, 1, 6), 100);
    t.tableChecks = [blanks];
    t.update(info(1, 2, 7), true);
    t.shiftCols(0, 1);
    expect(t.tableChecks).toEqual([{ kind: { type: "no_blanks", col: 4 } }]);
    t.tableChecks = [...t.tableChecks, { kind: { type: "no_blanks", col: 6 } }];
    t.update(info(0, 3, 6), true); // Undo
    expect(t.tableChecks).toEqual([blanks, { kind: { type: "no_blanks", col: 5 } }]);
  });
});

/** A browser-preview tab with a name and comma-separated rows. */
async function named(m: MockBackend, name: string, csv: string): Promise<number> {
  const d = await m.newDocument();
  const text = csv.split("\n").map((r) => r.split(",").join("\t")).join("\r\n");
  await m.dataOp(d.id, { op: "paste", spec: { row: 0, col: 0, op: "none", skip_blanks: false, transpose: false }, text });
  (m as unknown as { docs: Map<number, { info: { name: string } }> }).docs.get(d.id)!.info.name = name;
  return d.id;
}

describe("the browser preview runs table checks", () => {
  it("counts, blanks, other files and totals within a tolerance", async () => {
    const b = new MockBackend();
    const votes = await named(b, "votes.csv", "P,Early,Day,Total\nA,10,5,15\nB,20,7,27\nC,,3,3");
    await named(b, "roster.csv", "P\nA\nB");
    const rep = await b.runChecks(votes, {
      checks: [],
      keep_first: true,
      read: { decimal_comma: false, date_order: "mdy" },
      now: 0,
      table_checks: [
        { kind: { type: "row_count", compare: "equals", value: 3 } },
        { kind: { type: "row_count_same_as", table: "roster" } },
        { kind: { type: "row_count_same_as", table: "nowhere" } },
        { kind: { type: "no_blanks", col: 1 } },
        { kind: { type: "aggregate", expr: "SUM([Early]) + SUM([Day]) = SUM([Total])" } },
        { kind: { type: "aggregate", expr: "SUM([Total]) = 46", tolerance: { kind: "relative", value: 0.05 } } },
      ],
    });
    expect(rep.table?.map((r) => r.ok)).toEqual([true, false, null, false, true, true]);
    expect(rep.table?.[2]?.why).toBe("open nowhere");
  });
});
