// File contracts (C9) for the browser stand-in: the same rules as the engine's contract.rs, on
// small tables (titles and rows given as text). The desktop app does this in Rust.

import type { Contract, DriftReport, ContractColumn } from "./backend";

export const TITLE_SHARE = 0.8;

const key = (t: string): string => t.trim().toLowerCase();

/** `*` any run, `?` one character, ignoring case. */
export function patternMatches(pattern: string, name: string): boolean {
  const esc = pattern
    .toLowerCase()
    .split("")
    .map((c) => (c === "*" ? ".*" : c === "?" ? "." : c.replace(/[.+^${}()|[\]\\]/g, "\\$&")))
    .join("");
  return new RegExp(`^${esc}$`, "s").test(name.toLowerCase());
}

export function titleShare(c: Contract, titles: string[]): number {
  if (!c.columns.length) return 0;
  const keys = new Set(titles.map(key));
  const found = c.columns.filter((col) => keys.has(key(col.title)) || (col.aliases ?? []).some((a) => keys.has(key(a)))).length;
  return found / c.columns.length;
}

/** The contracts a file looks like, best first. */
export function recognise(list: Contract[], file: string | null, titles: string[] | null): { index: number; by_pattern: boolean; title_share: number }[] {
  const out: { index: number; by_pattern: boolean; title_share: number }[] = [];
  list.forEach((c, index) => {
    const by_pattern = !!(c.recognise.pattern && file && patternMatches(c.recognise.pattern, file));
    const share = titles ? titleShare(c, titles) : 0;
    if (by_pattern || (c.recognise.titles && titles && share >= TITLE_SHARE)) out.push({ index, by_pattern, title_share: share });
  });
  const score = (m: { by_pattern: boolean; title_share: number }) => (m.by_pattern ? 1 : 0) + m.title_share;
  return out.sort((a, b) => score(b) - score(a));
}

export interface MockPairing {
  map: (number | null)[];
  renamed: number[];
  by_alias: number[];
  missing: number[];
  added: number[];
  moved: number[];
}

/** Exact titles, then other titles, then by place (one unknown column in a gap only). */
export function pair(c: Contract, titles: string[]): MockPairing {
  const queue = new Map<string, number[]>();
  titles.forEach((t, j) => queue.set(key(t), [...(queue.get(key(t)) ?? []), j]));
  const take = (t: string): number | null => queue.get(key(t))?.shift() ?? null;
  const map = c.columns.map((col) => take(col.title));
  const by_alias: number[] = [];
  c.columns.forEach((col, k) => {
    if (map[k] !== null) return;
    for (const a of col.aliases ?? []) {
      const j = take(a);
      if (j !== null) {
        map[k] = j;
        by_alias.push(k);
        break;
      }
    }
  });
  const taken = new Set(map.filter((j): j is number => j !== null));
  const renamed: number[] = [];
  const n = c.columns.length;
  for (let k = 0; k < n; ) {
    if (map[k] !== null) {
      k++;
      continue;
    }
    const start = k;
    while (k < n && map[k] === null) k++;
    if (k - start !== 1) continue;
    const lo = start === 0 ? 0 : map[start - 1]! + 1;
    const hi = k === n ? titles.length : map[k]!;
    if (lo > hi) continue;
    const free: number[] = [];
    for (let j = lo; j < hi; j++) if (!taken.has(j)) free.push(j);
    if (free.length === 1) {
      map[start] = free[0]!;
      taken.add(free[0]!);
      renamed.push(start);
    }
  }
  const missing = map.flatMap((j, k) => (j === null ? [k] : []));
  const added = titles.flatMap((_, j) => (taken.has(j) ? [] : [j]));
  // Moved: paired columns outside the longest rising run of their places.
  const paired = map.flatMap((j, k) => (j === null ? [] : [[k, j] as [number, number]]));
  const len: number[] = [];
  const prev: number[] = [];
  paired.forEach(([, j], i) => {
    len[i] = 1;
    prev[i] = -1;
    for (let h = 0; h < i; h++) if (paired[h]![1] < j && len[h]! + 1 > len[i]!) [len[i], prev[i]] = [len[h]! + 1, h];
  });
  const keep = new Set<number>();
  let at = len.length ? len.indexOf(Math.max(...len)) : -1;
  while (at >= 0) {
    keep.add(at);
    at = prev[at]!;
  }
  const moved = paired.flatMap(([k], i) => (keep.has(i) ? [] : [k]));
  return { map, renamed, by_alias, missing, added, moved };
}

