// Display format codes: the grid's copy of crates/engine/src/numfmt.rs (a
// closed subset of Excel's number format codes, and the time and
// date-and-time formats). Both run crates/engine/tests/data/numfmt-cases.json,
// so what the screen shows and what "Write the display format into the cells"
// writes are the same. Keep the two in step.

export const MAX_CODE = 200;
const MAX_SECTIONS = 4;
export const MAX_FRAC = 30;
export const MAX_INT = 30;
export const MAX_SCALE = 4;
export const MAX_PCT = 2;
export const MAX_CONDS = 2;
export const MAX_DIGITS = 15;

export type CodeColor = "red" | "blue" | "green" | "black" | "white" | "yellow" | "magenta" | "cyan";
const COLORS: CodeColor[] = ["red", "blue", "green", "black", "white", "yellow", "magenta", "cyan"];

export class CodeError extends Error {
  constructor(
    readonly at: number | null,
    readonly reason: string,
  ) {
    super(at === null ? `Not supported: ${reason}.` : `Not supported at character ${at}: ${reason}.`);
  }
}

type Op = "<" | "<=" | ">" | ">=" | "=" | "<>";
type Tok =
  | { t: "lit"; s: string }
  | { t: "slot"; c: "0" | "#" | "?" }
  | { t: "point" }
  | { t: "comma" }
  | { t: "group" }
  | { t: "scale" }
  | { t: "pct" }
  | { t: "text" }
  | { t: "year"; four: boolean }
  | { t: "month"; n: number }
  | { t: "minute"; n: number }
  | { t: "day"; n: number }
  | { t: "hour"; n: number }
  | { t: "second"; n: number }
  | { t: "ampm"; up: boolean }
  | { t: "ap"; up: boolean };

const isDate = (k: Tok): boolean => ["year", "month", "minute", "day", "hour", "second", "ampm", "ap"].includes(k.t);
const isNumber = (k: Tok): boolean => k.t === "slot" || k.t === "pct";
const isSlot = (k: Tok): boolean => k.t === "slot";

type Kind = "number" | "date" | "text" | "plain";
interface Section {
  toks: Tok[];
  color: CodeColor | null;
  cond: [Op, number] | null;
  kind: Kind;
  point: number | null;
  grouping: boolean;
  shift: number;
  frac: number;
  twelve: boolean;
  needsDate: boolean;
  needsTime: boolean;
  /** Number sections: the integer and decimal slot indexes, and each slot's character. */
  intSlots: number[];
  fracSlots: number[];
  slotChar: string[];
}

/** How a cell shows: its text and, for a custom code, a colour. */
export interface Shown {
  text: string;
  color: CodeColor | null;
}

function newSection(): Section {
  return { toks: [], color: null, cond: null, kind: "plain", point: null, grouping: false, shift: 0, frac: 0, twelve: false, needsDate: false, needsTime: false, intSlots: [], fracSlots: [], slotChar: [] };
}

function sigDigits(s: string): number {
  const d = s.replace(/[^0-9]/g, "").replace(/^0+/, "");
  return s.includes(".") ? d.replace(/0+$/, "").length : d.length;
}

function condNumber(s: string): number | null {
  const body = s.startsWith("-") ? s.slice(1) : s;
  let point = false;
  let digits = 0;
  for (const c of body) {
    if (c === "." && !point) point = true;
    else if (c >= "0" && c <= "9") digits++;
    else return null;
  }
  if (digits === 0 || sigDigits(body) > MAX_DIGITS) return null;
  const v = Number(s);
  return Number.isFinite(v) ? v : null;
}

function bracket(inner: string, at: number, sec: Section, conds: { n: number }): void {
  const l = inner.trim().toLowerCase();
  if (l.startsWith("$")) throw new CodeError(at, "locale and currency tags ([$...])");
  if (["h", "hh", "m", "mm", "s", "ss"].includes(l)) throw new CodeError(at, "elapsed time ([h], [m], [s])");
  if ((COLORS as string[]).includes(l)) {
    if (sec.color) throw new CodeError(at, "more than one colour in a section");
    sec.color = l as CodeColor;
    return;
  }
  const op = (["<=", ">=", "<>", "<", ">", "="] as Op[]).find((p) => l.startsWith(p));
  if (op) {
    if (sec.cond) throw new CodeError(at, "more than one condition in a section");
    conds.n++;
    if (conds.n > MAX_CONDS) throw new CodeError(at, "more than 2 conditions");
    const v = condNumber(l.slice(op.length).trim());
    if (v === null) throw new CodeError(at, "a condition number (at most 15 digits, like [>100] or [<=-2.5])");
    sec.cond = [op, v];
    return;
  }
  throw new CodeError(at, "an unknown [...] part");
}

