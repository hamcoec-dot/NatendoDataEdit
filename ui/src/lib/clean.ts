// Cleaning tools and cleaners: the shapes the engine reads (see the
// engine's `clean` module), the tool list the screen shows, and helpers.

import type { Pattern, ReadOptions, Scope } from "./backend";

export type At =
  | { kind: "start" }
  | { kind: "end" }
  | { kind: "position"; n: number }
  | { kind: "before"; text: string }
  | { kind: "after"; text: string };
export type CharKind = "letters" | "digits" | "punctuation" | "spaces" | "non_ascii" | "control" | "invisible";
export type Remove =
  | { kind: "first"; n: number }
  | { kind: "last"; n: number }
  | { kind: "range"; from: number; count: number }
  | { kind: "before"; text: string; inclusive: boolean }
  | { kind: "after"; text: string; inclusive: boolean }
  | { kind: "matching"; find: Pattern }
  | { kind: "kind"; of: CharKind }
  | { kind: "characters"; chars: string };
export type SpacesHow = "trim" | "trim_start" | "trim_end" | "collapse" | "remove_all" | "line_breaks" | "letters_digits" | "before_capitals" | "plain";
export type CaseTo = "upper" | "lower" | "title" | "title_small" | "names" | "sentence" | "first" | "toggle";
export type Charset =
  | { kind: "letters" }
  | { kind: "upper" }
  | { kind: "lower" }
  | { kind: "digits" }
  | { kind: "letters_digits" }
  | { kind: "hex" }
  | { kind: "custom"; chars: string };
export type NamePart = "title" | "first" | "middle" | "last" | "suffix" | "first_last" | "last_first";

export type Step =
  | { tool: "replace"; find: Pattern; with: string }
  | { tool: "dates"; format: string; from?: string; past?: boolean }
  | { tool: "numbers"; decimals: number | null; thousands: boolean; decimal_comma: boolean }
  | { tool: "add"; text: string; at: At }
  | { tool: "random"; length: number; chars: Charset }
  | { tool: "pad"; width: number; with: string; right: boolean }
  | { tool: "remove"; what: Remove }
  | { tool: "reverse" }
  | { tool: "spaces"; how: SpacesHow }
  | { tool: "number"; start: number; step: number; digits: number; at_end: boolean; separator: string; replace: boolean }
  | { tool: "digits"; keep: boolean }
  | { tool: "now"; format: string; at_end: boolean; separator: string; replace: boolean }
  | { tool: "case"; to: CaseTo }
  | { tool: "name"; part: NamePart }
  | { tool: "html_encode" }
  | { tool: "html_decode" }
  | { tool: "html_text" }
  | { tool: "accents" }
  | { tool: "fix_number"; how: NumberFix }
  | { tool: "calc"; op: CalcOp; value: number }
  | { tool: "sort_parts"; separator: string; descending: boolean; distinct: boolean; trim: boolean }
  | { tool: "url_decode" }
  | { tool: "url_encode" }
  | { tool: "words"; money: boolean }
  | { tool: "random_number"; min: number; max: number; unique: boolean }
  | { tool: "mask"; keep_last: number; with: string; all: boolean }
  | { tool: "date_band"; to: DateTo }
  | { tool: "scramble"; phrase: string; loose?: boolean }
  | { tool: "address"; style: "short" | "long" }
  | { tool: "zip"; style: "keep" | "five" }
  | { tool: "phone"; style: PhoneStyle }
  | { tool: "email"; fix_domains: boolean };
