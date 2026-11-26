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

/** What to know before joining this row onto the previous one (which keeps the result). */
export function previousNotes(previous: NextRow, thisNoted: boolean): string {
  const out = [`Joins onto row ${previous.file_row.toLocaleString()} of the file.`];
  if (previous.hidden) out.push("The previous row is hidden; this row will be joined onto it.");
  if (previous.edited) out.push("It has been changed; its cells as they are now are joined.");
  if (previous.noted) out.push("It has a note, which stays with it.");
  if (thisNoted) out.push("This row has a note, which goes with it when the row is removed.");
  return out.join(" ");
}

/** The warning when the row that keeps the result has a note matched on a column the repair changes ("" if not). */
export function noteWarning(before: readonly string[], after: readonly string[], keys: readonly number[], title: (col: number) => string): string {
  const changed = keys.filter((k) => (before[k] ?? "") !== (after[k] ?? ""));
  if (!changed.length) return "";
  return `This row's note is matched on ${keys.map(title).join(", ")}, and this repair changes ${changed.map(title).join(", ")}: the note will no longer match the row.`;
}

/** What "Fix all like this" offers. */
export function sameShapeWords(total: number, hidden: number, fields: number): string {
  const rows = `${total.toLocaleString()} other ${total === 1 ? "row has" : "rows have"} the same shape (${fields.toLocaleString()} fields)`;
  return hidden ? `${rows}; ${hidden.toLocaleString()} of them ${hidden === 1 ? "is" : "are"} not on screen.` : `${rows}.`;
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