const LITERALS = new Set(["$", "-", "+", "(", ")", ":", "/", "'", " ", "€", "£", "¥", "¢"]);

/** A compiled code. */
export class Code {
  private constructor(
    private readonly sections: Section[],
    private readonly conds: boolean,
  ) {}

  /** Compile a code, or throw a CodeError saying what is not supported. */
  static compile(code: string): Code {
    const s = Array.from(code);
    if (!code.trim()) throw new CodeError(null, "the code is empty");
    if (s.length > MAX_CODE) throw new CodeError(null, "the code is longer than 200 characters");
    const sections: Section[] = [newSection()];
    const pos: number[][] = [[]];
    const conds = { n: 0 };
    let i = 0;
    while (i < s.length) {
      const c = s[i]!;
      const at = i + 1;
      const sec = sections[sections.length - 1]!;
      const p = pos[pos.length - 1]!;
      const started = sec.toks.length > 0;
      const push = (t: Tok) => {
        sec.toks.push(t);
        p.push(at);
      };
      if (c === ";") {
        if (sections.length === MAX_SECTIONS) throw new CodeError(at, "more than 4 sections");
        sections.push(newSection());
        pos.push([]);
        i++;
        continue;
      }
      if (c === '"') {
        const j = s.indexOf('"', i + 1);
        if (j < 0) throw new CodeError(at, "an unclosed quote");
        push({ t: "lit", s: s.slice(i + 1, j).join("") });
        i = j + 1;
        continue;
      }
      if (c === "\\") {
        if (i + 1 >= s.length) throw new CodeError(at, "a backslash at the end");
        push({ t: "lit", s: s[i + 1]! });
        i += 2;
        continue;
      }
      if (c === "[") {
        const j = s.indexOf("]", i + 1);
        if (j < 0) throw new CodeError(at, "an unclosed [");
        if (started) throw new CodeError(at, "a colour or condition after the start of a section");
        bracket(s.slice(i + 1, j).join(""), at, sec, conds);
        i = j + 1;
        continue;
      }
      if (c === "0" || c === "#" || c === "?") {
        if (s[i + 1] === "/") throw new CodeError(at, "fractions (?/?)");
        push({ t: "slot", c });
      } else if (c === ".") push({ t: "point" });
      else if (c === ",") push({ t: "comma" });
      else if (c === "%") push({ t: "pct" });
      else if (c === "@") push({ t: "text" });
      else if (c === "*") throw new CodeError(at, "fill (*)");
      else if (c === "_") throw new CodeError(at, "spacing (_)");
      else if ((c === "E" || c === "e") && (s[i + 1] === "+" || s[i + 1] === "-")) throw new CodeError(at, "scientific notation (E+)");
      else if (c === "a" || c === "A") {
        const rest = s
          .slice(i, Math.min(s.length, i + 5))
          .join("")
          .toLowerCase();
        if (rest === "am/pm") {
          push({ t: "ampm", up: c === "A" });
          i += 5;
          continue;
        }
        if (rest.startsWith("a/p")) {
          push({ t: "ap", up: c === "A" });
          i += 3;
          continue;
        }
        throw new CodeError(at, `the character '${c}'`);
      } else if ("ymdhsYMDHS".includes(c)) {
        const l = c.toLowerCase();
        let n = 0;
        while (i + n < s.length && s[i + n]!.toLowerCase() === l) n++;
        const max = l === "y" || l === "d" ? 4 : l === "m" ? 5 : 2;
        if (n > max) throw new CodeError(at, `a run of ${n} '${l}'`);
        push(
          l === "y"
            ? { t: "year", four: n >= 3 }
            : l === "m"
              ? { t: "month", n }
              : l === "d"
                ? { t: "day", n }
                : l === "h"
                  ? { t: "hour", n }
                  : { t: "second", n },
        );
        i += n;
        continue;
      } else if (LITERALS.has(c)) push({ t: "lit", s: c });
      else throw new CodeError(at, `the character '${c}'`);
      i++;
    }
    sections.forEach((sec, k) => finish(sec, pos[k]!, k, sections.length));
    return new Code(
      sections,
      sections.some((x) => x.cond !== null),
    );
  }

