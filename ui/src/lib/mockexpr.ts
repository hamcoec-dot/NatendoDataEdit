// A small stand-in for the engine's expressions, for the browser preview
// only (the desktop app evaluates in Rust). Knows numbers, text, [Column]
// and [#n], + - * / & and comparisons, and a few functions.

import { shapeOf } from "./profile";

type V = string | number | boolean;
/** An argument of IFERROR that came out as an error. */
const ERR = "\u0000#error";

const FUNCS: Record<string, (a: V[]) => V> = {
  UPPER: (a) => String(a[0] ?? "").toUpperCase(),
  LOWER: (a) => String(a[0] ?? "").toLowerCase(),
  LEN: (a) => Array.from(String(a[0] ?? "")).length,
  TRIM: (a) => String(a[0] ?? "").trim().replace(/ +/g, " "),
  CONCAT: (a) => a.map(String).join(""),
  IF: (a) => (truthy(a[0]!) ? (a[1] ?? true) : (a[2] ?? false)),
  ROUND: (a) => {
    const f = 10 ** Number(a[1] ?? 0);
    return Math.round(toNum(a[0]!) * f) / f;
  },
  ISBLANK: (a) => a[0] === "",
  BLANK: () => "",
  SHAPE: (a) => shapeOf(String(a[0] ?? "")),
  AND: (a) => a.every(truthy),
  OR: (a) => a.some(truthy),
  NOT: (a) => !truthy(a[0]!),
  SUM: (a) => a.filter((v) => v !== "").reduce<number>((s, v) => s + toNum(v), 0),
  AVERAGE: (a) => {
    const v = a.filter((x) => x !== "");
    if (!v.length) throw new Error("#DIV/0!");
    return v.reduce<number>((s, x) => s + toNum(x), 0) / v.length;
  },
  MIN: (a) => Math.min(...a.filter((v) => v !== "").map(toNum)),
  MAX: (a) => Math.max(...a.filter((v) => v !== "").map(toNum)),
  COUNTA: (a) => a.filter((v) => v !== "").length,
  PADLEFT: (a) => String(a[0] ?? "").padStart(toNum(a[1] ?? 0), String(a[2] ?? "0")),
  LEFT: (a) => Array.from(String(a[0] ?? ""))
    .slice(0, toNum(a[1] ?? 1))
    .join(""),
  PROPER: (a) => String(a[0] ?? "").replace(/[\p{L}\p{N}]+/gu, (w) => w[0]!.toUpperCase() + w.slice(1).toLowerCase()),
  REGEXEXTRACT: (a) => {
    const src = String(a[1] ?? "");
    let re: RegExp;
    try {
      re = new RegExp(src, "su");
    } catch {
      re = new RegExp(src, "s");
    }
    const m = re.exec(String(a[0] ?? ""));
    const g = m?.[toNum(a[2] ?? 0)];
    if (g === undefined) throw new Error("#N/A");
    return g;
  },
  IFERROR: (a) => (a[0] === ERR ? (a[1] ?? "") : (a[0] ?? "")),
  TEXTBEFORE: (a) => {
    const s = String(a[0] ?? "");
    const i = s.indexOf(String(a[1] ?? ""));
    if (i < 0) throw new Error("#N/A");
    return s.slice(0, i);
  },
};

function toNum(v: V): number {
  if (typeof v === "number") return v;
  if (typeof v === "boolean") return v ? 1 : 0;
  if (v.trim() === "") return 0;
  const n = Number(v.replace(/[$,]/g, ""));
  if (Number.isNaN(n)) throw new Error("#VALUE!");
  return n;
}
function truthy(v: V): boolean {
  return typeof v === "boolean" ? v : typeof v === "number" ? v !== 0 : v.toLowerCase() === "true";
}
function text(v: V): string {
  if (typeof v === "boolean") return v ? "TRUE" : "FALSE";
  if (typeof v === "number") return String(Math.round(v * 1e12) / 1e12);
  return v;
}

