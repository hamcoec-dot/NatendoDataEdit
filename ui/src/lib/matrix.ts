// Clean > Clean matrix: columns by cleaners. The ticks of a column become a
// plain list of steps in a fixed order; the engine runs the list as stored.

import type { ReadOptions } from "./backend";
import type { CaseTo, CleanChange, Cleaner, Step } from "./clean";
import { USUAL_SANITIZE } from "./invisible";

export interface MatrixEntry {
  col: number;
  title: string;
  steps: Step[];
  /** The file a saved cleaner's steps were copied from. */
  from_file?: string;
}
export interface CleanMatrix {
  entries: MatrixEntry[];
  skip_first: boolean;
  read: ReadOptions;
}
export interface MatrixColumn {
  col: number;
  changed: number;
  /** Would change, but the column is protected. */
  protected: number;
  left: number;
  other_order: number;
  homoglyph_cells: number;
}
export interface MatrixPreview {
  columns: MatrixColumn[];
  examples: CleanChange[];
}
export interface NamedCleaner {
  cleaner: Cleaner;
  file: string;
}

export type TickId = "sanitize" | "invisible" | "breaks" | "trim" | "collapse" | "digits";
export type CasePick = "" | Extract<CaseTo, "upper" | "lower" | "title">;

/** The tick columns, in the order they run. */
export const TICKS: { id: TickId; label: string; step: Step }[] = [
  { id: "sanitize", label: "Fix look-alikes", step: { tool: "sanitize", ...USUAL_SANITIZE } },
  { id: "invisible", label: "Strip non-printing", step: { tool: "remove", what: { kind: "kind", of: "invisible" } } },
  { id: "breaks", label: "Remove line breaks", step: { tool: "spaces", how: "line_breaks" } },
  { id: "trim", label: "Trim", step: { tool: "spaces", how: "trim" } },
  { id: "collapse", label: "Collapse spaces", step: { tool: "spaces", how: "collapse" } },
  { id: "digits", label: "Digits only", step: { tool: "digits", keep: true } },
];
/** A saved cleaner runs before this tick (and everything after it). */
const AFTER_SAVED: TickId = "digits";

export const ORDER_TEXT = "Steps run in this order: fix look-alikes, strip non-printing, remove line breaks, trim, collapse spaces, saved cleaner, digits only, case.";

export interface ColumnPick {
  ticks: TickId[];
  case: CasePick;
  saved: NamedCleaner | null;
}

export function emptyPick(): ColumnPick {
  return { ticks: [], case: "", saved: null };
}

/** The steps one column's ticks stand for, in the fixed order. */
export function stepsFor(pick: ColumnPick): Step[] {
  const out: Step[] = [];
  for (const t of TICKS) {
    if (t.id === AFTER_SAVED && pick.saved) out.push(...pick.saved.cleaner.steps);
    if (pick.ticks.includes(t.id)) out.push(t.step);
  }
  if (pick.case) out.push({ tool: "case", to: pick.case });
  return out;
}

/** The matrix of the ticked columns that have something to do. */
export function buildMatrix(cols: number[], picks: Record<number, ColumnPick>, titles: string[], header: boolean, read: ReadOptions): CleanMatrix {
  const entries: MatrixEntry[] = [];
  for (const col of [...cols].sort((a, b) => a - b)) {
    const pick = picks[col];
    if (!pick) continue;
    const steps = stepsFor(pick);
    if (!steps.length) continue;
    entries.push({ col, title: header ? (titles[col] ?? "") : "", steps, ...(pick.saved ? { from_file: pick.saved.file } : {}) });
  }
  return { entries, skip_first: header, read };
}