  /** The code has one section, of dates and times only. */
  isDateOnly(): boolean {
    return this.sections.length === 1 && this.sections[0]!.kind === "date";
  }

  /** How `text` shows under this code: null when it shows as it is. */
  render(text: string): Shown | null {
    if (isBlank(text)) return null;
    const secs = this.sections;
    if (secs.length === 1 && secs[0]!.kind === "text") return textSection(secs[0]!, text);
    const d = readDecimal(text);
    if (d) {
      const picked = this.pick(d);
      if (!picked) return null;
      const [sec, minus] = picked;
      if (sec.kind !== "number" && sec.kind !== "plain") return null;
      const out = numberSection(sec, d, minus);
      return out === null ? null : { text: out, color: sec.color };
    }
    if (secs[0]!.kind === "date") {
      const st = readStamp(text);
      if (st) {
        const out = dateSection(secs[0]!, st);
        return out === null ? null : { text: out, color: secs[0]!.color };
      }
    }
    if (secs.length === 4) return textSection(secs[3]!, text);
    return null;
  }

  private pick(d: Dec): [Section, boolean] | null {
    const secs = this.sections;
    const nums = secs.length === 4 ? 3 : secs.length;
    if (this.conds) {
      const v = decValue(d);
      for (const s of secs.slice(0, nums)) {
        if (!s.cond) continue;
        const [op, c] = s.cond;
        const hit = op === "<" ? v < c : op === "<=" ? v <= c : op === ">" ? v > c : op === ">=" ? v >= c : op === "=" ? v === c : v !== c;
        if (hit) return [s, true];
      }
      const other = secs.slice(0, nums).find((s) => !s.cond);
      return other ? [other, true] : null;
    }
    const zero = d.digits.length === 0;
    if (nums === 1) return [secs[0]!, true];
    if (nums === 2) return [d.neg && !zero ? secs[1]! : secs[0]!, false];
    return [zero ? secs[2]! : d.neg ? secs[1]! : secs[0]!, false];
  }
}

function lit(t: Tok): Tok {
  if (t.t === "point") return { t: "lit", s: "." };
  if (t.t === "comma") return { t: "lit", s: "," };
  return t;
}

