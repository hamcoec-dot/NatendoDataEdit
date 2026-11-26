// Which rows an operation acts on while a filter or hidden rows are on: the counts of the active tab and the words
// said about them, in dialogs and in the status line.

import { untrack } from "svelte";

/** The data rows of the active tab (the title row is not counted): all of them, and those hidden. Set by the window. */
export const hiddenRows = $state({ hidden: 0, total: 0, including: 0 });

/** While `on`, an open screen includes hidden rows (the status pill turns amber). Call inside `$effect` and return
 *  what it gives. */
export function includeWhile(on: boolean): (() => void) | undefined {
  if (!on) return undefined;
  untrack(() => hiddenRows.including++);
  return () => {
    untrack(() => hiddenRows.including--);
  };
}

/** The status pill: which rows an edit acts on. */
export function actingWords(hidden: number, total: number, including: boolean): string {
  if (!hidden) return "Edits act on every row: none is hidden.";
  if (including) return `Including hidden rows (${n(hidden)} hidden)`;
  const shown = total - hidden;
  return `Acting on ${n(shown)} shown ${rows(shown)} (${n(hidden)} hidden, untouched)`;
}

/** The rows missing between two shown rows, said when pointing at the dotted line there. `byHand` of them were
 *  hidden by hand; the rest are filtered out. */
export function seamWords(count: number, byHand: number): string {
  const some = `${n(count)} ${rows(count)}`;
  if (byHand >= count) return `${some} hidden here. Click to show ${count === 1 ? "it" : "them"}.`;
  if (!byHand) return `${some} filtered out here. Click for the filter.`;
  return `${some} hidden here (${n(byHand)} by hand, the rest filtered out). Click to show ${byHand === 1 ? "the one" : "those"} hidden by hand.`;
}

/** How many file rows are missing before the shown row whose place is `place` (`before`: the place of the shown row
 *  above, -1 at the top). */
export const gapBetween = (before: number | undefined, place: number | undefined): number => (before === undefined || place === undefined ? 0 : Math.max(0, place - before - 1));

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
