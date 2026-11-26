import { describe, expect, it } from "vitest";
import { isTextLike, readDateFor, readNumberFor, textCols, typeBadge, typeName, typeOf, type TypedColumns } from "./readers";

describe("what a typed column reads as", () => {
  const typed: TypedColumns = [
    [1, "id"],
    [2, "text"],
    [3, "number"],
    [4, { date: "dmy" }],
    [5, { date: null }],
  ];

  it("reads numbers only where the column may hold them", () => {
    expect(readNumberFor(typed, 0, "12")).toBe(12); // Auto
    expect(readNumberFor(typed, 1, "12")).toBeNull(); // Id
    expect(readNumberFor(typed, 2, "12")).toBeNull(); // Text
    expect(readNumberFor(typed, 3, "1,234")).toBe(1234); // Number
    expect(readNumberFor(typed, 4, "12")).toBeNull(); // Date
    expect(readNumberFor(undefined, 1, "12")).toBe(12); // no types at all
  });

  it("reads dates only where the column may hold them, in the column's own order", () => {
    expect(readDateFor(typed, 0, "03/04/2026")?.getUTCMonth()).toBe(2); // Auto, month first
    expect(readDateFor(typed, 1, "03/04/2026")).toBeNull(); // Id
    expect(readDateFor(typed, 3, "03/04/2026")).toBeNull(); // Number
    expect(readDateFor(typed, 4, "03/04/2026")?.getUTCMonth()).toBe(3); // day first, whatever the file says
    expect(readDateFor(typed, 5, "03/04/2026", true)?.getUTCMonth()).toBe(3); // the file's order
    expect(readDateFor(typed, 5, "03/04/2026", false)?.getUTCMonth()).toBe(2);
  });

  it("names the types, and marks Text and Id columns as text", () => {
    expect(typeOf(typed, 99)).toBe("auto");
    expect([...textCols(typed)].sort()).toEqual([1, 2]);
    expect(isTextLike("id") && isTextLike("text") && !isTextLike("number")).toBe(true);
    expect(["auto", "text", "number", "id", { date: null }, { date: "dmy" }].map((t) => typeBadge(t as never))).toEqual(["", "T", "#", "ID", "D", "D"]);
    expect(typeName({ date: "mdy" })).toBe("Date (month first)");
  });
});

// The scan (R3 for the screen): the code that draws or copies cells does not read a number from cell text
// on its own. Everything that does goes through readNumberFor / readDateFor, or is on this reviewed list
// (file, how many reads, why it is not a cell read). A new read fails here until it is reviewed.
describe("no number is read from a cell outside the helper", () => {
  const files = import.meta.glob(["./*.ts", "./*.svelte", "../*.svelte", "../*.ts"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;
  const READ = /(?<![\w.])Number\(|parseFloat\(|Number\.parseFloat\(|(?<![\w.])readNumber\(|(?<![\w.])plainNumber\(|(?<![\w.])isNumber\(|(?<![\w.])readDate\(/g;

  function counts(): Record<string, number> {
    const out: Record<string, number> = {};
    for (const [path, text] of Object.entries(files)) {
      const name = path.replace(/^\.\.?\//, "").replace(/^\.\//, "");
      if (/\.test\.ts$/.test(name) || /(^|\/)mock/.test(name)) continue;
      let n = 0;
      for (const line of text.split("\n")) {
        const t = line.trim();
        if (t.startsWith("//") || t.startsWith("*") || t.startsWith("/*")) continue;
        n += (line.match(READ) ?? []).length;
      }
      if (n) out[path] = n;
    }
    return out;
  }

  it("matches the reviewed list exactly", () => {
    expect(counts()).toEqual(ALLOWED);
  });
});

/** file -> reads, and why each is not a cell read. */
const ALLOWED: Record<string, number> = {
  // Which list is picked in a drop-down (the value is a list number).
  "./SetsDialog.svelte": 2,
  // What the person typed in a dialog field, or an id, a row or column number, an offset or a size.
  "../App.svelte": 14,
  "./ChecksDialog.svelte": 2,
  "./CleanDialog.svelte": 2,
  "./ComputeDialog.svelte": 2,
  // dates.ts: the two parts of a date already known to be ambiguous, for its tooltip.
  "./dates.ts": 2,
  "./ExportDialog.svelte": 1,
  "./FilterPanel.svelte": 2,
  "./HighlightDialog.svelte": 1,
  "./JoinDialog.svelte": 5,
  "./LayoutDialog.svelte": 1,
  "./OpDialog.svelte": 3,
  "./OptionsDialog.svelte": 1,
  "./PivotDialog.svelte": 10,
  "./SidePanel.svelte": 1,
  "./SortDialog.svelte": 1,
  "./StreetDialog.svelte": 1,
  "./binary.ts": 2,
  "./goto.ts": 6,
  "./insert.ts": 1,
  "./mail.ts": 2,
  "./rename.ts": 1,
  "./cellformula.ts": 1, // a row number in an A1 reference
  "./lint.ts": 3, // the parts of a date written in a rule
  "./views.ts": 1, // formats a number for a label
  "./totalsRow.ts": 1, // rounds an engine total for copying
  // The one reader of cell text as numbers and dates (the parsers, and the typed wrappers around them).
  "./m9.ts": 12,
  "./readers.ts": 3,
  "./m10.ts": 4, // value-type colours: Auto columns use the parsers, typed columns use valueTypeFor
  "./m8.ts": 1, // print: the number class on Auto columns (textCols leaves Text and Id columns out)
  // Mirrors the engine's Save As quoting: a type never changes the bytes that are saved.
  "./transfer.ts": 2,
  // Browser stand-ins for the engine (the desktop app's results come from the engine, which reads cells through its own
  // typed readers): a small reference of its behaviour for the browser preview and for tests.
  "./clean.ts": 7,
  "./colcompare.ts": 2,
  "./editing.ts": 11,
  "./facets.ts": 1,
  "./filters.ts": 1,
  "./insights.ts": 1,
  "./joins.ts": 6,
  "./pivot.ts": 1,
  "./profile.ts": 2,
};
