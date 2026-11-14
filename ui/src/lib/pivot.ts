// Pivot tables: the spec sent to the engine (mirrors crates/engine/src/
// pivot.rs), friendly names for the screen, and a small pivot for the
// browser preview (the desktop app does the real work in Rust).
import type { ReadOptions } from "./backend";

export type Grouping =
  | { kind: "none" }
  | { kind: "year" }
  | { kind: "quarter" }
  | { kind: "month" }
  | { kind: "year_quarter" }
  | { kind: "year_month" }
  | { kind: "day" }
  | { kind: "weekday" }
  | { kind: "hour" }
  | { kind: "bins"; start: number; size: number; end: number | null };

export type SortBy = "label" | "value";
export interface FieldSort {
  by: SortBy;
  value: number;
  desc: boolean;
}
export interface ItemPick {
  exclude: boolean;
  items: string[];
}
export type LabelOp =
  | "equals"
  | "not_equals"
  | "begins_with"
  | "not_begins_with"
  | "ends_with"
  | "not_ends_with"
  | "contains"
  | "not_contains"
  | "greater"
  | "less"
  | "between"
  | "not_between";
export interface LabelFilter {
  op: LabelOp;
  text: string;
  text2: string;
}
export type CmpOp = "greater" | "greater_or_equal" | "less" | "less_or_equal" | "equals" | "not_equals" | "between" | "not_between";
export interface ValueFilter {
  value: number;
  op: CmpOp;
  a: number;
  b: number;
}
export interface TopN {
  n: number;
  value: number;
  bottom: boolean;
}
export interface PivotField {
  col: number;
  group: Grouping;
  sort: FieldSort;
  items: ItemPick | null;
  label_filter: LabelFilter | null;
  value_filter: ValueFilter | null;
  top: TopN | null;
  subtotal: boolean;
}
export type Agg =
  | "count"
  | "count_rows"
  | "count_numbers"
  | "count_distinct"
  | "count_blank"
  | "sum"
  | "average"
  | "min"
  | "max"
  | "median"
  | "std_dev"
  | "std_dev_p"
  | "var"
  | "var_p"
  | "product"
  | "first"
  | "last"
  | "join";
export type ShowAs =
  | "value"
  | "pct_of_grand"
  | "pct_of_column"
  | "pct_of_row"
  | "pct_of_parent_row"
  | "pct_of_parent_column"
  | "running_total"
  | "pct_running_total"
  | "difference_from_previous"
  | "pct_difference_from_previous"
  | "rank_largest"
  | "rank_smallest"
  | "index";
export type Along = "rows" | "columns";
export interface NumFormat {
  decimals: number | null;
  thousands: boolean;
  prefix: string;
  suffix: string;
}
export interface ValueField {
  col: number;
  agg: Agg;
  show_as: ShowAs;
  along: Along;
  name: string;
  format: NumFormat;
  separator: string;
}
export interface PivotLayout {
  grand_row: boolean;
  grand_col: boolean;
  row_subtotals: boolean;
  col_subtotals: boolean;
  subtotals_top: boolean;
  repeat_labels: boolean;
  blank_label: string;
  empty_cells: string;
  row_depth: number;
  col_depth: number;
  collapsed_rows: string[][];
  expanded_rows: string[][];
  collapsed_cols: string[][];
  expanded_cols: string[][];
}
export interface PivotSpec {
  rows: PivotField[];
  cols: PivotField[];
  values: ValueField[];
  filters: PivotField[];
  keep_first: boolean;
  read: ReadOptions;
  match_case: boolean;
  keep_spaces: boolean;
  layout: PivotLayout;
}