function finish(sec: Section, p: number[], k: number, n: number): void {
  const first = (f: (t: Tok) => boolean): number | null => {
    const i = sec.toks.findIndex(f);
    return i < 0 ? null : p[i]!;
  };
  const hasNum = sec.toks.some(isNumber);
  const hasDate = sec.toks.some(isDate);
  const textAt = first((t) => t.t === "text");
  const textSec = k === 3 || n === 1;
  if (textAt !== null && (!textSec || hasNum || hasDate)) throw new CodeError(textAt, "text (@) outside the text section");
  if (k === 3) {
    const bad = first((t) => isNumber(t) || isDate(t));
    if (bad !== null) throw new CodeError(bad, "number or date parts in the text section");
    if (sec.cond) throw new CodeError(null, "a condition in the text section");
  }
  if (hasNum && hasDate) throw new CodeError(first(isDate) ?? 0, "number and date parts in one section");
  if (textAt !== null) {
    sec.kind = "text";
    sec.toks = sec.toks.map(lit);
    return;
  }
  if (hasDate) {
    sec.kind = "date";
    dateMinutes(sec);
    sec.twelve = sec.toks.some((t) => t.t === "ampm" || t.t === "ap");
    sec.needsDate = sec.toks.some((t) => t.t === "year" || t.t === "month" || t.t === "day");
    sec.needsTime = sec.toks.some((t) => ["hour", "minute", "second", "ampm", "ap"].includes(t.t));
    sec.toks = sec.toks.map(lit);
    return;
  }
  if (!sec.toks.some(isSlot)) {
    sec.kind = "plain";
    sec.toks = sec.toks.map((t) => (t.t === "pct" ? { t: "lit", s: "%" } : lit(t)));
    return;
  }
  sec.kind = "number";
  let point: number | null = null;
  sec.toks.forEach((t, i) => {
    if (t.t !== "point") return;
    if (point !== null) throw new CodeError(p[i]!, "two decimal points");
    point = i;
  });
  sec.point = point;
  const endInt = point ?? sec.toks.length;
  const toks = sec.toks.slice();
  let scale = 0;
  for (let i = 0; i < toks.length; i++) {
    if (toks[i]!.t !== "comma") continue;
    const [lo, hi] = i < endInt ? [0, endInt] : [endInt + 1, toks.length];
    const before = toks.slice(lo, i).some(isSlot);
    const after = toks.slice(i + 1, hi).some(isSlot);
    let j = i;
    while (j > lo && toks[j - 1]!.t === "comma") j--;
    const runFromSlot = j > lo && isSlot(toks[j - 1]!);
    if (i < endInt && before && after) {
      sec.grouping = true;
      sec.toks[i] = { t: "group" };
    } else if (before && !after && runFromSlot) {
      scale++;
      if (scale > MAX_SCALE) throw new CodeError(p[i]!, "more than 4 commas for thousands");
      sec.toks[i] = { t: "scale" };
    } else sec.toks[i] = { t: "lit", s: "," };
  }
  const intSlots: number[] = [];
  const fracSlots: number[] = [];
  sec.toks.forEach((t, i) => {
    if (isSlot(t)) (i < endInt ? intSlots : fracSlots).push(i);
  });
  if (intSlots.length > MAX_INT) throw new CodeError(p[intSlots[MAX_INT]!]!, "more than 30 digits before the point");
  if (fracSlots.length > MAX_FRAC) throw new CodeError(p[fracSlots[MAX_FRAC]!]!, "more than 30 digits after the point");
  const pcts: number[] = [];
  sec.toks.forEach((t, i) => {
    if (t.t === "pct") pcts.push(i);
  });
  if (pcts.length > MAX_PCT) throw new CodeError(p[pcts[MAX_PCT]!]!, "more than 2 percent signs");
  if (sec.grouping && intSlots.length) {
    const [a, b] = [intSlots[0]!, intSlots[intSlots.length - 1]!];
    for (let i = a; i < b; i++) {
      const t = sec.toks[i]!;
      if (!isSlot(t) && t.t !== "group") throw new CodeError(p[i]!, "thousands commas with text between the digits");
    }
  }
  sec.frac = fracSlots.length;
  sec.shift = 2 * pcts.length - 3 * scale;
  sec.intSlots = intSlots;
  sec.fracSlots = fracSlots;
  sec.slotChar = sec.toks.map((t) => (t.t === "slot" ? t.c : ""));
}

function dateMinutes(sec: Section): void {
  const dates: number[] = [];
  sec.toks.forEach((t, i) => {
    if (isDate(t)) dates.push(i);
  });
  dates.forEach((i, k) => {
    const t = sec.toks[i]!;
    if (t.t !== "month" || t.n > 2) return;
    const prevHour = k > 0 && sec.toks[dates[k - 1]!]!.t === "hour";
    const nextSec = k + 1 < dates.length && sec.toks[dates[k + 1]!]!.t === "second";
    if (prevHour || nextSec) sec.toks[i] = { t: "minute", n: t.n };
  });
}

function textSection(sec: Section, text: string): Shown {
  let out = "";
  for (const t of sec.toks) {
    if (t.t === "lit") out += t.s;
    else if (t.t === "text") out += text;
  }
  return { text: out, color: sec.color };
}

/** A number written in a cell, exactly: value = 0.DIGITS × 10^exp. */
export interface Dec {
  neg: boolean;
  /** No leading or trailing zeros; empty for zero. */
  digits: number[];
  exp: number;
  /** The number as JavaScript reads it (for conditions), worked out when asked. */
  src: string;
  pct: boolean;
  value?: number;
}

/** The value of a number read from a cell, as a double (for conditions). */
export function decValue(d: Dec): number {
  if (d.value === undefined) {
    const v = d.digits.length ? Number(d.src) * (d.pct ? 0.01 : 1) : 0;
    d.value = d.neg ? -v : v;
  }
  return d.value;
}

