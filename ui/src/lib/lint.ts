// Data rules (schema linting): how a rule is described, a column shape as a rule, and the
// words for a jump to the next problem. Mirrors the engine (`validate.rs` describe, `profile.rs`
// shape_regex); shared fixtures keep the two together.

import type { Check, JumpResult } from "./backend";
import { shapeOf } from "./insights";

const MAX_SHAPE_CHARS = 40;

function range(what: string, min: number | string | null, max: number | string | null): string {
  if (min !== null && max !== null) return `${what} from ${min} to ${max}`;
  if (min !== null) return `${what} of at least ${min}`;
  if (max !== null) return `${what} of at most ${max}`;
  return what;
}

/** What a rule says: its name, else a sentence made from its kind (the engine's `describe`). */
export function describeCheck(c: Check): string {
  const name = c.name?.trim();
  if (name) return name;
  const k = c.kind;
  switch (k.type) {
    case "required":
      return "Required (not blank)";
    case "number": {
      const r = range("Number", k.min, k.max);
      return k.whole ? `Whole ${r.toLowerCase()}` : r;
    }
    case "date":
      return range("Date", k.min, k.max);
    case "date_iso":
      return "Date written YYYY-MM-DD";
    case "length":
      return range("Length", k.min, k.max);
    case "list":
      return `One of ${k.values.length} values`;
    case "pattern":
      return `Matches ${k.pattern.text}`;
    case "unique":
      if (k.also?.length) return `Unique together with ${k.also.length} other column${k.also.length === 1 ? "" : "s"} (no repeats)`;
      return "Unique (no repeats)";
    case "expression":
      return `Row rule: ${k.expr}`;
    case "foreign_key":
      return `Value must exist in ${k.table} > ${k.column}`;
  }
}

/** A real calendar date written YYYY-MM-DD. */
export function isIsoDate(s: string): boolean {
  const m = /^([0-9]{4})-([0-9]{2})-([0-9]{2})$/.exec(s);
  if (!m) return false;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const leap = y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][mo - 1];
  return days !== undefined && d >= 1 && d <= days;
}

function escapeRegex(c: string): string {
  // Rust's regex::escape set: every character with a meaning in a pattern.
  return /[\\.+*?()|[\]{}^$#&\-~]/.test(c) ? `\\${c}` : c;
}

/** A pattern (Rust regex syntax, which the engine compiles) that accepts exactly the values with this
 *  shape: digits are `\p{N}`, letters `\p{Alphabetic}`, other characters as they are, and a shape cut at
 *  40 characters ("…") takes any rest. */
export function shapeRegex(shape: string): string {
  const chars = Array.from(shape);
  const capped = chars.length === MAX_SHAPE_CHARS + 1 && chars[MAX_SHAPE_CHARS] === "…";
  const body = capped ? chars.slice(0, MAX_SHAPE_CHARS) : chars;
  let out = "";
  for (const c of body) out += c === "9" ? "\\p{N}" : c === "A" ? "\\p{Alphabetic}" : escapeRegex(c);
  if (capped) out += "(?s:.+)";
  return out;
}

/** Whether the pill's rule accepts a value (the same as `shapeOf(v) === shape`). */
export function shapeRuleAccepts(shape: string, v: string): boolean {
  return shapeOf(v) === shape;
}

/** The status line for a jump. */
export function jumpText(r: JumpResult, rule: string | null): string {
  if (!r.hit) {
    return r.hidden_skipped > 0 ? `No problems in shown columns (${r.hidden_skipped} in hidden columns)` : "No problems found";
  }
  const parts = [rule ?? "Problem"];
  if (r.wrapped) parts.push("back at the top");
  if (r.hidden_skipped > 0) parts.push(`${r.hidden_skipped} problem${r.hidden_skipped === 1 ? "" : "s"} in hidden columns passed`);
  return parts.join(" · ");
}
