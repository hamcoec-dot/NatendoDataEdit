// Helpers for fixed-width import, log patterns, data checks, computed
// columns and printing. Pure functions, unit-tested; the engine does the
// real work on files.

import type { Check, CheckKind, Compare, PageSetup, Paper, TableCheck, TableCheckKind } from "./backend";

// ---- fixed width ----

/** Tabs to spaces (next multiple of 8), as the engine does. */
export function expandTabs(line: string): string {
  if (!line.includes("\t")) return line;
  let out = "";
  for (const c of line) {
    if (c === "\t") out += " ".repeat(8 - (Array.from(out).length % 8));
    else out += c;
  }
  return out;
}

/** Column starts: where a run of positions that are spaces in every line ends. */
export function guessStarts(lines: string[]): number[] {
  const gap: boolean[] = [];
  let width = 0;
  let seen = false;
  for (const raw of lines) {
    if (!raw.trim()) continue;
    const chars = Array.from(expandTabs(raw));
    chars.forEach((c, i) => {
      if (i >= gap.length) gap.push(true);
      if (c !== " ") gap[i] = false;
    });
    width = Math.max(width, chars.length);
    seen = true;
  }
  const starts = [0];
  if (!seen) return starts;
  for (let i = 1; i < width; i++) if (!gap[i] && gap[i - 1]) starts.push(i);
  return starts;
}

/** Cut a line at the column starts (characters). */
export function splitFixed(line: string, starts: number[], keepSpaces = false): string[] {
  const chars = Array.from(expandTabs(line));
  const s = [...new Set([0, ...starts])].sort((a, b) => a - b);
  return s.map((a, k) => {
    const b = k + 1 < s.length ? s[k + 1]! : chars.length;
    const cell = chars.slice(a, Math.max(a, b)).join("");
    return keepSpaces ? cell : cell.trim();
  });
}

/** Add a column start at a position, or take it away when it is there. */
export function toggleStart(starts: number[], at: number): number[] {
  if (at <= 0) return starts;
  return starts.includes(at) ? starts.filter((s) => s !== at) : [...starts, at].sort((a, b) => a - b);
}

// ---- log files ----

/** Ready-made line patterns (the engine's regular expression syntax). */
export const LOG_PRESETS: { name: string; pattern: string }[] = [
  {
    name: "Web server access log",
    pattern: String.raw`^(?P<host>\S+) (?P<ident>\S+) (?P<user>\S+) \[(?P<time>[^\]]+)\] "(?P<request>[^"]*)" (?P<status>\d{3}) (?P<size>\S+)(?: "(?P<referrer>[^"]*)" "(?P<agent>[^"]*)")?`,
  },
  {
    name: "System log (syslog)",
    pattern: String.raw`^(?P<time>[A-Z][a-z]{2} [ \d]\d \d\d:\d\d:\d\d) (?P<host>\S+) (?P<process>[^:\[\s]+)(?:\[(?P<pid>\d+)\])?: (?P<message>.*)$`,
  },
  {
    name: "Date and time, level, message",
    pattern: String.raw`^(?P<time>\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:[.,]\d+)?(?:Z|[+-]\d\d:?\d\d)?)\s+\[?(?P<level>TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|FATAL|CRITICAL|trace|debug|info|warn|warning|error|fatal|critical)\]?:?\s+(?P<message>.*)$`,
  },
  { name: "Date and time, message", pattern: String.raw`^(?P<time>\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d\S*)\s+(?P<message>.*)$` },
  { name: "Whole line", pattern: String.raw`^(?P<line>.*)$` },
];

