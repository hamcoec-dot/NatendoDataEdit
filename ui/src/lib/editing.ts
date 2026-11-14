// Everyday editing tools (Milestone 12): helpers for the value editor, the
// column builder (template and if-then rules written as an expression for
// computed columns), and the option lists of the other tools.

import { columnRef } from "./m8";

/** An expression text literal ("" doubles a quote). */
export function quote(s: string): string {
  return `"${s.replace(/"/g, '""')}"`;
}

/**
 * A template like "{First} {Last}" as an expression: `[First] & " " &
 * [Last]`. Names in braces are column titles (or #3 for the third column);
 * "{{" and "}}" are literal braces. Unknown names are reported.
 */
export function templateExpr(template: string, titles: string[]): { expr: string; unknown: string[] } {
  const parts: string[] = [];
  const unknown: string[] = [];
  let text = "";
  let i = 0;
  const flush = () => {
    if (text) parts.push(quote(text));
    text = "";
  };
  while (i < template.length) {
    const c = template[i]!;
    if (c === "{" && template[i + 1] === "{") {
      text += "{";
      i += 2;
    } else if (c === "}" && template[i + 1] === "}") {
      text += "}";
      i += 2;
    } else if (c === "{") {
      const end = template.indexOf("}", i + 1);
      if (end < 0) {
        text += template.slice(i);
        break;
      }
      const name = template.slice(i + 1, end).trim();
      const num = /^#(\d+)$/.exec(name);
      const col = num ? Number(num[1]) - 1 : titles.findIndex((t) => t.trim().toLowerCase() === name.toLowerCase());
      if (col < 0 || col >= titles.length) unknown.push(name);
      else {
        flush();
        parts.push(columnRef(titles[col]!, col));
      }
      i = end + 1;
    } else {
      text += c;
      i++;
    }
  }
  flush();
  return { expr: parts.length ? parts.join(" & ") : '""', unknown };
}

export type RuleOp = "is" | "is_not" | "contains" | "starts" | "ends" | "blank" | "not_blank" | "gt" | "ge" | "lt" | "le";

export const RULE_OPS: [RuleOp, string][] = [
  ["is", "is"],
  ["is_not", "is not"],
  ["contains", "contains"],
  ["starts", "starts with"],
  ["ends", "ends with"],
  ["blank", "is blank"],
  ["not_blank", "is not blank"],
  ["gt", "is more than"],
  ["ge", "is at least"],
  ["lt", "is less than"],
  ["le", "is at most"],
];

export interface Rule {
  col: number;
  op: RuleOp;
  value: string;
  /** What the new cell gets when the rule holds (a template). */
  result: string;
}

const isNum = (v: string) => /^[+-]?(\d+\.?\d*|\.\d+)$/.test(v.trim());

/** One rule's test as an expression. */
export function ruleTest(r: Rule, titles: string[]): string {
  const c = columnRef(titles[r.col] ?? "", r.col);
  const v = r.value;
  const lit = isNum(v) ? v.trim() : quote(v);
  switch (r.op) {
    case "is":
      return `${c} = ${lit}`;
    case "is_not":
      return `${c} <> ${lit}`;
    case "contains":
      return `ISNUMBER(SEARCH(${quote(v)}, ${c}))`;
    case "starts":
      return `LOWER(LEFT(${c}, ${[...v].length})) = LOWER(${quote(v)})`;
    case "ends":
      return `LOWER(RIGHT(${c}, ${[...v].length})) = LOWER(${quote(v)})`;
    case "blank":
      return `ISBLANK(${c})`;
    case "not_blank":
      return `NOT(ISBLANK(${c}))`;
    case "gt":
      return `${c} > ${lit}`;
    case "ge":
      return `${c} >= ${lit}`;
    case "lt":
      return `${c} < ${lit}`;
    case "le":
      return `${c} <= ${lit}`;
  }
}

/** If-then rules (first that holds wins) and an "otherwise" as one IFS. */
export function rulesExpr(rules: Rule[], otherwise: string, titles: string[]): { expr: string; unknown: string[] } {
  const unknown: string[] = [];
  const res = (t: string) => {
    const r = templateExpr(t, titles);
    unknown.push(...r.unknown);
    return r.expr;
  };
  const pairs = rules.map((r) => `${ruleTest(r, titles)}, ${res(r.result)}`);
  pairs.push(`TRUE, ${res(otherwise)}`);
  return { expr: `IFS(${pairs.join(", ")})`, unknown };
}

/**
 * A pasted translation list: one pair per line, old and new separated by a
 * tab (as copied from a spreadsheet), "=" or "→", or a comma.
 */
