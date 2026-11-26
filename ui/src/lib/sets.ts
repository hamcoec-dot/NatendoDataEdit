// Data › Compare lists: the shapes of the engine's `sets` module and the
// words made from them.

export interface SetList {
  cols: number[];
  header: boolean;
}
export interface SetsSpec {
  lists: SetList[];
  match_case: boolean;
  trim: boolean;
  /** The join's loose keys. */
  loose: boolean;
}
export interface SetSlice {
  /** Bit 0 is the first list. */
  mask: number;
  keys: number;
  rows: number[];
}
export interface SetsReport {
  lists: { rows: number; keys: number; blank: number }[];
  slices: SetSlice[];
}
export interface SetsRun {
  id: number;
  report: SetsReport;
  names: string[];
}

export const MAX_LISTS = 5;
export const MAX_SET_KEYS = 8;

/** `A and B, not C`: the same words as the engine's. */
export function sliceName(mask: number, names: string[]): string {
  const pick = (held: boolean) => names.filter((_, i) => ((mask & (1 << i)) !== 0) === held);
  const join = (v: string[], last: string) => (v.length < 2 ? (v[0] ?? "") : `${v.slice(0, -1).join(", ")} ${last} ${v.at(-1)}`);
  const inside = pick(true);
  const outside = pick(false);
  if (!outside.length) return join(inside, "and");
  if (inside.length === 1) return `${inside[0]} only`;
  return `${join(inside, "and")}, not ${join(outside, "or")}`;
}

/** The lists in slice `mask`, by place. */
export function listsIn(mask: number, count: number): number[] {
  return Array.from({ length: count }, (_, i) => i).filter((i) => (mask & (1 << i)) !== 0);
}

/** The counts as tab-separated text to paste into a sheet. */
export function countsText(run: SetsRun, empty: boolean): string {
  const cell = (s: string) => s.replace(/[\t\r\n]+/g, " ");
  const lines = [["Slice", "Different keys", ...run.names.map((n) => `Rows in ${cell(n)}`)]];
  for (const s of run.report.slices) if (empty || s.keys) lines.push([cell(sliceName(s.mask, run.names)), String(s.keys), ...s.rows.map(String)]);
  run.report.lists.forEach((l, i) => lines.push([`${cell(run.names[i]!)}: ${l.rows} rows, ${l.keys} different keys, ${l.blank} with a blank key`]));
  return lines.map((l) => `${l.join("\t")}\n`).join("");
}
