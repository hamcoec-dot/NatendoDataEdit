// Changes from a newer file (Milestone 15).

export interface UpdateSpec {
  keys: number[];
  keep_first: boolean;
  match_case: boolean;
  trim: boolean;
}

export interface UpdateList {
  cells: { row: number; col: number; old: string; new: string }[];
  cells_total: number;
  added: string[][];
  added_total: number;
  removed: number[];
  removed_total: number;
  repeated_keys: number;
  /** Columns here the newer file lacks (never compared or changed). */
  here_only: string[];
  /** Columns of the newer file not here (not compared). */
  newer_only: string[];
  /** Columns found under the same title at another position. */
  moved: number;
}

export interface Choice {
  all: boolean;
  picked: number[];
}

export interface UpdateChoice {
  cells: Choice;
  added: Choice;
  removed: Choice;
}

/** A choice from "take all" and the items flipped from it. */
export function choiceOf(all: boolean, flipped: Set<number>): Choice {
  return { all, picked: [...flipped].sort((a, b) => a - b) };
}

/** Plain warnings about how the two files' columns were paired. */
export function columnNotes(l: Pick<UpdateList, "here_only" | "newer_only" | "moved">, byTitle: boolean): string[] {
  const out: string[] = [];
  const names = (v: string[]) => (v.length > 5 ? `${v.slice(0, 5).join(", ")} and ${v.length - 5} more` : v.join(", "));
  if (l.here_only.length)
    out.push(`Not in the newer file, so not compared or changed: ${names(l.here_only)}.`);
  if (l.newer_only.length)
    out.push(`Only in the newer file, so not compared (add the column here first to take it): ${names(l.newer_only)}.`);
  if (l.moved && byTitle)
    out.push(`${l.moved === 1 ? "One column is" : `${l.moved} columns are`} in another position in the newer file; matched by title.`);
  return out;
}
