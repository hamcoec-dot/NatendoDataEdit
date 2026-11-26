// Which rows an operation acts on while a filter or hidden rows are on: the counts of the active tab and the words
// said about them, in dialogs and in the status line.

/** The data rows of the active tab (the title row is not counted): all of them, and those hidden. Set by the window. */
export const hiddenRows = $state({ hidden: 0, total: 0 });

/** Which kind of note an operation needs: it changes every row; its new column is left empty in hidden rows. */
export type HiddenKind = "all" | "empty";

const n = (v: number) => v.toLocaleString();
const rows = (v: number) => (v === 1 ? "row" : "rows");

/** Operations that change every row, hidden ones too, because the column they read is changed too. */
const ALL = new Set(["split_column", "merge_columns", "split_name", "extract"]);
/** Operations whose new column stays empty in hidden rows. */
const EMPTY = new Set(["count_repeats", "group_calc", "check_column", "compare_column"]);

export const kindOfOp = (op: string): HiddenKind | null => (ALL.has(op) ? "all" : EMPTY.has(op) ? "empty" : null);

/** The note for a kind ("" when no row is hidden). */
export function hiddenWords(kind: HiddenKind, hidden: number, total: number): string {
  if (!hidden) return "";
  if (kind === "all") return `This changes all ${n(total)} ${rows(total)}, including ${n(hidden)} hidden ${hidden === 1 ? "one" : "ones"}, because the source column is changed too.`;
  return `${n(hidden)} hidden ${rows(hidden)} left empty.`;
}

/** The line of a sort of the shown rows only. */
export function sortWords(hidden: number, total: number): string {
  const shown = total - hidden;
  return `Sorting the ${n(shown)} shown ${rows(shown)}; ${n(hidden)} hidden ${rows(hidden)} ${hidden === 1 ? "stays where it is" : "stay where they are"}.`;
}