export type Kind = "item" | "group" | "subtotal" | "grand";
export interface HeadCell {
  text: string;
  span: number;
  toggle: boolean | null;
  path: string[];
}
export interface GridRow {
  kind: Kind;
  depth: number;
  labels: string[];
  path: string[];
  toggle: boolean | null;
  cells: string[];
}
export interface GridCol {
  kind: Kind;
  path: string[];
  value: number;
  text: boolean;
}
export interface PivotGrid {
  label_cols: number;
  head: HeadCell[][];
  cols: GridCol[];
  rows: GridRow[];
  total_rows: number;
  total_cols: number;
  truncated: boolean;
  rows_read: number;
  rows_used: number;
  sampled: boolean;
}
export interface ItemInfo {
  key: string;
  label: string;
  rows: number;
}
export interface ItemList {
  items: ItemInfo[];
  total: number;
}

export type Area = "rows" | "cols" | "values" | "filters";

/** Rows read first, for a quick look at a big file. */
export const QUICK_ROWS = 200_000;
/** Files with more rows get the quick look first. */
export const QUICK_FROM = 400_000;

export function newField(col: number): PivotField {
  return {
    col,
    group: { kind: "none" },
    sort: { by: "label", value: 0, desc: false },
    items: null,
    label_filter: null,
    value_filter: null,
    top: null,
    subtotal: true,
  };
}

export function newValue(col: number, agg: Agg): ValueField {
  return {
    col,
    agg,
    show_as: "value",
    along: "rows",
    name: "",
    format: { decimals: null, thousands: false, prefix: "", suffix: "" },
    separator: ", ",
  };
}

export function newLayout(): PivotLayout {
  return {
    grand_row: true,
    grand_col: true,
    row_subtotals: true,
    col_subtotals: true,
    subtotals_top: true,
    repeat_labels: false,
    blank_label: "(blank)",
    empty_cells: "",
    row_depth: 0,
    col_depth: 0,
    collapsed_rows: [],
    expanded_rows: [],
    collapsed_cols: [],
    expanded_cols: [],
  };
}

export function newSpec(header: boolean): PivotSpec {
  return {
    rows: [],
    cols: [],
    values: [],
    filters: [],
    keep_first: header,
    read: { decimal_comma: false, date_order: "mdy" },
    match_case: false,
    keep_spaces: false,
    layout: newLayout(),
  };
}

export const AGGS: [Agg, string][] = [
  ["count", "Count (cells not blank)"],
  ["count_rows", "Count of rows (blanks too)"],
  ["count_distinct", "Count of different values"],
  ["count_numbers", "Count of numbers"],
  ["count_blank", "Count of blanks"],
  ["sum", "Sum"],
  ["average", "Average"],
  ["min", "Smallest"],
  ["max", "Largest"],
  ["median", "Median"],
  ["std_dev", "Standard deviation (sample)"],
  ["std_dev_p", "Standard deviation (population)"],
  ["var", "Variance (sample)"],
  ["var_p", "Variance (population)"],
  ["product", "Product"],
  ["first", "First value"],
  ["last", "Last value"],
  ["join", "List of values"],
];

export const SHOW_AS: [ShowAs, string][] = [
  ["value", "The value"],
  ["pct_of_grand", "% of grand total"],
  ["pct_of_column", "% of column total"],
  ["pct_of_row", "% of row total"],
  ["pct_of_parent_row", "% of parent row"],
  ["pct_of_parent_column", "% of parent column"],
  ["running_total", "Running total"],
  ["pct_running_total", "% running total"],
  ["difference_from_previous", "Difference from previous"],
  ["pct_difference_from_previous", "% difference from previous"],
  ["rank_largest", "Rank (largest = 1)"],
  ["rank_smallest", "Rank (smallest = 1)"],
  ["index", "Index"],
];

/** Show-as choices that go along rows or columns. */
export function usesAlong(s: ShowAs): boolean {
  return ["running_total", "pct_running_total", "difference_from_previous", "pct_difference_from_previous", "rank_largest", "rank_smallest"].includes(s);
}