export type PhoneStyle = "dashes" | "brackets" | "dots" | "digits" | "international";
export type DateTo = { kind: "year" } | { kind: "year_month" } | { kind: "age_band"; size: number; top: number };
/** Shortest secret phrase the scramble accepts (the engine checks again). */
export const MIN_PHRASE = 16;
export type ToolId = Step["tool"];
export type NumberFix = "trailing_minus" | "flip_sign" | "positive" | "negative" | "percent" | "julian";
export type CalcOp = "add" | "subtract" | "multiply" | "divide" | "power" | "round" | "round_up" | "round_down" | "abs";
export const FIX_CHOICES: Record<NumberFix, string> = {
  trailing_minus: "Minus at the end or brackets: 50- and (50) → -50",
  flip_sign: "Flip the sign: 5 → -5, -5 → 5",
  positive: "Make positive: -5 → 5",
  negative: "Make negative: 5 → -5",
  percent: "Percent to number: 12.5% → 0.125",
  julian: "Julian date to a date: 24060, 2024060 or 124060 → 2024-02-29",
};
export const CALC_CHOICES: Record<CalcOp, string> = {
  add: "Add",
  subtract: "Subtract",
  multiply: "Multiply by",
  divide: "Divide by",
  power: "Raise to the power",
  round: "Round to decimals",
  round_up: "Round up (away from zero) to decimals",
  round_down: "Round down (towards zero) to decimals",
  abs: "Absolute value (drop the minus)",
};

export interface Cleaner {
  version: number;
  name: string;
  steps: Step[];
}
export interface CleanSpec {
  cleaner: Cleaner;
  scope: Scope;
  skip_first: boolean;
  read: ReadOptions;
}
export interface CleanChange {
  row: number;
  number: number;
  col: number;
  before: string;
  after: string;
}
export interface CleanPreview {
  changes: CleanChange[];
  rows: number;
  changed: number;
  left: number;
  /** Dates that could only be read the other way round from the chosen order. */
  other_order?: number;
  more: boolean;
}
export interface SplitName {
  col: number;
  parts: NamePart[];
  keep_original: boolean;
  keep_first: boolean;
}
export interface ExampleSpec {
  col: number;
  skip_first: boolean;
}

/** Same limits as the engine (it checks again). */
export const MAX_STEPS = 100;
export const MAX_WIDTH = 10_000;
export const MAX_TEXT = 65_536;

export interface Tool {
  id: ToolId;
  label: string;
  group: string;
  /** A new step with the usual options. */
  make: () => Step;
}

const plain = (text = ""): Pattern => ({ text, kind: "plain", match_case: false, whole_cell: false });

