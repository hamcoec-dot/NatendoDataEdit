// Run a recipe on many files now (Tools › Run recipe on files…). The files,
// the recipe and the results folder are chosen in the app's own dialogs;
// the files chosen are never changed.

import type { Question } from "./recipe";

export interface FileResult {
  name: string;
  ok: boolean;
  rows_in: number;
  rows_out: number;
  /** Where it was written, or why not. */
  message: string;
}

export interface BatchStatus {
  files: string[];
  recipe: string | null;
  /** The recipe's steps in words. */
  steps: string[];
  questions: Question[];
  out: string | null;
  running: boolean;
  done: number;
  failed: number;
  /** The file being run now. */
  current: string | null;
  results: FileResult[];
}

/** The summary line under the results. */
export function batchSummary(st: BatchStatus): string {
  const rowsIn = st.results.reduce((a, r) => a + r.rows_in, 0);
  const rowsOut = st.results.reduce((a, r) => a + r.rows_out, 0);
  const files = (n: number) => `${n.toLocaleString()} file${n === 1 ? "" : "s"}`;
  const parts = [`${files(st.done)} done`];
  if (st.failed) parts.push(`${files(st.failed)} not done`);
  if (st.done) parts.push(`${rowsIn.toLocaleString()} rows in, ${rowsOut.toLocaleString()} rows out`);
  return parts.join("; ") + ".";
}