export const GROUPS: [Grouping["kind"], string][] = [
  ["none", "Each value"],
  ["year", "Years"],
  ["quarter", "Quarters"],
  ["month", "Months"],
  ["year_quarter", "Year and quarter"],
  ["year_month", "Year and month"],
  ["day", "Days"],
  ["weekday", "Days of the week"],
  ["hour", "Hours"],
  ["bins", "Number ranges"],
];

export const LABEL_OPS: [LabelOp, string][] = [
  ["equals", "is"],
  ["not_equals", "is not"],
  ["begins_with", "begins with"],
  ["not_begins_with", "does not begin with"],
  ["ends_with", "ends with"],
  ["not_ends_with", "does not end with"],
  ["contains", "contains"],
  ["not_contains", "does not contain"],
  ["greater", "comes after"],
  ["less", "comes before"],
  ["between", "is between"],
  ["not_between", "is not between"],
];

export const CMP_OPS: [CmpOp, string][] = [
  ["greater", "is more than"],
  ["greater_or_equal", "is at least"],
  ["less", "is less than"],
  ["less_or_equal", "is at most"],
  ["equals", "equals"],
  ["not_equals", "does not equal"],
  ["between", "is between"],
  ["not_between", "is not between"],
];

const AGG_TITLE: Record<Agg, string> = {
  count: "Count of",
  count_rows: "Rows of",
  count_numbers: "Count numbers of",
  count_distinct: "Distinct count of",
  count_blank: "Blank count of",
  sum: "Sum of",
  average: "Average of",
  min: "Min of",
  max: "Max of",
  median: "Median of",
  std_dev: "StdDev of",
  std_dev_p: "StdDevP of",
  var: "Var of",
  var_p: "VarP of",
  product: "Product of",
  first: "First of",
  last: "Last of",
  join: "List of",
};

/** A value field's heading, as the engine writes it. */
export function valueName(v: ValueField, titles: string[]): string {
  return v.name || `${AGG_TITLE[v.agg]} ${titles[v.col] ?? `Column ${v.col + 1}`}`;
}

/** A row/column/filter field's chip text. */
export function fieldName(f: PivotField, titles: string[]): string {
  const t = titles[f.col] ?? `Column ${f.col + 1}`;
  const g = GROUPS.find(([k]) => k === f.group.kind)?.[1];
  const marks = [f.items ? "picked" : "", f.label_filter || f.value_filter ? "filtered" : "", f.top ? `top ${f.top.n}` : ""].filter(Boolean);
  return `${t}${f.group.kind === "none" ? "" : ` (${g})`}${marks.length ? ` · ${marks.join(", ")}` : ""}`;
}

/** Most values that read as numbers: Sum fits better than Count. */
export function looksNumeric(values: string[]): boolean {
  const filled = values.map((v) => v.trim()).filter((v) => v !== "");
  if (filled.length < 1) return false;
  const nums = filled.filter((v) => /^[-+(]?[$€£¥]?\s*\d[\d,]*(\.\d+)?\)?%?$/.test(v) || /^[-+]?\d*\.\d+$/.test(v));
  return nums.length >= Math.max(1, filled.length * 0.8);
}

/** The part of the spec that needs the file read again (as the engine's
 *  scan key: fields, groups, picks, value columns and summaries). */
export function scanKey(s: PivotSpec): string {
  const dim = (f: PivotField) => [f.col, f.group, f.items];
  return JSON.stringify([
    s.rows.map(dim),
    s.cols.map(dim),
    s.filters.map(dim),
    s.values.map((v) => [v.col, v.agg, v.separator]),
    s.keep_first,
    s.read,
    s.match_case,
    s.keep_spaces,
  ]);
}

/** Expand / collapse one row or column group. */
export function toggled(l: PivotLayout, rows: boolean, path: string[], collapsed: boolean): PivotLayout {
  const same = (a: string[]) => a.length === path.length && a.every((x, i) => x === path[i]);
  const col = rows ? l.collapsed_rows : l.collapsed_cols;
  const exp = rows ? l.expanded_rows : l.expanded_cols;
  const depth = rows ? l.row_depth : l.col_depth;
  let c = col.filter((p) => !same(p));
  let e = exp.filter((p) => !same(p));
  if (collapsed) {
    // Open it.
    if (depth > 0 && path.length >= depth) e = [...e, path];
  } else {
    c = [...c, path];
  }
  return rows ? { ...l, collapsed_rows: c, expanded_rows: e } : { ...l, collapsed_cols: c, expanded_cols: e };
}