export function parsePairs(text: string): [string, string][] {
  const out: [string, string][] = [];
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue;
    const m = /^(.*?)(?:\t|\s*(?:=>|→|=)\s*)(.*)$/.exec(line) ?? /^([^,]*),(.*)$/.exec(line);
    if (m) out.push([m[1]!.trim(), m[2]!.trim()]);
  }
  return out;
}

/** The pairs a value editor's edits make (only real changes). */
export function editPairs(edits: Map<string, string>): [string, string][] {
  return [...edits].filter(([a, b]) => a !== b);
}

export const GROUP_FUNCS: [string, string][] = [
  ["count", "Rows in the group (for example household size)"],
  ["running", "Running number within the group (1, 2, 3 …)"],
  ["sum", "Total of the group"],
  ["average", "Average of the group"],
  ["min", "Smallest in the group"],
  ["max", "Largest in the group"],
  ["first", "First value in the group"],
  ["last", "Last value in the group"],
  ["rank", "Rank within the group (largest is 1)"],
  ["rank_up", "Rank within the group (smallest is 1)"],
  ["percent", "Share of the group's total (%)"],
];

export const EXTRACTS: [string, string][] = [
  ["number", "The first number"],
  ["numbers", "Every number"],
  ["email", "An e-mail address"],
  ["phone", "A phone number"],
  ["zip", "A ZIP code"],
  ["unit", "A unit, suite or apartment number"],
  ["brackets", "Text in brackets"],
  ["first_word", "The first word"],
  ["last_word", "The last word"],
];

export const SPLIT_MODES: [string, string][] = [
  ["text", "At each separator"],
  ["last", "At the last separator only"],
  ["any", "At any of these characters"],
  ["positions", "At fixed character positions"],
  ["letters_digits", "Where letters meet digits"],
];

export const KEEP_COPIES: [string, string][] = [
  ["first", "The first copy"],
  ["last", "The last copy"],
  ["newest", "The newest (by a date column)"],
  ["oldest", "The oldest (by a date column)"],
  ["most_filled", "The one with the most cells filled in"],
];

/** "3, 7,12" → [3, 7, 12] (positions from 1). */
export function parsePositions(s: string): number[] {
  return [...new Set(s.split(/[,\s]+/).map(Number).filter((n) => Number.isInteger(n) && n > 0))].sort((a, b) => a - b);
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** The date a cell holds as yyyy-mm-dd (2026-03-14, 03/14/2026 month first,
 *  14.03.2026 day first, 2026/03/14), or today's for an empty cell; null
 *  when it holds something else. */
export function isoOf(text: string, now = new Date()): string | null {
  const t = text.trim();
  if (!t) return `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`;
  let y: number, m: number, d: number;
  let x = /^(\d{4})[-/](\d{1,2})[-/](\d{1,2})/.exec(t);
  if (x) [y, m, d] = [Number(x[1]), Number(x[2]), Number(x[3])];
  else if ((x = /^(\d{1,2})\.(\d{1,2})\.(\d{4})$/.exec(t))) [d, m, y] = [Number(x[1]), Number(x[2]), Number(x[3])];
  else if ((x = /^(\d{1,2})[/-](\d{1,2})[/-](\d{4})$/.exec(t))) [m, d, y] = [Number(x[1]), Number(x[2]), Number(x[3])];
  else return null;
  const dt = new Date(Date.UTC(y, m - 1, d));
  if (dt.getUTCFullYear() !== y || dt.getUTCMonth() !== m - 1 || dt.getUTCDate() !== d) return null;
  return `${String(y).padStart(4, "0")}-${pad2(m)}-${pad2(d)}`;
}

/** `iso` (yyyy-mm-dd) written in the style of `old`, the cell's text now. */
export function dateLike(old: string, iso: string): string {
  const [y, m, d] = iso.split("-") as [string, string, string];
  const t = old.trim();
  const p = (s: string, two: boolean) => (two ? s : String(Number(s)));
  let x: RegExpExecArray | null;
  if ((x = /^(\d{1,2})([/-])(\d{1,2})\2\d{4}$/.exec(t))) return `${p(m, x[1]!.length === 2)}${x[2]}${p(d, x[3]!.length === 2)}${x[2]}${y}`;
  if ((x = /^(\d{1,2})\.(\d{1,2})\.\d{4}$/.exec(t))) return `${p(d, x[1]!.length === 2)}.${p(m, x[2]!.length === 2)}.${y}`;
  if (/^\d{4}\/\d{1,2}\/\d{1,2}$/.test(t)) return `${y}/${m}/${d}`;
  return iso;
}
