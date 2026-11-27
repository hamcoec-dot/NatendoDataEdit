// File contracts (C9): what the banner and the Review panel say about a drift report.

import type { Contract, DriftReport } from "./backend";

const n = (k: number, one: string, many = `${one}s`): string => `${k.toLocaleString()} ${k === 1 ? one : many}`;

/** Points a share of blanks moved (rounded), signed. */
export function blankPoints(rate: number, baseline: number): number {
  return Math.round((rate - baseline) * 100);
}

/** The banner's short list of what drifted ("1 renamed column, 2 new values in party, …"), at most four parts. */
export function driftSummary(r: DriftReport): string {
  const parts: string[] = [];
  const renamed = r.renamed.filter((x) => !x.by_alias).length;
  if (renamed) parts.push(n(renamed, "renamed column"));
  if (r.missing.length) parts.push(n(r.missing.length, "missing column"));
  if (r.added.length) parts.push(n(r.added.length, "new column"));
  if (r.moved.length) parts.push(n(r.moved.length, "moved column"));
  for (const c of r.columns) if (c.new_values) parts.push(`${n(c.new_values, "new value")} in ${c.title}`);
  for (const c of r.columns) if (c.type_misses) parts.push(`${n(c.type_misses, "cell")} in ${c.title} not of its type`);
  for (const c of r.columns)
    if (c.blank_jump && c.baseline !== null) {
      const p = blankPoints(c.blank_rate, c.baseline);
      parts.push(`blanks in ${c.title} ${p >= 0 ? "up" : "down"} ${n(Math.abs(p), "point")}`);
    }
  const broken = r.rules.filter((x) => x.failing).length;
  if (broken) parts.push(n(broken, "rule") + " broken");
  if (r.table.some((t) => t.ok === false)) parts.push("a table check fails");
  if (r.rows_out && r.row_range) parts.push(`${n(r.rows, "row")} (expected ${r.row_range.min.toLocaleString()} to ${r.row_range.max.toLocaleString()})`);
  if (!parts.length) return "nothing drifted";
  if (parts.length > 4) return `${parts.slice(0, 3).join(", ")} and ${parts.length - 3} more`;
  return parts.join(", ");
}

/** The contract after "Accept these new values" for the columns given (at most 500 values a column). */
export function acceptValues(c: Contract, r: DriftReport, cols: number[]): { contract: Contract; full: string[] } {
  const next: Contract = JSON.parse(JSON.stringify(c)) as Contract;
  const full: string[] = [];
  for (const k of cols) {
    const d = r.columns.find((x) => x.col === k);
    const col = next.columns[k];
    if (!d || !col?.allowed) continue;
    const have = new Set(col.allowed.values);
    const add = d.first_new.filter((v) => !have.has(v));
    if (col.allowed.values.length + add.length > 500) {
      full.push(col.title);
      continue;
    }
    col.allowed = { ...col.allowed, values: [...col.allowed.values, ...add] };
  }
  return { contract: next, full };
}

/** The contract after "Update the baseline": today's blank shares, and the row range around today's count (±20%). */
export function updateBaseline(c: Contract, r: DriftReport): Contract {
  const next: Contract = JSON.parse(JSON.stringify(c)) as Contract;
  for (const d of r.columns) {
    const col = next.columns[d.col];
    if (col) col.blank_rate = d.blank_rate;
  }
  if (next.rows) next.rows = { min: Math.floor(r.rows * 0.8), max: Math.ceil(r.rows * 1.2) };
  return next;
}

/** The contract with a file's title kept as another title of its column. */
export function addAlias(c: Contract, col: number, title: string): Contract {
  const next: Contract = JSON.parse(JSON.stringify(c)) as Contract;
  const x = next.columns[col];
  if (x && !x.aliases?.includes(title) && x.title !== title) x.aliases = [...(x.aliases ?? []), title].slice(0, 20);
  return next;
}

/** Single-column moves that turn the order 0..n into `order` (new place i holds old column order[i]), as the
 *  grid's column state follows them one by one. */
export function movesFor(order: number[]): { at: number; to: number }[] {
  const cur = order.map((_, i) => i);
  const out: { at: number; to: number }[] = [];
  order.forEach((want, i) => {
    const at = cur.indexOf(want);
    if (at !== i) {
      out.push({ at, to: i });
      cur.splice(at, 1);
      cur.splice(i, 0, want);
    }
  });
  return out;
}
