// C10: the rule fill by example learns, as the engine sends it, and what the rule card says
// about it. The browser preview also learns and runs rules here (a small search, enough to try
// the card); the desktop app does all of it in the engine.

export type Cut = "whole" | "part" | "word" | "number" | "run" | { split: { by: string } };
export type Case = "keep" | "upper" | "lower" | "title";
export type Piece = { kind: "text"; text: string } | { kind: "from"; col: number; cut: Cut; index: number; first: boolean; case: Case };
export interface Program {
  pieces: Piece[];
}

export interface Conflict {
  row: number;
  id: number;
  value: string;
}
export interface LearnReport {
  program: Program | null;
  words: string;
  examples: number;
  conflict: Conflict | null;
  revision: number;
}
export interface RulePreviewRow {
  row: number;
  reads: [number, string][];
  result: string | null;
}
export interface RulePreview {
  rows: RulePreviewRow[];
  looked: number;
}
export type OddKind = "empty_piece" | "shape" | "length";
export interface Outlier {
  row: number;
  kinds: OddKind[];
  value: string;
}
export interface Outliers {
  fills: number;
  missed: number;
  empty_piece: number;
  odd_shape: number;
  odd_length: number;
  rows: Outlier[];
  odd_rows: number;
}
/** What a fill by example used (OpReport.fill_rule). */
export interface FillRule {
  program: Program;
  example_ids: number[];
}

/** Same limits as the engine. */
export const MAX_PIECES = 32;
export const MAX_TEXT_CHARS = 1000;
export const PREVIEW_LOOK = 1000;
export const PREVIEW_ROWS = 20;
export const MAX_OUTLIERS = 200;

export const CUTS: [string, string][] = [
  ["whole", "the whole value"],
  ["part", "a part (split at spaces)"],
  ["word", "a word (letters)"],
  ["number", "a number (digits)"],
  ["run", "a run of letters and digits"],
  ["split", "a piece split at…"],
];
export const CASES: [Case, string][] = [
  ["keep", "as it is"],
  ["upper", "in capitals"],
  ["lower", "in small letters"],
  ["title", "capitalized"],
];

export function cutKey(c: Cut): string {
  return typeof c === "string" ? c : "split";
}

export function splitBy(c: Cut): string {
  return typeof c === "string" ? "" : c.split.by;
}

const isLetter = (c: string) => /\p{L}/u.test(c);
const isDigit = (c: string) => c >= "0" && c <= "9";
const isAlnum = (c: string) => /[\p{L}\p{N}]/u.test(c);

export function tokens(v: string, cut: Cut): string[] {
  if (cut === "whole") return v ? [v] : [];
  if (cut === "part") return v.split(/\s+/u).filter((x) => x);
  if (typeof cut !== "string") return v.includes(cut.split.by) ? v.split(cut.split.by) : [];
  const keep = cut === "word" ? isLetter : cut === "number" ? isDigit : isAlnum;
  const out: string[] = [];
  let cur = "";
  for (const c of v) {
    if (keep(c)) cur += c;
    else if (cur) {
      out.push(cur);
      cur = "";
    }
  }
  if (cur) out.push(cur);
  return out;
}

export function pick(toks: string[], index: number): string | null {
  const i = index >= 0 ? index : toks.length + index;
  return i >= 0 && i < toks.length ? toks[i]! : null;
}

export function changeCase(s: string, c: Case): string {
  if (c === "upper") return s.toUpperCase();
  if (c === "lower") return s.toLowerCase();
  if (c === "keep") return s;
  let out = "";
  let start = true;
  for (const ch of s) {
    if (isAlnum(ch)) {
      out += start ? ch.toUpperCase() : ch.toLowerCase();
      start = false;
    } else {
      out += ch;
      start = ch !== "'";
    }
  }
  return out;
}

export function pieceText(p: Piece, row: (c: number) => string | null): string | null {
  if (p.kind === "text") return p.text;
  const v = row(p.col);
  if (v === null) return null;
  let t = pick(tokens(v, p.cut), p.index);
  if (t === null) return null;
  if (p.first) {
    const c = [...t][0];
    if (c === undefined) return null;
    t = c;
  }
  return changeCase(t, p.case);
}

/** The result and whether a piece came out empty; null when a piece is missing. */
export function runParts(p: Program, row: (c: number) => string | null): [string, boolean] | null {
  let out = "";
  let empty = false;
  for (const x of p.pieces) {
    const t = pieceText(x, row);
    if (t === null) return null;
    if (x.kind === "from" && !t) empty = true;
    out += t;
  }
  return [out, empty];
}

