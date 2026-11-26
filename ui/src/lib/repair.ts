// The repair bar's working out: which fields are joined, what the row will be, and the words about the next row.
// The engine does the change itself; this only previews it.

import type { Between, NextRow, RowText } from "./backend";

/** How many fields go into each cell, from the boundaries clicked (boundary `i` joins fields i and i + 1). */
export function groups(fields: number, joined: ReadonlySet<number>): number[] {
  const out: number[] = [];
  for (let i = 0; i < fields; i++) {
    if (i > 0 && joined.has(i - 1)) out[out.length - 1]! += 1;
    else out.push(1);
  }
  return out;
}

/** The cells after the join. */
export function joinPreview(fields: readonly string[], joined: ReadonlySet<number>, separator: string): string[] {
  let at = 0;
  return groups(fields.length, joined).map((n) => fields.slice(at, (at += n)).join(separator));
}

/** The cells after joining with the next row of the file. */
export function nextPreview(fields: readonly string[], next: readonly string[], between: Between): string[] {
  const text = between === "space" ? " " : between === "line_break" ? "\n" : "";
  const [head = "", ...rest] = next;
  if (!fields.length) return [head, ...rest];
  return [...fields.slice(0, -1), fields[fields.length - 1] + text + head, ...rest];
}

/** What to know about the next row before joining it. */
export function nextNotes(next: NextRow): string {
  const out = [`Joins row ${next.file_row.toLocaleString()} of the file.`];
  if (next.hidden) out.push("The next row is hidden; it will be joined.");
  if (next.edited) out.push("It has been changed; its cells as they are now are joined.");
  if (next.noted) out.push("It has a note, which goes with it when the row is removed.");
  return out.join(" ");
}

/** The line about the row itself. */
export function rowHeading(t: RowText): string {
  const which = t.file_row ? `Row ${t.file_row.toLocaleString()} of the file` : "A row added by hand";
  const hidden = t.row === null ? " (not on screen)" : "";
  return `${which}${hidden}: ${t.width.toLocaleString()} ${t.width === 1 ? "field" : "fields"}, usual ${t.usual.toLocaleString()}.`;
}

/** The id to show after row `current`: the next of `ids` in the list's order (`step` 1 or -1), wrapping. A current row
 *  no longer in the list (repaired or removed) keeps its place `at`. */
export function stepRow(ids: readonly number[], current: number, at: number, step: 1 | -1): number | null {
  if (!ids.length) return null;
  const i = ids.indexOf(current);
  const to = i >= 0 ? i + step : step === 1 ? at : at - 1;
  return ids[((to % ids.length) + ids.length) % ids.length]!;
}
