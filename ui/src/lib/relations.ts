// Data › Find relationships: the shapes of the engine's `relations` module
// and the words made from them.

import type { Check } from "./backend";

export const MAX_RELATION_COLS = 40;

export interface RelationsSpec {
  cols: number[];
  header: boolean;
  min_percent: number;
  seconds: number;
}
export interface Holds {
  coverage: number;
  agreeing: number;
  values: number;
  broken_values: number;
  percent: number;
}
export type RelationKind = "depends" | "one_to_one" | "co_blank";
export interface Finding {
  kind: RelationKind;
  a: number;
  b: number;
  there: Holds;
  back: Holds | null;
  exact: boolean;
  few_rows: boolean;
  few_per_value: boolean;
}
export interface RelationsReport {
  findings: Finding[];
  left_out: [number, "constant" | "unique"][];
  mostly_one: [number, number][];
  rows: number;
  sampled: number;
  unchecked: number;
}

/** Rounded down, so 99.96 never reads as 100. */
export function percentText(p: number): string {
  return `${(Math.floor(p * 10 + 1e-9) / 10).toFixed(1)}%`;
}

const n = (x: number) => x.toLocaleString();

/** One finding in words: titles and figures, never a value. */
export function findingText(f: Finding, title: (c: number) => string): string {
  const a = title(f.a);
  const b = title(f.b);
  const t = f.there;
  if (f.kind === "co_blank") return `${a} and ${b} are blank together in ${percentText(t.percent)} of the rows where either is blank (${n(t.agreeing)} of ${n(t.coverage)})`;
  const one = (from: string, to: string, h: Holds) => `${from} settles ${to} in ${percentText(h.percent)} (${n(h.agreeing)} of ${n(h.coverage)} rows whose ${from} repeats; ${n(h.broken_values)} ${h.broken_values === 1 ? "value breaks" : "values break"} it)`;
  if (f.kind === "one_to_one" && f.back) return `${a} <-> ${b} (one to one): ${one(a, b, t)}; ${one(b, a, f.back)}`;
  return `${a} -> ${b}: holds in ${percentText(t.percent)} (${n(t.agreeing)} of ${n(t.coverage)} rows whose ${a} repeats; ${n(t.broken_values)} ${t.broken_values === 1 ? "value breaks" : "values break"} it)`;
}

/** The cautions that go with a finding. */
export function cautions(f: Finding): string[] {
  const out: string[] = [];
  if (!f.exact) out.push("estimated from a sample");
  if (f.few_rows) out.push("based on few rows");
  if (f.few_per_value) out.push("few rows per value");
  return out;
}

export function breaks(f: Finding): boolean {
  return f.there.agreeing < f.there.coverage || (!!f.back && f.back.agreeing < f.back.coverage);
}

/** A column in an expression: its title when no other column has it, else its number. */
function ref(c: number, titles: string[], header: boolean): string {
  const t = titles[c] ?? "";
  const once = header && t !== "" && titles.filter((x) => x === t).length === 1;
  return `[${once ? t.replace(/]/g, "]]") : `#${c + 1}`}]`;
}

/** The data checks a finding becomes: "B settled by A" (both ways for one to one), or for
 *  blank together a row rule that counts blank the way the scan does (spaces only is blank). */
export function checksFor(f: Finding, titles: string[], header: boolean): Check[] {
  const title = (c: number) => titles[c] || `Column ${c + 1}`;
  const settled = (col: number, by: number): Check => ({ col, name: `${title(col)} settled by ${title(by)}`, kind: { type: "settled_by", by, match_case: true } });
  if (f.kind === "co_blank") {
    const e = `ISBLANK(TRIM(${ref(f.a, titles, header)})) = ISBLANK(TRIM(${ref(f.b, titles, header)}))`;
    return [{ col: null, name: `${title(f.a)} and ${title(f.b)} blank together`, kind: { type: "expression", expr: e } }];
  }
  if (f.kind === "one_to_one") return [settled(f.b, f.a), settled(f.a, f.b)];
  return [settled(f.b, f.a)];
}

/** The same rule (kind, columns and settings; the name aside). */
function same(a: Check, b: Check): boolean {
  return a.col === b.col && JSON.stringify(a.kind) === JSON.stringify(b.kind);
}

/** `existing` with `made` added, leaving out those already there. */
export function addChecks(existing: Check[], made: Check[]): { checks: Check[]; added: Check[]; already: number } {
  const added = made.filter((m) => !existing.some((e) => same(e, m)));
  return { checks: [...existing, ...added], added, already: made.length - added.length };
}

/** The rows a finding says break it (both directions for one to one; for blank together, blank in one only). */
export function brokenRows(f: Finding): number {
  if (f.kind === "co_blank") return f.there.coverage - f.there.agreeing;
  return f.there.coverage - f.there.agreeing + (f.back ? f.back.coverage - f.back.agreeing : 0);
}

/** What the status line says once the new rules ran: `found` rows break them. The scan's
 *  figure is only matched when it counted every row; otherwise both are said. */
export function addedText(f: Finding, names: string[], already: number, found: number | null): string {
  const what = names.length === 1 ? `the check "${names[0]}"` : `the checks ${names.map((n) => `"${n}"`).join(" and ")}`;
  if (!names.length) return already === 1 ? "That check is already in Data checks." : "Those checks are already in Data checks.";
  const n = (x: number) => x.toLocaleString();
  const rows = (x: number) => `${n(x)} ${x === 1 ? "row breaks" : "rows break"}`;
  const count = found === null ? "" : f.exact ? `: ${rows(found)} ${names.length === 1 ? "it" : "them"}` : `: ${rows(found)} ${names.length === 1 ? "it" : "them"} (the scan estimated ${n(brokenRows(f))})`;
  const was = already ? ` (${already === 1 ? "1 was" : `${already} were`} already there)` : "";
  return `Added ${what}${count}${was}. Data › Data checks lists them; F7 goes to the next problem.`;
}

/** The findings as lines to copy. */
export function copyText(r: RelationsReport, title: (c: number) => string): string {
  return r.findings.map((f) => `${findingText(f, title)}${cautions(f).map((c) => ` [${c}]`).join("")}\n`).join("");
}
