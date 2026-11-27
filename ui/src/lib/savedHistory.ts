// What the status line says after a save about the undo history (undo after save).
import type { HistoryKept } from "./backend";

const steps = (n: number): string => (n === 1 ? "1 step" : `${n.toLocaleString()} steps`);

/** "" when the save kept the whole history (or there was none); otherwise one sentence. */
export function savedHistoryNote(h: HistoryKept | null | undefined): string {
  if (!h) return "";
  const cols = h.undo_cut_columns + h.redo_cut_columns;
  const mem = h.undo_cut_memory + h.redo_cut_memory;
  const work = h.undo_cut_work + h.redo_cut_work;
  const parts: string[] = [];
  if (h.undo_cut_columns > 0) parts.push("Undo goes back to the column change before it; earlier steps were let go.");
  if (h.redo_cut_columns > 0) parts.push(`Redo past a column change was let go (${steps(h.redo_cut_columns)}).`);
  if (mem > 0) parts.push(`The ${steps(mem)} furthest back could not be kept (not enough memory for the undo history).`);
  if (work > 0) parts.push(`The ${steps(work)} furthest back ${work === 1 ? "was" : "were"} let go: they changed too many rows to carry across a save without slowing it.`);
  if (h.cleared && cols === 0 && mem === 0 && work === 0) parts.push("The undo history could not be kept across the save and starts again here.");
  return parts.join(" ");
}
