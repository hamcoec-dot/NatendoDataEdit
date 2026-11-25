// What a person says a column holds (Text, Number, Date, Id), and the ONE place the screen reads a cell as a
// number or a date. Everything the grid draws or copies that depends on a cell being a number (colour scales,
// the value-type colours, quoting on Save As, number alignment in print) goes through here, so a ZIP column
// typed Id never gets a colour scale or a bare, unquoted copy. The engine has its own reader for the same
// rule (crates/engine/src/coltype.rs); a vitest scan (readers.test.ts) keeps other number reads off the
// screen code.

import { readDate, readNumber } from "./m9";

export type DateOrder = "mdy" | "dmy";
/** As the engine writes it: "auto" | "text" | "number" | "id" | { date: order or null (the file's own order) }. */
export type ColumnType = "auto" | "text" | "number" | "id" | { date: DateOrder | null };
/** The typed columns of a tab as shown: [logical column, type] (Auto columns are left out). */
export type TypedColumns = readonly (readonly [number, ColumnType])[] | undefined;
/** The types by physical column, as the engine keeps them (for Undo, saved views and the session). */
export interface ColumnTypes {
  types: ColumnType[];
}

export const NO_TYPES: ColumnTypes = { types: [] };

export function typeOf(typed: TypedColumns, col: number): ColumnType {
  if (!typed) return "auto";
  for (const [c, t] of typed) if (c === col) return t;
  return "auto";
}

/** Text and Id columns: never numbers, never dates. */
export function isTextLike(t: ColumnType): boolean {
  return t === "text" || t === "id";
}

/** The logical columns that are Text or Id. */
export function textCols(typed: TypedColumns): Set<number> {
  const s = new Set<number>();
  for (const [c, t] of typed ?? []) if (isTextLike(t)) s.add(c);
  return s;
}

/** The number a cell of column `col` holds (null in a Text, Id or Date column). */
export function readNumberFor(typed: TypedColumns, col: number, text: string): number | null {
  const t = typeOf(typed, col);
  if (t === "auto" || t === "number") return readNumber(text);
  return null;
}

/** The date a cell of column `col` holds (null in a Text, Id or Number column). `fileDayFirst`: the file-wide
 *  date order; a Date column with an order of its own overrides it. */
export function readDateFor(typed: TypedColumns, col: number, text: string, fileDayFirst = false): Date | null {
  const t = typeOf(typed, col);
  if (t === "auto") return readDate(text, fileDayFirst);
  if (typeof t === "object") return readDate(text, t.date === null ? fileDayFirst : t.date === "dmy");
  return null;
}

/** The short label of a type, for the badge in the column strip ("" for Auto). */
export function typeBadge(t: ColumnType): string {
  if (t === "text") return "T";
  if (t === "number") return "#";
  if (t === "id") return "ID";
  if (typeof t === "object") return "D";
  return "";
}

/** The name of a type, for menus and the status bar. */
export function typeName(t: ColumnType): string {
  if (t === "auto") return "Auto";
  if (t === "text") return "Text";
  if (t === "number") return "Number";
  if (t === "id") return "Id";
  return t.date === "dmy" ? "Date (day first)" : t.date === "mdy" ? "Date (month first)" : "Date";
}
