// Milestone 9 helpers (Google Sheets gaps): quick totals, clickable links,
// display formats, tick boxes, wrapped text and pick lists. Pure functions,
// so they are tested without a grid.

import type { Totals } from "./backend";
import type { TotalKind } from "./prefs.svelte";
import type { Step } from "./clean";

// ---- quick totals ---------------------------------------------------------------

export function totalLabel(k: TotalKind): string {
  return { sum: "Sum", average: "Average", count: "Count", numbers: "Count numbers", min: "Min", max: "Max", distinct: "Different values", chars: "Characters" }[k];
}

/** A total as shown: a sum or average past the largest number says so. */
export function totalText(t: Totals, k: TotalKind, fmt: (n: number | null) => string): string {
  if (t.too_large && (k === "sum" || k === "average")) return "too large";
  if (k === "distinct" && t.distinct !== null && !t.distinct_exact) return `at least ${fmt(t.distinct)}`;
  if (k === "distinct" && t.distinct === null) return "…";
  return fmt(totalValue(t, k));
}

export function totalValue(t: Totals, k: TotalKind): number | null {
  switch (k) {
    case "sum":
      return t.sum;
    case "average":
      return t.average;
    case "count":
      return t.filled;
    case "numbers":
      return t.numbers;
    case "min":
      return t.min;
    case "max":
      return t.max;
    case "distinct":
      return t.distinct;
    case "chars":
      return t.chars;
  }
}

// ---- links ----------------------------------------------------------------------

/** Longest address that is opened. */
export const MAX_LINK = 2048;
const EMAIL = /^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+$/;

/** The web or e-mail address a cell holds, ready to open, or null. Only
 *  http, https and mailto are ever returned (never files or other schemes);
 *  "www.x" becomes https and a bare e-mail address becomes mailto. The
 *  shell checks the address again before it opens it. */
