// Match people (Milestone 15): types and the accept/reject list.

export type FieldKind = "name" | "text" | "exact";

export interface MatchField {
  col: number;
  other_col: number;
  kind: FieldKind;
  weight: number;
}

export interface MatchSpec {
  fields: MatchField[];
  keep_first: boolean;
  other_header: boolean;
  min_score: number;
  read: { decimal_comma: boolean; date_order: "mdy" | "dmy" | "ymd" };
}

export interface Candidate {
  row: number;
  id: number;
  other_row: number;
  score: number;
  /** Each field's likeness, 0 to 1 (-1: blank on one side). */
  parts: number[];
  here: string[];
  there: string[];
}

export interface MatchReport {
  listed: Candidate[];
  total: number;
  rows_matched: number;
  compared: number;
  /** Rows in a very large group (a common name with nothing to narrow
   * it), compared only with their nearest rows there. */
  windowed: number;
}

export const KIND_LABELS: [FieldKind, string][] = [
  ["name", "Name (nicknames, spelling)"],
  ["text", "Other text (spelling)"],
  ["exact", "Must be the same (date, ZIP, id)"],
];

/** A first guess at how to compare a column, from its title. */
export function guessKind(title: string): FieldKind {
  const t = title.toLowerCase();
  if (/(birth|dob|date|born|zip|postal|phone|ssn|\bid\b|number|no\.?$)/.test(t)) return "exact";
  if (/(name|first|last|given|family|surname|middle)/.test(t)) return "name";
  return "text";
}

/** Pair this table's columns with the other's by title (case and spaces
 *  ignored), for name, date and ZIP-like titles first. */
export function guessFields(mine: string[], theirs: string[]): MatchField[] {
  const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
  const out: MatchField[] = [];
  mine.forEach((t, c) => {
    if (out.length >= 4) return;
    const kind = guessKind(t);
    if (kind === "text") return;
    const o = theirs.findIndex((x) => norm(x) === norm(t));
    if (o >= 0) out.push({ col: c, other_col: o, kind, weight: 1 });
  });
  return out;
}

/** Accepted pairs as the app takes them: [row id, other row, score]. */
export function acceptedPairs(list: Candidate[], accepted: Set<number>): [number, number, number][] {
  const out: [number, number, number][] = [];
  list.forEach((c, i) => {
    if (accepted.has(i)) out.push([c.id, c.other_row, c.score]);
  });
  return out;
}
