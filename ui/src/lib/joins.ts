// Joins (Milestone 11): the option lists shown in the Join screen, the
// summary written after a join, and a small reference join over plain
// arrays that the browser stub backend uses (the real one is the engine's).

import type { JoinPreview, JoinReport, JoinSpec } from "./backend";

export type JoinWrite = JoinSpec["write"];
export type KeepRows = JoinSpec["keep_rows"];
export type Repeats = JoinSpec["repeats"];
export type KeyMatch = JoinSpec["key_match"];
export type Place = JoinSpec["place"];

/** How keys are compared, as one choice in the screen. */
export type Compare = "exact" | "loose" | "below" | "above";

export const WRITES: [JoinWrite, string][] = [
  ["new", "Add the values as new columns"],
  ["overwrite", "Update columns of this table (replace what is there)"],
  ["fill_blanks", "Fill only the blank cells of columns of this table"],
  ["nothing", "Don't bring in values (only keep or drop rows)"],
];

export const KEEPS: [KeepRows, string][] = [
  ["all", "Keep every row"],
  ["matched", "Keep only rows that found a match"],
  ["unmatched", "Keep only rows that found no match"],
];

export const REPEATS: [Repeats, string][] = [
  ["first", "Use the first match"],
  ["last", "Use the last match"],
  ["every", "Every match as its own row"],
  ["joined", "All matches in one cell"],
  ["count", "Count the matches (one column)"],
  ["sum", "Add up the numbers of the matches"],
  ["min", "The smallest number of the matches"],
  ["max", "The largest number of the matches"],
];

export const COMPARES: [Compare, string][] = [
  ["exact", "Exactly"],
  ["loose", "Loosely: ignore spaces, punctuation and leading zeros; 1.0 = 1"],
  ["below", "Nearest smaller or equal value (age bands, price tiers, dates)"],
  ["above", "Nearest larger or equal value"],
];

export function compareOf(s: Pick<JoinSpec, "loose" | "key_match">): Compare {
  if (s.key_match !== "exact") return s.key_match;
  return s.loose ? "loose" : "exact";
}

export function withCompare(c: Compare): Pick<JoinSpec, "loose" | "key_match"> {
  if (c === "below" || c === "above") return { loose: false, key_match: c };
  return { loose: c === "loose", key_match: "exact" };
}

export function defaultSpec(): JoinSpec {
  return {
    keys: [0],
    other_keys: [0],
    columns: [],
    keep_first: true,
    other_header: true,
    match_case: false,
    trim: true,
    add_unmatched: false,
    write: "new",
    targets: [],
    keep_rows: "all",
    repeats: "first",
    separator: "; ",
    loose: false,
    not_found: "",
    place: "end",
    prefix: "",
    key_match: "exact",
  };
}

/** The values a join brings in: one per column, or one count. */
export function valueCount(spec: Pick<JoinSpec, "repeats">, columns: number): number {
  return spec.repeats === "count" ? 1 : columns;
}

/** For "update", this table's column for each value: same name, else none. */
export function guessTargets(mine: string[], theirs: string[], cols: number[], keys: number[]): number[] {
  const low = mine.map((t) => t.trim().toLowerCase());
  return cols.map((c) => {
    const i = low.indexOf((theirs[c] ?? "").trim().toLowerCase());
    return i >= 0 && !keys.includes(i) ? i : -1;
  });
}

const n = (v: number, what: string) => `${v.toLocaleString()} ${what}${v === 1 ? "" : "s"}`;

/** The message after a join (or the counts line of the preview). */
export function joinCounts(r: JoinReport, spec: Pick<JoinSpec, "repeats" | "keep_rows" | "key_match">): string {
  let s = `${n(r.matched, "row")} matched, ${n(r.unmatched, "row")} found no match`;
  if (r.blank_keys) s += `, ${n(r.blank_keys, "row")} with a blank key`;
  s += ".";
  const did: string[] = [];
  if (r.cols) did.push(`${n(r.cols, "column")} added`);
  if (r.changed) did.push(`${n(r.changed, "cell")} filled in or changed`);
  if (r.removed) did.push(`${n(r.removed, "row")} taken out`);
  if (r.extra_rows) did.push(`${n(r.extra_rows, "extra row")} for repeated matches`);
  if (r.added) did.push(`${n(r.added, "row")} only the other table had added at the end`);
  if (did.length) s += ` ${did.join(", ")}.`;
  else s += " Nothing would change.";
  if (r.repeated_keys && spec.repeats === "first") s += ` The other table repeats ${n(r.repeated_keys, "key")}; the first of each is used.`;
  if (r.repeated_keys && spec.repeats === "last") s += ` The other table repeats ${n(r.repeated_keys, "key")}; the last of each is used.`;
  if (r.bad_keys) s += ` ${n(r.bad_keys, "key")} of the other table ${r.bad_keys === 1 ? "is" : "are"} not a number or date and ${r.bad_keys === 1 ? "was" : "were"} skipped.`;
  return s;
}