export function linkOf(text: string): string | null {
  const t = text.trim();
  if (!t || t.length > MAX_LINK || /[\s\u0000-\u001f\u007f<>"\\^`{|}]/.test(t)) return null;
  let candidate: string;
  if (/^https?:\/\//i.test(t)) candidate = t;
  else if (/^mailto:/i.test(t)) candidate = t;
  else if (/^www\.[^./]+\.[^/]/i.test(t)) candidate = `https://${t}`;
  else if (EMAIL.test(t)) candidate = `mailto:${t}`;
  else return null;
  let u: URL;
  try {
    u = new URL(candidate);
  } catch {
    return null;
  }
  if (u.protocol === "mailto:") {
    const addr = decodeURIComponent(u.pathname.split("?")[0] ?? "");
    return EMAIL.test(addr) ? u.href : null;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  if (!u.hostname || u.username || u.password) return null;
  return u.href;
}

// ---- display formats (view only: never written to the file) ---------------------

export type FormatKind = "number" | "percent" | "currency" | "date" | "checkbox";
export type DateStyle = "iso" | "mdy" | "dmy" | "long" | "medium";
export interface DisplayFormat {
  kind: FormatKind;
  /** 0 to 10 (number, percent, currency). */
  decimals: number;
  thousands: boolean;
  /** Currency sign, at most 4 characters. */
  symbol: string;
  date: DateStyle;
}

export const DATE_STYLES: { id: DateStyle; label: string }[] = [
  { id: "iso", label: "2026-03-14" },
  { id: "mdy", label: "03/14/2026" },
  { id: "dmy", label: "14/03/2026" },
  { id: "medium", label: "Mar 14, 2026" },
  { id: "long", label: "Saturday, March 14, 2026" },
];

export function defaultFormat(kind: FormatKind): DisplayFormat {
  return { kind, decimals: kind === "percent" ? 0 : 2, thousands: true, symbol: "$", date: "iso" };
}

/** A display format from a view or the UI (untrusted), or null if unusable. */
export function cleanFormat(raw: unknown): DisplayFormat | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Partial<DisplayFormat>;
  const kinds: FormatKind[] = ["number", "percent", "currency", "date", "checkbox"];
  if (!kinds.includes(r.kind as FormatKind)) return null;
  const d = defaultFormat(r.kind!);
  return {
    kind: r.kind!,
    decimals: typeof r.decimals === "number" && Number.isFinite(r.decimals) ? Math.min(10, Math.max(0, Math.round(r.decimals))) : d.decimals,
    thousands: typeof r.thousands === "boolean" ? r.thousands : d.thousands,
    symbol: typeof r.symbol === "string" ? [...r.symbol].slice(0, 4).join("").replace(/[\u0000-\u001f]/g, "") : d.symbol,
    date: DATE_STYLES.some((s) => s.id === r.date) ? r.date! : d.date,
  };
}

/** A number written in a cell: plain, with thousands commas, a currency
 *  sign, a percent sign (read as a fraction), or in brackets for minus. */
export function readNumber(text: string): number | null {
  let t = text.trim();
  if (!t || t.length > 64) return null;
  let neg = false;
  if (/^\(.*\)$/.test(t)) {
    neg = true;
    t = t.slice(1, -1).trim();
  }
  let pct = false;
  if (t.endsWith("%")) {
    pct = true;
    t = t.slice(0, -1).trim();
  }
  if (/^-/.test(t)) {
    neg = !neg;
    t = t.slice(1);
  } else if (/^\+/.test(t)) t = t.slice(1);
  t = t.replace(/^[$€£¥]\s?/, "");
  if (/^\d{1,3}(,\d{3})+(\.\d*)?$/.test(t)) t = t.replace(/,/g, "");
  if (!/^(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(t)) return null;
  const n = Number(t) * (pct ? 0.01 : 1);
  if (!Number.isFinite(n)) return null;
  return neg ? -n : n;
}

/** A calendar date in a cell (yyyy-mm-dd, yyyy/mm/dd, or m/d/yyyy), as a
 *  UTC date, or null. */
export function readDate(text: string, dayFirst = false): Date | null {
  const t = text.trim();
  let y: number, m: number, d: number;
  let x = /^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})(?:[T ].*)?$/.exec(t);
  if (x) {
    [y, m, d] = [Number(x[1]), Number(x[2]), Number(x[3])];
  } else if ((x = /^(\d{1,2})[/.-](\d{1,2})[/.-](\d{4})$/.exec(t))) {
    const a = Number(x[1]);
    const b = Number(x[2]);
    [m, d] = dayFirst ? [b, a] : [a, b];
    y = Number(x[3]);
  } else return null;
  if (m < 1 || m > 12 || d < 1 || d > 31) return null;
  // Date.UTC reads years 0-99 as 1900-1999: set the year on its own.
  const dt = new Date(0);
  dt.setUTCFullYear(y, m - 1, d);
  dt.setUTCHours(0, 0, 0, 0);
  return dt.getUTCMonth() === m - 1 ? dt : null;
}

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function groupDigits(n: number, decimals: number, thousands: boolean): string {
  const fixed = Math.abs(n).toFixed(decimals);
  const [i, f] = fixed.split(".");
  const int = thousands ? i!.replace(/\B(?=(\d{3})+(?!\d))/g, ",") : i!;
  return f ? `${int}.${f}` : int;
}

/** Significant digits written in a number's text (before any exponent). */
function significantDigits(text: string): number {
  const mantissa = text.split(/e/i)[0] ?? "";
  const digits = mantissa.replace(/\D/g, "").replace(/^0+/, "");
  // Zeros after the decimal point at the end add nothing.
  const trimmed = mantissa.includes(".") ? digits.replace(/0+$/, "") : digits;
  return trimmed.length;
}

/** How a cell shows under a display format. Values the format does not fit
 *  (text in a number column) show as they are. */
export function formatCell(text: string, f: DisplayFormat): string {
  if (f.kind === "checkbox") return text;
  if (f.kind === "date") {
    const d = readDate(text);
    if (!d) return text;
    const [y, m, day] = [String(d.getUTCFullYear()).padStart(4, "0"), d.getUTCMonth(), d.getUTCDate()];
    const p = (n: number) => String(n).padStart(2, "0");
    switch (f.date) {
      case "iso":
        return `${y}-${p(m + 1)}-${p(day)}`;
      case "mdy":
        return `${p(m + 1)}/${p(day)}/${y}`;
      case "dmy":
        return `${p(day)}/${p(m + 1)}/${y}`;
      case "medium":
        return `${MONTHS[m]!.slice(0, 3)} ${day}, ${y}`;
      case "long":
        return `${DAYS[d.getUTCDay()]}, ${MONTHS[m]} ${day}, ${y}`;
    }
  }
  const n = readNumber(text);
  if (n === null) return text;
  // More digits than a number holds exactly (long IDs, card numbers):
  // showing them through a number would change digits, so show the text.
  if (significantDigits(text) > 15) return text;
  const v = f.kind === "percent" ? n * 100 : n;
  if (Math.abs(v) >= 1e21) return text;
  const body = groupDigits(v, f.decimals, f.thousands);
  const neg = v < 0 && Number(body.replace(/,/g, "")) !== 0 ? "-" : "";
  if (f.kind === "percent") return `${neg}${body}%`;
  if (f.kind === "currency") return `${neg}${f.symbol}${body}`;
  return `${neg}${body}`;
}

/** The cleaning steps that write a display format into the cells, so the
 *  file holds the text as it is shown (none for tick boxes). */
export function formatSteps(f: DisplayFormat): Step[] {
  const numbers: Step = { tool: "numbers", decimals: f.decimals, thousands: f.thousands, decimal_comma: false };
  switch (f.kind) {
    case "checkbox":
      return [];
    case "date": {
      const formats: Record<DateStyle, string> = { iso: "yyyy-mm-dd", mdy: "mm/dd/yyyy", dmy: "dd/mm/yyyy", medium: "mmm d, yyyy", long: "dddd, mmmm d, yyyy" };
      return [{ tool: "dates", format: formats[f.date] }];
    }
    case "number":
      return [numbers];
    case "percent":
      return [{ tool: "calc", op: "multiply", value: 100 }, numbers, { tool: "add", text: "%", at: { kind: "end" } }];
    case "currency":
      if (!f.symbol) return [numbers];
      return [
        numbers,
        { tool: "add", text: f.symbol, at: { kind: "start" } },
        { tool: "replace", find: { text: `${f.symbol}-`, kind: "plain", match_case: true, whole_cell: false }, with: `-${f.symbol}` },
      ];
  }
}

export function describeFormat(f: DisplayFormat): string {
  switch (f.kind) {
    case "number":
      return `number, ${f.decimals} decimals${f.thousands ? ", 1,000s" : ""}`;
    case "percent":
      return `percent, ${f.decimals} decimals`;
    case "currency":
      return `currency ${f.symbol}, ${f.decimals} decimals`;
    case "date":
      return `date like ${DATE_STYLES.find((s) => s.id === f.date)?.label}`;
    case "checkbox":
      return "tick boxes";
  }
}

// ---- tick boxes ------------------------------------------------------------------

const PAIRS: [string, string][] = [
  ["true", "false"],
  ["yes", "no"],
  ["y", "n"],
  ["1", "0"],
  ["x", ""],
];

/** Whether a cell reads as ticked (true), not ticked (false) or neither. */
export function tickOf(text: string): boolean | null {
  const t = text.trim().toLowerCase();
  if (t === "") return false;
  for (const [on, off] of PAIRS) {
    if (t === on) return true;
    if (t === off) return false;
  }
  return null;
}

/** The value after clicking a tick box, written in the same style as the
 *  cell (TRUE/FALSE, Yes/No, Y/N, 1/0, x/blank); `others` are the column's
 *  nearby values, used to pick the style for a blank cell. */
export function toggled(text: string, others: string[] = []): string | null {
  const now = tickOf(text);
  if (now === null) return null;
  const t = text.trim();
  const sample = t !== "" ? t : (others.map((o) => o.trim()).find((o) => o !== "" && tickOf(o) !== null) ?? "");
  const low = sample.toLowerCase();
  const pair = PAIRS.find(([on, off]) => low === on || (low === off && off !== "")) ?? PAIRS[0]!;
  const word = now ? pair[1] : pair[0];
  if (sample === sample.toUpperCase() && sample !== sample.toLowerCase()) return word.toUpperCase();
  if (sample[0] && sample[0] === sample[0].toUpperCase() && sample[0] !== sample[0].toLowerCase()) return word.charAt(0).toUpperCase() + word.slice(1);
  return sample === "" ? word.toUpperCase() : word;
}

// ---- wrapped text ----------------------------------------------------------------

/** Lines of `text` broken to fit `width` (by words, long words cut), at
 *  most `max` lines (the last ends in "…" when more was left). */
export function wrapLines(text: string, width: number, measure: (s: string) => number, max: number): string[] {
  const out: string[] = [];
  let cut = false;
  const paras = text.split(/\r\n|\r|\n/);
  // Push a line; false once `max` lines are full and more text is left.
  const push = (s: string): boolean => {
    if (out.length >= max) {
      cut = true;
      return false;
    }
    out.push(s.trimEnd());
    return true;
  };
  outer: for (const para of paras) {
    let line = "";
    for (const w of para.split(/(\s+)/)) {
      if (!w) continue;
      if (measure(line + w) <= width) {
        line += w;
        continue;
      }
      if (/^\s+$/.test(w)) {
        if (!push(line)) break outer;
        line = "";
        continue;
      }
      if (line.trim()) {
        if (!push(line)) break outer;
        line = "";
      }
      if (measure(w) <= width) {
        line = w;
        continue;
      }
      // One word wider than the cell: cut it.
      for (const ch of w) {
        if (line && measure(line + ch) > width) {
          if (!push(line)) break outer;
          line = "";
        }
        line += ch;
      }
    }
    if (!push(line)) break;
  }
  if (cut && out.length) {
    let last = out[out.length - 1]!;
    while (last && measure(last + "…") > width) last = last.slice(0, -1);
    out[out.length - 1] = last + "…";
  }
  return out;
}

/** Lines per row the grid allows (rows all have the same height). */
export const MAX_ROW_LINES = 10;

// ---- pick lists ------------------------------------------------------------------

/** Most values a pick list shows. */
export const MAX_PICKS = 200;

/** The allowed values to offer for what was typed: those starting with it
 *  first, then those containing it (case ignored). */
export function pickMatches(values: string[], typed: string, limit = MAX_PICKS): string[] {
  const q = typed.trim().toLowerCase();
  if (!q) return values.slice(0, limit);
  const starts: string[] = [];
  const has: string[] = [];
  for (const v of values) {
    const l = v.toLowerCase();
    if (l.startsWith(q)) starts.push(v);
    else if (l.includes(q)) has.push(v);
    if (starts.length >= limit) break;
  }
  return [...starts, ...has].slice(0, limit);
}

// ---- shortcut list ---------------------------------------------------------------

/** Shortcuts of one menu, for the shortcut list. */
export interface KeyGroup {
  name: string;
  items: { label: string; keys: string[] }[];
}

/** Keys the grid handles itself (not commands), for the shortcut list. */
export const BASIC_KEYS: { label: string; keys: string }[] = [
  { label: "Move", keys: "Arrows, Tab, Enter" },
  { label: "Extend the selection", keys: "Shift+arrows" },
  { label: "Jump to the edge of the data", keys: "Ctrl+arrows" },
  { label: "Select the row", keys: "Shift+Space" },
  { label: "Select the column", keys: "Ctrl+Space" },
  { label: "Page up or down", keys: "PageUp, PageDown" },
  { label: "First or last cell", keys: "Ctrl+Home, Ctrl+End" },
  { label: "Edit the cell", keys: "F2 or start typing" },
  { label: "While typing: arrows keep the text and move on; F2 switches to moving inside the text (and back)", keys: "F2" },
  { label: "Line break in a cell", keys: "Alt+Enter" },
  { label: "Put the value in every selected cell", keys: "Ctrl+Enter" },
  { label: "Move columns, rows or cells somewhere else", keys: "Ctrl+X, pick the place, Ctrl+V" },
  { label: "Pick from the column's values (or its list of allowed values)", keys: "Alt+Down" },
  { label: "Tick or untick a tick box", keys: "Space" },
  { label: "Open the right-click menu", keys: "Shift+F10 or the Menu key" },
  { label: "Edit, starting with the value of the cell above", keys: "Ctrl+'" },
  { label: "Select the block of data, then everything", keys: "Ctrl+A (Ctrl+Shift+* for the block)" },
  { label: "Scroll back to the active cell", keys: "Ctrl+Backspace" },
  { label: "Select just the active cell", keys: "Shift+Backspace" },
  { label: "Move the active cell round the selection's corners", keys: "Ctrl+." },
  { label: "Extend mode: arrows grow the selection (F8 or Esc ends it)", keys: "F8" },
  { label: "A screen left or right", keys: "Alt+PageUp, Alt+PageDown" },
  { label: "Drop the copy outline (the selection stays)", keys: "Esc" },
  { label: "Open a web or e-mail address", keys: "Ctrl+click" },
  { label: "Open a menu", keys: "Alt+letter" },
  { label: "Find bar: find all", keys: "Alt+Enter" },
  { label: "Find bar: replace all (shows what changes first)", keys: "Ctrl+Alt+Enter" },
  { label: "Find bar: match case, whole word, regular expression, selection on or off", keys: "Alt+C, Alt+W, Alt+R, Alt+L" },
  { label: "Find bar: earlier searches", keys: "Up, Down" },
  { label: "Find bar list of matches: move, go to a match", keys: "Up, Down, Enter" },
];

/** Shortcut groups from menus: each menu's commands that have keys. */
export function keyGroups(
  menus: { label: string; items: string[] }[],
  get: (id: string) => { label: string; keys: string[] } | null,
): KeyGroup[] {
  const seen = new Set<string>();
  const out: KeyGroup[] = [];
  for (const m of menus) {
    const items: KeyGroup["items"] = [];
    for (const id of m.items) {
      if (id === "-" || seen.has(id)) continue;
      const c = get(id);
      if (!c || !c.keys.length) continue;
      seen.add(id);
      items.push({ label: c.label.replace(/&/g, "").replace(/…$/, ""), keys: c.keys });
    }
    if (items.length) out.push({ name: m.label.replace(/&/g, ""), items });
  }
  return out;
}

// ---- clean-up suggestions --------------------------------------------------------

export type SuggestKind = "spaces" | "number_text" | "mixed_dates" | "duplicate_rows" | "blank_rows";
export type SuggestFix = { fix: "clean"; cols: number[]; step: Step } | { fix: "remove_duplicates" } | { fix: "remove_blank_rows" };
export interface Suggestion {
  kind: SuggestKind;
  col: number | null;
  count: number;
  examples: string[];
  fix: SuggestFix;
}
export interface Suggestions {
  items: Suggestion[];
  rows: number;
  partial: boolean;
}

/** A suggestion in words: what was found, and what the fix does. */
export function describeSuggestion(s: Suggestion, title: (col: number) => string): { found: string; fix: string } {
  const n = s.count.toLocaleString();
  const col = s.col === null ? "" : `“${title(s.col)}”`;
  switch (s.kind) {
    case "spaces": {
      const collapse = s.fix.fix === "clean" && s.fix.step.tool === "spaces" && s.fix.step.how === "collapse";
      return {
        found: `${n} ${s.count === 1 ? "cell" : "cells"} in ${col} ${s.count === 1 ? "has" : "have"} extra spaces`,
        fix: collapse ? "Remove spaces at the ends and make double spaces single" : "Remove spaces at the start and end",
      };
    }
    case "number_text":
      return { found: `${n} ${s.count === 1 ? "number" : "numbers"} in ${col} ${s.count === 1 ? "is" : "are"} written with currency signs, thousands separators or brackets`, fix: "Write them as plain numbers (1234.5)" };
    case "mixed_dates":
      return { found: `Dates in ${col} are written in more than one style`, fix: "Write every date as year-month-day (2026-03-14)" };
    case "duplicate_rows":
      return { found: `${n} ${s.count === 1 ? "row repeats" : "rows repeat"} an earlier row exactly`, fix: "Remove the repeated rows (the first of each is kept)" };
    case "blank_rows":
      return { found: `${n} ${s.count === 1 ? "row is" : "rows are"} completely blank`, fix: "Remove the blank rows" };
  }
}

// ---- autosave and recovery -------------------------------------------------------

export interface RecoveryItem {
  index: number;
  title: string;
  path: string | null;
  saved: number;
  rows: number;
  bytes: number;
}
export interface VersionItem {
  index: number;
  saved: number;
  bytes: number;
}
export type Autosaved =
  | { kind: "written"; at: number; bytes: number }
  | { kind: "unchanged" }
  | { kind: "too_large" }
  | { kind: "full" }
  | { kind: "skipped" }
  | { kind: "off" };

/** Whether a tab is due a recovery copy: unsaved changes, loaded, nothing
 *  running, and `minutes` since its last copy (or since it changed first). */
export function autosaveDue(now: number, last: number | null, minutes: number, modified: boolean, ready: boolean): boolean {
  if (!modified || !ready) return false;
  return last === null || now - last >= Math.max(1, minutes) * 60_000;
}
