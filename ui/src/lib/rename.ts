// Renaming a column: the expressions the tab keeps (data checks' row rules)
// name columns as [title]. These follow a rename, so a rule that pointed at
// the column still does afterwards.
import type { Check } from "./backend";

/** The column an expression's [name] points at, as the engine finds it:
 *  #n by number, else the exact title, else the title ignoring case. */
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

/** `expr` with every [name] that points at column `col` (under the titles
 *  before the rename) written as the new name. Text in quotes and #n
 *  references are left alone; an expression with an unclosed quote or
 *  bracket is returned unchanged. */
export function renameInExpr(expr: string, titles: string[], col: number, name: string): string {
  let out = "";
  let i = 0;
  while (i < expr.length) {
    const ch = expr[i]!;
    if (ch === '"') {
      let j = i + 1;
      for (;;) {
        if (j >= expr.length) return expr;
        if (expr[j] === '"' && expr[j + 1] === '"') j += 2;
        else if (expr[j] === '"') break;
        else j++;
      }
      out += expr.slice(i, j + 1);
      i = j + 1;
    } else if (ch === "[") {
      let j = i + 1;
      let ref = "";
      for (;;) {
        if (j >= expr.length) return expr;
        if (expr[j] === "]" && expr[j + 1] === "]") {
          ref += "]";
          j += 2;
        } else if (expr[j] === "]") break;
        else ref += expr[j++];
      }
      out += !ref.startsWith("#") && resolveColumn(ref, titles) === col ? columnRef(name) : expr.slice(i, j + 1);
      i = j + 1;
    } else {
      out += ch;
      i++;
    }
  }
  return out;
}

/** The checks with their row rules following a rename of column `col`
 *  (titles as they were before), and how many rules changed. */
export function renameInChecks(checks: Check[], titles: string[], col: number, name: string): { checks: Check[]; changed: number } {
  let changed = 0;
  const out = checks.map((c) => {
    if (c.kind.type !== "expression") return c;
    const expr = renameInExpr(c.kind.expr, titles, col, name);
    if (expr === c.kind.expr) return c;
    changed++;
    return { ...c, kind: { ...c.kind, expr } };
  });
  return { checks: out, changed };
}