/** Warnings worth a second look before running. */
export function joinWarnings(r: JoinReport, rows: number): string[] {
  const w: string[] = [];
  if (r.extra_rows > 100_000 || (rows > 0 && r.extra_rows > rows)) w.push(`Every match as its own row adds ${r.extra_rows.toLocaleString()} rows.`);
  if (r.removed > 0 && r.removed >= rows && rows > 0) w.push("Every row would be taken out.");
  if (r.matched === 0) w.push("No keys match. Check the key columns and how keys are compared.");
  return w;
}

// ---- reference join over arrays (browser stub backend) ----

function decimal(t: string): string | null {
  if (/^[+-]?\d{1,3}(,\d{3})+(\.\d*)?$/.test(t)) t = t.replace(/,/g, "");
  const m = /^([+-]?)(\d*)(?:\.(\d*))?$/.exec(t);
  if (!m || (m[2] === "" && (m[3] ?? "") === "")) return null;
  const int = m[2]!.replace(/^0+/, "");
  const frac = (m[3] ?? "").replace(/0+$/, "");
  if (!int && !frac) return "0";
  return `${m[1] === "-" ? "-" : ""}${int || "0"}${frac ? `.${frac}` : ""}`;
}

/** One key part as the engine compares it. */
export function keyPart(p: string, s: Pick<JoinSpec, "loose" | "match_case" | "trim">): string {
  if (!s.loose) {
    const t = s.trim ? p.trim() : p;
    return s.match_case ? t : t.toLowerCase();
  }
  const t = p.trim();
  const d = decimal(t);
  if (d !== null) return d;
  // Runs of letters and digits; a boundary stays between two runs of
  // digits ("12-3" is not "1-23"); leading zeros of digit runs go.
  const toks = t.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  let out = "";
  let prevDigit = false;
  for (const tok of toks) {
    if (prevDigit && /^[0-9]/.test(tok)) out += "|";
    out += tok.replace(/(^|[^0-9])0+(?=[0-9])/g, "$1");
    prevDigit = /[0-9]$/.test(tok);
  }
  return s.match_case ? out : out.toLowerCase();
}

function rangeValue(s: string): number | null {
  const t = s.trim();
  if (/^[+-]?(\d+\.?\d*|\.\d+)$/.test(t)) return Number(t);
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(t);
  if (m) return Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) / 86_400_000;
  return null;
}

function aggregate(rule: Repeats, vals: string[], sep: string): string {
  if (rule === "joined") return vals.filter((v) => v !== "").join(sep);
  const nums = vals.map((v) => v.trim()).filter((v) => v !== "" && Number.isFinite(Number(v))).map(Number);
  if (!nums.length) return "";
  const r = rule === "sum" ? nums.reduce((a, b) => a + b, 0) : rule === "min" ? Math.min(...nums) : Math.max(...nums);
  return String(Number(r.toPrecision(15)));
}

export interface SimRow {
  kind: "matched" | "unmatched" | "blank" | "removed";
  /** Cells written, by column after the insert. */
  set: [number, string][];
  /** Extra rows right after this one (every match), whole rows. */
  extra: string[][];
}

export interface SimResult {
  insertAt: number;
  insertN: number;
  titles: string[];
  rows: SimRow[];
  append: string[][];
  report: JoinReport;
}

/**
 * The join over plain arrays: `mine` and `other` are whole tables (header
 * rows included when the spec says so); `list` replaces `other` with keys.
 */