export function programCols(p: Program): number[] {
  return [...new Set(p.pieces.flatMap((x) => (x.kind === "from" ? [x.col] : [])))].sort((a, b) => a - b);
}

/** The rule in words, as the engine says it. */
export function describe(p: Program, names: (c: number) => string): string {
  return p.pieces
    .map((x) => {
      if (x.kind === "text") return `"${x.text}"`;
      let which = "";
      if (x.cut !== "whole") {
        let kind = x.cut === "part" ? "part" : x.cut === "word" ? "word" : x.cut === "number" ? "number" : typeof x.cut === "string" ? "letters-and-digits run" : "piece";
        if (typeof x.cut !== "string") kind = `${kind} (split at "${x.cut.split.by}")`;
        which = x.index === 0 ? `first ${kind} of ` : x.index === -1 ? `last ${kind} of ` : x.index > 0 ? `${kind} ${x.index + 1} of ` : `${kind} ${-x.index} from the end of `;
      }
      const first = x.first ? "first letter of " : "";
      const c = x.case === "upper" ? ", in capitals" : x.case === "lower" ? ", in small letters" : x.case === "title" ? ", capitalized" : "";
      return `${first}${which}${names(x.col)}${c}`;
    })
    .join(" + ");
}

/** Why a rule cannot be used, as the engine checks it; "" when it can. */
export function ruleProblem(p: Program, ncols: number, target: number): string {
  if (!p.pieces.length) return "the rule has no pieces";
  if (p.pieces.length > MAX_PIECES) return `a rule has at most ${MAX_PIECES} pieces`;
  for (const x of p.pieces) {
    if (x.kind === "text") {
      if (!x.text) return "a piece of text is empty";
      if ([...x.text].length > MAX_TEXT_CHARS) return `a piece of text is over ${MAX_TEXT_CHARS} characters`;
    } else {
      if (x.col >= ncols) return "the rule reads a column the table no longer has; learn it again";
      if (x.col === target) return "the rule reads the column it fills";
      if (typeof x.cut !== "string" && [...x.cut.split.by].length !== 1) return "split at one character";
    }
  }
  return "";
}

/** A result's shape as the engine compares them: each run of letters one A, of digits one 9. */
export function shape(v: string): string {
  let s = "";
  for (const c of v) {
    const k = /\p{N}/u.test(c) ? "9" : isLetter(c) ? "A" : c;
    if ((k === "A" || k === "9") && s.endsWith(k)) continue;
    s += k;
  }
  return s;
}

/** The status-line words after Ctrl+E counted the odd results. */
export function oddWords(o: Outliers): string {
  if (!o.odd_rows) return "None of the results look odd.";
  const why: string[] = [];
  if (o.empty_piece) why.push(`${o.empty_piece.toLocaleString()} with an empty piece`);
  if (o.odd_shape) why.push(`${o.odd_shape.toLocaleString()} shaped unlike the examples`);
  if (o.odd_length) why.push(`${o.odd_length.toLocaleString()} far longer or shorter`);
  const n = o.odd_rows;
  return `${n.toLocaleString()} ${n === 1 ? "result looks" : "results look"} odd (${why.join(", ")}).`;
}

export function kindWords(k: OddKind): string {
  return k === "empty_piece" ? "a piece is empty" : k === "shape" ? "shaped unlike the examples" : "far longer or shorter";
}

// ---- the browser preview's learner (a small search; the engine's is the real one) ----------

const CASE_LIST: Case[] = ["keep", "upper", "lower", "title"];

function candidates(rows: string[][], sources: number[]): Piece[] {
  const out: Piece[] = [];
  const first = rows[0] ?? [];
  for (const col of sources) {
    const v = first[col] ?? "";
    const cuts: Cut[] = ["whole", "part", "word", "number", "run"];
    for (const ch of v) if (!isAlnum(ch) && !/\s/u.test(ch) && !cuts.some((c) => typeof c !== "string" && c.split.by === ch)) cuts.push({ split: { by: ch } });
    for (const cut of cuts)
      for (const index of cut === "whole" ? [0] : [0, 1, 2, -1, -2])
        for (const first of [false, true]) for (const c of CASE_LIST) out.push({ kind: "from", col, cut, index, first, case: c });
  }
  return out;
}

