// Every call that changes a file says what it does while rows are hidden: it respects the view, or it includes hidden
// rows on purpose. A new write method fails this test until it is put in one of the two tables. The engine proof is
// src-tauri/src/audit.rs (run on a filtered table); "by code" names where that was read instead of run.
import { describe, expect, it } from "vitest";
import { WRITE_METHODS } from "./backend";

/** Rows a filter or the person hid are left as they were. */
const RESPECTS_THE_VIEW: Record<string, string> = {
  setCells: "audit: set_cells",
  fillCells: "audit: fill_rect",
  fillAreas: "audit: fill_areas",
  moveCells: "audit: move_cells",
  sortRows: "audit: sort_shown (the screens ask for shown rows while some are hidden)",
  findRows: "by code: row actions act on shown matches",
  replaceConfirm: "audit: replace_all",
  replaceCell: "audit: replace_all (one cell of the shown rows)",
  replaceAll: "audit: replace_all",
  batchConfirm: "audit: batch_replace",
  tidyTitles: "audit: title_operations_change_no_data_row",
  setFirstRowCell: "audit: title_operations_change_no_data_row",
  setColumnNames: "audit: title_operations_change_no_data_row",
  replaceTitles: "title row only (a preview of new names)",
  matchAccept: "audit: match_accept; a row hidden after the matches were found refuses the accept",
  streetLookup: "audit: street_lookup",
};

/** Hidden rows are reached, and the screen or the command says so. */
const INCLUDES_HIDDEN: Record<string, string> = {
  dataOp: "per operation: audit.rs names each; split, merge, split name and extract change every row and say so",
  change: "column operations change every row; deleting and inserting rows leave hidden rows (audit)",
  setFilter: "chooses which rows are hidden",
  clearFilter: "shows rows again",
  repairRow: "repairs the one row named",
  fixAll: "every row of the same shape, counted, hidden ones too",
  removeEmptyExtraColumns: "a column operation",
  encodingApply: "reads the whole file again in another encoding",
  updatesAccept: "compares and changes every row; hidden ones are marked, a hidden delete needs its own tick",
  recipeRun: "each step as when done by hand",
  stepsGoto: "puts the table back as it was at a step",
  stepsChange: "runs the steps again",
  checkpointRestore: "puts the table back as it was at the checkpoint",
};

describe("writes while rows are hidden", () => {
  it("every write method is in exactly one table", () => {
    const missing = WRITE_METHODS.filter((m) => !(m in RESPECTS_THE_VIEW) && !(m in INCLUDES_HIDDEN));
    expect(missing).toEqual([]);
    expect(Object.keys(RESPECTS_THE_VIEW).filter((m) => m in INCLUDES_HIDDEN)).toEqual([]);
  });
  it("names no method that is not a write", () => {
    const all = [...Object.keys(RESPECTS_THE_VIEW), ...Object.keys(INCLUDES_HIDDEN)];
    expect(all.filter((m) => !WRITE_METHODS.includes(m))).toEqual([]);
  });
});