export const TOOLS: Tool[] = [
  { id: "replace", group: "Replace", label: "Replace text", make: () => ({ tool: "replace", find: plain(), with: "" }) },
  { id: "dates", group: "Replace", label: "Dates and times", make: () => ({ tool: "dates", format: "yyyy-mm-dd" }) },
  { id: "numbers", group: "Replace", label: "Numbers", make: () => ({ tool: "numbers", decimals: 2, thousands: false, decimal_comma: false }) },
  { id: "add", group: "Text", label: "Add text", make: () => ({ tool: "add", text: "", at: { kind: "start" } }) },
  { id: "random", group: "Text", label: "Random text", make: () => ({ tool: "random", length: 8, chars: { kind: "letters_digits" } }) },
  { id: "pad", group: "Text", label: "Pad text", make: () => ({ tool: "pad", width: 5, with: "0", right: false }) },
  { id: "remove", group: "Text", label: "Remove text", make: () => ({ tool: "remove", what: { kind: "first", n: 1 } }) },
  { id: "reverse", group: "Text", label: "Reverse text", make: () => ({ tool: "reverse" }) },
  { id: "case", group: "Text", label: "Change case", make: () => ({ tool: "case", to: "title" }) },
  { id: "spaces", group: "Spaces", label: "Add or remove spaces", make: () => ({ tool: "spaces", how: "collapse" }) },
  { id: "number", group: "Numbers", label: "Number the cells", make: () => ({ tool: "number", start: 1, step: 1, digits: 0, at_end: false, separator: "", replace: true }) },
  { id: "digits", group: "Numbers", label: "Remove or keep digits", make: () => ({ tool: "digits", keep: false }) },
  { id: "now", group: "Dates", label: "Add date and time", make: () => ({ tool: "now", format: "yyyy-mm-dd hh:mm", at_end: true, separator: " ", replace: false }) },
  { id: "name", group: "Names", label: "Name part", make: () => ({ tool: "name", part: "last_first" }) },
  { id: "html_encode", group: "HTML", label: "HTML encode", make: () => ({ tool: "html_encode" }) },
  { id: "html_decode", group: "HTML", label: "HTML decode", make: () => ({ tool: "html_decode" }) },
  { id: "html_text", group: "HTML", label: "Text from HTML", make: () => ({ tool: "html_text" }) },
  { id: "accents", group: "Text", label: "Remove accents", make: () => ({ tool: "accents" }) },
  { id: "sort_parts", group: "Text", label: "Sort inside the cell", make: () => ({ tool: "sort_parts", separator: ", ", descending: false, distinct: false, trim: true }) },
  { id: "fix_number", group: "Numbers", label: "Fix numbers", make: () => ({ tool: "fix_number", how: "trailing_minus" }) },
  { id: "calc", group: "Numbers", label: "Calculate", make: () => ({ tool: "calc", op: "multiply", value: 1 }) },
  { id: "url_decode", group: "Web", label: "URL decode (%20 → space)", make: () => ({ tool: "url_decode" }) },
  { id: "url_encode", group: "Web", label: "URL encode (space → %20)", make: () => ({ tool: "url_encode" }) },
  { id: "words", group: "Numbers", label: "Numbers in words", make: () => ({ tool: "words", money: false }) },
  { id: "random_number", group: "Numbers", label: "Random numbers", make: () => ({ tool: "random_number", min: 1, max: 100, unique: true }) },
  { id: "mask", group: "Privacy", label: "Mask (keep the last few)", make: () => ({ tool: "mask", keep_last: 4, with: "*", all: false }) },
  { id: "date_band", group: "Privacy", label: "Date to year or age band", make: () => ({ tool: "date_band", to: { kind: "age_band", size: 10, top: 90 } }) },
  { id: "scramble", group: "Privacy", label: "Scramble with a secret phrase", make: () => ({ tool: "scramble", phrase: "", loose: false }) },
  { id: "address", group: "Addresses", label: "Street words (St, Ave, N, Apt)", make: () => ({ tool: "address", style: "short" }) },
  { id: "zip", group: "Addresses", label: "ZIP codes (leading zeros, ZIP+4)", make: () => ({ tool: "zip", style: "keep" }) },
  { id: "phone", group: "Addresses", label: "Phone numbers", make: () => ({ tool: "phone", style: "dashes" }) },
  { id: "email", group: "Addresses", label: "E-mail addresses", make: () => ({ tool: "email", fix_domains: true }) },
];

export const toolOf = (id: ToolId): Tool => TOOLS.find((t) => t.id === id) ?? TOOLS[0]!;

const AT_WORDS: Record<At["kind"], string> = { start: "at the start", end: "at the end", position: "after", before: "before", after: "after" };
const CASE_WORDS: Record<CaseTo, string> = {
  upper: "CAPITALS",
  lower: "small letters",
  title: "Title Case",
  title_small: "Title Case, small words (of, and, the) kept small",
  names: "Names (McDonald, O'Brien, Smith-Jones, Henry VIII)",
  sentence: "Sentence case",
  first: "Capital first letter only, the rest as it is",
  toggle: "tOGGLE cASE",
};
const SPACE_WORDS: Record<SpacesHow, string> = {
  trim: "Trim spaces at both ends",
  trim_start: "Trim spaces at the start",
  trim_end: "Trim spaces at the end",
  collapse: "Trim and make runs of spaces one space",
  remove_all: "Remove every space",
  line_breaks: "Line breaks to spaces",
  letters_digits: "Space between letters and digits",
  before_capitals: "Space before capitals (firstName → first Name)",
  plain: "Odd spaces (no-break, thin, wide) to plain spaces",
};
export const SPACES_CHOICES = SPACE_WORDS;
export const CASE_CHOICES = CASE_WORDS;
export const NAME_CHOICES: Record<NamePart, string> = {
  title: "Title (Dr, Ms, …)",
  first: "First name",
  middle: "Middle names",
  last: "Last name",
  suffix: "Suffix (Jr, PhD, …)",
  first_last: "First Middle Last",
  last_first: "Last, First Middle",
};
export const KIND_CHOICES: Record<CharKind, string> = {
  letters: "letters",
  digits: "digits",
  punctuation: "punctuation and symbols",
  spaces: "spaces and line breaks",
  non_ascii: "characters outside plain ASCII",
  control: "control characters",
  invisible: "invisible characters (zero-width, byte-order mark, soft hyphen)",
};