/** Every group open (or every group shut to its first level). */
export function allOpen(l: PivotLayout, rows: boolean, open: boolean): PivotLayout {
  return rows
    ? { ...l, row_depth: open ? 0 : 1, collapsed_rows: [], expanded_rows: [] }
    : { ...l, col_depth: open ? 0 : 1, collapsed_cols: [], expanded_cols: [] };
}

// ---- a small pivot for the browser preview ----

function num(v: string): number | null {
  const t = v.trim().replace(/[$,]/g, "");
  if (t === "" || !/^[-+]?\d*\.?\d+(e[-+]?\d+)?$/i.test(t)) return null;
  return Number(t);
}

/** Pivot `data` (rows of cells, titles left out) for the browser preview:
 *  item fields (no grouping), totals at the top, the common summaries. */
export function mockPivot(data: string[][], titles: string[], s: PivotSpec, sampleRows: number | null): PivotGrid {
  const rowsRead = sampleRows === null ? data.length : Math.min(sampleRows, data.length);
  const key = (v: string) => {
    const t = s.keep_spaces ? v : v.trim();
    return s.match_case ? t : t.toLowerCase();
  };
  const keeps = (f: PivotField, v: string) => !f.items || f.items.items.includes(key(v)) !== f.items.exclude;
  const used = data.slice(0, rowsRead).filter((r) => [...s.filters, ...s.rows, ...s.cols].every((f) => keeps(f, r[f.col] ?? "")));
  const label = new Map<string, string>();
  const path = (r: string[], fs: PivotField[]) =>
    fs.map((f) => {
      const k = key(r[f.col] ?? "");
      if (!label.has(`${f.col}|${k}`)) label.set(`${f.col}|${k}`, k === "" ? s.layout.blank_label : (r[f.col] ?? "").trim());
      return k;
    });
  const summary = (rs: string[][], v: ValueField): string => {
    const vals = rs.map((r) => (r[v.col] ?? "").trim());
    const nums = vals.map(num).filter((x): x is number => x !== null);
    const sum = nums.reduce((a, b) => a + b, 0);
    switch (v.agg) {
      case "count_rows":
        return String(vals.length);
      case "count_blank":
        return String(vals.filter((x) => x === "").length);
      case "count_numbers":
        return String(nums.length);
      case "count_distinct":
        return String(new Set(vals.filter((x) => x !== "")).size);
      case "sum":
        return nums.length ? String(sum) : "";
      case "average":
        return nums.length ? String(sum / nums.length) : "";
      case "min":
        return nums.length ? String(Math.min(...nums)) : "";
      case "max":
        return nums.length ? String(Math.max(...nums)) : "";
      case "first":
        return vals.find((x) => x !== "") ?? "";
      case "last":
        return [...vals].reverse().find((x) => x !== "") ?? "";
      case "join":
        return vals.filter((x) => x !== "").join(v.separator);
      default:
        return String(vals.filter((x) => x !== "").length);
    }
  };
  const prefixes = (fs: PivotField[]) => {
    const seen = new Map<string, string[]>();
    for (const r of used) {
      const p = path(r, fs);
      for (let d = 1; d <= p.length; d++) seen.set(JSON.stringify(p.slice(0, d)), p.slice(0, d));
    }
    const sorted = [...seen.values()].sort((a, b) => {
      for (let i = 0; i < Math.min(a.length, b.length); i++) {
        const x = a[i] ?? "";
        const y = b[i] ?? "";
        if (x !== y) return x === "" ? 1 : y === "" ? -1 : x.localeCompare(y, undefined, { numeric: true });
      }
      return a.length - b.length;
    });
    return sorted;
  };
  const under = (r: string[], fs: PivotField[], p: string[]) => {
    const q = path(r, fs);
    return p.every((k, i) => q[i] === k);
  };
  const lab = (fs: PivotField[], p: string[]) => label.get(`${fs[p.length - 1]?.col}|${p[p.length - 1]}`) ?? "";
  const colPaths = s.cols.length ? prefixes(s.cols).filter((p) => p.length === s.cols.length) : [];
  const cols: { path: string[]; kind: "item" | "grand" }[] = [...colPaths.map((p) => ({ path: p, kind: "item" as const }))];
  if (!s.cols.length || s.layout.grand_col) cols.push({ path: [], kind: "grand" });
  const gridCols = s.values.length ? cols.flatMap((c) => s.values.map((_, v) => ({ kind: c.kind, path: c.path, value: v, text: ["first", "last", "join"].includes(s.values[v]!.agg) }))) : [];
  const labelCols = Math.max(1, s.rows.length);
  const head: HeadCell[][] = [];
  const corner = (text: string[]) => Array.from({ length: labelCols }, (_, i) => ({ text: text[i] ?? "", span: 1, toggle: null, path: [] }));
  if (s.cols.length) {
    head.push([...corner([s.values.length === 1 ? valueName(s.values[0]!, titles) : ""]), ...(gridCols.length ? [{ text: s.cols.map((f) => titles[f.col]).join(" / "), span: gridCols.length, toggle: null, path: [] }] : [])]);
    for (let lvl = 0; lvl < s.cols.length; lvl++)
      head.push([
        ...corner(lvl === s.cols.length - 1 && s.values.length <= 1 ? s.rows.map((f) => titles[f.col] ?? "") : []),
        ...cols.map((c) => ({ text: c.kind === "grand" ? (lvl === 0 ? "Grand Total" : "") : lab(s.cols, c.path.slice(0, lvl + 1)), span: Math.max(1, s.values.length), toggle: null, path: c.path.slice(0, lvl + 1) })),
      ]);
  }
  if (!s.cols.length || s.values.length > 1) head.push([...corner(s.rows.map((f) => titles[f.col] ?? "")), ...gridCols.map((c) => ({ text: valueName(s.values[c.value]!, titles), span: 1, toggle: null, path: [] }))]);
  const cellsOf = (rs: string[][]) => gridCols.map((c) => {
    const inCol = c.kind === "grand" ? rs : rs.filter((r) => under(r, s.cols, c.path));
    return inCol.length ? summary(inCol, s.values[c.value]!) : s.layout.empty_cells;
  });
  const rows: GridRow[] = [];
  if (!s.rows.length) rows.push({ kind: "grand", depth: 0, labels: ["Total"], path: [], toggle: null, cells: cellsOf(used) });
  else {
    const isCollapsed = (p: string[]) =>
      s.layout.collapsed_rows.some((c) => JSON.stringify(c) === JSON.stringify(p)) ||
      (s.layout.row_depth > 0 && p.length >= s.layout.row_depth && !s.layout.expanded_rows.some((c) => JSON.stringify(c) === JSON.stringify(p)));
    for (const p of prefixes(s.rows)) {
      if (p.slice(0, -1).some((_, i) => isCollapsed(p.slice(0, i + 1)))) continue;
      const deepest = p.length === s.rows.length;
      const labels = Array.from({ length: labelCols }, (_, i) => (i === p.length - 1 ? lab(s.rows, p) : ""));
      const shut = !deepest && isCollapsed(p);
      const show = deepest || shut || s.layout.row_subtotals;
      rows.push({ kind: deepest ? "item" : "group", depth: p.length, labels, path: p, toggle: deepest ? null : shut, cells: show ? cellsOf(used.filter((r) => under(r, s.rows, p))) : gridCols.map(() => "") });
    }
    if (s.layout.grand_row) rows.push({ kind: "grand", depth: 0, labels: ["Grand Total", ...Array(labelCols - 1).fill("")], path: [], toggle: null, cells: cellsOf(used) });
  }
  return {
    label_cols: labelCols,
    head,
    cols: gridCols,
    rows: rows.slice(0, 2000),
    total_rows: rows.length,
    total_cols: gridCols.length,
    truncated: rows.length > 2000,
    rows_read: rowsRead,
    rows_used: used.length,
    sampled: rowsRead < data.length,
  };
}

