// Check fields (Milestone 15): one pattern box per column, kept as the
// Data checks' pattern checks so they go with views and the command line.
import type { Check } from "./backend";

/** The pattern box text per column from the tab's checks (the first
 *  regular-expression pattern check on each column). */
export function patternChecks(checks: Check[], cols: number): string[] {
  const out: string[] = Array.from({ length: cols }, () => "");
  for (const c of checks) {
    if (c.kind.type !== "pattern" || c.col === null || c.col === undefined) continue;
    if (c.kind.pattern.kind !== "regex" || c.col >= cols || out[c.col]) continue;
    out[c.col] = c.kind.pattern.text;
  }
  return out;
}

/** The tab's checks with the boxes put in: each column's first regex
 *  pattern check is replaced (or removed when its box is empty), new ones
 *  are added; other checks stay as they are. */
export function withPatterns(checks: Check[], boxes: string[], whole: boolean): Check[] {
  const done = new Set<number>();
  const out: Check[] = [];
  for (const c of checks) {
    const col = c.col ?? -1;
    if (c.kind.type === "pattern" && c.kind.pattern.kind === "regex" && col >= 0 && col < boxes.length && !done.has(col)) {
      done.add(col);
      const text = (boxes[col] ?? "").trim();
      if (text) out.push({ col, kind: { type: "pattern", pattern: { ...c.kind.pattern, text, whole_cell: whole } } });
      continue;
    }
    out.push(c);
  }
  boxes.forEach((b, col) => {
    const text = b.trim();
    if (text && !done.has(col)) out.push({ col, kind: { type: "pattern", pattern: { text, kind: "regex", match_case: false, whole_cell: whole } } });
  });
  return out;
}