const q = (s: string) => `“${s.length > 24 ? `${s.slice(0, 24)}…` : s}”`;

/** A step in a few words, for the cleaner's list. */
export function describe(s: Step): string {
  switch (s.tool) {
    case "replace":
      return `Replace ${q(s.find.text)} with ${q(s.with)}${s.find.kind === "regex" ? " (pattern)" : s.find.kind === "wildcard" ? " (wildcards)" : ""}`;
    case "dates":
      return s.from ? `Dates from ${s.from} as ${s.format}` : `Dates as ${s.format}`;
    case "numbers":
      return `Numbers with ${s.decimals === null ? "their own" : s.decimals} decimals${s.thousands ? ", thousands separated" : ""}${s.decimal_comma ? ", decimal comma" : ""}`;
    case "add":
      return `Add ${q(s.text)} ${AT_WORDS[s.at.kind]}${s.at.kind === "position" ? ` ${s.at.n} characters` : s.at.kind === "before" || s.at.kind === "after" ? ` ${q(s.at.text)}` : ""}`;
    case "random":
      return `Random text, ${s.length} characters`;
    case "pad":
      return `Pad to ${s.width} with ${q(s.with)} on the ${s.right ? "right" : "left"}`;
    case "remove": {
      const w = s.what;
      switch (w.kind) {
        case "first":
          return `Remove the first ${w.n} characters`;
        case "last":
          return `Remove the last ${w.n} characters`;
        case "range":
          return `Remove ${w.count} characters after ${w.from}`;
        case "before":
          return `Remove what comes before ${q(w.text)}${w.inclusive ? " (and it)" : ""}`;
        case "after":
          return `Remove what comes after ${q(w.text)}${w.inclusive ? " (and it)" : ""}`;
        case "matching":
          return `Remove ${q(w.find.text)}`;
        case "kind":
          return `Remove ${KIND_CHOICES[w.of]}`;
        case "characters":
          return `Remove the characters ${q(w.chars)}`;
      }
      return "Remove text";
    }
    case "reverse":
      return "Reverse the text";
    case "spaces":
      return SPACE_WORDS[s.how];
    case "number":
      return `Number from ${s.start} by ${s.step}${s.replace ? "" : s.at_end ? " at the end" : " at the start"}`;
    case "digits":
      return s.keep ? "Keep only digits" : "Remove digits";
    case "now":
      return `Add the date and time (${s.format})`;
    case "case":
      return `Case: ${CASE_WORDS[s.to]}`;
    case "name":
      return `Name: ${NAME_CHOICES[s.part]}`;
    case "html_encode":
      return "HTML encode (&, <, > as entities)";
    case "html_decode":
      return "HTML decode (entities as characters)";
    case "html_text":
      return "Text from HTML";
    case "accents":
      return "Remove accents (é → e)";
    case "fix_number":
      return FIX_CHOICES[s.how].split(":")[0]!;
    case "calc":
      return s.op === "abs" ? "Absolute value" : `${CALC_CHOICES[s.op]} ${s.value}`;
    case "sort_parts":
      return `Sort the parts between ${q(s.separator)} ${s.descending ? "Z → A" : "A → Z"}${s.distinct ? ", each once" : ""}`;
    case "url_decode":
      return "URL decode (%20 → space)";
    case "url_encode":
      return "URL encode (space → %20)";
    case "words":
      return s.money ? "Amounts in words (dollars and cents)" : "Numbers in words";
    case "random_number":
      return `Random ${s.unique ? "different " : ""}numbers ${s.min} to ${s.max}`;
    case "mask":
      return s.keep_last ? `Mask all but the last ${s.keep_last}` : "Mask everything";
    case "date_band":
      return s.to.kind === "year" ? "Date to year" : s.to.kind === "year_month" ? "Date to year and month" : `Date to age band (${s.to.size} years${s.to.top ? `, ${s.to.top}+` : ""})`;
    case "scramble":
      return "Scramble with a secret phrase";
    case "address":
      return s.style === "short" ? "Abbreviate street words (Street → St)" : "Write out street words (St → Street)";
    case "zip":
      return s.style === "five" ? "ZIP codes as five digits" : "Tidy ZIP codes";
    case "phone":
      return `Phone numbers like ${PHONE_LOOK[s.style]}`;
    case "email":
      return s.fix_domains ? "Tidy e-mail addresses and fix common misspellings" : "Tidy e-mail addresses";
  }
}

