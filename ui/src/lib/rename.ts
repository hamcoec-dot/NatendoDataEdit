// Expressions the app keeps (data checks' row rules) name columns as
// [title]. When a column is renamed, or a saved view's columns are matched
// to another file's by place, those names are rewritten so each rule still
// points at the same column. Where a name would no longer lead there (two
// columns share it), the reference is pinned to the column's number, [#n],
// which always does.
import type { Check } from "./backend";

/** Titles as the engine names columns in expressions: an empty title is
 *  "Column N". */
export function engineTitles(titles: string[]): string[] {
  return titles.map((t, c) => (t === "" ? `Column ${c + 1}` : t));
}

/** The column an expression's [name] points at, as the engine finds it:
 *  #n by number, else the first exact title, else the first title equal
 *  ignoring case. `titles` as from engineTitles. */
export function resolveColumn(name: string, titles: string[]): number | null {
  if (name.startsWith("#")) {
    const k = Number(name.slice(1).trim());
    return Number.isInteger(k) && k >= 1 && k <= titles.length ? k - 1 : null;
  }
  const exact = titles.indexOf(name);
  if (exact >= 0) return exact;
  const low = name.toLowerCase();
  const loose = titles.findIndex((t) => t.toLowerCase() === low);
  return loose >= 0 ? loose : null;
}

/** [name] as an expression writes it (] doubled). */
export function columnRef(name: string): string {
  return `[${name.replace(/]/g, "]]")}]`;
}

/** Each [name] in `expr`, by the text it covers. Text in quotes is skipped;
 *  null when a quote or bracket is not closed (the expression is left as
 *  it is). */
function refs(expr: string): { start: number; end: number; name: string }[] | null {
  const out: { start: number; end: number; name: string }[] = [];
  let i = 0;
  while (i < expr.length) {
    const ch = expr[i]!;
    if (ch === '"') {
      let j = i + 1;
      for (;;) {
        if (j >= expr.length) return null;
        if (expr[j] === '"' && expr[j + 1] === '"') j += 2;
        else if (expr[j] === '"') break;
        else j++;
      }
      i = j + 1;
    } else if (ch === "[") {
      let j = i + 1;
      let name = "";
      for (;;) {
        if (j >= expr.length) return null;
        if (expr[j] === "]" && expr[j + 1] === "]") {
          name += "]";
          j += 2;
        } else if (expr[j] === "]") break;
        else name += expr[j++];
      }
      out.push({ start: i, end: j + 1, name });
      i = j + 1;
    } else i++;
  }
  return out;
}

/** `expr`, written against columns titled `from`, rewritten for columns
 *  titled `to`, where column k of `from` is column `place(k)` of `to`
 *  (null: gone, its references are left as they are). A reference keeps
 *  its text while it still leads to the right column; otherwise it takes
 *  that column's title, or its number [#n] when the title would lead
 *  elsewhere. #n references are left alone. Titles raw (engineTitles is
 *  applied here). Returns the expression and how many references were
 *  pinned to a number. */
export function remapExpr(
  expr: string,
  from: string[],
  to: string[],
  place: (k: number) => number | null,
): { expr: string; pinned: number } {
  const list = refs(expr);
  if (!list) return { expr, pinned: 0 };
  const f = engineTitles(from);
  const t = engineTitles(to);
  let out = "";
  let last = 0;
  let pinned = 0;
  for (const r of list) {
    if (r.name.startsWith("#")) continue;
    const k = resolveColumn(r.name, f);
    if (k === null) continue;
    const c = place(k);
    if (c === null || c >= t.length || resolveColumn(r.name, t) === c) continue;
    let ref: string;
    if (resolveColumn(t[c]!, t) === c) ref = columnRef(t[c]!);
    else {
      ref = `[#${c + 1}]`;
      pinned++;
    }
    out += expr.slice(last, r.start) + ref;
    last = r.end;
  }
  return { expr: out + expr.slice(last), pinned };
}

/** `expr` after column `col` (of `titles`, raw) is renamed to `name`. */
export function renameInExpr(expr: string, titles: string[], col: number, name: string): string {
  const after = titles.slice();
  after[col] = name;
  return remapExpr(expr, titles, after, (k) => k).expr;
}

/** Row rules among `checks` rewritten by remapExpr; how many changed and
 *  how many references were pinned to a number. */
export function remapChecks(
  checks: Check[],
  from: string[],
  to: string[],
  place: (k: number) => number | null,
): { checks: Check[]; changed: number; pinned: number } {
  let changed = 0;
  let pinned = 0;
  const out = checks.map((c) => {
    if (c.kind.type !== "expression") return c;
    const r = remapExpr(c.kind.expr, from, to, place);
    if (r.expr === c.kind.expr) return c;
    changed++;
    pinned += r.pinned;
    return { ...c, kind: { ...c.kind, expr: r.expr } };
  });
  return { checks: out, changed, pinned };
}

/** The checks with their row rules following a rename of column `col`
 *  (titles as they were before). */
export function renameInChecks(checks: Check[], titles: string[], col: number, name: string): { checks: Check[]; changed: number; pinned: number } {
  const after = titles.slice();
  after[col] = name;
  return remapChecks(checks, titles, after, (k) => k);
}
