// The date radar's words and rules for the screen: the pill, the summary and the tooltip of an amber cell.
// Which cells are ambiguous is the engine's answer (datescan.rs).

import type { DateScan } from "./backend";
import type { ColumnType } from "./readers";

/** Rows counted first; the rest only when asked. */
export const DATE_SAMPLE = 200_000;

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** What an amber cell could mean: "03/04/2026 could be 4 March or 3 April". */
export function dateTip(value: string): string {
  const m = /^\s*(\d{1,2})[-/. ,]+(\d{1,2})[-/. ,]+(\d{2}|\d{4})(?!\d)/.exec(value);
  const a = Number(m?.[1]);
  const b = Number(m?.[2]);
  if (!m || a < 1 || a > 12 || b < 1 || b > 12 || a === b) return "This date can only be read the other way round from this column's setting.";
  return `${value.trim()} could be ${b} ${MONTHS[a - 1]} or ${a} ${MONTHS[b - 1]}`;
}

/** Why the radar cannot look at a column of this type ("" when it can). */
export function dateWhy(type: ColumnType | undefined): string {
  if (type === "text") return "this column is set to Text: its cells are not read as dates";
  if (type === "id") return "this column is set to Id: its cells are not read as dates";
  return "";
}

const n = (v: number, one: string, many = `${one}s`) => `${v.toLocaleString()} ${v === 1 ? one : many}`;

/** The pill's text. */
export function datePill(s: DateScan): string {
  if (s.set_order) return `Dates the other way round: ${s.other_order.toLocaleString()}`;
  return `Ambiguous dates: ${s.ambiguous.toLocaleString()}`;
}

/** Whether only the first rows were counted. */
export const dateSampled = (s: DateScan): boolean => s.looked < s.rows;

/** The line said when the radar is switched on. */
export function dateSummary(s: DateScan, column: string): string {
  const from = dateSampled(s) ? ` (from the first ${s.looked.toLocaleString()} of ${s.rows.toLocaleString()} rows)` : "";
  if (s.set_order) {
    const order = s.set_order === "dmy" ? "day first" : "month first";
    const other = s.set_order === "dmy" ? "month first" : "day first";
    return `${column} is set to ${order}. ${n(s.other_order, "cell")} can only be read ${other}${from}.`;
  }
  const parts = [`${n(s.ambiguous, "date")} could be read two ways`];
  if (s.month_first_only) parts.push(`${s.month_first_only.toLocaleString()} only month first`);
  if (s.day_first_only) parts.push(`${s.day_first_only.toLocaleString()} only day first`);
  const other = s.iso + s.written + s.same_either + s.compact;
  if (other) parts.push(`${other.toLocaleString()} read one way`);
  if (s.not_date) parts.push(`${s.not_date.toLocaleString()} not dates`);
  let text = `${column}: ${parts.join(", ")}${from}.`;
  if (s.month_first_only && s.day_first_only) text += " This column mixes both orders (maybe two sources).";
  else if (s.ambiguous && !s.month_first_only && !s.day_first_only) text += " Nothing in this column tells the order.";
  if (s.compact_numbers) text += ` ${n(s.compact_numbers, "eight-digit value")} could be dates and ${s.compact_numbers === 1 ? "is" : "are"} taken as numbers.`;
  return text;
}