/** The engine's named groups written the browser's way (for the preview stub). */
export function toJsPattern(p: string): string {
  return p.replace(/\(\?P</g, "(?<");
}

// ---- data checks ----

export const CHECK_KINDS: { type: CheckKind["type"]; label: string }[] = [
  { type: "required", label: "Must not be blank" },
  { type: "number", label: "Number" },
  { type: "date", label: "Date" },
  { type: "date_iso", label: "Date written YYYY-MM-DD" },
  { type: "length", label: "Length (characters)" },
  { type: "list", label: "One of a list" },
  { type: "pattern", label: "Matches a pattern" },
  { type: "unique", label: "No repeats" },
  { type: "expression", label: "Row rule (expression)" },
  { type: "foreign_key", label: "Value must exist in another file" },
  { type: "settled_by", label: "Same for each value of another column" },
];

/** A new check of a kind, with empty settings. */
export function newCheck(type: CheckKind["type"], col: number): Check {
  const kind: CheckKind =
    type === "required" ? { type }
    : type === "number" ? { type, min: null, max: null, whole: false }
    : type === "date" ? { type, min: null, max: null }
    : type === "date_iso" ? { type }
    : type === "length" ? { type, min: null, max: null }
    : type === "list" ? { type, values: [], match_case: false }
    : type === "pattern" ? { type, pattern: { text: "", kind: "regex", match_case: false, whole_cell: true } }
    : type === "unique" ? { type, match_case: false }
    : type === "foreign_key" ? { type, table: "", column: "", match_case: false, trim: true, allow_blank: true }
    : type === "settled_by" ? { type, by: col === 0 ? 1 : 0, match_case: true }
    : { type: "expression", expr: "" };
  return { col: type === "expression" ? null : col, kind };
}

/** A check moved to other columns: its column and, for a unique rule, every `also` column
 *  (`place` gives null for a column that is gone, and then the rule is dropped: null). */
export function placeCheck(c: Check, place: (k: number) => number | null | undefined): Check | null {
  if (c.col === null) return c;
  const col = place(c.col);
  if (col === null || col === undefined || col < 0) return null;
  if (c.kind.type === "settled_by") {
    const by = place(c.kind.by);
    if (by === null || by === undefined || by < 0) return null;
    return { ...c, col, kind: { ...c.kind, by } };
  }
  if (c.kind.type !== "unique" || !c.kind.also?.length) return { ...c, col };
  const also: number[] = [];
  for (const a of c.kind.also) {
    const k = place(a);
    if (k === null || k === undefined || k < 0) return null;
    also.push(k);
  }
  return { ...c, col, kind: { ...c.kind, also } };
}

/** Rules after `count` columns were put in at `at` (or, with a negative count, taken out from `at`):
 *  columns to the right move along, and a rule that used a deleted column (its own or one of
 *  its key) is dropped and counted. */
export function shiftChecks(checks: Check[], at: number, count: number): { checks: Check[]; dropped: number } {
  const place = (c: number): number | null => {
    if (c < at) return c;
    if (count >= 0) return c + count;
    return c < at - count ? null : c + count;
  };
  let dropped = 0;
  const out = checks.flatMap((c) => {
    const k = placeCheck(c, place);
    if (k === null) dropped++;
    return k === null ? [] : [k];
  });
  return { checks: out, dropped };
}

const num = (n: number | null): string => (n === null ? "" : n.toLocaleString());

/** A check in words, for lists and the problem panel. */
export function describeCheck(c: Check, titles: string[]): string {
  const col = c.col === null ? "" : `${titles[c.col] ?? `Column ${c.col + 1}`}: `;
  const k = c.kind;
  const range = (lo: string, hi: string, unit = ""): string =>
    lo && hi ? ` from ${lo} to ${hi}${unit}` : lo ? ` of at least ${lo}${unit}` : hi ? ` of at most ${hi}${unit}` : "";
  switch (k.type) {
    case "required":
      return `${col}must not be blank`;
    case "number":
      return `${col}${k.whole ? "whole number" : "number"}${range(num(k.min), num(k.max))}`;
    case "date":
      return `${col}date${range(k.min ?? "", k.max ?? "")}`;
    case "date_iso":
      return `${col}date written YYYY-MM-DD`;
    case "length":
      return `${col}length${range(num(k.min), num(k.max), " characters")}`;
    case "list":
      return `${col}one of ${k.values.slice(0, 5).join(", ")}${k.values.length > 5 ? ", …" : ""}`;
    case "pattern":
      return `${col}matches ${k.pattern.text}`;
    case "unique":
      if (k.also?.length) return `${col}no repeated values together with ${k.also.map((a) => titles[a] ?? `Column ${a + 1}`).join(", ")}`;
      return `${col}no repeated values`;
    case "expression":
      return `row rule: ${k.expr}`;
    case "foreign_key":
      return `${col}must exist in ${k.table || "another file"} > ${k.column || "a column"}`;
    case "settled_by":
      return `${col}the same for each ${titles[k.by] ?? `Column ${k.by + 1}`}`;
  }
}

/** Ready-made patterns for the pattern check. */
export const CHECK_PATTERNS: { name: string; text: string }[] = [
  { name: "Email address", text: String.raw`[^@\s]+@[^@\s]+\.[^@\s]+` },
  { name: "US ZIP code", text: String.raw`\d{5}(-\d{4})?` },
  { name: "US phone number", text: String.raw`(\+1[ .-]?)?\(?\d{3}\)?[ .-]?\d{3}[ .-]?\d{4}` },
  { name: "Web address", text: String.raw`https?://\S+` },
  { name: "Letters only", text: String.raw`\p{L}+` },
  { name: "Digits only", text: String.raw`\d+` },
];

// ---- computed columns ----

/** Function names with a short hint, for the expression editor. */
export const FUNCTIONS: { name: string; hint: string }[] = [
  { name: "IF", hint: "IF(test, then, else)" },
  { name: "IFS", hint: "IFS(test1, value1, test2, value2, …)" },
  { name: "IFERROR", hint: "IFERROR(value, if error)" },
  { name: "AND", hint: "AND(a, b, …)" },
  { name: "OR", hint: "OR(a, b, …)" },
  { name: "NOT", hint: "NOT(a)" },
  { name: "ISBLANK", hint: "ISBLANK(value)" },
  { name: "ISNUMBER", hint: "ISNUMBER(value)" },
  { name: "ISTEXT", hint: "ISTEXT(value)" },
  { name: "ISDATE", hint: "ISDATE(value)" },
  { name: "ISERROR", hint: "ISERROR(value)" },
  { name: "SWITCH", hint: "SWITCH(value, match1, result1, …, [otherwise])" },
  { name: "CHOOSE", hint: "CHOOSE(number, first, second, …)" },
  { name: "IN", hint: "IN(value, a, b, …): whether it is one of these" },
  { name: "BETWEEN", hint: "BETWEEN(value, low, high): low and high count" },
  { name: "BLANK", hint: "BLANK(): an empty cell" },
  { name: "COALESCE", hint: "COALESCE(a, b, …): the first that is not blank" },
  { name: "LEN", hint: "LEN(text)" },
  { name: "LEFT", hint: "LEFT(text, count)" },
  { name: "RIGHT", hint: "RIGHT(text, count)" },
  { name: "MID", hint: "MID(text, start, count)" },
  { name: "UPPER", hint: "UPPER(text)" },
  { name: "LOWER", hint: "LOWER(text)" },
  { name: "PROPER", hint: "PROPER(text): Each Word Capitalized" },
  { name: "TRIM", hint: "TRIM(text): no extra spaces" },
  { name: "CONCAT", hint: "CONCAT(a, b, …)" },
  { name: "TEXTJOIN", hint: "TEXTJOIN(separator, skip blanks, a, b, …)" },
  { name: "SUBSTITUTE", hint: "SUBSTITUTE(text, old, new, [which])" },
  { name: "REPLACE", hint: "REPLACE(text, start, count, new)" },
  { name: "FIND", hint: "FIND(look for, text, [start]): position, case matters" },
  { name: "SEARCH", hint: "SEARCH(look for, text, [start]): position, any case" },
  { name: "EXACT", hint: "EXACT(a, b)" },
  { name: "REPT", hint: "REPT(text, times)" },
  { name: "TEXTBEFORE", hint: "TEXTBEFORE(text, separator, [which]): -1 is the last" },
  { name: "TEXTAFTER", hint: "TEXTAFTER(text, separator, [which]): -1 is the last" },
  { name: "SPLITPART", hint: "SPLITPART(text, separator, part): -1 is the last" },
  { name: "PADLEFT", hint: "PADLEFT(text, length, [with]): zeros unless given" },
  { name: "CLEAN", hint: "CLEAN(text): no hidden or control characters" },
  { name: "SHAPE", hint: "SHAPE(text): digits as 9 and letters as A, like 99999-9999" },
  { name: "COUNTCHAR", hint: "COUNTCHAR(text, what): how many times it appears" },
  { name: "TEXT", hint: 'TEXT(value, "pattern"): like "0.00", "#,##0", "yyyy-mm-dd"' },
  { name: "VALUE", hint: "VALUE(text): the number in it" },
  { name: "FIXED", hint: "FIXED(number, [decimals], [no commas])" },
  { name: "ROUND", hint: "ROUND(number, [decimals])" },
  { name: "ROUNDUP", hint: "ROUNDUP(number, [decimals])" },
  { name: "ROUNDDOWN", hint: "ROUNDDOWN(number, [decimals])" },
  { name: "INT", hint: "INT(number)" },
  { name: "ABS", hint: "ABS(number)" },
  { name: "SIGN", hint: "SIGN(number)" },
  { name: "MOD", hint: "MOD(number, divisor)" },
  { name: "POWER", hint: "POWER(number, power)" },
  { name: "SQRT", hint: "SQRT(number)" },
  { name: "EXP", hint: "EXP(number)" },
  { name: "LN", hint: "LN(number)" },
  { name: "LOG10", hint: "LOG10(number)" },
  { name: "MIN", hint: "MIN(a, b, …)" },
  { name: "MAX", hint: "MAX(a, b, …)" },
  { name: "SUM", hint: "SUM(a, b, …)" },
  { name: "AVERAGE", hint: "AVERAGE(a, b, …)" },
  { name: "COUNT", hint: "COUNT(a, b, …): how many are numbers" },
  { name: "COUNTA", hint: "COUNTA(a, b, …): how many are not blank" },
  { name: "DATE", hint: "DATE(year, month, day)" },
  { name: "YEAR", hint: "YEAR(date)" },
  { name: "MONTH", hint: "MONTH(date)" },
  { name: "DAY", hint: "DAY(date)" },
  { name: "HOUR", hint: "HOUR(date and time)" },
  { name: "MINUTE", hint: "MINUTE(date and time)" },
  { name: "WEEKDAY", hint: "WEEKDAY(date): 1 is Sunday" },
  { name: "DAYS", hint: "DAYS(end, start)" },
  { name: "DATEVALUE", hint: "DATEVALUE(text): the date in it" },
  { name: "AGE", hint: "AGE(born, [on]): whole years, today unless given" },
  { name: "DATEDIF", hint: 'DATEDIF(start, end, "Y", "M", "D", "YM", "MD" or "YD")' },
  { name: "EDATE", hint: "EDATE(date, months): same day, months later" },
  { name: "EOMONTH", hint: "EOMONTH(date, months): last day of the month" },
  { name: "NETWORKDAYS", hint: "NETWORKDAYS(start, end, [holidays…]): weekdays" },
  { name: "WEEKNUM", hint: "WEEKNUM(date, [1 Sunday, 2 Monday, 21 ISO])" },
  { name: "TODAY", hint: "TODAY()" },
  { name: "NOW", hint: "NOW()" },
  { name: "ROW", hint: "ROW(): the row's number" },
  { name: "PREV", hint: "PREV([column]): the value in the row above, as shown" },
  { name: "NEXT", hint: "NEXT([column]): the value in the row below, as shown" },
  { name: "REGEXMATCH", hint: 'REGEXMATCH(text, "pattern")' },
  { name: "REGEXEXTRACT", hint: 'REGEXEXTRACT(text, "pattern", [group])' },
  { name: "REGEXREPLACE", hint: 'REGEXREPLACE(text, "pattern", "new")' },
  { name: "LOOKUP", hint: 'LOOKUP(value, "other tab", "key column", "result column", "if not found")' },
];

/** A column written for an expression: [Title], or [#3] when the title is blank or awkward. */
export function columnRef(title: string, index: number): string {
  return title.trim() && !title.includes("]") && title === title.trim() ? `[${title}]` : `[#${index + 1}]`;
}

/** A column in a table check's formula: its title in brackets (a `]` in it written twice),
 *  or its number when the title is blank, has spaces at the ends or starts with #. */
export function tableColRef(title: string, index: number): string {
  return title && title === title.trim() && !title.startsWith("#") ? `[${title.replaceAll("]", "]]")}]` : `[#${index + 1}]`;
}

/** A table check after its column moved (`place` gives the new place; null or undefined: gone). */
export function placeTableCheck(c: TableCheck, place: (k: number) => number | null | undefined): TableCheck | null {
  const k = c.kind;
  if (k.type !== "no_blanks" && k.type !== "distinct") return c;
  const col = place(k.col);
  return col === null || col === undefined || col < 0 ? null : { ...c, kind: { ...k, col } };
}

/** Table checks after `count` columns were put in at `at` (or taken out, with a negative
 *  count): the same as [`shiftChecks`]. A formula names its columns by title, so it stays. */
export function shiftTableChecks(checks: TableCheck[], at: number, count: number): { checks: TableCheck[]; dropped: number } {
  const place = (c: number): number | null => {
    if (c < at) return c;
    if (count >= 0) return c + count;
    return c < at - count ? null : c + count;
  };
  let dropped = 0;
  const out = checks.flatMap((c) => {
    const k = placeTableCheck(c, place);
    if (k === null) dropped++;
    return k === null ? [] : [k];
  });
  return { checks: out, dropped };
}

const COMPARE_WORDS: Record<Compare, string> = { equals: "exactly", at_least: "at least", at_most: "at most" };

/** A table check in words (its name when it has one). */
export function describeTableCheck(c: TableCheck, titles: string[], tolerance = true): string {
  if (c.name?.trim()) return c.name.trim();
  const k = c.kind;
  const title = (x: number): string => titles[x] || `Column ${x + 1}`;
  switch (k.type) {
    case "row_count":
      return `Rows: ${COMPARE_WORDS[k.compare]} ${k.value.toLocaleString()}`;
    case "row_count_same_as":
      return `As many rows as ${k.table || "another file"}`;
    case "no_blanks":
      return `No blanks in ${title(k.col)}`;
    case "distinct":
      return `${COMPARE_WORDS[k.compare]} ${k.value.toLocaleString()} different values in ${title(k.col)}`;
    case "aggregate": {
      const t = k.tolerance ?? { kind: "absolute", value: 0.005 };
      return tolerance ? `${k.expr || "(no formula)"} (within ${t.kind === "relative" ? `${t.value * 100}%` : t.value})` : k.expr || "(no formula)";
    }
  }
}

/** A new table check of a kind, on column `col` where it reads one. */
export function newTableCheck(type: TableCheckKind["type"], col: number): TableCheck {
  switch (type) {
    case "row_count":
      return { kind: { type, compare: "at_least", value: 1 } };
    case "row_count_same_as":
      return { kind: { type, table: "" } };
    case "no_blanks":
      return { kind: { type, col } };
    case "distinct":
      return { kind: { type, col, compare: "at_most", value: 10 } };
    case "aggregate":
      return { kind: { type, expr: "", tolerance: { kind: "absolute", value: 0.005 } } };
  }
}

/** The table check kinds, as the dialog lists them. */
export const TABLE_CHECK_KINDS: { type: TableCheckKind["type"]; label: string }[] = [
  { type: "row_count", label: "Number of rows" },
  { type: "row_count_same_as", label: "As many rows as another file" },
  { type: "no_blanks", label: "No blank cells in a column" },
  { type: "distinct", label: "Number of different values" },
  { type: "aggregate", label: "Totals that must add up (formula)" },
];

/** Local date and time as days since 1970 (for TODAY and NOW). */
export function localDays(d = new Date()): number {
  return (d.getTime() - d.getTimezoneOffset() * 60_000) / 86_400_000;
}

// ---- printing ----

export const PAPERS: { value: Paper; label: string; size: [number, number] }[] = [
  { value: "letter", label: "Letter (8.5 × 11 in)", size: [215.9, 279.4] },
  { value: "a4", label: "A4 (210 × 297 mm)", size: [210, 297] },
  { value: "legal", label: "Legal (8.5 × 14 in)", size: [215.9, 355.6] },
  { value: "a3", label: "A3 (297 × 420 mm)", size: [297, 420] },
];

export function defaultPage(title: string): PageSetup {
  return {
    paper: "letter",
    landscape: false,
    margin_mm: 12.7,
    font_size: 9,
    fit_width: true,
    grid: true,
    repeat_header: true,
    title,
    page_numbers: true,
    rows: null,
    cols: null,
  };
}

/** Most rows and cells the Print command sends to the printer (PDF has no limit). */
export const PRINT_MAX_ROWS = 50_000;
export const PRINT_MAX_CELLS = 1_000_000;

/** Text for a CSS string: everything but letters, digits and spaces as
 *  escapes, so a title can never end the string or the style block. */
export function cssString(s: string): string {
  return Array.from(s.slice(0, 500))
    .map((c) => (/[\p{L}\p{N} ]/u.test(c) ? c : `\\${c.codePointAt(0)!.toString(16)} `))
    .join("");
}

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

const isNumber = (v: string): boolean => /^\s*[-+(]?[$€£¥]?\s*\d[\d,]*(\.\d+)?\)?%?\s*$/.test(v);

/** A whole printable page (for a print frame): the table with its titles
 *  repeated on each page, sized by the page setup. */
/** `textCols`: by printed column, true for a column typed Text or Id (its values are never set as numbers). */
export function printHtml(titles: string[] | null, rows: string[][], p: PageSetup, fontFamily: string, preview = false, textCols: readonly boolean[] = []): string {
  const paper = PAPERS.find((x) => x.value === p.paper) ?? PAPERS[0]!;
  const [w, h] = p.landscape ? [paper.size[1], paper.size[0]] : paper.size;
  const margin = Math.max(0, Math.min(p.margin_mm, Math.min(w, h) / 4));
  const size = Math.max(4, Math.min(72, p.font_size));
  const head = titles
    ? `<thead><tr>${titles.map((t) => `<th>${escapeHtml(t)}</th>`).join("")}</tr></thead>`
    : "";
  const body = rows
    .map((r) => `<tr>${r.map((v, k) => `<td${!textCols[k] && isNumber(v) ? ' class="n"' : ""}>${escapeHtml(v)}</td>`).join("")}</tr>`)
    .join("\n");
  const pageNo = p.page_numbers ? `@bottom-center { content: "Page " counter(page) " of " counter(pages); font: ${size}pt ${fontFamily}; }` : "";
  const title = p.title.trim() ? `@top-left { content: "${cssString(p.title)}"; font: ${size + 3}pt ${fontFamily}; }` : "";
  const border = p.grid ? "0.5pt solid #999" : "none";
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapeHtml(p.title)}</title><style>
@page { size: ${w}mm ${h}mm; margin: ${margin}mm; ${title} ${pageNo} }
html, body { margin: 0; background: #fff; color: #000; }
body { font: ${size}pt ${fontFamily}; }
table { border-collapse: collapse; ${p.fit_width ? "width: 100%; table-layout: auto;" : ""} }
thead { display: ${p.repeat_header ? "table-header-group" : "table-row-group"}; }
th { background: #ebebeb; text-align: left; font-weight: 600; }
th, td { border: ${border}; padding: 0.15em 0.35em; vertical-align: top; white-space: pre-wrap; overflow-wrap: anywhere; }
td.n { text-align: right; }
tr { break-inside: avoid; }
${preview ? `body { padding: ${margin}mm; width: ${w}mm; height: ${h}mm; box-sizing: border-box; overflow: hidden; } .title { font-size: ${size + 3}pt; margin: 0 0 0.6em; }` : ""}
</style></head><body>${preview && p.title.trim() ? `<p class="title">${escapeHtml(p.title)}</p>` : ""}<table>${head}<tbody>
${body}
</tbody></table></body></html>`;
}