/** The items of one field for the browser preview. */
export function mockItems(data: string[][], col: number, s: PivotSpec): ItemList {
  const m = new Map<string, ItemInfo>();
  for (const r of data) {
    const raw = r[col] ?? "";
    const t = s.keep_spaces ? raw : raw.trim();
    const k = s.match_case ? t : t.toLowerCase();
    const it = m.get(k) ?? { key: k, label: k === "" ? s.layout.blank_label : t, rows: 0 };
    it.rows++;
    m.set(k, it);
  }
  const items = [...m.values()].sort((a, b) => (a.key === "" ? 1 : b.key === "" ? -1 : a.label.localeCompare(b.label, undefined, { numeric: true })));
  return { items: items.slice(0, 10_000), total: items.length };
}

/** The source rows behind a cell, for the browser preview. */
export function mockDrill(data: string[][], s: PivotSpec, rowPath: string[], colPath: string[]): string[][] {
  const key = (v: string) => {
    const t = s.keep_spaces ? v : v.trim();
    return s.match_case ? t : t.toLowerCase();
  };
  const keeps = (f: PivotField, v: string) => !f.items || f.items.items.includes(key(v)) !== f.items.exclude;
  return data.filter(
    (r) =>
      [...s.filters, ...s.rows, ...s.cols].every((f) => keeps(f, r[f.col] ?? "")) &&
      rowPath.every((k, i) => key(r[s.rows[i]!.col] ?? "") === k) &&
      colPath.every((k, i) => key(r[s.cols[i]!.col] ?? "") === k),
  );
}