const misses = (col: ContractColumn, v: string): boolean => {
  const ty = col.ty;
  if (ty === "number") return !/^[-+]?(\d[\d,]*)(\.\d+)?$/.test(v.replace(/[$€£\s]/g, ""));
  if (ty && typeof ty === "object" && "date" in ty) return Number.isNaN(Date.parse(v));
  return false;
};

const isNew = (col: ContractColumn, v: string): boolean => {
  const a = col.allowed;
  if (!a) return false;
  return a.match_case ? !a.values.some((x) => x.trim() === v) : !a.values.some((x) => x.trim().toLowerCase() === v.toLowerCase());
};

/** What a cell is, for "show these rows". */
export function cellHits(col: ContractColumn, kind: "blanks" | "type_misses" | "new_values", raw: string): boolean {
  const v = raw.trim();
  if (kind === "blanks") return v === "";
  if (v === "") return false;
  return kind === "type_misses" ? misses(col, v) : isNew(col, v);
}

/** The drift report for a table given as its titles and data rows. */
export function drift(c: Contract, p: MockPairing, titles: string[], rows: string[][]): DriftReport {
  const total = rows.length;
  let drifted = false;
  const columns = p.map.flatMap((j, k) => {
    if (j === null) return [];
    const col = c.columns[k]!;
    let blanks = 0;
    let type_misses = 0;
    let new_values = 0;
    const first_new: string[] = [];
    for (const r of rows) {
      const v = (r[j] ?? "").trim();
      if (!v) {
        blanks++;
        continue;
      }
      if (misses(col, v)) type_misses++;
      if (isNew(col, v)) {
        new_values++;
        if (first_new.length < 5 && !first_new.includes(v)) first_new.push(v);
      }
    }
    const rate = total ? blanks / total : 0;
    const blank_jump = col.blank_rate !== null && col.blank_rate !== undefined && Math.abs(rate - col.blank_rate) * 100 > c.blank_points;
    drifted ||= blank_jump || type_misses > 0 || new_values > 0;
    return [{ col: k, file_col: j, title: col.title, blanks, blank_rate: rate, baseline: col.blank_rate ?? null, blank_jump, type_misses, new_values, first_new }];
  });
  const renamed = p.map.flatMap((j, k) =>
    j !== null && (p.renamed.includes(k) || p.by_alias.includes(k)) ? [{ col: k, file_col: j, from: titles[j] ?? "", to: c.columns[k]!.title, by_alias: p.by_alias.includes(k) }] : [],
  );
  const missing = p.missing.map((k) => c.columns[k]!.title);
  const added = p.added.map((j) => titles[j] ?? "");
  const moved = p.moved.map((k) => c.columns[k]!.title);
  const rows_out = !!c.rows && (total < c.rows.min || total > c.rows.max);
  drifted ||= renamed.some((r) => !r.by_alias) || missing.length > 0 || added.length > 0 || moved.length > 0 || rows_out;
  return { contract: c.name, rows: total, row_range: c.rows ?? null, rows_out, renamed, missing, added, moved, columns, rules: [], not_run: [], table: [], drifted };
}

/** The same refusals as the engine's `check_name`. */
export function nameProblem(name: string): string | null {
  if (!name.trim()) return "a contract needs a name";
  if ([...name].length > 100) return "a contract name can be at most 100 characters";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(name)) return "a contract name cannot hold a control character";
  const bad = [...name].find((ch) => '/\\:*?"<>|'.includes(ch));
  if (bad) return `a contract name cannot hold ${bad}`;
  if (/[. ]$/.test(name)) return "a contract name cannot end with a dot or a space";
  const stem = name.split(".")[0]!.trimEnd().toUpperCase();
  if (/^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/.test(stem)) return `${stem} is a name Windows keeps for itself: choose another`;
  return null;
}
