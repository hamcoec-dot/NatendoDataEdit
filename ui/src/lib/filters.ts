// Helpers for filter rules, sort keys and searches as the UI builds them.
// Pure functions, so they are easy to test.

import type { CmpOp, Condition, FilterSpec, FindSpec, Pattern, PatternKind, Rule, Scope, TextOp } from "./backend";

/** The spec with column `col`'s rule replaced (or removed with null). */
export function withRule(spec: FilterSpec | null, col: number, rule: Rule | null, keepFirst: boolean): FilterSpec {
  const columns = (spec?.columns ?? []).filter((c) => c.col !== col);
  if (rule) columns.push({ col, rule });
  columns.sort((a, b) => a.col - b.col);
  return { columns, keep_first: keepFirst };
}

export function ruleFor(spec: FilterSpec | null, col: number): Rule | null {
  return spec?.columns.find((c) => c.col === col)?.rule ?? null;
}

export type CondKind = "text" | "number" | "date" | "blank" | "pattern" | "top";

/** What the condition editor holds. */
export interface CondForm {
  kind: CondKind;
  /** Text op, comparison, "between", or for blanks "blank"/"not_blank". */
  op: string;
  a: string;
  b: string;
  matchCase: boolean;
  patternKind: PatternKind;
  negate: boolean;
}

export const emptyForm = (): CondForm => ({
  kind: "text",
  op: "contains",
  a: "",
  b: "",
  matchCase: false,
  patternKind: "wildcard",
  negate: false,
});

export const OPS: Record<CondKind, [string, string][]> = {
  text: [
    ["contains", "contains"],
    ["not_contains", "does not contain"],
    ["equals", "equals"],
    ["not_equals", "does not equal"],
    ["begins_with", "begins with"],
    ["ends_with", "ends with"],
  ],
  number: [
    ["eq", "="],
    ["ne", "≠"],
    ["gt", ">"],
    ["ge", "≥"],
    ["lt", "<"],
    ["le", "≤"],
    ["between", "between"],
  ],
  date: [
    ["eq", "on"],
    ["lt", "before"],
    ["gt", "after"],
    ["le", "on or before"],
    ["ge", "on or after"],
    ["between", "between"],
  ],
  blank: [
    ["blank", "is empty"],
    ["not_blank", "is not empty"],
  ],
  pattern: [
    ["match", "matches"],
    ["not_match", "does not match"],
  ],
  top: [
    ["top", "largest (rows)"],
    ["top_percent", "largest (percent)"],
    ["bottom", "smallest (rows)"],
    ["bottom_percent", "smallest (percent)"],
  ],
};

/** A condition from the form, or an error message. */
export function conditionOf(f: CondForm): Condition | string {
  const num = (s: string) => {
    const v = Number(s.trim().replace(/,/g, ""));
    return s.trim() !== "" && Number.isFinite(v) ? v : null;
  };
  switch (f.kind) {
    case "text":
      return { type: "text", op: f.op as TextOp, text: f.a, match_case: f.matchCase };
    case "number": {
      const a = num(f.a);
      if (a === null) return "Enter a number.";
      if (f.op === "between") {
        const b = num(f.b);
        if (b === null) return "Enter both numbers.";
        return { type: "number_between", low: a, high: b };
      }
      return { type: "number", op: f.op as CmpOp, value: a };
    }
    case "date":
      if (f.a.trim() === "") return "Enter a date.";
      if (f.op === "between") {
        if (f.b.trim() === "") return "Enter both dates.";
        return { type: "date_between", low: f.a.trim(), high: f.b.trim() };
      }
      return { type: "date", op: f.op as CmpOp, value: f.a.trim() };
    case "blank":
      return f.op === "not_blank" ? { type: "not_blank" } : { type: "blank" };
    case "pattern":
      if (f.a === "") return "Enter a pattern.";
      return {
        type: "pattern",
        pattern: { text: f.a, kind: f.patternKind, match_case: f.matchCase, whole_cell: false },
        negate: f.op === "not_match",
      };
    case "top": {
      const n = num(f.a);
      const percent = f.op.endsWith("_percent");
      if (n === null || n < 0) return "Enter how many.";
      if (percent && n > 100) return "Enter a percent up to 100.";
      if (!percent && !Number.isInteger(n)) return "Enter a whole number of rows.";
      return { type: "top", count: n, percent, bottom: f.op.startsWith("bottom") };
    }
  }
}

/** The form for an existing condition (to edit a rule again). */
export function formOf(c: Condition): CondForm {
  const f = emptyForm();
  switch (c.type) {
    case "text":
      return { ...f, kind: "text", op: c.op, a: c.text, matchCase: !!c.match_case };
    case "number":
      return { ...f, kind: "number", op: c.op, a: String(c.value) };
    case "number_between":
      return { ...f, kind: "number", op: "between", a: String(c.low), b: String(c.high) };
    case "date":
      return { ...f, kind: "date", op: c.op, a: c.value };
    case "date_between":
      return { ...f, kind: "date", op: "between", a: c.low, b: c.high };
    case "blank":
      return { ...f, kind: "blank", op: "blank" };
    case "not_blank":
      return { ...f, kind: "blank", op: "not_blank" };
    case "pattern":
      return {
        ...f,
        kind: "pattern",
        op: c.negate ? "not_match" : "match",
        a: c.pattern.text,
        matchCase: c.pattern.match_case,
        patternKind: c.pattern.kind,
      };
    case "top":
      return { ...f, kind: "top", op: `${c.bottom ? "bottom" : "top"}${c.percent ? "_percent" : ""}`, a: String(c.count) };
  }
}

