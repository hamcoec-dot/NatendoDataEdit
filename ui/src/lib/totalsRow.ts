// The totals row under the table: one number per column over the shown rows. What a cell says, and the
// results kept per tab (asked for only for columns on screen, and again after any change).

import type { ColumnTotal, ColumnTotals } from "./backend";
import { fmt } from "./views";

export type TotalChoice = "sum" | "average" | "count" | "blank" | "min" | "max" | "none";

export const TOTAL_CHOICES: [TotalChoice, string][] = [
  ["sum", "Sum"],
  ["average", "Average"],
  ["count", "Count"],
  ["blank", "Count blank"],
  ["min", "Minimum"],
  ["max", "Maximum"],
  ["none", "None"],
];

const SHORT: Record<TotalChoice, string> = { sum: "Sum", average: "Avg", count: "Count", blank: "Blank", min: "Min", max: "Max", none: "" };

/** Columns asked for in one request (the engine's limit). */
export const MAX_ASKED = 256;
/** Wait before asking again: after a filter or hidden rows, and after the cells changed. */
export const FILTER_DELAY = 150;
export const EDIT_DELAY = 1000;

/** The number of a choice, or null when the column has none (text, nothing filled). */
export function totalNumber(t: ColumnTotal, c: TotalChoice): number | null {
  switch (c) {
    case "sum":
      return t.sum;
    case "average":
      return t.average;
    case "count":
      return t.filled;
    case "blank":
      return t.blank;
    case "min":
      return t.min;
    case "max":
      return t.max;
    case "none":
      return null;
  }
}

/** The choice's short name and the number as shown. */
export function totalParts(t: ColumnTotal, chosen: TotalChoice | undefined): { name: string; value: string } {
  const c = chosen ?? t.suggested;
  if (c === "none") return { name: "", value: "" };
  const x = totalNumber(t, c);
  const value = x !== null ? fmt(x) : t.too_large && (c === "sum" || c === "average") ? "too large" : "\u2014";
  return { name: SHORT[c], value };
}

/** The number as copied: plain digits, no thousands separators; empty when there is none. */
export function totalCopy(t: ColumnTotal | undefined, chosen: TotalChoice | undefined): string {
  if (!t) return "";
  const c = chosen ?? t.suggested;
  const x = totalNumber(t, c);
  return x === null ? "" : String(Number(x.toPrecision(15)));
}

export function totalsLabel(rows: number | null, filtered: boolean): string {
  if (rows === null) return "Total";
  return filtered ? `Total of ${rows.toLocaleString()} shown ${rows === 1 ? "row" : "rows"}` : "Total";
}

/** What a tab keeps: results for one state of its table, and the state they belong to. Old results stay
 *  on screen (drawn faded) until new ones arrive. */
export class TotalsCache {
  /** The state the table is in now (changes with edits, filters, hidden rows, the title row). */
  private wanted = "";
  /** The state of its cells (a change here waits longer before asking). */
  private data = "";
  private found = new Map<number, { key: string; total: ColumnTotal }>();
  private head: { key: string; rows: number; filtered: boolean } | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private timerKey = "";
  private asking = "";

  get(col: number): { total: ColumnTotal; stale: boolean } | null {
    const f = this.found.get(col);
    return f ? { total: f.total, stale: f.key !== this.wanted } : null;
  }

  label(): string {
    return totalsLabel(this.head?.rows ?? null, this.head?.filtered ?? false);
  }

  /** Ask for what is missing among `cols`. `done` runs when new results arrived. */
  want(key: string, data: string, cols: number[], ask: (cols: number[]) => Promise<ColumnTotals>, done: () => void): void {
    const edited = this.data !== "" && data !== this.data;
    this.wanted = key;
    this.data = data;
    const missing = cols.filter((c) => this.found.get(c)?.key !== key).slice(0, MAX_ASKED);
    if (!missing.length) return;
    const want = `${key}|${missing.join(",")}`;
    if (want === this.timerKey || want === this.asking) return;
    clearTimeout(this.timer);
    this.timerKey = want;
    // Scrolling to new columns of the same state asks at once; a change waits for typing to pause.
    const fresh = this.head?.key === key;
    this.timer = setTimeout(
      () => {
        this.timerKey = "";
        this.asking = want;
        ask(missing)
          .then((r) => {
            // A reply for an older state is dropped.
            if (this.wanted !== key) return;
            this.head = { key, rows: r.rows, filtered: r.filtered };
            for (const t of r.columns) this.found.set(t.col, { key, total: t });
            done();
          })
          .catch(() => {
            // Stopped by a change, or the file was busy: ask again on the next draw.
            if (this.wanted === key) setTimeout(done, 300);
          })
          .finally(() => {
            if (this.asking === want) this.asking = "";
          });
      },
      fresh ? 0 : edited ? EDIT_DELAY : FILTER_DELAY,
    );
  }

  /** The row was switched off: nothing is asked for. */
  stop(): void {
    clearTimeout(this.timer);
    this.timerKey = "";
    this.wanted = "";
    this.data = "";
    this.found.clear();
    this.head = null;
  }
}
