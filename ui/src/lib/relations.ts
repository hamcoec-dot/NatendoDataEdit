// Data › Find relationships: the shapes of the engine's `relations` module
// and the words made from them.

export const MAX_RELATION_COLS = 40;
/** Why "Make this a check" is greyed. */
export const NO_TABLE_CHECKS = "Waits for table checks (a check across two columns), which this version does not have yet.";

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

/** The findings as lines to copy. */
export function copyText(r: RelationsReport, title: (c: number) => string): string {
  return r.findings.map((f) => `${findingText(f, title)}${cautions(f).map((c) => ` [${c}]`).join("")}\n`).join("");
}
