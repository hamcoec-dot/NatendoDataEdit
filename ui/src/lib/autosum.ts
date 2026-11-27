// AutoSum (Alt+=): the totals row with Sum under the selected columns that hold numbers. Which columns hold numbers
// comes from what is already known (the column's type, or the totals row's own count once it has run), never from
// reading the column again on the key press. Columns not known yet wait for the totals row's first result.

import type { ColumnTotal } from "./backend";
import { isTextLike, typeOf, type TypedColumns } from "./readers";
import type { TotalChoice } from "./totalsRow";

export interface AutoSumPlan {
  /** Hide the totals row (the second press). */
  hide: boolean;
  /** Columns that get Sum now. */
  sum: number[];
  /** Columns that hold no numbers: they keep Automatic (a count). */
  text: number[];
  /** Columns whose totals have not come yet: Sum once they show numbers. */
  pending: number[];
}

/** Alt+= as the person typed it: "=" on any layout (Shift+0 on a German one), or the key in the "=" place. AltGr
 *  (Ctrl+Alt) is a character key, never this. */
export function isAutoSumKey(e: { key: string; code: string; altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): boolean {
  if (!e.altKey || e.ctrlKey || e.metaKey) return false;
  return e.key === "=" || (e.code === "Equal" && !e.shiftKey);
}

/** What one press does. `on`: the totals row shows now; `byKey`: this key turned it on. */
export function autoSumPlan(
  cols: readonly number[],
  typed: TypedColumns,
  totalOf: (c: number) => ColumnTotal | null,
  choices: ReadonlyMap<number, TotalChoice>,
  on: boolean,
  byKey: boolean,
): AutoSumPlan {
  const sum: number[] = [];
  const text: number[] = [];
  const pending: number[] = [];
  for (const c of cols) {
    const ty = typeOf(typed, c);
    if (ty === "number") sum.push(c);
    else if (isTextLike(ty) || typeof ty === "object") text.push(c);
    else {
      const t = totalOf(c);
      if (!t) pending.push(c);
      else if (t.numbers > 0) sum.push(c);
      else text.push(c);
    }
  }
  const chosen = [...sum, ...pending];
  // The second press hides the row only when it would change nothing else: every chosen column shows Sum already
  // (one set to Average by hand gets Sum instead), or, with no number columns, when this key turned the row on.
  const hide = on && (chosen.length ? chosen.every((c) => choices.get(c) === "sum") : byKey);
  return { hide, sum, text, pending };
}

/** Pending columns once their totals came: those with numbers get Sum, the rest hold none. */
export function settlePending(pending: readonly number[], totalOf: (c: number) => ColumnTotal | null): { sum: number[]; text: number[]; still: number[] } {
  const sum: number[] = [];
  const text: number[] = [];
  const still: number[] = [];
  for (const c of pending) {
    const t = totalOf(c);
    if (!t) still.push(c);
    else if (t.numbers > 0) sum.push(c);
    else text.push(c);
  }
  return { sum, text, still };
}

/** "amount", "amount and zip", "amount, zip and 3 more". */
export function nameList(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length <= 3) return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  return `${names.slice(0, 2).join(", ")} and ${names.length - 2} more`;
}

/** What the status bar says after a press (or once pending columns settled). */
export function autoSumWords(sum: readonly string[], text: readonly string[], waiting: number): string {
  const parts: string[] = [];
  if (sum.length) parts.push(`Totals row on: Sum of ${nameList(sum)}.`);
  else if (!waiting) parts.push("Totals row on.");
  if (waiting) parts.push(`Working out ${waiting === 1 ? "1 column" : `${waiting} columns`}…`);
  if (text.length) parts.push(`${nameList(text)} ${text.length === 1 ? "holds" : "hold"} no numbers, so ${text.length === 1 ? "it shows" : "they show"} a count.`);
  parts.push("Alt+= again hides it.");
  return parts.join(" ");
}
