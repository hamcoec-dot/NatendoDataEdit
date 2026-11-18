// One-shot cell formulas, as the UI and the browser stub see them. The
// engine (crates/engine/src/cellformula.rs) does the real work; this is its
// small mirror for the stub, plus what the UI decides before calling it.

/** What typing `value` in a cell means: a formula to run, or text to keep
 *  (a leading apostrophe keeps text that starts with "=" as it is). */
export function typedCell(value: string): { formula: string } | { text: string } {
  if (value.startsWith("'=")) return { text: value.slice(1) };
  if (value.length > 1 && value.startsWith("=")) return { formula: value };
  return { text: value };
}

export { colLetters } from "./goto";
import { colLetters } from "./goto";

function lettersToCol(s: string): number {
  let n = 0;
  for (const ch of s.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n - 1;
}

const REF = /^(\$?)([A-Za-z]{1,3})(\$?)(\d{1,12})(?![\w(.])/;

const quote = (s: string): string => `"${s.replace(/"/g, '""')}"`;

/** The stub's rewrite: references (moved from `from` to `to`, as
 *  [row position, column]) replaced by their cells' text. Null: a reference
 *  moved off the table. */
export function rewriteA1(
  src: string,
  from: [number, number],
  to: [number, number],
  size: [number, number],
  cellAt: (pos: number, col: number) => string,
): { run: string; keep: string } | null {
  const [dr, dc] = [to[0] - from[0], to[1] - from[1]];
  let run = "";
  let keep = "";
  const open: string[] = [];
  let word = "";
  let i = 0;
  const parse = (at: number) => {
    const m = REF.exec(src.slice(at));
    if (!m) return null;
    const col = lettersToCol(m[2]!);
    const row = Number(m[4]) - 1;
    if (row < 0) return null;
    return {
      len: m[0].length,
      col: m[1] ? col : col + dc,
      row: m[3] ? row : row + dr,
      cf: !!m[1],
      rf: !!m[3],
    };
  };
  const show = (r: { col: number; row: number; cf: boolean; rf: boolean }) =>
    `${r.cf ? "$" : ""}${colLetters(r.col)}${r.rf ? "$" : ""}${r.row + 1}`;
  const fits = (r: { col: number; row: number }) => r.col >= 0 && r.col < size[1] && r.row >= 0 && r.row < size[0];
  while (i < src.length) {
    const c = src[i]!;
    if (c === '"' || c === "[") {
      const close = c === '"' ? '"' : "]";
      const s = i++;
      while (i < src.length) {
        if (src[i] === close) {
          if (src[i + 1] === close) {
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      run += src.slice(s, i);
      keep += src.slice(s, i);
      word = "";
      continue;
    }
    const boundary = i === 0 || !/[\w]/.test(src[i - 1]!);
    if (boundary && /[$A-Za-z]/.test(c)) {
      const a = parse(i);
      if (a) {
        let end = i + a.len;
        let b: typeof a | null = null;
        if (src[end] === ":") {
          b = parse(end + 1);
          if (b) end += 1 + b.len;
        }
        if (!fits(a) || (b && !fits(b))) return null;
        if (!b) {
          const t = cellAt(a.row, a.col);
          run += t ? quote(t) : "BLANK()";
          keep += show(a);
        } else {
          const numbersOnly = ["SUM", "AVERAGE", "MIN", "MAX"].includes(open[open.length - 1] ?? "");
          const parts: string[] = [];
          for (let r = Math.min(a.row, b.row); r <= Math.max(a.row, b.row); r++)
            for (let k = Math.min(a.col, b.col); k <= Math.max(a.col, b.col); k++) {
              const t = cellAt(r, k);
              if (!t) continue;
              if (numbersOnly && !/^\s*[-+]?(\d+\.?\d*|\.\d+)\s*$/.test(t)) continue;
              parts.push(quote(t));
            }
          run += parts.length ? parts.join(",") : "BLANK()";
          keep += `${show(a)}:${show(b)}`;
        }
        i = end;
        word = "";
        continue;
      }
    }
    if (/[\w.]/.test(c)) word += c;
    else {
      if (c === "(") open.push(word.toUpperCase());
      else if (c === ")") open.pop();
      word = "";
    }
    run += c;
    keep += c;
    i++;
  }
  return { run, keep };
}

export interface FormulaHelp {
  /** Functions (or columns, after "[") that fit what is being typed. */
  suggest: { label: string; insert: string; hint: string }[];
  /** Where the typed word starts (the suggestion replaces from here). */
  from: number;
  /** The function the cursor is inside, with how it is used. */
  hint: string;
}

/** Help for the formula being typed: what fits the word before the cursor,
 *  and how the function around the cursor is used. */
export function formulaHelp(
  text: string,
  caret: number,
  funcs: { name: string; hint: string }[],
  columns: string[] = [],
): FormulaHelp {
  const before = text.slice(0, caret);
  // Inside text in quotes: no help.
  const quotes = (before.match(/"/g) ?? []).length;
  if (quotes % 2 === 1) return { suggest: [], from: caret, hint: "" };
  const col = /\[([^\]]*)$/.exec(before);
  let suggest: FormulaHelp["suggest"] = [];
  let from = caret;
  if (col) {
    const w = col[1]!.toLowerCase();
    from = caret - col[0].length;
    suggest = columns
      .map((t, i) => ({ t, i }))
      .filter(({ t }) => t.toLowerCase().includes(w))
      .slice(0, 8)
      .map(({ t, i }) => {
        const ref = t && !/[\]]/.test(t) ? `[${t}]` : `[#${i + 1}]`;
        return { label: ref, insert: ref, hint: "" };
      });
  } else {
    const m = /([A-Za-z][A-Za-z0-9]*)$/.exec(before);
    if (m && !/[\d.]$/.test(before.slice(0, before.length - m[0].length))) {
      const w = m[1]!.toUpperCase();
      from = caret - m[0].length;
      suggest = funcs
        .filter((f) => f.name.startsWith(w) && f.name !== w)
        .slice(0, 8)
        .map((f) => ({ label: f.name, insert: `${f.name}(`, hint: f.hint }));
    }
  }
  // The innermost function call still open at the cursor.
  let depth = 0;
  let hint = "";
  let inText = false;
  for (let i = before.length - 1; i >= 0; i--) {
    const c = before[i]!;
    if (c === '"') inText = !inText;
    if (inText) continue;
    if (c === ")") depth++;
    else if (c === "(") {
      if (depth === 0) {
        const name = /([A-Za-z][A-Za-z0-9]*)$/.exec(before.slice(0, i))?.[1]?.toUpperCase();
        hint = funcs.find((f) => f.name === name)?.hint ?? "";
        break;
      }
      depth--;
    }
  }
  return { suggest, from, hint };
}
