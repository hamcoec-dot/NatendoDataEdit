// The column insight strip (View › Column insights): how full each column
// is, how many different values it holds and a tiny bar chart, from a sample
// of the shown rows. A small sample comes first, a bigger one after.

import type { Insight, Insights } from "./backend";

/** Rows in the first, quick sample and in the second one. */
export const QUICK_SAMPLE = 5_000;
export const FULL_SAMPLE = 100_000;
/** Columns covered (the engine's limit). */
export const MAX_INSIGHT_COLS = 512;
/** Bars drawn (the engine's limit). */
export const BARS = 8;

/** A count in a few characters: 950, 1.2k, 34k, 1.5M. */
export function shortCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

/** The strip's text: "92% · 1.2k" (filled share, different values). */
export function insightText(i: Insight, sampled: number, rows: number): string {
  const pct = Math.round(i.filled * 100);
  const filled = pct === 100 && i.filled < 1 ? 99 : pct === 0 && i.filled > 0 ? 1 : pct;
  const more = sampled < rows && !i.unique ? "+" : "";
  const distinct = i.unique ? "all different" : `${shortCount(i.distinct)}${more}`;
  return `${filled}% · ${distinct}`;
}

/** A tooltip-length description, for the status line. */
export function insightSays(title: string, i: Insight, sampled: number, rows: number): string {
  const pct = Math.round(i.filled * 100);
  const of = sampled < rows ? ` (from a sample of ${shortCount(sampled)} of ${shortCount(rows)} rows)` : "";
  const what = i.unique ? "every value is different" : `${i.distinct.toLocaleString()} different values`;
  const bars = i.numbers ? "the bars show how the numbers spread" : "the bars are the most common values";
  return `${title}: ${pct}% filled, ${what}; ${bars}${of}. Click the strip for the column profile.`;
}

/** The browser preview's stand-in for the engine. */
export function stubInsights(cols: number[], values: (c: number) => string[]): Insights {
  let sampled = 0;
  const out: Insight[] = cols.map((col) => {
    const vals = values(col);
    sampled = vals.length;
    const counts = new Map<string, number>();
    const nums: number[] = [];
    let filled = 0;
    for (const v of vals) {
      if (!v.trim()) continue;
      filled++;
      counts.set(v, (counts.get(v) ?? 0) + 1);
      const x = Number(v.replace(/[$,]/g, ""));
      if (v.trim() && Number.isFinite(x)) nums.push(x);
    }
    const numbers = filled > 0 && nums.length * 10 >= filled * 9 && counts.size > BARS;
    let bars: number[];
    if (numbers) {
      const lo = Math.min(...nums);
      const hi = Math.max(...nums);
      bars = new Array<number>(BARS).fill(0);
      for (const x of nums) bars[hi > lo ? Math.min(BARS - 1, Math.floor(((x - lo) / (hi - lo)) * BARS)) : 0]!++;
    } else bars = [...counts.values()].sort((a, b) => b - a).slice(0, BARS);
    const most = Math.max(0, ...bars);
    return {
      col,
      filled: vals.length ? filled / vals.length : 0,
      distinct: counts.size,
      unique: filled > 1 && counts.size === filled,
      numbers,
      bars: most ? bars.map((b) => b / most) : bars,
    };
  });
  return { cols: out, sampled, rows: sampled };
}
