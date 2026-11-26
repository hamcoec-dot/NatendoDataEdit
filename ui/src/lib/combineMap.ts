// Combine with a column map: the pure parts of the mapper screen (the engine
// checks every plan again before combining).
import type { CombinePlan, FileHead } from "./backend";

/** A copy of a plan. */
export function clonePlan(p: CombinePlan): CombinePlan {
  return { titles: [...p.titles], files: p.files.map((m) => [...m]) };
}

/** Which column of file `f` feeds output `out` (null: none). */
export function sourceOf(p: CombinePlan, f: number, out: number): number | null {
  const c = p.files[f]?.indexOf(out) ?? -1;
  return c < 0 ? null : c;
}

/** Feed output `out` from column `col` of file `f` (null: from none). The
 *  column that fed it before is left out; `col` leaves the output it fed. */
export function setSource(p: CombinePlan, f: number, out: number, col: number | null): CombinePlan {
  const q = clonePlan(p);
  const map = q.files[f];
  if (!map) return q;
  for (let c = 0; c < map.length; c++) if (map[c] === out) map[c] = null;
  if (col !== null && col >= 0 && col < map.length) map[col] = out;
  return q;
}

/** Give a left-out column of file `f` an output of its own, at the end. */
export function addAsColumn(p: CombinePlan, f: number, col: number, title: string): CombinePlan {
  const q = clonePlan(p);
  const map = q.files[f];
  if (!map || col < 0 || col >= map.length) return q;
  q.titles.push(title.trim() || `Column ${col + 1}`);
  map[col] = q.titles.length - 1;
  return q;
}

/** Move every column feeding `merge` onto `keep`; refused (null) when a file
 *  feeds both, so two columns of one file never land on one output. */
export function mergePlan(p: CombinePlan, keep: number, merge: number): CombinePlan | null {
  if (keep === merge) return null;
  if (p.files.some((m) => m.includes(keep) && m.includes(merge))) return null;
  const q = clonePlan(p);
  for (const m of q.files) for (let c = 0; c < m.length; c++) if (m[c] === merge) m[c] = keep;
  return compactPlan(q);
}

/** Outputs no file feeds are taken out, the others renumbered. */
export function compactPlan(p: CombinePlan): CombinePlan {
  const fed = p.titles.map((_, i) => p.files.some((m) => m.includes(i)));
  if (fed.every(Boolean)) return p;
  const to: (number | null)[] = [];
  const titles: string[] = [];
  p.titles.forEach((t, i) => {
    if (fed[i]) {
      to[i] = titles.length;
      titles.push(t);
    } else to[i] = null;
  });
  return { titles, files: p.files.map((m) => m.map((x) => (x === null ? null : (to[x] ?? null)))) };
}

/** Columns of file `f` that go nowhere. */
export function leftOut(p: CombinePlan, f: number): number[] {
  return (p.files[f] ?? []).flatMap((x, c) => (x === null ? [c] : []));
}

/** The preview of file `f` as its rows will land: one row per preview row, one cell per output. */
export function landed(p: CombinePlan, files: FileHead[], f: number): string[][] {
  const map = p.files[f] ?? [];
  return (files[f]?.preview ?? []).map((row) => {
    const out = p.titles.map(() => "");
    map.forEach((to, c) => {
      if (to !== null && to < out.length) out[to] = row[c] ?? "";
    });
    return out;
  });
}

/** The same plan, ignoring nothing: true when a and b map every column alike. */
export function samePlan(a: CombinePlan, b: CombinePlan): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The files read again have the same names, widths and titles, in the same order: a map made by
 *  hand still means what it meant, so it is kept after a file changed. */
export function sameTitles(a: FileHead[], b: FileHead[]): boolean {
  return (
    a.length === b.length &&
    a.every((f, i) => {
      const g = b[i];
      return !!g && f.name === g.name && f.cols === g.cols && f.titles.length === g.titles.length && f.titles.every((t, k) => t === g.titles[k]);
    })
  );
}
