// Milestone 10 helpers (LibreOffice Calc and ASAP Utilities gaps): what the
// Tools dialogs ask the engine for, value-type colours, colour scales and
// data bars, group dividers, type-ahead and custom sort orders. Pure
// functions, so they are tested without a grid.

import type { Combine, DataOp, FillOrder, SplitParts, TableOp } from "./backend";
import { readDate, readNumber } from "./m9";
import type { ColumnType } from "./readers";

// ---- Tools dialogs ------------------------------------------------------------

export type ToolKind =
  | "fill_blanks"
  | "count_repeats"
  | "blank_repeats"
  | "breaks"
  | "reshape"
  | "stack"
  | "sample"
  | "compare"
  | "split"
  | "combine";

export interface ToolChoices {
  cols: number[];
  col: number;
  header: boolean;
  from: "above" | "below" | "value";
  text: string;
  spaces: boolean;
  matchCase: boolean;
  trim: boolean;
  byCol: boolean;
  every: number;
  /** Split: the CSV formula guard tick. */
  guard?: boolean;
  blankRows: number;
  perRow: number;
  down: boolean;
  skipBlanks: boolean;
  sizeKind: "rows" | "percent" | "every";
  size: number;
  seed: number;
  /** Sample: draw from each group of this column (-1: all rows at once). */
  per?: number;
  drawnOrder?: boolean;
  rowNumbers?: boolean;
  other: number;
  wholeRow: boolean;
  showSame: boolean;
  byTitle: boolean;
  source: boolean;
  /** Fill blank cells: stop at rows where this column is blank (-1: no). */
  fillGuard?: number;
  /** Fill blank cells: stop at rows with every cell blank. */
  stopBlankRows?: boolean;
  /** Fill blank cells: stop at total rows. */
  stopTotalRows?: boolean;
}

/** The fill's stop fields, only when one is on and the fill copies (old specs stay as they were). */
export function fillStops(c: Pick<ToolChoices, "from" | "fillGuard" | "stopBlankRows" | "stopTotalRows">): { guard?: number; stop_blank_rows?: boolean; stop_total_rows?: boolean } {
  if (c.from === "value") return {};
  const out: { guard?: number; stop_blank_rows?: boolean; stop_total_rows?: boolean } = {};
  if ((c.fillGuard ?? -1) >= 0) out.guard = c.fillGuard;
  if (c.stopBlankRows) out.stop_blank_rows = true;
  if (c.stopTotalRows) out.stop_total_rows = true;
  return out;
}

export interface ToolsRequest {
  data?: DataOp;
  table?: TableOp;
  split?: SplitParts;
  combine?: Combine;
}

const whole = (n: number) => Number.isFinite(n) && Number.isInteger(n);