/** JavaScript's white space (trim, \s). */
function wsCode(c: number): boolean {
  return (
    c === 32 || (c >= 9 && c <= 13) || c === 0xa0 || c === 0x1680 || (c >= 0x2000 && c <= 0x200a) || c === 0x2028 || c === 0x2029 || c === 0x202f || c === 0x205f || c === 0x3000 || c === 0xfeff
  );
}
function isBlank(t: string): boolean {
  for (let i = 0; i < t.length; i++) if (!wsCode(t.charCodeAt(i))) return false;
  return true;
}
function jsTrim(s: string): string {
  return s.trim();
}
const digitCode = (c: number) => c >= 48 && c <= 57;

/** The numbers display formats read (as readNumber in m9.ts), exactly: plain,
 *  with thousands commas, a currency sign, a percent sign (a fraction), or in
 *  brackets for minus; at most 15 significant digits. */
export function readDecimal(text: string): Dec | null {
  let s = 0;
  let e = text.length;
  while (s < e && wsCode(text.charCodeAt(s))) s++;
  while (e > s && wsCode(text.charCodeAt(e - 1))) e--;
  if (s === e || e - s > 64) return null;
  // Significant digits of the mantissa (the trimmed text before any e).
  {
    let n = 0;
    let lead = true;
    let point = false;
    let zeros = 0;
    for (let i = s; i < e; i++) {
      const c = text.charCodeAt(i);
      if (c === 101 || c === 69) break;
      if (c === 46) point = true;
      if (!digitCode(c)) continue;
      if (lead && c === 48) continue;
      lead = false;
      n++;
      zeros = c === 48 ? zeros + 1 : 0;
    }
    if ((point ? n - zeros : n) > MAX_DIGITS) return null;
  }
  let neg = false;
  if (e - s >= 2 && text.charCodeAt(s) === 40 && text.charCodeAt(e - 1) === 41) {
    neg = true;
    s++;
    e--;
    while (s < e && wsCode(text.charCodeAt(s))) s++;
    while (e > s && wsCode(text.charCodeAt(e - 1))) e--;
  }
  let pct = false;
  if (e > s && text.charCodeAt(e - 1) === 37) {
    pct = true;
    e--;
    while (s < e && wsCode(text.charCodeAt(s))) s++;
    while (e > s && wsCode(text.charCodeAt(e - 1))) e--;
  }
  const c0 = text.charCodeAt(s);
  if (c0 === 45) {
    neg = !neg;
    s++;
  } else if (c0 === 43) s++;
  const c1 = text.charCodeAt(s);
  if (s < e && (c1 === 36 || c1 === 0x20ac || c1 === 0xa3 || c1 === 0xa5)) {
    s++;
    if (s < e && wsCode(text.charCodeAt(s))) s++;
  }
  // The integer part: digits, and commas only in groups of three (then no exponent).
  const digits: number[] = [];
  let i = s;
  let commas = 0;
  let group = 0;
  let first = 0;
  while (i < e) {
    const c = text.charCodeAt(i);
    if (digitCode(c)) {
      digits.push(c - 48);
      group++;
    } else if (c === 44) {
      if (commas === 0) {
        if (group < 1 || group > 3) return null;
        first = group;
      } else if (group !== 3) return null;
      commas++;
      group = 0;
    } else break;
    i++;
  }
  if (commas && group !== 3) return null;
  void first;
  const intLen = digits.length;
  let hasPoint = false;
  if (i < e && text.charCodeAt(i) === 46) {
    hasPoint = true;
    i++;
    while (i < e && digitCode(text.charCodeAt(i))) digits.push(text.charCodeAt(i++) - 48);
  }
  if (!digits.length) return null;
  if (!intLen && !hasPoint) return null;
  let ex = 0;
  let hasEx = false;
  if (i < e && (text.charCodeAt(i) === 101 || text.charCodeAt(i) === 69)) {
    if (commas) return null;
    hasEx = true;
    i++;
    let sign = 1;
    const sc = text.charCodeAt(i);
    if (sc === 45 || sc === 43) {
      if (sc === 45) sign = -1;
      i++;
    }
    const at = i;
    while (i < e && digitCode(text.charCodeAt(i))) i++;
    if (i === at) return null;
    let k = at;
    while (k < i - 1 && text.charCodeAt(k) === 48) k++;
    // A long exponent: the value is 0 or not finite; Number says which.
    ex = i - k > 6 ? sign * 1e7 : sign * Number(text.slice(k, i));
  }
  if (i !== e) return null;
  const src = commas ? text.slice(s, e).replace(/,/g, "") : text.slice(s, e);
  let exp = intLen;
  let lead = 0;
  while (lead < digits.length && digits[lead] === 0) lead++;
  if (lead === digits.length) return { neg, digits: [], exp: 0, src, pct, value: 0 };
  if (hasEx) {
    const v = Number(src);
    if (!Number.isFinite(v)) return null;
    if (v === 0) return { neg, digits: [], exp: 0, src, pct, value: 0 };
  }
  if (lead) digits.splice(0, lead);
  exp -= lead;
  let end = digits.length;
  while (end > 0 && digits[end - 1] === 0) end--;
  digits.length = end;
  exp += ex;
  if (pct) exp -= 2;
  return { neg, digits, exp, src, pct };
}