/** A rule that makes every example's result, or null (the browser preview's search). */
export function mockLearn(examples: { row: string[]; out: string }[], sources: number[]): Program | null {
  if (!examples.length) return null;
  const cands = candidates(
    examples.map((e) => e.row),
    sources,
  );
  const texts = examples.map((e) => cands.map((p) => pieceText(p, (c) => e.row[c] ?? "")));
  let best: Piece[] | null = null;
  const search = (pos: number[], pieces: Piece[]): void => {
    if (best && pieces.length >= best.length) return;
    if (pieces.length > 8) return;
    if (pos.every((p, k) => p === examples[k]!.out.length)) {
      best = pieces.slice();
      return;
    }
    // Longest piece first.
    const fits: [number, number][] = [];
    cands.forEach((_, i) => {
      let len = -1;
      for (let k = 0; k < examples.length; k++) {
        const t = texts[k]![i];
        if (!t || !examples[k]!.out.startsWith(t, pos[k])) return;
        if (k === 0) len = t.length;
      }
      fits.push([i, len]);
    });
    fits.sort((a, b) => b[1] - a[1]);
    for (const [i] of fits.slice(0, 6)) {
      search(
        pos.map((p, k) => p + texts[k]![i]!.length),
        [...pieces, cands[i]!],
      );
    }
    // The same text in every example.
    const rest = examples.map((e, k) => e.out.slice(pos[k]));
    let common = "";
    for (const ch of rest[0] ?? "") {
      if (rest.every((r) => r.startsWith(common + ch))) common += ch;
      else break;
    }
    for (let n = common.length; n >= 1; n--) {
      const text = common.slice(0, n);
      const last = pieces[pieces.length - 1];
      if (last?.kind === "text") continue;
      search(
        pos.map((p) => p + n),
        [...pieces, { kind: "text", text }],
      );
    }
  };
  search(
    examples.map(() => 0),
    [],
  );
  return best ? { pieces: best } : null;
}

// ---- the formula form (as the engine writes it) ---------------------------------------------

const esc = (s: string) => s.replace(/[\\.+*?()|[\]{}^$#&\-~]/g, (c) => `\\${c}`);

export function piecePattern(cut: Cut, index: number, nonempty: boolean): string {
  if (cut === "whole") return "^(.+)$";
  if (typeof cut !== "string") {
    const e = esc(cut.split.by);
    const rep = nonempty ? "+" : "*";
    if (index >= 0) return index === 0 ? `^([^${e}]${rep})${e}` : `^(?:[^${e}]*${e}){${index}}([^${e}]${rep})`;
    const k = -index;
    return k === 1 ? `${e}([^${e}]${rep})$` : `(?:^|${e})([^${e}]${rep})(?:${e}[^${e}]*){${k - 1}}$`;
  }
  const yes = cut === "part" ? "\\S" : cut === "word" ? "[\\p{Alphabetic}]" : cut === "number" ? "[0-9]" : "[\\p{Alphabetic}\\p{N}]";
  const no = cut === "part" ? "\\s" : `[^${yes.slice(1, -1)}]`;
  if (index >= 0) return index === 0 ? `^${no}*(${yes}+)` : `^${no}*(?:${yes}+${no}+){${index}}(${yes}+)`;
  const k = -index;
  return k === 1 ? `(${yes}+)${no}*$` : `(${yes}+)(?:${no}+${yes}+){${k - 1}}${no}*$`;
}

const quote = (s: string) => `"${s.replace(/"/g, '""')}"`;

/** The rule as a formula (the browser preview's copy of the engine's), or why it has none. */
export function mockFormula(p: Program, names: (c: number) => string, typed: (c: number) => boolean): { formula: string } | { why: string } {
  const parts: string[] = [];
  for (const x of p.pieces) {
    if (x.kind === "text") {
      parts.push(quote(x.text));
      continue;
    }
    const name = names(x.col);
    if (typed(x.col)) return { why: `${name} is read as numbers or dates, so a formula would change its text` };
    let f = `REGEXEXTRACT([${name.replace(/]/g, "]]")}], ${quote(piecePattern(x.cut, x.index, x.first))}, 1)`;
    if (x.first) f = `LEFT(${f}, 1)`;
    if (x.case === "upper") f = `UPPER(${f})`;
    else if (x.case === "lower") f = `LOWER(${f})`;
    else if (x.case === "title") f = `PROPER(${f})`;
    parts.push(f);
  }
  return { formula: `IFERROR(${parts.join(" & ")}, "")` };
}