/** What to run for a Tools dialog's choices, or a problem to show. */
export function toolsRequest(kind: ToolKind, c: ToolChoices): ToolsRequest | string {
  const needCols = () => (c.cols.length ? "" : "Tick at least one column.");
  switch (kind) {
    case "fill_blanks": {
      if (needCols()) return needCols();
      if (c.from === "value" && !c.text) return "Type the text to put in blank cells.";
      const from = c.from === "value" ? { kind: "value" as const, text: c.text } : { kind: c.from };
      const stops = fillStops(c);
      if (stops.guard !== undefined && c.cols.includes(stops.guard))
        return "The column that stops the fill cannot also be filled: untick it, or choose another column to stop at.";
      return { data: { op: "fill_blanks", spec: { cols: c.cols, from, spaces: c.spaces, keep_first: c.header, ...stops } } };
    }
    case "count_repeats":
      if (needCols()) return needCols();
      return { data: { op: "count_repeats", spec: { cols: c.cols, match_case: c.matchCase, trim: c.trim, keep_first: c.header, title: "Count" } } };
    case "blank_repeats":
      if (needCols()) return needCols();
      return { data: { op: "blank_repeats", spec: { cols: c.cols, match_case: c.matchCase, trim: c.trim, keep_first: c.header } } };
    case "breaks":
      if (!whole(c.blankRows) || c.blankRows < 1 || c.blankRows > 100) return "Put between 1 and 100 blank rows.";
      if (!c.byCol && (!whole(c.every) || c.every < 1)) return "Choose how many rows go between breaks.";
      return { data: { op: "breaks", spec: { col: c.byCol ? c.col : null, every: c.byCol ? 0 : c.every, rows: c.blankRows, keep_first: c.header } } };
    case "reshape":
      if (!whole(c.perRow) || c.perRow < 1 || c.perRow > 16_384) return "Put between 1 and 16,384 cells on a row.";
      return { table: { op: "reshape", spec: { col: c.col, per_row: c.perRow, down: c.down, keep_first: c.header } } };
    case "stack":
      if (needCols()) return needCols();
      return { table: { op: "stack", spec: { cols: c.cols, skip_blanks: c.skipBlanks, keep_first: c.header } } };
    case "sample": {
      if (!Number.isFinite(c.size) || c.size < 0) return "Choose how many rows to take.";
      if (c.sizeKind === "percent" && c.size > 100) return "Choose a percent from 0 to 100.";
      if (c.sizeKind !== "percent" && !whole(c.size)) return "Choose a whole number of rows.";
      if (c.sizeKind === "every" && c.size < 1) return "Take every 1st row or more.";
      const size = c.sizeKind === "percent" ? { kind: "percent" as const, p: c.size } : { kind: c.sizeKind, n: c.size };
      const seed = whole(c.seed) && c.seed >= 0 ? c.seed : 0;
      const per = c.per !== undefined && c.per >= 0 ? c.per : null;
      if (per !== null && c.sizeKind === "every") return "A sample per group takes a number or a percent of rows.";
      return { table: { op: "sample", spec: { size, seed, keep_first: c.header, per, drawn_order: !!c.drawnOrder, row_numbers: !!c.rowNumbers } } };
    }
    case "compare":
      if (c.other < 0) return "Open the other file in a tab first.";
      if (!c.wholeRow && needCols()) return "Tick the key columns, or compare whole rows.";
      return {
        table: {
          op: "compare",
          other: c.other,
          spec: { keys: c.wholeRow ? [] : c.cols, match_case: c.matchCase, trim: c.trim, keep_first: c.header, show_same: c.showSame },
        },
      };
    case "split":
      if (!c.byCol && (!whole(c.every) || c.every < 1)) return "Choose how many rows go in each file.";
      return { split: { col: c.byCol ? c.col : null, every: c.byCol ? 0 : c.every, keep_first: c.header, guard: !!c.guard } };
    case "combine":
      return { combine: { header: c.header, by_title: c.header && c.byTitle, source: c.source } };
  }
}

// ---- value types (Show value types, like Calc's value highlighting) ------------

export type ValueType = "blank" | "number" | "date" | "text";

/** What a cell holds in a column the person typed (Text and Id are always text; a Number or Date column
 *  shows what does not fit as text). Auto columns use [`valueType`]. */
export function valueTypeFor(text: string, ty: ColumnType): ValueType {
  if (ty === "auto") return valueType(text);
  if (!text.trim()) return "blank";
  if (ty === "number") return readNumber(text) !== null ? "number" : "text";
  if (typeof ty === "object") return readDate(text, ty.date === "dmy") !== null ? "date" : "text";
  return "text";
}

/** What a cell holds, as the grid colours it. */
export function valueType(text: string): ValueType {
  const t = text.trim();
  if (!t) return "blank";
  if (readNumber(t) !== null) return "number";
  if (/\d/.test(t) && readDate(t) !== null) return "date";
  return "text";
}

// ---- colour scales and data bars -------------------------------------------------

export type ScaleKind = "scale2" | "scale3" | "bar";
export interface ColumnScale {
  col: number;
  kind: ScaleKind;
}