/** The digits of `d` moved by `shift` and rounded half away from zero to
 *  `frac` places: integer digits and exactly `frac` decimals; null at 1e21 or more. */
export function roundDigits(d: Dec, shift: number, frac: number): [string, string] | null {
  let digits = d.digits;
  let exp = d.exp + shift;
  if (digits.length) {
    const keep = exp + frac;
    if (keep < 0) digits = [];
    else if (keep < digits.length) {
      digits = digits.slice(0, keep + 1);
      const up = digits[keep]! >= 5;
      digits.length = keep;
      if (up) {
        let i = keep;
        for (;;) {
          if (i === 0) {
            digits.unshift(1);
            exp++;
            break;
          }
          i--;
          if (digits[i] === 9) digits[i] = 0;
          else {
            digits[i]!++;
            break;
          }
        }
      }
      let end = digits.length;
      while (end > 0 && digits[end - 1] === 0) end--;
      digits.length = end;
    }
    if (!digits.length) exp = 0;
  }
  if (digits.length && exp >= 22) return null;
  let int = "";
  if (digits.length && exp > 0) for (let i = 0; i < exp; i++) int += i < digits.length ? String.fromCharCode(48 + digits[i]!) : "0";
  let fr = "";
  for (let i = 0; i < frac; i++) {
    const k = exp + i;
    fr += k >= 0 && k < digits.length ? String.fromCharCode(48 + digits[k]!) : "0";
  }
  return [int, fr];
}

function numberSection(sec: Section, d: Dec, minus: boolean): string | null {
  if (sec.kind === "plain") {
    let out = "";
    for (const t of sec.toks) if (t.t === "lit") out += t.s;
    return out;
  }
  const r = roundDigits(d, sec.shift, sec.frac);
  if (!r) return null;
  const int = r[0];
  const frac = r[1];
  let zero = int.length === 0;
  if (zero) for (let k = 0; k < frac.length; k++) if (frac.charCodeAt(k) !== 48) zero = false;
  const intSlots = sec.intSlots;
  const fracSlots = sec.fracSlots;
  const ch = sec.slotChar;
  const pieces: string[] = new Array(sec.toks.length);
  let left = int.length;
  for (let n = intSlots.length - 1; n >= 0; n--) {
    const i = intSlots[n]!;
    let piece = "";
    if (left > 0) {
      left--;
      piece = int[left]!;
    } else if (ch[i] === "0") piece = "0";
    else if (ch[i] === "?") piece = " ";
    if (n === 0 && left > 0) piece = int.slice(0, left) + piece;
    pieces[i] = piece;
  }
  if (sec.grouping && intSlots.length) {
    let joined = "";
    for (const i of intSlots) joined += pieces[i];
    let spaces = 0;
    while (spaces < joined.length && joined.charCodeAt(spaces) === 32) spaces++;
    const len = joined.length - spaces;
    let g = joined.slice(0, spaces);
    for (let k = 0; k < len; k++) {
      if (k > 0 && (len - k) % 3 === 0) g += ",";
      g += joined[spaces + k];
    }
    for (const i of intSlots) pieces[i] = "";
    pieces[intSlots[0]!] = g;
  }
  let trim = true;
  for (let n = fracSlots.length - 1; n >= 0; n--) {
    const i = fracSlots[n]!;
    const c = frac[n]!;
    if (trim && c === "0" && ch[i] !== "0") pieces[i] = ch[i] === "?" ? " " : "";
    else {
      trim = false;
      pieces[i] = c;
    }
  }
  let out = minus && d.neg && !zero ? "-" : "";
  const toks = sec.toks;
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i]!;
    if (t.t === "slot") out += pieces[i];
    else if (t.t === "lit") out += t.s;
    else if (t.t === "point") out += ".";
    else if (t.t === "pct") out += "%";
  }
  return out;
}

