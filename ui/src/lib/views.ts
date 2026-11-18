// M7 shapes shared with the shell: stats, charts, highlight rules, saved
// views, recent files and the folder panel; plus checks the UI does before
// sending (the shell checks again).

import type { Check } from "./backend";
import type { DisplayFormat } from "./m9";
import type { FilterSpec, ReadOptions, Rule, SortSpec } from "./backend";

export type ColType = "empty" | "number" | "date" | "yes_no" | "text" | "mixed";
export interface TypeMix {
  blank: number;
  number: number;
  date: number;
  yes_no: number;
  text: number;
}
export interface Summary {
  col: number;
  rows: number;
  mix: TypeMix;
  kind: ColType;
  distinct: number;
  distinct_capped: boolean;
  sum: number | null;
  mean: number | null;
  min: number | null;
  max: number | null;
  min_len: number;
  max_len: number;
  /** `cut`: the value was shortened to 200 characters and "…". */
  top: { value: string; cut: boolean; count: number }[];
  /** Too many distinct values: the list may miss rarely seen values. */
  top_partial: boolean;
}
export type ChartKind = "line" | "bar" | "pie" | "count";
export interface ChartSpec {
  kind: ChartKind;
  label: number | null;
  values: number[];
  skip_first: boolean;
  read: ReadOptions;
}
export interface ChartData {
  labels: string[];
  series: (number | null)[][];
  per_point: number;
  rows: number;
  other: boolean;
}
export type Total = "sum" | "count" | "average" | "min" | "max";
export interface SubtotalSpec {
  by: number;
  cols: number[];
  total: Total;
  keep_first: boolean;
  read: ReadOptions;
}
export interface Highlight {
  col: number;
  rule: Rule;
  color: number;
  whole_row: boolean;
}
export interface Mark {
  row: number;
  col: number | null;
  color: number;
}
export interface ViewColumn {
  title: string;
  width: number | null;
  hidden: boolean;
  /** Text wraps (older views have none). */
  wrap?: boolean;
  /** Alignment (automatic when missing). */
  align?: "left" | "center" | "right";
  /** Long text is cut at the edge instead of spilling over. */
  clip?: boolean;
  /** Display format (view only). */
  format?: DisplayFormat | null;
  /** Protected from changes (older views have none). */
  protect?: boolean;
}
export interface View {
  version: number;
  name: string;
  header: boolean;
  columns: ViewColumn[];
  freeze_rows: number;
  freeze_cols: number;
  sort: SortSpec | null;
  filter: FilterSpec | null;
  highlights: Highlight[];
  /** Data checks (older views have none). */
  checks?: Check[];
  /** Alternating row colours. */
  banded?: boolean;
  /** Lines of text per row (1 to 10). */
  row_lines?: number;
}
/** A view as "Pick up where you left off" keeps it on disk: only how the
 *  table is laid out. Filters, highlight rules and data checks can hold
 *  values from the file (a picked name, "contains Smith"), so they are left
 *  out; sorting changed the rows, so it is not replayed either. */
export function sessionView(v: View): View {
  return { ...v, sort: null, filter: null, highlights: [], checks: [] };
}

export interface RecentItem {
  /** Stable while the app runs; opaque (the full path stays in the backend). */
  id: string;
  name: string;
  /** The folder's own name, not its path. */
  folder: string;
  /** The open tab showing this file, matched by full path in the backend. */
  open_doc: number | null;
}
export interface FolderItem {
  index: number;
  name: string;
  size: number;
  table: boolean;
}
export interface Listing {
  folder: string;
  items: FolderItem[];
  more: boolean;
}

/** Highlight colours (palette numbers 0-7), drawn see-through over cells. */
export const PALETTE: { name: string; color: string }[] = [
  { name: "Red", color: "#e5484d" },
  { name: "Orange", color: "#f76b15" },
  { name: "Yellow", color: "#ffc53d" },
  { name: "Green", color: "#30a46c" },
  { name: "Teal", color: "#12a594" },
  { name: "Blue", color: "#3e63dd" },
  { name: "Purple", color: "#8e4ec6" },
  { name: "Grey", color: "#8b8d98" },
];