/** How each phone style looks. */
export const PHONE_LOOK: Record<PhoneStyle, string> = {
  dashes: "423-555-0123",
  brackets: "(423) 555-0123",
  dots: "423.555.0123",
  digits: "4235550123",
  international: "+1 423-555-0123",
};

/** A problem with a step the screen can name before asking the engine. */
export function stepProblem(s: Step): string {
  const long = (t: string) => t.length > MAX_TEXT;
  switch (s.tool) {
    case "replace":
      if (!s.find.text) return "Type the text to find.";
      if (long(s.find.text) || long(s.with)) return "The text is too long.";
      return "";
    case "add":
      if (!s.text) return "Type the text to add.";
      if ((s.at.kind === "before" || s.at.kind === "after") && !s.at.text) return "Type the text to look for.";
      return long(s.text) ? "The text is too long." : "";
    case "pad":
      if ([...s.with].length !== 1) return "Pad with exactly one character.";
      return s.width < 1 || s.width > MAX_WIDTH ? `Pad to between 1 and ${MAX_WIDTH} characters.` : "";
    case "random":
      if (s.chars.kind === "custom" && !s.chars.chars) return "Type the characters to use.";
      return s.length < 1 || s.length > MAX_WIDTH ? `Choose between 1 and ${MAX_WIDTH} characters.` : "";
    case "dates":
      if (s.from && !/[ymdhs]/i.test(s.from)) return "The pattern the cells are in needs at least one of yyyy, mm, dd, hh, mm, ss.";
      return /[ymdhs]/i.test(s.format) ? "" : "The format needs at least one of yyyy, mm, dd, hh, mm, ss.";
    case "now":
      return /[ymdhs]/i.test(s.format) ? "" : "The format needs at least one of yyyy, mm, dd, hh, mm, ss.";
    case "mask":
      if ([...s.with].length !== 1) return "Mask with exactly one character.";
      return Number.isInteger(s.keep_last) && s.keep_last >= 0 && s.keep_last <= MAX_WIDTH ? "" : `Keep between 0 and ${MAX_WIDTH} characters.`;
    case "date_band":
      if (s.to.kind !== "age_band") return "";
      return Number.isInteger(s.to.size) && s.to.size >= 1 && s.to.size <= 150 && Number.isInteger(s.to.top) && s.to.top >= 0 && s.to.top <= 150 ? "" : "Choose bands of 1 to 150 years.";
    case "scramble":
      return [...s.phrase].length >= MIN_PHRASE ? "" : `Type a secret phrase of at least ${MIN_PHRASE} characters (it is never saved).`;
    case "random_number":
      if (!Number.isInteger(s.min) || !Number.isInteger(s.max)) return "Choose whole numbers.";
      if (s.min > s.max) return "The smallest number must not be above the largest.";
      return Math.abs(s.min) > 1e15 || Math.abs(s.max) > 1e15 ? "Choose numbers between -10^15 and 10^15." : "";
    case "number":
      return Number.isFinite(s.start) && Number.isFinite(s.step) && Math.abs(s.start) <= 1e15 && Math.abs(s.step) <= 1e15 ? "" : "Choose numbers between -10^15 and 10^15.";
    case "remove":
      if (s.what.kind === "matching" && !s.what.find.text) return "Type the text to remove.";
      if ((s.what.kind === "before" || s.what.kind === "after") && !s.what.text) return "Type the text to look for.";
      if (s.what.kind === "characters" && !s.what.chars) return "Type the characters to remove.";
      return "";
    case "calc":
      if (!Number.isFinite(s.value) || Math.abs(s.value) > 1e15) return "Choose a number between -10^15 and 10^15.";
      if (s.op === "divide" && s.value === 0) return "Cannot divide by zero.";
      if ((s.op === "round" || s.op === "round_up" || s.op === "round_down") && !(Number.isInteger(s.value) && s.value >= 0 && s.value <= 15)) return "Round to between 0 and 15 decimals.";
      if (s.op === "power" && Math.abs(s.value) > 1000) return "Choose a power between -1000 and 1000.";
      return "";
    case "sort_parts":
      if (!s.separator) return "Type the separator between the parts.";
      return long(s.separator) ? "The separator is too long." : "";
    default:
      return "";
  }
}