/** A date, a time, or both, as written (a time zone is dropped). */
export interface Stamp {
  date: [number, number, number] | null;
  time: [number, number, number] | null;
}

function daysInMonth(y: number, m: number): number {
  if ([1, 3, 5, 7, 8, 10, 12].includes(m)) return 31;
  if ([4, 6, 9, 11].includes(m)) return 30;
  return y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0) ? 29 : 28;
}

const DATE_A = /^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})(?!\d)/;
const DATE_B = /^(\d{1,2})[-/.](\d{1,2})[-/.](\d{4})(?!\d)/;

function datePart(t: string): [[number, number, number], number] | null {
  let y: number, m: number, d: number;
  let x = DATE_A.exec(t);
  if (x) [y, m, d] = [Number(x[1]), Number(x[2]), Number(x[3])];
  else if ((x = DATE_B.exec(t))) [m, d, y] = [Number(x[1]), Number(x[2]), Number(x[3])];
  else return null;
  if (m < 1 || m > 12 || d < 1 || d > daysInMonth(y, m)) return null;
  return [[y, m, d], x[0].length];
}

const TIME = /^(\d{1,2}):(\d{2})(?::(\d{2})(?:[.,]\d+)?)?(?: ?(am|a\.m\.|am\.|a\.m|pm|p\.m\.|pm\.|p\.m))?$/i;

function timePart(t: string): [number, number, number] | null {
  const x = TIME.exec(t);
  if (!x) return null;
  const h = Number(x[1]);
  const mi = Number(x[2]);
  const s = x[3] ? Number(x[3]) : 0;
  let hour = h;
  if (x[4]) {
    if (h < 1 || h > 12) return null;
    hour = (h % 12) + (x[4].toLowerCase().startsWith("p") ? 12 : 0);
  }
  if (hour > 23 || mi > 59 || s > 59) return null;
  return [hour, mi, s];
}

function stripZone(t: string): string {
  if (t.endsWith("Z") || t.endsWith("z")) return t.slice(0, -1);
  const z = /[+-]\d{2}:?\d{2}$/.exec(t);
  return z ? t.slice(0, -z[0].length) : t;
}

