// Patch files (.ndpatch): the differences between two tables by key, saved to
// apply to another copy. Types shared with the desktop app, and the words the
// screen uses.

export interface PatchSpec {
  /** Key columns of the older table. */
  keys: number[];
  match_case: boolean;
  trim: boolean;
  /** Leave columns only one table has out of the patch (else refused). */
  leave_out: boolean;
}

export interface PatchSaved {
  changes: number;
  cells: number;
  adds: number;
  removes: number;
  left_out: string[];
  protected: boolean;
}

export type PatchOutcome =
  | "will_apply"
  | "already_applied"
  | "conflict"
  | "key_not_found"
  | "key_twice"
  | "protected"
  | "missing_column";

export interface PatchLine {
  outcome: PatchOutcome;
  kind: "change" | "add" | "remove";
  key: string[];
  column: string;
}

export interface PatchReport {
  will_apply: number;
  already: number;
  conflicts: number;
  not_found: number;
  twice: number;
  protected: number;
  missing_column: number;
  cells: number;
  added: number;
  removed: number;
  missing: string[];
  extra: string[];
  keys: string[];
  lines: PatchLine[];
  lines_total: number;
}

export interface PatchPreview {
  report: PatchReport;
  /** Applying marks the tab private. */
  private: boolean;
}

export interface PatchApplied {
  cells: number;
  added: number;
  removed: number;
}

/** Lines the screen lists (the saved report has more). */
export const PATCH_SHOWN = 200;

export const OUTCOME_WORDS: Record<PatchOutcome, string> = {
  will_apply: "will apply",
  already_applied: "already applied",
  conflict: "conflict: left alone",
  key_not_found: "key not found",
  key_twice: "key found more than once",
  protected: "protected column: left alone",
  missing_column: "column not in this table",
};

const n = (v: number, one: string, many = `${one}s`): string => `${v.toLocaleString("en-US")} ${v === 1 ? one : many}`;

export function savedText(s: PatchSaved): string {
  const out = s.left_out.length ? ` Left out: ${s.left_out.join(", ")}.` : "";
  return `Patch saved: ${n(s.changes, "changed row")} (${n(s.cells, "cell")}), ${n(s.adds, "added row")}, ${n(s.removes, "removed row")}. ${s.protected ? "It is protected with your phrase." : "It is not protected: it holds data from both tables in the clear."}${out}`;
}

/** What the patch would do, in one line. */
export function willText(r: PatchReport): string {
  if (!r.will_apply) return "Nothing to apply.";
  return `Apply would change ${n(r.cells, "cell")}, add ${n(r.added, "row")} and remove ${n(r.removed, "row")}.`;
}

/** What it would leave, as [words, count] (zero counts left out). */
export function leftCounts(r: PatchReport): [string, number][] {
  const all: [string, number][] = [
    ["already applied", r.already],
    ["conflicts (neither the old value nor the new one is there)", r.conflicts],
    ["keys not found", r.not_found],
    ["keys found more than once", r.twice],
    ["in protected columns", r.protected],
    ["in columns this table does not have", r.missing_column],
  ];
  return all.filter(([, c]) => c > 0);
}

export function appliedText(a: PatchApplied): string {
  return `Patch applied: ${n(a.cells, "cell")} changed, ${n(a.added, "row")} added, ${n(a.removed, "row")} removed. Undo (Ctrl+Z) puts it all back.`;
}
