// Rows with a different number of fields than most ("ragged" rows): the words
// of the open notice, the commands and their result.

import type { DocInfo } from "./backend";

/** The engine's own reason (document/ragged.rs), shown on the greyed commands. */
export const COLUMNS_CHANGED =
  "columns were added, removed or moved: saving writes every row at the same width; undo that change to repair shifted rows first";

export type RaggedPick = "all" | "short" | "long";

/** Rows of the file with another number of fields, the title row left out. */
export const raggedAtOpen = (d: DocInfo, header: boolean): number => Math.max(0, (d.ragged_rows ?? 0) - (header && d.first_row_ragged ? 1 : 0));

/** The open notice ("" when there is nothing to say or nothing can be done). */
export function raggedNotice(d: DocInfo, header: boolean): string {
  const n = raggedAtOpen(d, header);
  if (!n || d.column_changes) return "";
  return `${n.toLocaleString()} ${n === 1 ? "row has" : "rows have"} a different number of fields than most (usual: ${(d.usual_fields ?? 0).toLocaleString()}).`;
}

/** Why "Repair this row" is greyed for a row of `width` fields ("" when it can run). */
export function repairWhy(d: DocInfo, width: number, usual: number): string {
  if (d.column_changes) return COLUMNS_CHANGED;
  if (!d.indexed) return "the file is still being read";
  return width === usual ? "this row has the usual number of fields" : "";
}

const WORDS: Record<RaggedPick, string> = { all: "with a different number of fields", short: "with too few fields", long: "with too many fields" };

/** What to say after showing them. `short` counts the short ones among all. */
export function raggedResult(pick: RaggedPick, rows: number, short: number): string {
  if (!rows) return `No row ${pick === "all" ? "has a different number of fields than most" : pick === "short" ? "has too few fields" : "has too many fields"}.`;
  const split = pick === "all" ? ` (${short.toLocaleString()} short, ${(rows - short).toLocaleString()} long)` : "";
  return `Showing the ${rows.toLocaleString()} ${rows === 1 ? "row" : "rows"} ${WORDS[pick]}${split}. Click “Filtered ✕” to show every row again.`;
}
