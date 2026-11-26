// Data › Find gaps in a sequence: the shapes of the engine's report and the
// words made from it.

export interface GapPattern {
  prefix: string;
  suffix: string;
  /** Digits the number is padded to with zeros (0: not padded). */
  width: number;
}
export interface GapsSpec {
  col: number;
  series: number[];
  keep_first: boolean;
  /** Digit run counted from the right (0 = last); null: chosen for you. */
  run: number | null;
  pattern: GapPattern | null;
  each_shape: boolean;
  start: string;
  end: string;
}
/** A row's place in the file (first line 1); null: added by hand. */
export type FileRow = number | null;
export interface GapRange {
  series: string[];
  from: string;
  to: string;
  count: string;
}
export interface GapReport {
  run: number;
  pattern: GapPattern | null;
  patterns: { pattern: GapPattern; label: string; count: number }[];
  pattern_total: number;
  shown: number;
  rows: number;
  distinct: number;
  blank: number;
  other_shape: number;
  other_rows: FileRow[];
  outside: number;
  drops: number;
  drop_rows: FileRow[];
  added: number;
  duplicate_values: number;
  duplicate_extra: number;
  duplicates: { series: string[]; value: string; count: number; rows: FileRow[] }[];
  series: { key: string[]; first: string; last: string; rows: number; distinct: number; missing: string; too_few: boolean }[];
  series_total: number;
  ranges: GapRange[];
  range_total: number;
  missing: string;
  looks_like_amounts: boolean;
}

/** A whole number of any size with thousands separators. */
export function grouped(n: string | number): string {
  return BigInt(n).toLocaleString();
}

/** `INV-01005 to INV-01009 (5 missing)`: the same words as the engine's. */
export function rangeText(r: GapRange): string {
  const series = r.series.length ? `${r.series.join(" / ")}: ` : "";
  const to = r.from === r.to ? "" : ` to ${r.to}`;
  return `${series}${r.from}${to} (${r.count} missing)`;
}

/** The range lines to copy (never one line per missing number). */
export function copyText(r: GapReport): string {
  const lines = r.ranges.map(rangeText);
  const more = r.range_total - r.ranges.length;
  if (more > 0) lines.push(`${more} more ranges not copied`);
  return lines.map((l) => `${l}\n`).join("");
}

export function rowsText(rows: FileRow[]): string {
  return rows.map((r) => (r === null ? "added by hand" : r.toLocaleString())).join(", ");
}

/** `1 row`, `2 rows`. */
export function counted(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** ` (file row 7)`, ` (file rows 7, 9)`, or nothing. */
export function fileRows(rows: FileRow[], more = false): string {
  return rows.length ? ` (file ${rows.length === 1 && !more ? "row" : "rows"} ${rowsText(rows)}${more ? ", …" : ""})` : "";
}