/** A cell's date, time, or date and time; null for anything else. */
export function readStamp(text: string): Stamp | null {
  const t = jsTrim(text);
  if (!t || t.length > 64) return null;
  const tm = timePart(t);
  if (tm) return { date: null, time: tm };
  const dp = datePart(t);
  if (!dp) return null;
  const [date, end] = dp;
  const rest = t.slice(end);
  if (!rest) return { date, time: null };
  if (rest[0] !== "T" && rest[0] !== " ") return null;
  const time = timePart(stripZone(rest.slice(1)));
  return time ? { date, time } : null;
}

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function weekday(y0: number, m: number, d: number): number {
  const y = m <= 2 ? y0 - 1 : y0;
  const era = Math.floor(y / 400);
  const yoe = y - era * 400;
  const doy = Math.floor((153 * (m > 2 ? m - 3 : m + 9) + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  const days = era * 146097 + doe - 719468;
  return (((days + 4) % 7) + 7) % 7;
}

function dateSection(sec: Section, st: Stamp): string | null {
  if ((sec.needsDate && !st.date) || (sec.needsTime && !st.time)) return null;
  const [y, mo, d] = st.date ?? [0, 1, 1];
  const [h, mi, s] = st.time ?? [0, 0, 0];
  const p2 = (n: number) => String(n).padStart(2, "0");
  let out = "";
  for (const t of sec.toks) {
    switch (t.t) {
      case "lit":
        out += t.s;
        break;
      case "year":
        out += t.four ? String(y).padStart(4, "0") : p2(((y % 100) + 100) % 100);
        break;
      case "month": {
        const name = MONTHS[mo - 1]!;
        out += t.n === 1 ? String(mo) : t.n === 2 ? p2(mo) : t.n === 3 ? name.slice(0, 3) : t.n === 4 ? name : name[0];
        break;
      }
      case "day": {
        const name = DAYS[weekday(y, mo, d)]!;
        out += t.n === 1 ? String(d) : t.n === 2 ? p2(d) : t.n === 3 ? name.slice(0, 3) : name;
        break;
      }
      case "hour": {
        const hv = sec.twelve ? (h % 12 === 0 ? 12 : h % 12) : h;
        out += t.n === 1 ? String(hv) : p2(hv);
        break;
      }
      case "minute":
        out += t.n === 1 ? String(mi) : p2(mi);
        break;
      case "second":
        out += t.n === 1 ? String(s) : p2(s);
        break;
      case "ampm":
        out += h >= 12 ? (t.up ? "PM" : "pm") : t.up ? "AM" : "am";
        break;
      case "ap":
        out += h >= 12 ? (t.up ? "P" : "p") : t.up ? "A" : "a";
        break;
    }
  }
  return out;
}

export const DATE_STYLE_CODES: Record<string, string> = {
  iso: "yyyy-mm-dd",
  mdy: "mm/dd/yyyy",
  dmy: "dd/mm/yyyy",
  medium: "mmm d, yyyy",
  long: "dddd, mmmm d, yyyy",
};
export const TIME_STYLE_CODES: Record<string, string> = {
  hm12: "h:mm AM/PM",
  hm24: "hh:mm",
  hms12: "h:mm:ss AM/PM",
  hms24: "hh:mm:ss",
};

/** A display format ready to show cells: a custom code, or a time (with a
 *  date code for date and time). Same fields as the engine's Step::Format. */
export class Display {
  private constructor(
    private readonly custom: Code | null,
    private readonly date: Code | null,
    private readonly time: Code | null,
  ) {}

  static make(code: string, date: string, time: string): Display {
    if (code) {
      if (date || time) throw new CodeError(null, "a custom code with a date or time style");
      return new Display(Code.compile(code), null, null);
    }
    const one = (c: string) => {
      const k = Code.compile(c);
      if (!k.isDateOnly()) throw new CodeError(null, "a date or time style that is not one date section");
      return k;
    };
    if (!time) throw new CodeError(null, "the code is empty");
    return new Display(null, date ? one(date) : null, one(time));
  }

  render(text: string): Shown | null {
    if (this.custom) return this.custom.render(text);
    const st = readStamp(text);
    if (!st) return null;
    const time = this.time!;
    const t = (time as unknown as { sections: Section[] }).sections[0]!;
    const dc = this.date ? (this.date as unknown as { sections: Section[] }).sections[0]! : null;
    let out: string | null;
    if (!dc) out = st.time ? dateSection(t, st) : null;
    else if (st.date && st.time) {
      const a = dateSection(dc, st);
      const b = dateSection(t, st);
      out = a === null || b === null ? null : `${a} ${b}`;
    } else if (st.date) out = dateSection(dc, st);
    else if (st.time) out = dateSection(t, st);
    else out = null;
    return out === null ? null : { text: out, color: null };
  }
}

/** Compiled displays by their fields' text (never by object, so a format
 *  changed in place or the dialog's preview never draws with an old one).
 *  Capped; the oldest is let go. A code that does not compile maps to null. */
const CACHE_MAX = 256;
const cache = new Map<string, Display | null>();
let lastCode = "";
let lastDate = "";
let lastTime = "";
let lastDisplay: Display | null | undefined;
export function displayOf(code: string, date: string, time: string): Display | null {
  if (lastDisplay !== undefined && code === lastCode && date === lastDate && time === lastTime) return lastDisplay;
  const d = displayOfMap(code, date, time);
  [lastCode, lastDate, lastTime, lastDisplay] = [code, date, time, d];
  return d;
}
function displayOfMap(code: string, date: string, time: string): Display | null {
  const key = `${code}\u0000${date}\u0000${time}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  let d: Display | null;
  try {
    d = Display.make(code, date, time);
  } catch {
    d = null;
  }
  if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value!);
  cache.set(key, d);
  return d;
}
export function cacheSize(): number {
  return cache.size;
}

/** The message for a code that cannot be shown, or null when it can. */
export function codeProblem(code: string): string | null {
  try {
    Code.compile(code);
    return null;
  } catch (e) {
    return e instanceof CodeError ? e.message : String(e);
  }
}
