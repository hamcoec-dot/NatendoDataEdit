import appSource from "../App.svelte?raw";
import { describe, expect, it } from "vitest";
import readme from "../../../README.md?raw";
import { FEATURE_LINKS, featureDocs, readmeBullets } from "./featureIndex";
import { near, norm, prepare, rank, stem, SYNONYMS, type LaunchItem } from "./launcher";
import { TOOLS } from "./clean";

const MENUS: Record<string, string> = { file: "File", edit: "Edit", view: "View", insert: "Insert", format: "Format", data: "Data", find: "Data", clean: "Clean", tools: "Tools", help: "Help" };

/** The app's real commands (id + name) with their README text. */
function realItems(): LaunchItem[] {
  const app = appSource;
  const pairs = [...app.matchAll(/\{ id: "([a-zA-Z0-9_-]+\.[a-zA-Z0-9._-]+)", label: "([^"]+)"/g)].map((m) => ({ id: m[1]!, label: m[2]! }));
  for (const t of TOOLS) pairs.push({ id: `clean.tool.${t.id}`, label: `${t.label}…` });
  const plain = (l: string) => l.replace(/&/g, "").replace(/…$/, "");
  const labels = new Map(pairs.map((p) => [p.id, plain(p.label)]));
  const docs = featureDocs(readmeBullets(readme), FEATURE_LINKS, (id) => labels.get(id) ?? id);
  return pairs.map((p) => ({ id: p.id, name: plain(p.label), menu: MENUS[p.id.split(".")[0]!], docs: docs.get(p.id)?.text }));
}

const items = prepare(realItems());
const top = (q: string, n = 3) => rank(items, q, n).map((h) => h.item.id);

describe("command search ranking on the real commands", () => {
  it.each([
    ["dedupe", "data.removeDuplicates"],
    ["pivot table", "data.pivot"],
    ["count by", "data.pivot"],
    ["group by", "data.pivot"],
    ["drill down", "data.pivot"],
    ["crosstab", "data.pivot"],
    ["distinct count", "data.pivot"],
    ["remove dup", "data.removeDuplicates"],
    ["duplciates", "data.removeDuplicates"],
    ["vlookup", "data.join"],
    ["text to columns", "data.splitColumn"],
    ["nickname", "data.match"],
    ["precinct", "data.street"],
    ["trim", "clean.tool.spaces"],
    ["macro", "tools.recipe"],
    ["password", "file.openEncrypted"],
    ["pdf", "file.pdf"],
    ["freeze", "view.freeze"],
    ["sav", "file.save"],
    ["undo", "edit.undo"],
    ["proper case", "clean.tool.case"],
    ["mail merge", "file.mail"],
    ["pivot", "data.pivot"],
    ["anonymize", "clean.tool.mask"],
    ["ssn", "tools.findPrivate"],
    ["sqlite", "file.openSqlite"],
    ["leading zeros", "clean.tool.zip"],
    ["shortcut", "help.shortcuts"],
    ["dark", "view.themeDark"],
    ["watch folder", "tools.watch"],
    ["fitler", "data.filter"],
    ["conditional formatting", "view.highlights"],
    ["data validation", "data.checks"],
    ["find and replace", "find.replace"],
    ["dark mode", "view.themeDark"],
    ["blank cells", "tools.fillBlanks"],
    ["chart", "view.panelChart"],
    ["compare", "tools.compare"],
  ])("%s finds %s near the top", (q, id) => {
    expect(top(q, 5)).toContain(id);
  });

  it.each(["reorder rows", "rearrange rows", "reorder columns", "rearrange columns"])("%s leads with moving, not flipping", (q) => {
    expect(top(q, 1)[0]).toMatch(/^(edit\.move(Rows|Cols)|help\.mouse)/);
  });

  it("keeps results while the last word is half typed", () => {
    for (const q of ["dark m", "dark mo", "dark mod", "dark mode"]) expect(top(q, 3)).toContain("view.themeDark");
    for (const q of ["vlo", "vlook", "vlookup"]) expect(top(q, 3)).toContain("data.join");
    for (const q of ["cond", "conditional f", "conditional form"]) expect(top(q, 5)).toContain("view.highlights");
    for (const q of ["r", "re", "rem", "remo", "remove", "remove d", "remove du"]) expect(top(q, 50).length).toBeGreaterThan(0);
  });

  it.each([
    ["trim", "clean.tool.spaces"],
    ["insert row", "insert.rowsAbove"],
    ["add column", "insert.colsLeft"],
    ["text to columns", "data.splitColumn"],
    ["text to column", "data.splitColumn"],
    ["dedupe", "data.removeDuplicates"],
  ])("%s ranks %s first or second (review)", (q, id) => {
    expect(top(q, 2)).toContain(id);
  });

  it("history finds file.versions in the top three", () => {
    expect(top("history", 3)).toContain("file.versions");
  });

  it("diff finds tools.compare among the compare commands", () => {
    // Four: Compare two columns, the next difference and Compare lists are compare commands too.
    expect(top("diff", 4)).toContain("tools.compare");
  });

  it("command search does not match its own example words", () => {
    for (const q of ["dedupe", "vlookup", "trim", "mail merge", "fuzzy"]) expect(top(q, 10)).not.toContain("help.search");
  });

  it("puts a name match above a description-only match", () => {
    expect(top("sort", 1)[0]).toMatch(/^data\.sort/);
    expect(top("join", 1)).toEqual(["data.join"]);
  });

  it("needs every typed word to match", () => {
    expect(top("duplicates zzzqqq")).toEqual([]);
  });

  it("lists everything by name for an empty query", () => {
    const all = rank(items, "", 1000);
    expect(all.length).toBe(items.length);
  });

  it("is fast enough to run on every key press", () => {
    // The best of three runs: a busy build machine slows one run, a slow ranking slows all of them.
    const times = [0, 1, 2].map(() => {
      const t0 = performance.now();
      for (const q of ["d", "de", "ded", "dedu", "dedup", "remove blank rows", "xyzzy typo"]) rank(items, q);
      return performance.now() - t0;
    });
    expect(Math.min(...times)).toBeLessThan(100);
  });
});

describe("helpers", () => {
  it("normalises text", () => expect(norm("Sort &A → Z…")).toBe("sort a z"));
  it("stems word endings", () => {
    expect(stem("duplicates")).toBe(stem("duplicate"));
    expect(stem("matching")).toBe("match");
    expect(stem("copies")).toBe("copy");
  });
  it("forgives small slips", () => {
    expect(near("fitler", "filter", 1)).toBe(true);
    expect(near("dupe", "dope", 1)).toBe(true);
    expect(near("sort", "filter", 1)).toBe(false);
  });
  it("synonyms point at words the app uses", () => {
    for (const [k, vs] of Object.entries(SYNONYMS)) {
      expect(k).toBe(norm(k));
      for (const v of vs) expect(v).toBe(norm(v));
    }
  });
});
