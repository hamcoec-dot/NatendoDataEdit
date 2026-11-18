// The filter panel: cards of a column each (a value list with counts, or a
// histogram for numbers and dates). A card's picks are the tab's filter
// rule on that column, so the panel and the filter menus always agree and
// each click is one undo step. Counts come from the engine in one pass.
import type { FacetCard, Facets, FilterSpec, Rule } from "./backend";

export const FACET_BINS = 24;

/** The values ticked on a values card (null: no rule, everything shows). */
export function pickedValues(rule: Rule | null): { values: Set<string>; blanks: boolean } | null {
  if (!rule || rule.type !== "values") return null;
  return { values: new Set(rule.values), blanks: rule.blanks };
}

/** A values card's rule after ticking or unticking one value (null: none
 *  ticked, so the column's rule goes). `all`: the values listed. */
export function toggleValue(rule: Rule | null, value: string, on: boolean): Rule | null {
  const cur = pickedValues(rule) ?? { values: new Set<string>(), blanks: false };
  const values = new Set(cur.values);
  let blanks = cur.blanks;
  if (value === "") blanks = on;
  else if (on) values.add(value);
  else values.delete(value);
  if (!values.size && !blanks) return null;
  return { type: "values", values: [...values], blanks };
}

/** The range picked on a numbers or dates card (null: none). */
export function pickedRange(rule: Rule | null, kind: FacetCard["kind"]): [string, string] | null {
  if (!rule || rule.type !== "conditions") return null;
  const f = rule.first;
  if (kind === "numbers" && f.type === "number_between") return [String(f.low), String(f.high)];
  if (kind === "dates" && f.type === "date_between") return [f.low, f.high];
  return null;
}

export function dayText(d: number): string {
  return new Date(Math.round(d) * 86_400_000).toISOString().slice(0, 10);
}

/** The edges of bar `i` over `range`. */
export function barEdges(range: [number, number], i: number): [number, number] {
  const w = (range[1] - range[0]) / FACET_BINS;
  return [range[0] + w * i, i === FACET_BINS - 1 ? range[1] : range[0] + w * (i + 1)];
}

/** The rule for a range (numbers, or dates as yyyy-mm-dd). */
export function rangeRule(kind: FacetCard["kind"], low: number, high: number): Rule {
  if (kind === "dates") return { type: "conditions", first: { type: "date_between", low: dayText(low), high: dayText(high) } };
  const r = (x: number) => Math.round(x * 1e6) / 1e6;
  return { type: "conditions", first: { type: "number_between", low: r(low), high: r(high) } };
}

/** "Blank only" or "not blank" on a card. */
export function blankRule(blank: boolean): Rule {
  return { type: "conditions", first: { type: blank ? "blank" : "not_blank" } };
}

/** The browser stub's counts (the desktop app's come from the engine). */
export function stubFacets(rows: string[][], cards: FacetCard[], spec: FilterSpec, test: (r: Rule, v: string) => boolean): Facets {
  const num = (v: string): number | null => {
    const n = Number(v.replace(/[$,]/g, ""));
    return v.trim() === "" || Number.isNaN(n) ? null : n;
  };
  const day = (v: string): number | null => {
    const t = Date.parse(v);
    return Number.isNaN(t) ? null : Math.floor(t / 86_400_000);
  };
  const ranges = cards.map((c) => {
    if (c.kind === "values" || c.range) return c.range;
    let lo = Infinity;
    let hi = -Infinity;
    for (const r of rows) {
      const x = c.kind === "numbers" ? num(r[c.col] ?? "") : day(r[c.col] ?? "");
      if (x !== null) [lo, hi] = [Math.min(lo, x), Math.max(hi, x)];
    }
    return lo <= hi ? ([lo, hi] as [number, number]) : null;
  });
  let shown = 0;
  const out = cards.map((c, k) => ({ col: c.col, map: new Map<string, number>(), bins: new Array<number>(c.kind === "values" ? 0 : FACET_BINS).fill(0), other: 0, blanks: 0, rows: 0, range: ranges[k] ?? null }));
  for (const r of rows) {
    const failed = spec.columns.filter((f) => !test(f.rule, r[f.col] ?? ""));
    if (!failed.length) shown++;
    if (failed.length > 1) continue;
    cards.forEach((c, k) => {
      if (failed.length === 1 && failed[0]!.col !== c.col) return;
      const a = out[k]!;
      a.rows++;
      const v = r[c.col] ?? "";
      if (v.trim() === "") {
        a.blanks++;
        return;
      }
      if (c.kind === "values") {
        if (c.search && !v.toLowerCase().includes(c.search.toLowerCase())) return;
        a.map.set(v, (a.map.get(v) ?? 0) + 1);
        return;
      }
      const x = c.kind === "numbers" ? num(v) : day(v);
      const rg = a.range;
      if (x === null || !rg || x < rg[0] || x > rg[1]) {
        a.other++;
        return;
      }
      a.bins[rg[1] > rg[0] ? Math.min(FACET_BINS - 1, Math.floor(((x - rg[0]) / (rg[1] - rg[0])) * FACET_BINS)) : 0]!++;
    });
  }
  return {
    shown,
    cards: out.map((a) => {
      const values = [...a.map].map(([value, count]) => ({ value, count })).sort((x, y) => y.count - x.count || x.value.localeCompare(y.value));
      return { col: a.col, values: values.slice(0, 1000), distinct: values.length, complete: values.length <= 1000, bins: a.bins, range: a.range, other: a.other, blanks: a.blanks, rows: a.rows };
    }),
  };
}