/** The local date and time as days since 1970-01-01, for "add date and time". */
export function localNow(d = new Date()): number {
  return (d.getTime() - d.getTimezoneOffset() * 60_000) / 86_400_000;
}

/** A safe copy of a step (for keeping in a cleaner while the form changes). */
export const copyStep = (s: Step): Step => JSON.parse(JSON.stringify(s)) as Step;

// ---- a simple stand-in for the engine (browser preview only) ----

function titleCase(t: string): string {
  return t.toLowerCase().replace(/(^|[^\p{L}\p{N}'’])(\p{L})/gu, (_, a: string, b: string) => a + b.toUpperCase());
}

/** One step on one cell, roughly as the engine does it (the browser
 *  stand-in uses this; the desktop app runs the engine). */
export function runStep(s: Step, t: string, index: number): string {
  switch (s.tool) {
    case "replace":
      return s.find.text ? t.split(s.find.text).join(s.with) : t;
    case "numbers": {
      const v = Number(t.replace(/[$,]/g, ""));
      if (!t.trim() || Number.isNaN(v)) return t;
      let out = s.decimals === null ? String(v) : v.toFixed(s.decimals);
      if (s.thousands) out = out.replace(/^(-?\d+)/, (m) => m.replace(/\B(?=(\d{3})+(?!\d))/g, ","));
      return s.decimal_comma ? out.replace(/[.,]/g, (c) => (c === "." ? "," : ".")) : out;
    }
    case "add": {
      const at = s.at;
      const i =
        at.kind === "start" ? 0 : at.kind === "end" ? t.length : at.kind === "position" ? Math.min(at.n, t.length) : t.indexOf(at.text);
      if (i < 0) return t;
      const j = at.kind === "after" ? i + at.text.length : i;
      return t.slice(0, j) + s.text + t.slice(j);
    }
    case "random":
      return Array.from({ length: s.length }, () => "abcdefghijklmnopqrstuvwxyz0123456789"[Math.floor(Math.random() * 36)]).join("");
    case "pad":
      return s.right ? t.padEnd(s.width, s.with) : t.padStart(s.width, s.with);
    case "remove": {
      const w = s.what;
      if (w.kind === "first") return t.slice(w.n);
      if (w.kind === "last") return t.slice(0, Math.max(0, t.length - w.n));
      if (w.kind === "range") return t.slice(0, w.from) + t.slice(w.from + w.count);
      if (w.kind === "characters") return [...t].filter((c) => !w.chars.includes(c)).join("");
      if (w.kind === "matching") return w.find.text ? t.split(w.find.text).join("") : t;
      if (w.kind === "kind") return w.of === "digits" ? t.replace(/\d/g, "") : w.of === "spaces" ? t.replace(/\s/g, "") : t;
      const i = t.indexOf(w.text);
      if (i < 0 || !w.text) return t;
      return w.kind === "before" ? t.slice(w.inclusive ? i + w.text.length : i) : t.slice(0, w.inclusive ? i : i + w.text.length);
    }
    case "reverse":
      return [...t].reverse().join("");
    case "spaces":
      if (s.how === "trim") return t.trim();
      if (s.how === "trim_start") return t.trimStart();
      if (s.how === "trim_end") return t.trimEnd();
      if (s.how === "collapse") return t.trim().split(/\s+/).join(" ");
      if (s.how === "remove_all") return t.replace(/\s/g, "");
      if (s.how === "line_breaks") return t.replace(/\r\n|\r|\n/g, " ");
      if (s.how === "letters_digits") return t.replace(/(\p{L})(\d)/gu, "$1 $2").replace(/(\d)(\p{L})/gu, "$1 $2");
      return t.replace(/(\p{Ll}|\d)(\p{Lu})/gu, "$1 $2");
    case "number": {
      const n = String(s.start + s.step * index).padStart(s.digits, "0");
      if (s.replace) return n;
      return s.at_end ? `${t}${t ? s.separator : ""}${n}` : `${n}${t ? s.separator : ""}${t}`;
    }
    case "digits":
      return s.keep ? t.replace(/\D/g, "") : t.replace(/\d/g, "");
    case "now": {
      const n = new Date().toISOString().slice(0, 16).replace("T", " ");
      return s.replace ? n : s.at_end ? `${t}${t ? s.separator : ""}${n}` : `${n}${t ? s.separator : ""}${t}`;
    }
    case "case":
      if (s.to === "first") return t.replace(/\p{L}/u, (c) => c.toUpperCase());
      if (s.to === "title_small" || s.to === "names") return titleCase(t);
      if (s.to === "upper") return t.toUpperCase();
      if (s.to === "lower") return t.toLowerCase();
      if (s.to === "title") return titleCase(t);
      if (s.to === "toggle") return [...t].map((c) => (c === c.toUpperCase() ? c.toLowerCase() : c.toUpperCase())).join("");
      return t.toLowerCase().replace(/(^\s*|[.!?]\s+)(\p{L})/gu, (_, a: string, b: string) => a + b.toUpperCase());
    case "name": {
      const comma = t.indexOf(",");
      const words = (comma >= 0 ? `${t.slice(comma + 1)} ${t.slice(0, comma)}` : t).trim().split(/\s+/).filter(Boolean);
      const first = words[0] ?? "";
      const last = words.length > 1 ? words[words.length - 1]! : "";
      const middle = words.slice(1, -1).join(" ");
      if (s.part === "first") return first;
      if (s.part === "last") return last;
      if (s.part === "middle") return middle;
      if (s.part === "first_last") return [first, middle, last].filter(Boolean).join(" ");
      if (s.part === "last_first") return last ? `${last}, ${[first, middle].filter(Boolean).join(" ")}` : first;
      return "";
    }
    case "html_encode":
      return t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");
    case "html_decode":
      return t.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&");
    case "html_text":
      return t.replace(/<br\s*\/?>/gi, "\n").replace(/<[^>]*>/g, "").replace(/&amp;/g, "&").trim();
    case "dates":
      return t;
    case "accents":
      return t.normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/ß/g, "ss").replace(/Æ/g, "AE").replace(/æ/g, "ae").replace(/Ø/g, "O").replace(/ø/g, "o").replace(/Ł/g, "L").replace(/ł/g, "l");
    case "fix_number": {
      const v = t.trim();
      if (s.how === "trailing_minus") {
        const m = /^\(?\s*([\d.,]+)\s*(-|\))$/.exec(v);
        return m ? `-${m[1]}` : t;
      }
      if (s.how === "percent") return /%$/.test(v) && !Number.isNaN(Number(v.slice(0, -1))) ? String(Number(v.slice(0, -1)) / 100) : t;
      if (s.how === "julian") return t;
      if (Number.isNaN(Number(v.replace(/,/g, ""))) || !v) return t;
      const body = v.replace(/^[-+]/, "");
      const neg = s.how === "flip_sign" ? !v.startsWith("-") : s.how === "negative";
      return neg && Number(body.replace(/,/g, "")) !== 0 ? `-${body}` : body;
    }
    case "calc": {
      const v = Number(t.trim().replace(/,/g, ""));
      if (!t.trim() || Number.isNaN(v)) return t;
      const x = s.value;
      const f = 10 ** x;
      const r = { add: v + x, subtract: v - x, multiply: v * x, divide: v / x, power: v ** x, abs: Math.abs(v), round: Math.round(v * f) / f, round_up: (v < 0 ? -Math.ceil(-v * f) : Math.ceil(v * f)) / f, round_down: Math.trunc(v * f) / f }[s.op];
      return Number.isFinite(r) ? String(Number(r.toPrecision(12))) : t;
    }
    case "sort_parts": {
      if (!s.separator || !t.includes(s.separator)) return t;
      let parts = t.split(s.separator);
      if (s.trim) parts = parts.map((p) => p.trim()).filter(Boolean);
      parts.sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }));
      if (s.descending) parts.reverse();
      if (s.distinct) parts = [...new Set(parts)];
      return parts.join(s.separator);
    }
    case "url_decode":
      try {
        return decodeURIComponent(t);
      } catch {
        return t;
      }
    case "url_encode":
      return encodeURIComponent(t).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
    case "words":
      return t;
    case "random_number":
      return String(s.min + Math.floor(Math.random() * (s.max - s.min + 1)));
    case "mask": {
      const hide = (c: string) => s.all || /[\p{L}\p{N}]/u.test(c);
      const chars = [...t];
      const n = chars.filter(hide).length;
      // Never more than half shown; short values hidden whole.
      let left = n <= s.keep_last ? n : n - Math.min(s.keep_last, Math.floor(n / 2));
      return chars.map((c) => (hide(c) && left-- > 0 ? s.with : c)).join("");
    }
    case "date_band": {
      const m = /^(\d{4})-(\d{2})/.exec(t.trim());
      if (!m) return t;
      return s.to.kind === "year" ? m[1]! : s.to.kind === "year_month" ? `${m[1]}-${m[2]}` : t;
    }
    case "address":
    case "zip":
    case "phone":
    case "email":
      return tidyPreview(s, t);
    case "scramble": {
      // Browser stand-in only (the app stretches the phrase and uses a
      // keyed SHA-256).
      let v = t.trim();
      if (s.loose) v = v.replace(/[^\p{L}\p{N}]/gu, "").toLowerCase() || t.trim();
      let h = 2166136261;
      for (const c of s.phrase + "\u0000" + v) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
      return v ? h.toString(16).padStart(8, "0").repeat(2) : t;
    }
  }
}