/** Where `v` sits between `lo` and `hi` (0 to 1). */
export function scalePos(v: number, lo: number, hi: number): number {
  if (!(hi > lo)) return 0.5;
  return Math.min(1, Math.max(0, (v - lo) / (hi - lo)));
}

type Rgb = [number, number, number];
const mix = (a: Rgb, b: Rgb, t: number): Rgb => [0, 1, 2].map((i) => Math.round(a[i]! + (b[i]! - a[i]!) * t)) as Rgb;
const LOW: Rgb = [248, 105, 107];
const MID: Rgb = [255, 235, 132];
const HIGH: Rgb = [99, 190, 123];
const WHITE: Rgb = [255, 255, 255];

/** The fill of a cell in a colour scale (2-colour: white to green; 3-colour:
 *  red, yellow, green), as an rgba() colour, nearly solid so the dark text on it reads in
 *  either theme. */
export function scaleColour(kind: "scale2" | "scale3", pos: number): string {
  const c = kind === "scale2" ? mix(WHITE, HIGH, pos) : pos < 0.5 ? mix(LOW, MID, pos * 2) : mix(MID, HIGH, (pos - 0.5) * 2);
  return `rgba(${c[0]}, ${c[1]}, ${c[2]}, 0.85)`;
}

/** A data bar's length as a part of the cell (negatives from the zero line). */
export function barSpan(v: number, lo: number, hi: number): { from: number; to: number } {
  const a = Math.min(lo, 0);
  const b = Math.max(hi, 0);
  if (!(b > a)) return { from: 0, to: 0 };
  const zero = (0 - a) / (b - a);
  const at = (v - a) / (b - a);
  return v >= 0 ? { from: zero, to: at } : { from: at, to: zero };
}

// ---- group dividers ---------------------------------------------------------------

/** Rows (indexes into `values`) after which a divider line is drawn: where
 *  the next value differs. */
export function dividerAfter(values: string[]): number[] {
  const out: number[] = [];
  for (let i = 0; i + 1 < values.length; i++) if (values[i] !== values[i + 1]) out.push(i);
  return out;
}

// ---- type-ahead ---------------------------------------------------------------------

/** The text shown while typing: what was typed, then the rest of the
 *  suggestion (selected, so typing on replaces it). Null when the
 *  suggestion does not continue what was typed. */
export function completion(typed: string, suggestion: string | null): { text: string; from: number } | null {
  if (!suggestion || !typed || suggestion.length <= typed.length) return null;
  if (!suggestion.toLowerCase().startsWith(typed.toLowerCase())) return null;
  return { text: typed + suggestion.slice(typed.length), from: typed.length };
}

// ---- custom sort orders ---------------------------------------------------------------

export interface SortList {
  name: string;
  items: string[];
}

/** Lists offered before the user adds any. */
export const BUILTIN_LISTS: SortList[] = [
  { name: "Low, Medium, High", items: ["Low", "Medium", "High"] },
  { name: "Weekdays (Mon to Sun)", items: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"] },
  { name: "Weekdays short (Mon to Sun)", items: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] },
  { name: "Months (Jan to Dec)", items: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"] },
  { name: "Months short (Jan to Dec)", items: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] },
];

/** A typed list: one item per line or comma (per line only with
 *  `commas` false, for values taken from cells), blanks and repeats
 *  dropped, at most 10,000 items of 1,000 characters. */
export function parseList(text: string, commas = true): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(commas ? /\r\n|\r|\n|,/ : /\r\n|\r|\n/)) {
    const v = raw.trim().slice(0, 1000);
    if (!v || seen.has(v.toLowerCase())) continue;
    seen.add(v.toLowerCase());
    out.push(v);
    if (out.length >= 10_000) break;
  }
  return out;
}

/** Most saved custom orders, values in one, characters in a value. */
export const MAX_ORDERS = 60;
export const MAX_ORDER_ITEMS = 10_000;

/** A custom order the user saved. `id` stays the same through rename and
 *  move, so a sort level keeps pointing at it. */