// ---- value field order (sorts, top N and value filters name values by
// their place in the Values list) ----

/** Point every row/column field's value references through `map` (old
 *  place → new place, or null when that value field is gone). */
export function remapValueRefs(s: PivotSpec, map: (i: number) => number | null): void {
  for (const f of [...s.rows, ...s.cols]) {
    if (f.sort.by === "value") {
      const n = map(f.sort.value);
      f.sort = n === null ? { by: "label", value: 0, desc: false } : { ...f.sort, value: n };
    }
    if (f.value_filter) {
      const n = map(f.value_filter.value);
      f.value_filter = n === null ? null : { ...f.value_filter, value: n };
    }
    if (f.top) {
      const n = map(f.top.value);
      f.top = n === null ? null : { ...f.top, value: n };
    }
  }
}

/** Put a value field at `at`; later ones move down one place. */
export function insertValue(s: PivotSpec, at: number, v: ValueField): void {
  s.values.splice(at, 0, v);
  remapValueRefs(s, (i) => (i >= at ? i + 1 : i));
}

/** Take out the value field at `i`; what used it goes back to plain. */
export function removeValue(s: PivotSpec, i: number): void {
  s.values.splice(i, 1);
  remapValueRefs(s, (j) => (j === i ? null : j > i ? j - 1 : j));
}

/** Move the value field at `from` so it ends up at `to`. */
export function moveValue(s: PivotSpec, from: number, to: number): void {
  if (from === to || from < 0 || from >= s.values.length) return;
  to = Math.max(0, Math.min(to, s.values.length - 1));
  const [v] = s.values.splice(from, 1);
  s.values.splice(to, 0, v!);
  remapValueRefs(s, (j) => {
    if (j === from) return to;
    if (from < to && j > from && j <= to) return j - 1;
    if (to < from && j >= to && j < from) return j + 1;
    return j;
  });
}