/** Every step of a cleaner on one cell. */
export function runCleaner(c: Cleaner, t: string, index: number): string {
  return c.steps.reduce((v, s) => runStep(s, v, index), t);
}

/** Browser stand-in for the tidy steps (the app's engine has the full word
 *  lists and rules). */
function tidyPreview(s: Extract<Step, { tool: "address" | "zip" | "phone" | "email" }>, t: string): string {
  const v = t.trim();
  if (!v) return t;
  switch (s.tool) {
    case "address": {
      const pairs: [string, string][] = [["St", "Street"], ["Ave", "Avenue"], ["Rd", "Road"], ["Dr", "Drive"], ["Blvd", "Boulevard"], ["Ln", "Lane"], ["Ct", "Court"], ["Apt", "Apartment"], ["Ste", "Suite"], ["N", "North"], ["S", "South"], ["E", "East"], ["W", "West"]];
      return t.replace(/\b([A-Za-z]+)\.?(?=\s|,|$)/g, (w, word: string) => {
        const p = pairs.find(([a, b]) => a.toLowerCase() === word.toLowerCase() || b.toLowerCase() === word.toLowerCase());
        return p ? (s.style === "short" ? p[0] : p[1]) : w;
      });
    }
    case "zip": {
      const m = /^(\d{3,5})(?:[- ]?(\d{4}))?(?:\.0)?$/.exec(v);
      if (!m) return t;
      const five = m[1]!.padStart(5, "0");
      return m[2] && s.style === "keep" ? `${five}-${m[2]}` : five;
    }
    case "phone": {
      let d = v.replace(/\D/g, "");
      if (d.length === 11 && d.startsWith("1")) d = d.slice(1);
      if (d.length !== 10) return t;
      return PHONE_LOOK[s.style].replace("423", d.slice(0, 3)).replace("555", d.slice(3, 6)).replace("0123", d.slice(6));
    }
    case "email": {
      const e = v.replace(/^mailto:/i, "").replace(/\s+/g, "").toLowerCase().replace(/[.,;:]+$/, "");
      const fixed = s.fix_domains ? e.replace(/@gmial\.com$/, "@gmail.com").replace(/@hotmial\.com$/, "@hotmail.com") : e;
      return /^[^@\s]+@[^@\s]+\.[a-z]{2,}$/.test(fixed) ? fixed : t;
    }
  }
}