export function simulateJoin(mine: string[][], other: string[][] | null, spec: JoinSpec, list: string | null): SimResult {
  const nk = spec.keys.length;
  const range = spec.key_match !== "exact";
  let oRows: string[][];
  let cols: number[];
  let oTitles: string[];
  if (list !== null) {
    oRows = list.split(/\r?\n/).filter((l, i, a) => l !== "" || i < a.length - 1).map((l) => {
      const p = l.split("\t").slice(0, nk);
      while (p.length < nk) p.push("");
      return p;
    });
    cols = [];
    oTitles = [];
  } else {
    const o = other ?? [];
    const width = Math.max(0, ...o.map((r) => r.length));
    cols = spec.columns.length ? spec.columns : Array.from({ length: width }, (_, i) => i).filter((c) => !spec.other_keys.includes(c));
    oTitles = cols.map((c) => (spec.other_header ? o[0]?.[c] : "") || `Column ${c + 1}`);
    oRows = o.slice(spec.other_header ? 1 : 0).map((r) => [...spec.other_keys.map((c) => r[c] ?? ""), ...cols.map((c) => r[c] ?? "")]);
  }
  const report: JoinReport = { matched: 0, unmatched: 0, added: 0, repeated_keys: 0, cols: 0, dropped_cols: 0, blank_keys: 0, changed: 0, removed: 0, extra_rows: 0, bad_keys: 0 };
  const groups: number[][] = [];
  const lookup = new Map<string, number>();
  const sorted: [number, number][] = [];
  oRows.forEach((r, i) => {
    if (r.slice(0, nk).every((p) => p.trim() === "")) return;
    if (!range && spec.loose && r.slice(0, nk).every((p) => keyPart(p, spec) === "")) return;
    let key: string;
    if (range) {
      const v = rangeValue(r[0] ?? "");
      if (v === null) {
        report.bad_keys++;
        return;
      }
      key = String(v);
      if (!lookup.has(key)) sorted.push([v, groups.length]);
    } else key = JSON.stringify(r.slice(0, nk).map((p) => keyPart(p, spec)));
    const g = lookup.get(key);
    if (g === undefined) {
      lookup.set(key, groups.length);
      groups.push([i]);
    } else {
      report.repeated_keys++;
      groups[g]!.push(i);
    }
  });
  sorted.sort((a, b) => a[0] - b[0]);
  const find = (parts: string[]): number | "blank" | null => {
    if (parts.every((p) => p.trim() === "")) return "blank";
    if (!range && spec.loose && parts.every((p) => keyPart(p, spec) === "")) return "blank";
    if (!range) return lookup.get(JSON.stringify(parts.map((p) => keyPart(p, spec)))) ?? null;
    const v = rangeValue(parts[0] ?? "");
    if (v === null) return null;
    if (spec.key_match === "below") {
      const hit = sorted.filter((x) => x[0] <= v).pop();
      return hit ? hit[1] : null;
    }
    const hit = sorted.find((x) => x[0] >= v);
    return hit ? hit[1] : null;
  };
  const titlesOut = spec.repeats === "count" ? ["Matches"] : oTitles;
  const w = titlesOut.length;
  const values = (g: number): string[] => {
    const ms = groups[g]!;
    const at = (m: number) => oRows[m]!.slice(nk);
    switch (spec.repeats) {
      case "first":
      case "every":
        return at(ms[0]!);
      case "last":
        return at(ms[ms.length - 1]!);
      case "count":
        return [String(ms.length)];
      default:
        return Array.from({ length: w }, (_, k) => aggregate(spec.repeats, ms.map((m) => oRows[m]![nk + k] ?? ""), spec.separator));
    }
  };
  const writes = spec.write !== "nothing" && spec.keep_rows !== "unmatched";
  const update = spec.write === "overwrite" || spec.write === "fill_blanks";
  const myWidth = Math.max(0, ...mine.map((r) => r.length));
  const insert = writes && spec.write === "new";
  const insertAt = insert ? (spec.place === "end" ? myWidth : Math.max(...spec.keys) + 1) : myWidth;
  const insertN = insert ? w : 0;
  const shift = (c: number) => (c >= insertAt ? c + insertN : c);
  const dest = !writes ? [] : update ? spec.targets : Array.from({ length: w }, (_, k) => insertAt + k);
  const cellsFor = (vals: string[], cur: (k: number) => string): [number, string][] => {
    const out: [number, string][] = [];
    dest.forEach((c, k) => {
      const v = vals[k] ?? "";
      if (v === "") return;
      const ok = spec.write === "new" || (spec.write === "overwrite" ? cur(k) !== v : cur(k).trim() === "" && cur(k) !== v);
      if (ok) out.push([c, v]);
    });
    return out;
  };
  report.cols = insertN;
  const used = new Set<number>();
  const start = spec.keep_first && mine.length ? 1 : 0;
  const rows: SimRow[] = [];
  for (let r = start; r < mine.length; r++) {
    const row = mine[r]!;
    const hit = find(spec.keys.map((c) => row[c] ?? ""));
    const matched = typeof hit === "number";
    if (hit === "blank") report.blank_keys++;
    else if (matched) {
      report.matched++;
      used.add(hit);
    } else report.unmatched++;
    const remove = spec.keep_rows === "matched" ? !matched : spec.keep_rows === "unmatched" ? matched : false;
    const out: SimRow = { kind: remove ? "removed" : hit === "blank" ? "blank" : matched ? "matched" : "unmatched", set: [], extra: [] };
    if (remove) report.removed++;
    else if (writes && matched) {
      const cur = (k: number) => row[dest[k]!] ?? "";
      out.set = cellsFor(values(hit), cur);
      if (spec.repeats === "every") {
        const base: string[] = [];
        row.forEach((v, c) => (base[shift(c)] = v));
        for (const m of groups[hit]!.slice(1)) {
          const copy = Array.from({ length: myWidth + insertN }, (_, c) => base[c] ?? "");
          for (const [c, v] of cellsFor(oRows[m]!.slice(nk), (k) => copy[dest[k]!] ?? "")) copy[c] = v;
          out.extra.push(copy);
        }
      }
    } else if (writes && insertN > 0 && spec.not_found !== "") out.set = dest.map((c) => [c, spec.not_found]);
    report.changed += out.set.length;
    report.extra_rows += out.extra.length;
    rows.push(out);
  }
  const append: string[][] = [];
  if (spec.add_unmatched && !range && writes) {
    groups.forEach((ms, g) => {
      if (used.has(g)) return;
      for (const m of spec.repeats === "every" ? ms : [ms[0]!]) {
        const line = Array.from({ length: myWidth + insertN }, () => "");
        spec.keys.forEach((c, k) => (line[shift(c)] = oRows[m]![k] ?? ""));
        const vals = spec.repeats === "every" ? oRows[m]!.slice(nk) : values(g);
        dest.forEach((c, k) => (line[c] = vals[k] ?? ""));
        append.push(line);
      }
    });
  }
  report.added = append.length;
  const mineTitles = Array.from({ length: myWidth }, (_, c) => (spec.keep_first ? mine[0]?.[c] : "") || `Column ${c + 1}`);
  const titles: string[] = [];
  mineTitles.forEach((t, c) => (titles[shift(c)] = t));
  for (let k = 0; k < insertN; k++) titles[insertAt + k] = `${spec.prefix}${titlesOut[k]}`;
  return { insertAt, insertN, titles, rows, append, report };
}

/** The preview the engine would give, from a simulated join. */
export function previewOf(mine: string[][], sim: SimResult, spec: JoinSpec, limit = 200): JoinPreview {
  const width = sim.titles.length;
  const start = spec.keep_first && mine.length ? 1 : 0;
  const columns = spec.write === "new" ? Array.from({ length: sim.insertN }, (_, k) => sim.insertAt + k) : spec.write === "nothing" ? [] : [...spec.targets];
  const rows: JoinPreview["rows"] = [];
  sim.rows.slice(0, limit).forEach((o, i) => {
    const values = Array.from({ length: width }, () => "");
    (mine[start + i] ?? []).forEach((v, c) => (values[c >= sim.insertAt ? c + sim.insertN : c] = v));
    const changed: number[] = [];
    if (o.kind !== "removed")
      for (const [c, v] of o.set) {
        values[c] = v;
        changed.push(c);
      }
    rows.push({ kind: o.kind, row: start + i + 1, values, changed });
    for (const e of o.extra) rows.push({ kind: "extra", row: null, values: e, changed: columns });
  });
  for (const a of sim.append.slice(0, limit)) rows.push({ kind: "added", row: null, values: a, changed: columns });
  return { titles: sim.titles, columns, rows, report: sim.report };
}