/** A short description of a rule, for tooltips and the status bar. */
export function describeRule(r: Rule): string {
  if (r.type === "values") {
    const n = r.values.length + (r.blanks ? 1 : 0);
    if (n === 1) return r.values.length ? `is "${r.values[0]}"` : "is empty";
    return `is one of ${n} values`;
  }
  const one = (c: Condition): string => {
    switch (c.type) {
      case "text":
        return `${OPS.text.find((o) => o[0] === c.op)?.[1] ?? c.op} "${c.text}"`;
      case "number":
        return `${OPS.number.find((o) => o[0] === c.op)?.[1] ?? c.op} ${c.value}`;
      case "number_between":
        return `between ${c.low} and ${c.high}`;
      case "date":
        return `${OPS.date.find((o) => o[0] === c.op)?.[1] ?? c.op} ${c.value}`;
      case "date_between":
        return `between ${c.low} and ${c.high}`;
      case "blank":
        return "is empty";
      case "not_blank":
        return "is not empty";
      case "pattern":
        return `${c.negate ? "does not match" : "matches"} ${c.pattern.text}`;
      case "top":
        return `${c.bottom ? "smallest" : "largest"} ${c.count}${c.percent ? " %" : ""}`;
    }
  };
  return r.second ? `${one(r.first)} ${r.or ? "or" : "and"} ${one(r.second)}` : one(r.first);
}

/** Where Find looks: the whole file, the selected block, the selected
 *  columns, columns picked by name, or every open tab. */
export type ScopeChoice = "all" | "selection" | "column" | "pick" | "tabs";

export interface FindOptions {
  text: string;
  kind: PatternKind;
  matchCase: boolean;
  wholeCell: boolean;
  /** Only whole words: "Ann" finds "Ann Lee", not "Annette". */
  wholeWord: boolean;
  /** "Jose" finds "José" too. */
  ignoreAccents: boolean;
  /** \t \n \r \\ \u{..} stand for those characters (plain and wildcards;
   *  also in the replacement). */
  escapes: boolean;
  /** Search hidden columns too (they are skipped otherwise, as in Excel). */
  includeHidden: boolean;
  scope: ScopeChoice;
  /** Find empty cells (the text box is not used). */
  blank: boolean;
  /** Search the column titles too (the first row when it holds titles). */
  titles: boolean;
  /** Look in the cells or in the row notes. */
  lookIn: "cells" | "notes";
  /** The columns picked by name (scope "pick"). */
  cols: number[];
  /** A replacement takes the case of what it replaces. */
  preserveCase: boolean;
  /** Find next goes down each column before moving right. */
  byColumns: boolean;
  /** Dates by value are read day first (03/04/2026 is 3 April). */
  dayFirst: boolean;
}

export const FIND_DEFAULTS: FindOptions = {
  text: "",
  kind: "plain",
  matchCase: false,
  wholeCell: false,
  wholeWord: false,
  ignoreAccents: false,
  escapes: false,
  includeHidden: false,
  scope: "all",
  blank: false,
  titles: false,
  lookIn: "cells",
  cols: [],
  preserveCase: false,
  byColumns: false,
  dayFirst: false,
};

/** The search the find bar describes, for a selection and header setting. */
export function findSpecOf(
  o: FindOptions,
  sel: { row: number; rows: number; col: number; cols: number },
  header: boolean,
  hidden: number[] = [],
  /** Several selected blocks (Ctrl+click): Selection searches all of them. */
  blocks: { row: number; rows: number; col: number; cols: number }[] = [],
): FindSpec {
  // Empty cells: a regular expression the engine matches specially.
  const pattern: Pattern = o.blank
    ? { text: "^$", kind: "regex", match_case: false, whole_cell: false }
    : {
        text: o.text,
        kind: o.kind,
        match_case: o.matchCase,
        whole_cell: o.wholeCell,
        whole_word: o.wholeWord,
        ignore_accents: o.ignoreAccents,
        escapes: o.escapes,
      };
  let scope: Scope = { kind: "all" };
  const several = blocks.length > 1;
  if (o.scope === "selection") scope = several ? { kind: "areas", areas: blocks.map((b) => ({ row: b.row, rows: b.rows, col: b.col, cols: b.cols })) } : { kind: "range", ...sel };
  else if (o.scope === "column" && several) scope = { kind: "columns", cols: [...new Set(blocks.flatMap((b) => Array.from({ length: b.cols }, (_, i) => b.col + i)))].sort((a, b) => a - b) };
  else if (o.scope === "column") scope = { kind: "columns", cols: Array.from({ length: sel.cols }, (_, i) => sel.col + i) };
  else if (o.scope === "pick") scope = { kind: "columns", cols: [...(o.cols ?? [])].sort((a, b) => a - b) };
  if (o.preserveCase && !o.blank) pattern.preserve_case = true;
  if (o.dayFirst && o.kind === "date" && !o.blank) pattern.read = { decimal_comma: false, date_order: "dmy" };
  // "Include hidden": hidden columns and hidden rows (filtered out or hidden
  // by hand) are searched too, and a replace says how many of its cells
  // are hidden.
  return {
    pattern,
    scope,
    skip_first: header && !o.titles,
    skip_cols: o.includeHidden ? [] : hidden,
    ...(o.includeHidden ? { hidden_rows: true, hidden_cols: hidden } : {}),
    ...(o.byColumns ? { by_columns: true } : {}),
  };
}