/** Evaluate `src` for one row; errors come back as their code. */
export function mockEval(src: string, names: string[], cells: string[]): string {
  const s = src.replace(/^\s*=/, "");
  let i = 0;
  const ws = () => {
    while (i < s.length && /\s/.test(s[i]!)) i++;
  };
  const fail = (m: string): never => {
    throw new SyntaxError(`${m} (at character ${i + 1})`);
  };
  const atom = (): V => {
    ws();
    const c = s[i];
    if (c === undefined) return fail("the expression ends too soon");
    if (c === "(") {
      i++;
      const v = cmp();
      ws();
      if (s[i] !== ")") fail("a ( is missing its )");
      i++;
      return v;
    }
    if (c === "-") {
      i++;
      return -toNum(atom());
    }
    if (c === '"') {
      let out = "";
      i++;
      for (;;) {
        if (i >= s.length) fail("text is missing its closing quote");
        if (s[i] === '"' && s[i + 1] === '"') {
          out += '"';
          i += 2;
        } else if (s[i] === '"') {
          i++;
          return out;
        } else out += s[i++];
      }
    }
    if (c === "[") {
      const end = s.indexOf("]", i);
      if (end < 0) fail("a column name is missing its ]");
      const name = s.slice(i + 1, end);
      i = end + 1;
      const k = name.startsWith("#") ? Number(name.slice(1)) - 1 : names.findIndex((n) => n.toLowerCase() === name.toLowerCase());
      if (!(k >= 0 && k < names.length)) fail(`there is no column named [${name}]`);
      return cells[k] ?? "";
    }
    const m = /^(\d+(\.\d+)?|[A-Za-z_]+)/.exec(s.slice(i));
    if (!m) return fail(`unexpected character '${c}'`);
    i += m[0].length;
    if (/^\d/.test(m[0])) return Number(m[0]);
    const up = m[0].toUpperCase();
    if (up === "TRUE" || up === "FALSE") return up === "TRUE";
    ws();
    const f = FUNCS[up];
    if (!f || s[i] !== "(") fail(`unknown function ${m[0]} (the browser preview knows only ${Object.keys(FUNCS).join(", ")})`);
    i++;
    const args: V[] = [];
    ws();
    if (s[i] === ")") i++;
    else
      for (;;) {
        const start = i;
        try {
          args.push(cmp());
        } catch (e) {
          if (up !== "IFERROR" || args.length || e instanceof SyntaxError) throw e;
          // Skip the rest of the argument that failed, quotes and brackets kept whole.
          let depth = 0;
          for (i = start; i < s.length; i++) {
            const ch = s[i];
            if (ch === '"') {
              i++;
              while (i < s.length && !(s[i] === '"' && s[i + 1] !== '"')) i += s[i] === '"' ? 2 : 1;
            } else if (ch === "[") i = Math.max(i, s.indexOf("]", i));
            else if (ch === "(") depth++;
            else if (ch === ")") {
              if (!depth) break;
              depth--;
            } else if (ch === "," && !depth) break;
          }
          args.push(ERR);
        }
        ws();
        if (s[i] === ",") i++;
        else if (s[i] === ")") {
          i++;
          break;
        } else fail(`${up}( is missing its )`);
      }
    return f!(args);
  };
  const prod = (): V => {
    let l = atom();
    for (;;) {
      ws();
      const o = s[i];
      if (o !== "*" && o !== "/") return l;
      i++;
      const r = toNum(atom());
      if (o === "/" && r === 0) throw new Error("#DIV/0!");
      l = o === "*" ? toNum(l) * r : toNum(l) / r;
    }
  };
  const sum = (): V => {
    let l = prod();
    for (;;) {
      ws();
      const o = s[i];
      if (o !== "+" && o !== "-") return l;
      i++;
      const r = toNum(prod());
      l = o === "+" ? toNum(l) + r : toNum(l) - r;
    }
  };
  const join = (): V => {
    let l = sum();
    for (;;) {
      ws();
      if (s[i] !== "&") return l;
      i++;
      l = text(l) + text(sum());
    }
  };
  const cmp = (): V => {
    let l = join();
    for (;;) {
      ws();
      const m = /^(<>|<=|>=|=|<|>)/.exec(s.slice(i));
      if (!m) return l;
      i += m[0].length;
      const r = join();
      const both = typeof l !== "boolean" && typeof r !== "boolean" && !Number.isNaN(Number(l)) && !Number.isNaN(Number(r)) && String(l) !== "" && String(r) !== "";
      const a: number | string = both ? Number(l) : text(l).toLowerCase();
      const b: number | string = both ? Number(r) : text(r).toLowerCase();
      l = { "=": a === b, "<>": a !== b, "<": a < b, "<=": a <= b, ">": a > b, ">=": a >= b }[m[0]]!;
    }
  };
  try {
    const v = cmp();
    ws();
    if (i < s.length) fail("unexpected text after the expression");
    return text(v);
  } catch (e) {
    if (e instanceof SyntaxError) throw e;
    return e instanceof Error && e.message.startsWith("#") ? e.message : "#VALUE!";
  }
}
