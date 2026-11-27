import { describe, expect, it } from "vitest";
import { AREAS_SPEC_METHODS, backend, WRITE_METHODS, writeGuard } from "./backend";
import { Registry, SELECTION_FREE } from "./commands";

const sources = import.meta.glob(["../**/*.ts", "../**/*.svelte", "!../**/*.test.ts"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const read = (f: string): string => {
  const hit = Object.entries(sources).find(([k]) => k.endsWith(`/${f}`));
  if (!hit) throw new Error(`no source ${f}`);
  return hit[1];
};

/** Every other Backend method: opens, reads, previews, saves, makes a new tab,
 *  or changes a flag or a note, never the cells of the file in front of the
 *  person. EVERY method of the interface must be on this list or on
 *  WRITE_METHODS (a new one fails the test until someone decides which), and
 *  WRITE_METHODS are refused at the backend while several blocks are selected. */
const NOT_WRITES = [
  "jsonCheck",
  "saveJson",
  "checkpointList",
  "checkpointSince",
  "checkpointSinceReport",
  "raggedList",
  "dateScan",
  "dateMarks",
  "dateFind",
  "dateList",
  "gapHidden",
  "setAsideCheck",
  "reportNotes",
  "totalRow",
  "dictionaryList",
  "explodeDetect",
  "explodeScan",
  "explodePreview",
  "dictionaryPick",
  "dictionaryPickSave",
  "dictionaryRead",
  "dictionaryWrite",
  "dictionaryUnmatched",
  "fillBlanksCount",
  "fillBlanksCountStop",
  "fillMarks",
  "fillMarksNext",
  "fillMarksClear",
  "keepNotes",
  "rowText",
  "rowWidth",
  "sameShape",
  "emptyExtraColumns",
  "checkpointAdd",
  "checkpointRemove",
  "appInfo",
  "openFile",
  "importFile",
  "importText",
  "importInspect",
  "importPreview",
  "importNotes",
  "importFinish",
  "cleanPreviewPage",
  "xrayReport",
  "xrayShow",
  "setColumnType",
  "setColumnTypes",
  "importProgress",
  "importCancel",
  "importClose",
  "exportPreview",
  "exportFile",
  "exportBook",
  "exportText",
  "reopen",
  "layoutChoices",
  "previewLayout",
  "newDocument",
  "closeDocument",
  "docInfo",
  "viewport",
  "cellText",
  "hiddenBetween",
  "copyCells",
  "save",
  "jobProgress",
  "cancelJob",
  "filterSpec",
  "filterValues",
  "findNext",
  "findAll",
  "findCount",
  "findMarks",
  "replacePreview",
  "findNotes",
  "showChanged",
  "columnPairs",
  "batchPreview",
  "findCountCancel",
  "setDocTitle",
  "replacePage",
  "findInText",
  "newTable",
  "tableReport",
  "pivotPreview",
  "pivotTable",
  "pivotText",
  "pivotDrill",
  "pivotItems",
  "pivotClose",
  "pivotSave",
  "pivotOpen",
  "findChange",
  "findExtreme",
  "specialCells",
  "compareCols", "gapAudit", "goldenPlan", "goldenPage", "goldenCreate", "goldenClose", "patchPickSave", "patchMake", "patchPick", "patchProtected", "patchPreview", "patchReportSave", "patchClose", "findRelations", "setsRun", "setsOpen", "setsTable", "setsClose", "cleanMatrixPreview", "cleanerOpenNamed",
  "numberRange",
  "typeahead",
  "renameFile",
  "splitToFiles",
  "combineFiles",
  "combinePick",
  "combineCheck",
  "combineRefresh",
  "combineRun",
  "combineCancel",
  "combineSaveMapping",
  "combineLoadMapping",
  "showInFolder",
  "cleanPreview",
  "cleanerSave",
  "cleanerOpen",
  "recentFiles",
  "sessionSave",
  "sessionList",
  "sessionOpen",
  "sessionClear",
  "zipOpen",
  "setProtected",
  "tidyTitlesPreview",
  "openRecent",
  "tabFolders",
  "firstRowCell",
  "firstRowNames",
  "columnNamesLine",
  "folderList",
  "folderOpen",
  "viewSave",
  "viewOpen",
  "setReadOnly",
  "encodingChoices",
  "encodingPreview",
  "reopenEncoding",
  "scanEncodings",
  "encodingShowRows",
  "columnTypes",
  "columnSummary",
  "autosave",
  "recoveryList",
  "versionsKeep",
  "versionsClear",
  "recoveryOpen",
  "recoveryDiscard",
  "versionsList",
  "versionOpen",
  "cleanSuggestions",
  "openLink",
  "selectionTotals",
  "selectionTotalsAreas",
  "columnTotals",
  "chartData",
  "highlights",
  "dataEdge",
  "currentRegion",
  "shownRowOf",
  "modifiedDocs",
  "quitApp",
  "windowMove",
  "windowTake",
  "windowMoveBack",
  "windowSync",
  "windowClaim",
  "windowCloseSelf",
  "onHandover",
  "onCloseRequested",
  "computePreview",
  "facetCounts",
  "facetForget",
  "columnProfile",
  "columnInsights",
  "cellFormula",
  "cellFormulaOf",
  "valueTally",
  "lookalikeGroups",
  "joinPreview",
  "joinList",
  "joinClose",
  "lookupClose",
  "findPrivate",
  "saveSafeCopy",
  "changeList",
  "diskCheck",
  "diskCompare",
  "pickSearchFolder",
  "searchFolder",
  "folderSearchProgress",
  "folderSearchCancel",
  "openFolderHit",
  "editedCells",
  "saveChangeReport",
  "watchPick",
  "watchStatus",
  "watchStart",
  "receiptNote",
  "receiptVerify",
  "watchStop",
  "notesInfo",
  "noteMarks",
  "noteSet",
  "notesSetKeys",
  "noteList",
  "notesShow",
  "saveEncrypted",
  "encryptedOrigin",
  "pickEncrypted",
  "openEncrypted",
  "sqlitePick",
  "sqliteOpen",
  "sqliteSave",
  "updatesFind",
  "updatesClose",
  "matchFind",
  "matchClose",
  "recipeGet",
  "recipeSave",
  "recipeOpen",
  "stepsView",
  "stepsAsk",
  "batchPick",
  "batchStatus",
  "batchStart",
  "batchStop",
  "labelSheets",
  "mailPreview",
  "saveMail",
  "privateState",
  "setPrivate",
  "privateLeftovers",
  "privateForget",
  "forgetScrambleKey",
  "runChecks",
  "checkMarks",
  "checksSave",
  "diffFiles",
  "diffMarks",
  "diffNext",
  "diffClear",
  "nextProblem",
  "suggestRules",
  "checksSidecarLoad",
  "checksSidecarKeep",
  "onFreshInfo",
  "replaceGhosts",
  "replaceCount",
  "replaceCountCancel",
  "checksOpen",
  "takeOpened",
  "onOpened",
  "onAgentGoto",
  "fileTypes",
  "setFileTypes",
  "openDefaultApps",
  "agentStatus",
  "setAgentLink",
  "logEvent",
  "openLogFolder",
];

describe("every call that changes a file is guarded while several blocks are selected", () => {
  it("EVERY Backend method is classified as a write or not (a new method fails until it is)", () => {
    const text = read("backend.ts");
    const i = text.indexOf("export interface Backend");
    const body = text.slice(i, text.indexOf("\n}", i));
    const names = [...body.matchAll(/^ {2}(\w+)\(/gm)].map((m) => m[1] as string);
    expect(names.length).toBeGreaterThan(150);
    const unclassified = names.filter((n) => !WRITE_METHODS.includes(n) && !NOT_WRITES.includes(n));
    expect(unclassified).toEqual([]);
    // Nothing on both lists, nothing listed that is not a method (a rename cannot slip past).
    expect(WRITE_METHODS.filter((n) => NOT_WRITES.includes(n))).toEqual([]);
    for (const w of [...WRITE_METHODS, ...NOT_WRITES]) expect(names, `${w} is not a Backend method`).toContain(w);
    // The ones that change cells, rows or columns of an open file are writes.
    for (const w of ["setCells", "fillAreas", "dataOp", "change", "sortRows", "replaceTitles", "replaceAll", "streetLookup"]) expect(WRITE_METHODS).toContain(w);
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

  it("only Replace all, its confirm step, Replace of one cell and Find's row actions pass, and only with a search over the blocks", async () => {
    expect(Object.keys(AREAS_SPEC_METHODS).sort()).toEqual(["findRows", "replaceAll", "replaceCell", "replaceConfirm"]);
    for (const m of Object.keys(AREAS_SPEC_METHODS)) expect(WRITE_METHODS).toContain(m);
    const b = backend() as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>;
    const doc = (await backend().newDocument()).id;
    const pattern = { text: "x", kind: "plain", match_case: false, whole_cell: false };
    const over = (kind: string) => ({ pattern, skip_first: false, scope: kind === "areas" ? { kind, areas: [{ row: 0, rows: 1, col: 0, cols: 1 }] } : { kind, row: 0, rows: 1, col: 0, cols: 1 } });
    writeGuard.check = () => "That works on one block.";
    try {
      // With a search over the blocks the guard lets the call through (what the call does then is its own business).
      for (const m of Object.keys(AREAS_SPEC_METHODS)) {
        const r = await b[m]!(doc, over("areas"), "y", { kind: "count" }).then(
          () => null,
          (e: { kind?: string }) => e,
        );
        expect(r?.kind, `${m} with areas`).not.toBe("one_block");
        // With any other scope it is refused as before.
        for (const k of ["range", "all", "columns"]) await expect(b[m]!(doc, over(k), "y", { kind: "count" }), `${m} with ${k}`).rejects.toMatchObject({ kind: "one_block" });
      }
      // The other writes that take a search stay refused even with one over the blocks.
      for (const m of ["replaceTitles", "batchConfirm"]) {
        await expect(b[m]!(doc, over("areas"), "y"), m).rejects.toMatchObject({ kind: "one_block" });
      }
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