export const TYPE_NAMES: Record<ColType, string> = {
  empty: "empty",
  number: "numbers",
  date: "dates",
  yes_no: "yes/no",
  text: "text",
  mixed: "mixed",
};

export const MAX_HIGHLIGHTS = 200;
export const MAX_VIEWS = 50;

/** A font family name safe to put in a style: letters, digits, spaces and
 *  hyphens, at most 60 characters. `null` if not. */
export function safeFont(name: string): string | null {
  const t = name.trim();
  if (!t || t.length > 60 || !/^[\p{L}\p{N} -]+$/u.test(t)) return null;
  return t;
}

/** Match a view's columns to a file's titles: by title (case and spaces
 *  ignored), then by position for titles not found. Returns, for each view
 *  column, the file's column (or null). */
export function matchColumns(view: ViewColumn[], titles: string[]): (number | null)[] {
  const norm = (s: string) => s.trim().toLowerCase();
  const used = new Set<number>();
  const out: (number | null)[] = view.map((v) => {
    const i = titles.findIndex((t, k) => !used.has(k) && norm(t) === norm(v.title));
    if (i >= 0) used.add(i);
    return i >= 0 ? i : null;
  });
  return out.map((m, k) => {
    if (m !== null) return m;
    if (k < titles.length && !used.has(k)) {
      used.add(k);
      return k;
    }
    return null;
  });
}

/** Custom shortcuts: command id -> keys. Bounded and cleaned when loaded. */
export type Shortcuts = Record<string, string[]>;
export const MAX_BINDINGS = 500;
const KEY_RE = /^((Ctrl|Alt|Shift)\+){0,3}[A-Za-z0-9`\-=[\];',./\\]$|^((Ctrl|Alt|Shift)\+){0,3}(F([1-9]|1[0-2])|Arrow(Up|Down|Left|Right)|Home|End|PageUp|PageDown|Insert|Delete|Backspace|Enter|Escape|Tab|Space)$/;

/** Why a key cannot be a custom shortcut, or null when it can. Typing
 * keys (letters, digits, Space) and the keys that move or edit in the grid
 * (Enter, Tab, arrows, Delete, ...) need Ctrl or Alt, so a custom key never
 * takes over typing or moving; F1-F12 may stand alone. Escape is kept for
 * closing and cancelling. */
export function keyProblem(k: string): string | null {
  if (k.length > 40 || !KEY_RE.test(k)) return `${k} cannot be used as a shortcut.`;
  const parts = k.split("+");
  const key = parts[parts.length - 1] ?? "";
  if (key === "Escape") return "Escape is kept for closing and cancelling.";
  if (/^F\d+$/.test(key)) return null;
  if (!parts.includes("Ctrl") && !parts.includes("Alt")) return `${k} is used for typing or moving; add Ctrl or Alt.`;
  return null;
}

export function validKey(k: string): boolean {
  return keyProblem(k) === null;
}

export function cleanShortcuts(raw: unknown, known: (id: string) => boolean): Shortcuts {
  const out: Shortcuts = {};
  if (!raw || typeof raw !== "object") return out;
  let n = 0;
  for (const [id, keys] of Object.entries(raw as Record<string, unknown>)) {
    if (!known(id) || !Array.isArray(keys)) continue;
    const ks = keys.filter((k): k is string => typeof k === "string" && validKey(k)).slice(0, 4);
    if (n + ks.length > MAX_BINDINGS) break;
    n += ks.length;
    out[id] = ks;
  }
  return out;
}

/** Words for a number in a summary. */
export function fmt(n: number | null): string {
  if (n === null || !Number.isFinite(n)) return "—";
  if (Math.abs(n) >= 1e15 || (n !== 0 && Math.abs(n) < 1e-6)) return n.toExponential(6);
  // 12 significant digits hide the last-bit noise of adding many decimals.
  return Number(n.toPrecision(12)).toLocaleString(undefined, { maximumFractionDigits: 10 });
}
