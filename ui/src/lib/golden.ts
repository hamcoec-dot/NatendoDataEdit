// Golden records: fold the accepted pairs of a one-list match into one row
// per person, as a new table. Types shared with the desktop app, and the
// words the screen uses.

import type { DocInfo } from "./backend";

export type GoldenRuleName = "keep" | "newest" | "oldest" | "longest" | "most_common" | "first" | "last" | "union" | "sum" | "min" | "max";

export type GoldenRule =
  | { rule: Exclude<GoldenRuleName, "newest" | "union"> }
  | { rule: "newest"; blank_counts: boolean }
  | { rule: "union"; separator: string };

export interface GoldenSpec {
  rules: { col: number; rule: GoldenRule }[];
  /** The column that names merged rows (null: file row numbers). */
  id_col: number | null;
  /** The date column for newest and oldest. */
  date_col: number | null;
  max_cluster: number;
  require_all: boolean;
  /** Hubs (row ids) of clusters left unmerged. */
  skip: number[];
  keep_first: boolean;
  read: { decimal_comma: boolean; date_order: "mdy" | "dmy" | "ymd" };
}

export interface GoldenCounts {
  clusters: number;
  folded: number;
  hub_only: number;
  skipped: number;
  review: number;
  unmerged_pairs: number;
  flagged: number;
  rows: number;
  hidden: number;
}

export interface GoldenSummary {
  counts: GoldenCounts;
  listed: number;
  never_offered: number;
  review: { name: string; reason: string }[];
}

export interface GoldenCell {
  col: number;
  value: string;
  differ: string[];
  flag: string | null;
}

export interface GoldenCluster {
  hub: number;
  /** What names each member, the hub first. */
  members: string[];
  hub_only: boolean;
  skipped: boolean;
  cells: GoldenCell[];
}

export interface GoldenMade {
  info: DocInfo;
  away: DocInfo | null;
}

export const GOLDEN_PAGE = 200;
/** Review lines the screen lists. */
export const REVIEW_SHOWN = 100;

export const RULE_LABELS: [GoldenRuleName, string][] = [
  ["keep", "Keep the main row's (else the first)"],
  ["first", "First non-blank"],
  ["last", "Last non-blank"],
  ["longest", "Longest"],
  ["most_common", "Most common"],
  ["newest", "From the newest date"],
  ["oldest", "From the oldest date"],
  ["union", "All different values, joined"],
  ["sum", "Add up (exact)"],
  ["min", "Smallest number"],
  ["max", "Largest number"],
];

export function makeRule(name: GoldenRuleName, separator: string, blankCounts: boolean): GoldenRule {
  if (name === "union") return { rule: "union", separator };
  if (name === "newest") return { rule: "newest", blank_counts: blankCounts };
  return { rule: name };
}

/** A column that looks like an identifier, or null. */
export function guessIdColumn(titles: string[]): number | null {
  const i = titles.findIndex((t) => /(^|[\s_-])id($|[\s_-])|identifier|\bnumber\b|\bno\.?$/i.test(t));
  return i < 0 ? null : i;
}

const n = (v: number, one: string, many = `${one}s`): string => `${v.toLocaleString("en-US")} ${v === 1 ? one : many}`;

export function summaryText(c: GoldenCounts): string {
  return `${n(c.clusters, "cluster")} would merge: ${n(c.folded, "row")} folded away, ${n(c.rows, "row")} in the result.`;
}

/** The things to know before creating, in words. */
export function notes(s: GoldenSummary): string[] {
  const c = s.counts;
  const out: string[] = [];
  if (c.flagged) out.push(`${n(c.flagged, "cell")} could not follow its rule, so the first value was kept (highlighted below).`);
  if (c.hub_only) out.push(`${n(c.hub_only, "cluster")} joined through one row only: some members were never accepted with each other.`);
  if (c.review) out.push(`${n(c.review, "row")} left unmerged for you to review.`);
  if (c.unmerged_pairs) out.push(`${n(c.unmerged_pairs, "accepted pair")} not merged.`);
  if (c.skipped) out.push(`${n(c.skipped, "cluster")} you chose not to merge.`);
  if (c.hidden) out.push(`${n(c.hidden, "hidden row")} were not matched and are kept as they are.`);
  if (s.never_offered) out.push(`${n(s.never_offered, "pair")} above the score were never listed on the match screen, so nobody has checked those people.`);
  return out;
}

export function madeText(c: GoldenCounts, away: boolean): string {
  return `Golden records made as a new tab: ${n(c.clusters, "cluster")} merged, ${n(c.rows, "row")}.${away ? " The folded rows are in a second tab." : ""} Your table is unchanged.`;
}
