import { describe, expect, it } from "vitest";
import { backend, WRITE_METHODS, writeGuard } from "./backend";
import { Registry, SELECTION_FREE } from "./commands";

const sources = import.meta.glob(["../**/*.ts", "../**/*.svelte", "!../**/*.test.ts"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const read = (f: string): string => {
  const hit = Object.entries(sources).find(([k]) => k.endsWith(`/${f}`));
  if (!hit) throw new Error(`no source ${f}`);
  return hit[1];
};

/** Backend methods that hand back a document's state but do not change the
 *  cells of the file in front of the person (they open, read, preview, make a
 *  new tab, or change only a flag). Every such method must be here or in
 *  WRITE_METHODS: a new one fails this test until someone decides which. */
const NOT_WRITES = [
  "importFinish",
  "reopen",
  "newDocument",
  "docInfo",
  "replacePreview",
  "findRows",
  "showChanged",
  "batchPreview",
  "setDocTitle",
  "newTable",
  "tableReport",
  "pivotTable",
  "pivotDrill",
  "compareCols",
  "setProtected",
  "setReadOnly",
  "recoveryOpen",
  "versionOpen",
  "modifiedDocs",
  "cellFormula",
  "notesShow",
  "openEncrypted",
  "sqliteOpen",
  "runChecks",
];

describe("every call that changes a file is guarded while several blocks are selected", () => {
  it("each Backend method that returns a document's state is classified as a write or not", () => {
    const text = read("backend.ts");
    const i = text.indexOf("export interface Backend");
    const body = text.slice(i, text.indexOf("\n}\n", i));
    const names = [...body.matchAll(/^ {2}(\w+)\([^;]*?\):\s*Promise<(?:DocInfo|OpReport|\w*Report\w*)>;/gm)].map((m) => m[1] as string);
    expect(names.length).toBeGreaterThan(30);
    const unclassified = names.filter((n) => !WRITE_METHODS.includes(n) && !NOT_WRITES.includes(n));
    expect(unclassified).toEqual([]);
    // A write on the list must exist, so a rename cannot slip past.
    for (const w of WRITE_METHODS) expect(body).toContain(`  ${w}(`);
  });

  it("while several blocks are selected the backend refuses each write and passes it through otherwise", async () => {
    const b = backend() as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>;
    const doc = (await backend().newDocument()).id;
    writeGuard.check = () => "That works on one block.";
    try {
      for (const w of WRITE_METHODS) {
        await expect(b[w]!(doc)).rejects.toMatchObject({ kind: "one_block" });
      }
      writeGuard.check = null;
      // Passed through: setCells works again.
      const info = await backend().setCells(doc, [[0, 0, "x"]]);
      expect(info.id).toBe(doc);
    } finally {
      writeGuard.check = null;
    }
  });

  it("the old tab.rect is gone and drawRect is used only by the grid", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20);
    for (const [f, t] of Object.entries(sources)) {
      expect(/\b(tab|t|ct|nt|src|dst|x)\??\.rect\b/.test(t), `${f} still reads .rect`).toBe(false);
      if (!/\/(Grid\.svelte|tab\.svelte\.ts)$/.test(f)) expect(t.includes("drawRect"), `${f} uses drawRect`).toBe(false);
    }
  });
});

describe("the command registry's several-blocks guard", () => {
  function reg(blocks: () => boolean) {
    const r = new Registry();
    const ran: string[] = [];
    const told: string[] = [];
    const mk = (id: string, extra: object = {}) => ({ id, label: id, run: () => void ran.push(id), ...extra });
    r.add([mk("edit.clear", { multiArea: true }), mk("data.sort"), mk("file.save"), mk("view.zoomIn"), mk("find.find", { multiArea: true })]);
    r.guard = (c) => (blocks() && !c.multiArea && !SELECTION_FREE.test(c.id) ? "one block" : null);
    r.onRefuse = (why) => told.push(why);
    return { r, ran, told };
  }

  it("refuses a command without the flag before any of its code runs, and says why", async () => {
    const { r, ran, told } = reg(() => true);
    expect(await r.run("data.sort")).toBe(false);
    expect(ran).toEqual([]);
    expect(told).toEqual(["one block"]);
    expect(r.enabled("data.sort")).toBe(false);
  });

  it("lets flagged commands and the file, view, help and window groups through", async () => {
    const { r, ran } = reg(() => true);
    for (const id of ["edit.clear", "file.save", "view.zoomIn", "find.find"]) expect(await r.run(id)).toBe(true);
    expect(ran).toEqual(["edit.clear", "file.save", "view.zoomIn", "find.find"]);
  });

  it("runs everything with one block, and the multi-block pass is on only while a flagged command runs", async () => {
    const one = reg(() => false);
    expect(await one.r.run("data.sort")).toBe(true);
    const r = new Registry();
    const seen: boolean[] = [];
    r.add([
      { id: "edit.clear", label: "x", multiArea: true, run: () => void seen.push(r.multiOk) },
      { id: "edit.paste", label: "y", run: () => void seen.push(r.multiOk) },
    ]);
    await r.run("edit.clear");
    await r.run("edit.paste");
    expect(seen).toEqual([true, false]);
    expect(r.multiOk).toBe(false);
    await r.withMulti(async () => expect(r.multiOk).toBe(true));
    expect(r.multiOk).toBe(false);
  });

  it("the selection-free groups are exactly file, view, help and window", () => {
    for (const id of ["file.save", "view.zoomIn", "help.about", "window.next"]) expect(SELECTION_FREE.test(id)).toBe(true);
    for (const id of ["edit.clear", "data.sort", "format.wrap", "insert.rowsAbove", "tools.sample", "find.find", "files.x", "viewer.x"]) expect(SELECTION_FREE.test(id)).toBe(false);
  });
});