export interface SavedOrder extends SortList {
  id: string;
  /** Made from selected cells (not typed). */
  fromCells?: boolean;
  /** The fill handle follows this order from one starting cell. On to
   *  start with for typed orders of up to 100 values; off for orders made
   *  from cells (a column of names must never fill other people's names). */
  fill?: boolean;
}

/** Longest order the fill handle follows from one cell. */
export const FILL_ONE_CELL_MAX = 100;

/** Whether a new order follows the fill handle from one cell to start with. */
export function fillByDefault(items: readonly string[], fromCells: boolean): boolean {
  return !fromCells && items.length <= FILL_ONE_CELL_MAX;
}

/** The orders the fill handle may follow, in the Manage panel's order (the
 *  first holding a value wins), then the built-in Low, Medium, High. One
 *  starting cell follows only the ticked orders of up to 100 values; two or
 *  more stepping evenly along an order follow any of them. Months and
 *  weekdays are the fill's own. */
export function fillOrders(saved: readonly SavedOrder[]): FillOrder[] {
  const own = saved.map((o) => ({ name: o.name, items: o.items, one_cell: o.fill === true && o.items.length <= FILL_ONE_CELL_MAX }));
  return [...own, { name: BUILTIN_LISTS[0]!.name, items: BUILTIN_LISTS[0]!.items, one_cell: false }];
}

/** Values from selected cells for a new order (Data > New custom order from
 *  the selected cells). */
export interface FromCells {
  items: string[];
  /** Hidden cells in the selection, left out. */
  leftOut: number;
  /** The tab is private or from an encrypted file. */
  private: boolean;
}

/** A new id, unlike every one in `taken`. */
export function newOrderId(taken: readonly { id: string }[]): string {
  const used = new Set(taken.map((o) => o.id));
  for (let n = taken.length + 1; ; n++) {
    const id = `o${n.toString(36)}`;
    if (!used.has(id)) return id;
  }
}

/** Saved orders as read from the settings file: bad entries dropped, values
 *  cut to the limits, and an id for each one that has none (orders saved
 *  before ids, in their order). */
export function readOrders(raw: unknown): SavedOrder[] {
  if (!Array.isArray(raw)) return [];
  const out: SavedOrder[] = [];
  for (const l of raw.slice(0, MAX_ORDERS) as Record<string, unknown>[]) {
    if (!l || typeof l !== "object" || typeof l.name !== "string" || !Array.isArray(l.items)) continue;
    const items = (l.items as unknown[]).filter((i): i is string => typeof i === "string").slice(0, MAX_ORDER_ITEMS).map((i) => i.slice(0, 1000));
    const id = typeof l.id === "string" && l.id && !out.some((o) => o.id === l.id) ? l.id.slice(0, 40) : "";
    const fromCells = l.fromCells === true;
    out.push({ id, name: l.name.slice(0, 200), items, ...(fromCells ? { fromCells: true } : {}), fill: typeof l.fill === "boolean" ? l.fill : fillByDefault(items, fromCells) });
  }
  for (const o of out) if (!o.id) o.id = newOrderId(out.filter((x) => x.id));
  return out;
}

/** The values of selected cells for a new order, row by row, left to right:
 *  trimmed, blanks and repeats (capitals aside) dropped. `tooMany` past
 *  10,000 different values. */
export function orderFromCells(rows: readonly (readonly string[])[]): { items: string[]; tooMany: boolean } {
  const seen = new Set<string>();
  const items: string[] = [];
  for (const r of rows)
    for (const raw of r) {
      const v = raw.trim().slice(0, 1000);
      if (!v || seen.has(v.toLowerCase())) continue;
      if (items.length >= MAX_ORDER_ITEMS) return { items: [], tooMany: true };
      seen.add(v.toLowerCase());
      items.push(v);
    }
  return { items, tooMany: false };
}

// ---- file chores ------------------------------------------------------------------------

/** Tabs that can be closed without asking (nothing unsaved). */
export function unchangedTabs<T extends { info: { modified: boolean } }>(tabs: T[]): T[] {
  return tabs.filter((t) => !t.info.modified);
}
