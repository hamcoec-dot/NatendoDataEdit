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

/** A value's shape as the engine's `shape_of` makes it: digits 9, letters A, other
 *  characters as they are, at most 40 characters and then an ellipsis. */
export function shapeOf(s: string): string {
  let out = "";
  let n = 0;
  for (const c of s) {
    if (n >= 40) return out + "…";
    out += /\p{N}/u.test(c) ? "9" : /\p{Alphabetic}/u.test(c) ? "A" : c;
    n++;
  }
  return out;
}

/** `shapeOf(s) === target`, the engine's exact rule. */
export function shapeMatches(s: string, target: string): boolean {
  return shapeOf(s) === target;
}

/** The pill's two parts, as drawn: the shape with its share ("~99.4% 999-9999", the ~ while the figure is
 *  from a sample) and the leftover ("0.6% differ"); null when the column has no pill. */
export function pillParts(i: Insight, sampled: number, rows: number): { shape: string; rest: string; share: number } | null {
  if (!i.top_shape || !i.top_share) return null;
  const tilde = sampled < rows ? "~" : "";
  const share = Math.min(99.9, Math.round(i.top_share * 1000) / 10);
  const odd = Math.max(0.1, Math.round((1 - i.top_share) * 1000) / 10);
  return { shape: `${tilde}${share}% ${i.top_shape}`, rest: ` · ${odd}% differ`, share: i.top_share };
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
export function stubInsights(cols: number[], values: (c: number) => string[], revision = 0): Insights {
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
    // The pattern pill: the shape making up at least 90% of the filled cells while another exists
    // (ties to the smallest shape text, as the engine does).
    const shapes = new Map<string, number>();
    for (const v of vals) if (v.trim()) shapes.set(shapeOf(v), (shapes.get(shapeOf(v)) ?? 0) + 1);
    const shapeMost = Math.max(0, ...shapes.values());
    const topShape = [...shapes].filter(([, n]) => n === shapeMost).map(([s]) => s).sort()[0];
    const dominant = filled >= 2 && shapeMost / filled >= 0.9 && shapeMost < filled;
    return {
      col,
      top_shape: dominant ? topShape : null,
      top_share: dominant ? shapeMost / filled : 0,
      filled: vals.length ? filled / vals.length : 0,
      distinct: counts.size,
      unique: filled > 1 && counts.size === filled,
      numbers,
      bars: most ? bars.map((b) => b / most) : bars,
    };
  });
  return { cols: out, sampled, rows: sampled, revision };
}
