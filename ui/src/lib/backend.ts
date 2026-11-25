// Typed access to the Rust side. In a plain browser (vite dev, tests) a stub
// backend with a generated document stands in, so the UI can be worked on
// without the desktop shell.
import { InfoGuard } from "./infoguard";
import type { ColumnType, ColumnTypes } from "./readers";
import type { Cleaner, CleanPreview, CleanSpec, ExampleSpec, SplitName } from "./clean";
import type { ChartData, ChartSpec, ColType, Highlight, Listing, Mark, RecentItem, SubtotalSpec, Summary, View } from "./views";
import type { Autosaved, RecoveryItem, Suggestions, VersionItem } from "./m9";

import { decodeViewport, type ViewportData } from "./binary";
import { MockBackend } from "./mock";
import type { ItemList, PivotField, PivotGrid, PivotSpec } from "./pivot";

export interface AppInfo {
  name: string;
  version: string;
  engine_version: string;
}

export interface DocInfo {
  id: number;
  name: string;
  path: string | null;
  /** Rows shown (all rows, or those a filter shows). */
  rows: number;
  /** Every row, shown or not. */
  total_rows: number;
  filtered: boolean;
  /** Columns with a filter rule. */
  filter_cols: number[];
  /** Looking at an earlier recipe step: changes and saving are refused. */
  peeking?: boolean;
  /** Answered from the last reading while a change held the document (see infoguard.ts); never kept as a tab's state. */
  stale?: boolean;
  /** Rises with every reading the engine makes; a reply older than what the tab holds is dropped (see Tab.update). */
  seq?: number;
  /** Moves when what the rows hold changes (not with filters). */
  data_revision?: number;
  /** A filter rule sits on a deleted column: rows stay hidden without a funnel. */
  filter_on_deleted: boolean;
  cols: number;
  indexed: boolean;
  progress: number;
  modified: boolean;
  can_undo: boolean;
  can_redo: boolean;
  history_pos: number;
  /** What the columns hold, by physical column (for Undo, saved views and the session). */
  column_types: ColumnTypes;
  /** The typed columns as shown: [column, type]; Auto columns are left out. */
  typed: [number, ColumnType][];
  encoding: string;
  bom: boolean;
  delimiter: string;
  quote: string | null;
  escape: string | null;
  merge: boolean;
  /** Lines above the table, kept as they are. */
  preamble_lines: number;
  /** How sure layout detection was (0 to 1); null when the user chose it. */
  layout_confidence: number | null;
  /** Locked: changes and saving over the file are refused. */
  read_only: boolean;
  /** Rows the user hid (still saved). */
  hidden_rows: number;
  /** Protected columns (changes pass them over). */
  protected?: number[];
  /** Cells the last change left alone in protected columns. */
  protected_skipped?: number;
  /** A filter is on (`filtered` is also true while rows are only hidden). */
  has_filter: boolean;
  line_break: string;
  mixed_line_breaks: boolean;
  lossy: boolean;
  invalid_bytes: boolean;
  line_by_line: boolean;
  column_changes: boolean;
  repair_count: number;
  unclosed_quotes: boolean;
  long_quoted_cells: boolean;
  line_bound_from: number | null;
  error: string | null;
}

/** How a file is laid out. */
export interface Layout {
  delimiter: string;
  quote: string | null;
  escape: string | null;
  merge: boolean;
  skip_lines: number;
}

export interface LayoutChoice {
  layout: Layout;
  columns: number;
  preview: string[][];
}

export interface LayoutChoices {
  confidence: number;
  sep_line: boolean;
  current: Layout;
  choices: LayoutChoice[];
  /** The lines above the table now set aside. */
  preamble: string;
}

/** Error from a command; `kind` decides which prompt the UI shows. */
export interface CmdError {
  kind: string;
  message: string;
  row: number | null;
}

export function isCmdError(e: unknown): e is CmdError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

export function errorText(e: unknown): string {
  if (isCmdError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export type Change =
  | { op: "insert_rows"; at: number; count: number }
  | { op: "delete_rows"; at: number; count: number }
  | { op: "move_rows"; at: number; count: number; to: number }
  | { op: "insert_cols"; at: number; count: number; /** Not a recipe step (Split into columns). */ unrecorded?: boolean }
  | { op: "delete_cols"; at: number; count: number }
  | { op: "move_cols"; at: number; count: number; to: number }
  | { op: "hide_rows"; at: number; count: number }
  | { op: "show_rows"; which: "all" | { around: { at: number; count: number } } }
  | { op: "shuffle_rows"; at: number; count: number }
  | { op: "flip_rows"; at: number; count: number }
  | { op: "delete_col_list"; cols: number[] }
  | { op: "undo" }
  | { op: "redo" };

/** Quick totals of a selection (status bar). */
export interface Totals {
  cells: number;
  filled: number;
  numbers: number;
  sum: number | null;
  average: number | null;
  min: number | null;
  max: number | null;
  /** The sum is past the largest number that can be held (about 1.8e308). */
  too_large: boolean;
  /** Characters in the cells. */
  chars: number;
  /** Different values (when asked for). */
  distinct: number | null;
  /** False when there were more different values than counted. */
  distinct_exact: boolean;
}

// ---- Milestone 10 tools (shapes match the engine's `tools` module) ----

export type FillFrom = { kind: "above" } | { kind: "below" } | { kind: "value"; text: string };
export interface FillBlanks {
  cols: number[];
  from: FillFrom;
  spaces: boolean;
  keep_first: boolean;
}
export interface Repeats {
  cols: number[];
  match_case: boolean;
  trim: boolean;
  keep_first: boolean;
  title: string;
}
export interface Breaks {
  col: number | null;
  every: number;
  rows: number;
  keep_first: boolean;
}
export interface Reshape {
  col: number;
  per_row: number;
  down: boolean;
  keep_first: boolean;
}
export interface Stack {
  cols: number[];
  skip_blanks: boolean;
  keep_first: boolean;
}
export type SampleSize = { kind: "rows"; n: number } | { kind: "percent"; p: number } | { kind: "every"; n: number };
export interface Sample {
  size: SampleSize;
  seed: number;
  keep_first: boolean;
  /** Draw from each group of this column's values on its own. */
  per?: number | null;
  /** Rows in the order drawn instead of table order. */
  drawn_order?: boolean;
  /** A first column "Row" with each row's number. */
  row_numbers?: boolean;
}
export interface CompareSpec {
  keys: number[];
  match_case: boolean;
  trim: boolean;
  keep_first: boolean;
  show_same: boolean;
}
export interface CompareReport {
  same: number;
  changed: number;
  added: number;
  removed: number;
  cells: number;
}
/** What a key-aware diff compares (the second table's columns are paired with the first's by title). */
export interface DiffSpec {
  keys: number[];
  keep_first: boolean;
  match_case: boolean;
  trim: boolean;
}
export interface DiffReport {
  same: number;
  changed: number;
  added: number;
  removed: number;
  cells: number;
  listed: number;
  truncated: boolean;
  only_a: string[];
  only_b: string[];
  moved: number;
  repeated: number;
  dup_a: string[];
  dup_b: string[];
  empty_a: number;
  empty_b: number;
}
export interface DiffReportOut {
  info: DocInfo;
  report: DiffReport;
  diff: number;
}
/** A difference on screen: `col` null for a whole row; kind 0 changed cell, 1 added row, 2 removed row. */
export interface DiffMark {
  row: number;
  col: number | null;
  kind: number;
}
export interface DiffMarksOut {
  diff: number | null;
  out_of_date: boolean;
  marks: DiffMark[];
}
export interface DiffJump {
  row: number;
  col: number | null;
  wrapped: boolean;
}
export interface TableReport {
  info: DocInfo;
  compare: CompareReport | null;
  /** A sample's audit record (seed, method, file fingerprint, groups). */
  record?: DocInfo | null;
}
export interface SplitParts {
  col: number | null;
  every: number;
  keep_first: boolean;
}
export interface SplitReport {
  files: number;
  rows: number;
  replaced: number;
  /** From a private tab: new files put in private file mode, and those that
   *  could not be (the list was full). */
  private?: number;
  private_missed?: number;
  /** The folder was empty, so the folder itself was made private. */
  private_folder?: boolean;
}
export interface Combine {
  header: boolean;
  by_title: boolean;
  source: boolean;
}

// ---- sort, filter, find (shapes match the engine's) ----

export type DateOrder = "mdy" | "dmy";
export interface ReadOptions {
  decimal_comma: boolean;
  date_order: DateOrder;
}
/** Data › Compare two columns. */
export interface CompareCols {
  a: number;
  b: number;
  /** Upper and lower case count as different. */
  case: boolean;
  /** Spaces count. */
  spaces: boolean;
  /** Numbers compared by value. */
  numbers: boolean;
  read?: ReadOptions;
  keep_first: boolean;
  title: string;
}
export interface CompareColsReport {
  same: number;
  different: number;
  only_first: number;
  only_second: number;
  /** Shown rows where the cells are not the same (at most 5 million). */
  rows: number[];
}
/** Format › Tidy column titles. */
export interface TitleSpec {
  spaces: boolean;
  case: "keep" | "title" | "snake" | "upper" | "lower";
  odd: boolean;
  unique: boolean;
}
/** A tab to remember for next time. */
export interface SessionTab {
  doc: number;
  view: unknown;
  row: number;
  col: number;
  active: boolean;
}
/** A tab on its way between windows (or its last state, kept by the app so a
 *  window's files can come back). `state` is built and read by the windows. */
export interface HandOver {
  doc: number;
  state: unknown;
  view?: unknown;
  row: number;
  col: number;
  /** Why the tab came back on its own, if it did. */
  note?: string | null;
}
/** A document with unsaved changes, as the quit question lists it. */
export interface ModifiedDoc {
  id: number;
  name: string;
  /** Counts every edit, undo and redo of the file (it never goes back). */
  generation: number;
}
/** What stands between the app and a quit. */
export interface ModifiedReport {
  docs: ModifiedDoc[];
  /** Files something holds right now (a save, a long change). */
  busy: string[];
  /** Long jobs running (quitting stops them). */
  jobs: string[];
}
/** A tab open when the app last closed. */
export interface SessionItem {
  index: number;
  name: string;
  folder: string;
  /** The file changed since: it opens with the plain view. */
  changed: boolean;
  missing: boolean;
  /** Open already (or being opened at start): not offered back. */
  open?: boolean;
  active: boolean;
}
export interface SessionPlace {
  view: unknown | null;
  row: number;
  col: number;
  changed: boolean;
}
/** Go To Special: which cells to pick. */
export type SpecialKind =
  | { kind: "blanks" }
  | { kind: "numbers" }
  | { kind: "text" }
  | { kind: "row_diff"; against: number; exact: boolean }
  | { kind: "edited" }
  | { kind: "problems"; checks: CheckSpec };
export interface SpecialSpec {
  kind: SpecialKind;
  row: number;
  rows: number;
  cols: number[];
  read?: ReadOptions;
}
export interface SpecialCells {
  /** (shown row, column), at most 5 million. */
  cells: [number, number][];
  total: number;
}
export type SortKind = "auto" | "text" | "natural" | "number" | "date" | "length" | "last_word" | { list: number };
export interface SortKey {
  col: number;
  descending: boolean;
  kind: SortKind;
  match_case: boolean;
}
export interface SortSpec {
  keys: SortKey[];
  keep_first: boolean;
  read?: ReadOptions;
  /** Custom orders, used by `{ list: i }` keys. */
  lists?: string[][];
}

export type CmpOp = "eq" | "ne" | "gt" | "ge" | "lt" | "le";
export type TextOp = "equals" | "not_equals" | "contains" | "not_contains" | "begins_with" | "ends_with";
/** "number" and "date" find cells by value (5, 10..20, >5); "similar"
 *  finds whole cells spelled nearly the same. */
export type PatternKind = "plain" | "wildcard" | "regex" | "number" | "date" | "similar";
export interface Pattern {
  text: string;
  kind: PatternKind;
  match_case: boolean;
  whole_cell: boolean;
  whole_word?: boolean;
  ignore_accents?: boolean;
  escapes?: boolean;
  /** The replacement takes the case of what it replaces. */
  preserve_case?: boolean;
  /** How numbers and dates are read (number and date kinds). */
  read?: ReadOptions;
}
export type Condition =
  | { type: "text"; op: TextOp; text: string; match_case?: boolean }
  | { type: "number"; op: CmpOp; value: number }
  | { type: "number_between"; low: number; high: number }
  | { type: "date"; op: CmpOp; value: string }
  | { type: "date_between"; low: string; high: string }
  | { type: "blank" }
  | { type: "not_blank" }
  | { type: "pattern"; pattern: Pattern; negate?: boolean }
  | { type: "top"; count: number; percent: boolean; bottom: boolean };
export type Rule =
  | { type: "values"; values: string[]; blanks: boolean }
  | { type: "conditions"; first: Condition; second?: Condition | null; or?: boolean };
export interface ColumnFilter {
  col: number;
  rule: Rule;
}
export interface FilterSpec {
  columns: ColumnFilter[];
  keep_first: boolean;
  read?: ReadOptions;
}
/** One card of the filter panel. */
export interface FacetCard {
  col: number;
  kind: "values" | "numbers" | "dates";
  /** Numbers and dates: the range the bars cover (null: the column's own). */
  range: [number, number] | null;
  search: string | null;
}
export interface Facet {
  col: number;
  values: { value: string; count: number }[];
  distinct: number;
  complete: boolean;
  /** Too many different values to list: only a search is counted. */
  many?: boolean;
  bins: number[];
  range: [number, number] | null;
  other: number;
  blanks: number;
  rows: number;
}
export interface Facets {
  cards: Facet[];
  shown: number;
}
export interface ValueList {
  values: { value: string; count: number }[];
  blanks: number;
  complete: boolean;
}

/** Milestone 13: private data. */
export type PrivateKind = "ssn" | "birth_date" | "phone" | "email" | "address" | "licence";
export interface Finding {
  col: number;
  kind: PrivateKind;
  title: string;
  looked: number;
  hits: number;
  by_title: boolean;
}
export type ColumnAction = { action: "keep" } | { action: "drop" } | { action: "clean"; cleaner: import("./clean").Cleaner };
export interface SafeCopy {
  columns: ColumnAction[];
  header: boolean;
  read: ReadOptions;
}
/** One changed cell (rows are positions in the whole table, from 0). */
export interface CellChange {
  row: number;
  file_row: number;
  col: number;
  old: string;
  new: string;
}
export interface ChangeList {
  cells: CellChange[];
  cells_total: number;
  added_rows: number[];
  added_total: number;
  deleted_rows: number[];
  deleted_total: number;
  added_cols: number[];
  deleted_cols: number[];
  /** Columns cleared at once (Delete on whole columns), by position now. */
  cleared_cols?: number[];
  moved_cols: boolean;
  reordered: boolean;
}
export interface SafeCopyReport {
  rows: number;
  cols: number;
  changed: number;
  bytes: number;
  /** Values no step could read, written blank (never as they were). */
  blanked?: number;
  blanked_cols?: [number, number][];
}
export interface PrivateState {
  file: boolean;
  folder: boolean;
}

export type Scope =
  | { kind: "all" }
  | { kind: "columns"; cols: number[] }
  | { kind: "range"; row: number; rows: number; col: number; cols: number }
  /** Several selected blocks (Ctrl+click); a cell in two is searched once. */
  | { kind: "areas"; areas: { row: number; rows: number; col: number; cols: number }[] };
export interface FindSpec {
  pattern: Pattern;
  scope: Scope;
  skip_first: boolean;
  /** Columns left out (hidden ones, unless "Include hidden" is ticked). */
  skip_cols?: number[];
  /** Count, list and replace in hidden rows too. */
  hidden_rows?: boolean;
  /** Hidden columns that are searched (counted as hidden in a replace). */
  hidden_cols?: number[];
  /** Find next goes down each column before moving right. */
  by_columns?: boolean;
}
export interface Found {
  row: number;
  col: number;
  wrapped: boolean;
}
export interface Hit {
  row: number;
  /** Row number in the whole file (differs from `row` while filtered). */
  number: number;
  col: number;
  text: string;
  start: number;
  len: number;
  /** In a hidden row (`row` is the nearest shown one). */
  hidden?: boolean;
}
export interface FindAll {
  hits: Hit[];
  cells: number;
  /** Matching cells per column: [column, count], all of them. */
  per_col?: [number, number][];
  /** The file's revision when searched (row actions refuse if it moved). */
  revision?: number;
}
/** What acting on Find all's rows did. */
export interface FindRowsReport {
  info: DocInfo;
  /** Rows whose flag had another colour (Flag). */
  recolored: number;
  /** Hidden columns left out of the new tab (Copy). */
  left_out_cols: number;
}
export type FindRowsAction = { kind: "show" } | { kind: "delete" } | { kind: "copy" } | { kind: "flag"; flag: number };
export interface BatchReplace {
  pairs: [string, string][];
  whole_cell: boolean;
  match_case: boolean;
  scope: Scope;
  skip_first: boolean;
  skip_cols?: number[];
}
export interface FindCount {
  cells: number;
  /** Matching cells before the one asked about (Find next order). */
  before: number;
  /** That cell matches. */
  here: boolean;
}
export interface ReplaceReport {
  cells: number;
  hits: number;
  /** `number`: the row in the whole file (unticking goes by it); `hidden`:
   *  in a hidden row or column. */
  preview: { row: number; number: number; col: number; before: string; after: string; hidden?: boolean }[];
  info: DocInfo | null;
  /** Of the cells, those in hidden rows or columns. */
  hidden_cells?: number;
}
export interface JobInfo {
  label: string;
  fraction: number;
}

// ---- data operations (Milestone 4b; shapes match the engine's) ----

export type Dir = "up" | "down" | "left" | "right";
export type DateUnit = "day" | "weekday" | "month" | "year";
export type FillMode =
  | { kind: "auto" }
  | { kind: "auto_swap" }
  | { kind: "copy" }
  | { kind: "linear"; step: number; stop: number | null }
  | { kind: "growth"; step: number; stop: number | null }
  | { kind: "date"; unit: DateUnit; step: number };
export interface FillSpec {
  row: number;
  col: number;
  rows: number;
  cols: number;
  dir: Dir;
  count: number;
  mode: FillMode;
  read?: ReadOptions;
  /** Columns left as they are (hidden ones). */
  skip_cols?: number[];
}
export type PasteOp = "none" | "add" | "subtract" | "multiply" | "divide";
export interface PasteSpec {
  row: number;
  col: number;
  op: PasteOp;
  skip_blanks: boolean;
  transpose: boolean;
  read?: ReadOptions;
  /** Columns passed over (hidden ones): pasted columns go to the next ones. */
  skip_cols?: number[];
  /** Taller than the shown rows left: stop at the last shown row. */
  stop_at_end?: boolean;
}
export interface Duplicates {
  cols: number[];
  match_case: boolean;
  trim: boolean;
  keep_first: boolean;
  /** Remove duplicates: which copy stays. */
  keep?: "first" | "last" | "newest" | "oldest" | "most_filled";
  date_col?: number | null;
  /** Fill the kept copy's blanks from the copies removed. */
  fill_blanks?: boolean;
  /** How the date column is read. */
  read?: ReadOptions;
}
export interface Blanks {
  spaces: boolean;
  keep_first: boolean;
}
export interface SplitColumn {
  col: number;
  separator: string;
  max_parts: number;
  keep_original: boolean;
  trim: boolean;
  keep_first: boolean;
  mode?: "text" | "last" | "any" | "positions" | "letters_digits";
  /** Character positions to split at, from 1. */
  positions?: number[];
}
export interface TallySpec {
  col: number;
  keep_first: boolean;
  /** Stop past this many distinct values (the result is then incomplete). */
  limit?: number | null;
}
export interface ValueTally {
  values: { value: string; count: number }[];
  blanks: number;
  rows: number;
  complete: boolean;
  /** Indexes into `values`; the first of each is the most common. */
  look_alikes: { members: number[] }[];
}
/** How to decide that two values look alike. */
export type LookAlikeMethod = "sounds" | "spelling" | "both";
export interface LookAlikeSpec {
  col: number;
  keep_first: boolean;
  method: LookAlikeMethod;
}
export interface LookAlikes {
  /** Each group's values with counts; the suggested spelling first. */
  groups: { value: string; count: number }[][];
  /** Distinct values looked at. */
  values: number;
  /** False when only the most common values of the column were looked at. */
  complete: boolean;
  /** Values too long for spelling grouping. */
  too_long: number;
  /** Deleted forms skipped for being shared by too many values. */
  crowded: number;
}
export interface MapValues {
  cols: number[];
  pairs: [string, string][];
  match_case: boolean;
  trim: boolean;
  keep_first: boolean;
}
export interface MapFrom {
  doc: number;
  old: number;
  new: number;
  header: boolean;
}
export type GroupFunc = "count" | "sum" | "average" | "min" | "max" | "first" | "last" | "rank" | "rank_up" | "running" | "percent";
export interface GroupCalc {
  keys: number[];
  value: number | null;
  func: GroupFunc;
  title: string;
  at: number | null;
  match_case: boolean;
  trim: boolean;
  keep_first: boolean;
}
export interface Arrange {
  order: number[];
  names: string[];
  keep_first: boolean;
}
export type ExtractWhat = "number" | "numbers" | "email" | "zip" | "brackets" | "first_word" | "last_word" | "unit" | "phone";
export interface Extract {
  col: number;
  what: ExtractWhat;
  title: string;
  remove: boolean;
  keep_first: boolean;
}
export interface MergeColumns {
  col: number;
  count: number;
  separator: string;
  skip_blanks: boolean;
}
export interface SplitRows {
  col: number;
  separator: string;
  trim: boolean;
  skip_empty: boolean;
  keep_first: boolean;
}
export interface MergeRows {
  keys: number[];
  separator: string;
  distinct: boolean;
  match_case: boolean;
  trim: boolean;
  keep_first: boolean;
}
export type DataOp =
  | { op: "paste"; spec: PasteSpec; text: string }
  | { op: "fill"; spec: FillSpec }
  | { op: "show_duplicates"; spec: Duplicates }
  | { op: "remove_duplicates"; spec: Duplicates }
  | { op: "remove_blank_rows"; spec: Blanks }
  | { op: "remove_blank_cols"; spec: Blanks }
  | { op: "split_column"; spec: SplitColumn }
  | { op: "merge_columns"; spec: MergeColumns }
  | { op: "split_rows"; spec: SplitRows }
  | { op: "merge_rows"; spec: MergeRows }
  | { op: "sort_columns"; col: number; count: number; descending: boolean }
  | { op: "join"; other: number | null; list?: string | null; list_id?: number | null; spec: JoinSpec }
  | { op: "append"; other: number; spec: AppendSpec }
  | { op: "clean"; spec: CleanSpec; now: number }
  | { op: "split_name"; spec: SplitName }
  | { op: "fill_example"; spec: ExampleSpec }
  | { op: "compute"; spec: ComputeSpec }
  | { op: "show_problems"; spec: CheckSpec; passing?: boolean }
  | { op: "show_shape"; spec: ShapeFilter }
  | { op: "sort_problems"; spec: CheckSpec }
  | { op: "check_column"; spec: CheckSpec; title?: string }
  | { op: "fill_blanks"; spec: FillBlanks }
  | { op: "count_repeats"; spec: Repeats }
  | { op: "blank_repeats"; spec: Duplicates }
  | { op: "breaks"; spec: Breaks }
  | { op: "delete_hidden" }
  | { op: "map_values"; spec: MapValues; from?: MapFrom | null }
  | { op: "group_calc"; spec: GroupCalc }
  | { op: "arrange"; spec: Arrange }
  | { op: "extract"; spec: Extract }
  | { op: "fill_rect"; spec: { row: number; rows: number; col: number; cols: number; value: string; skip_cols?: number[] } }
  | { op: "fill_special"; spec: SpecialSpec; value: string }
  | { op: "compare_show"; spec: CompareCols }
  | { op: "formula_filter"; spec: ComputeSpec }
  | { op: "compare_column"; spec: CompareCols }
  | { op: "shift_cells"; spec: { row: number; rows: number; col: number; cols: number; shift: "up" | "left" | "down" | "right" } };

/** A computed column: an expression over the row's cells. */
export interface ComputeSpec {
  expr: string;
  title: string;
  /** Write over this column instead of adding one. */
  target: number | null;
  /** Where a new column goes; null: at the end. */
  at: number | null;
  keep_first: boolean;
  read: ReadOptions;
  /** Local date and time as days since 1970 (TODAY, NOW). */
  now: number;
}
/** A formula typed in a cell (or filled over a block with Ctrl+Enter). */
export interface CellFormula {
  formula: string;
  /** The cell it was typed in: shown row, column. */
  row: number;
  col: number;
  /** Every cell it fills (shown row, column); empty: just that one. */
  targets: [number, number][];
  keep_first: boolean;
  read: ReadOptions;
  now: number;
}
export interface FormulaReport {
  info: DocInfo;
  written: number;
  /** Cells left as they were: the formula gave an error there. */
  errors: number;
  first_error: string | null;
}
export interface ComputePreview {
  /** Row number (from 1) and result. */
  rows: [number, string][];
  errors: number;
}

export type CheckKind =
  | { type: "required" }
  | { type: "number"; min: number | null; max: number | null; whole: boolean }
  | { type: "date"; min: string | null; max: string | null }
  | { type: "date_iso" }
  | { type: "length"; min: number | null; max: number | null }
  | { type: "list"; values: string[]; match_case: boolean }
  | { type: "pattern"; pattern: Pattern }
  | { type: "unique"; match_case: boolean }
  | { type: "expression"; expr: string };
/** A pivot layout column found by place, not title. */
export interface PlaceSwap {
  was: string;
  col: number;
  now: string;
}
export interface Check {
  /** Column (ignored for expressions). */
  col: number | null;
  kind: CheckKind;
  /** What the rule is called ("5-digit ZIP"); a sentence made from the kind when none. */
  name?: string | null;
}
/** Where a jump to the next problem landed. */
export interface Jump {
  row: number;
  col: number;
  check: number;
  row_rule: boolean;
}
export interface JumpResult {
  hit: Jump | null;
  wrapped: boolean;
  hidden_skipped: number;
}
/** A rule the data suggests (never applied until the person ticks it). */
export interface RuleSuggestion {
  check: Check;
  why: string;
  share: number;
}
/** Rules read from a `.ndchecks` file beside the table. */
export interface SidecarRules {
  checks: Check[];
  skipped: number;
  keep_first: boolean;
  file: string;
}
export interface CheckSpec {
  checks: Check[];
  keep_first: boolean;
  read: ReadOptions;
  now: number;
}
export interface Problem {
  row: number;
  col: number | null;
  check: number;
  value: string;
}
export interface CheckReport {
  checked: number;
  rows: number;
  problems: number;
  per_check: number[];
  listed: Problem[];
}
/** A cell Replace all would change, as the grid shows it before the change. */
export interface Ghost {
  row: number;
  /** The row's number in the document (a change unticked in the preview is kept by it). */
  number: number;
  col: number;
  after: string;
  /** `after` was cut at 20,000 characters. */
  cut: boolean;
}
export interface GhostWindow {
  ghosts: Ghost[];
  /** The time ran out: more changes are not shown. */
  partial: boolean;
}
export interface CheckMark {
  row: number;
  col: number | null;
  check: number;
}

export type Paper = "letter" | "a4" | "legal" | "a3";
export interface PageSetup {
  paper: Paper;
  landscape: boolean;
  margin_mm: number;
  font_size: number;
  fit_width: boolean;
  grid: boolean;
  repeat_header: boolean;
  title: string;
  page_numbers: boolean;
  /** Shown rows: [first, count]; null: all. */
  rows: [number, number] | null;
  /** Columns in order; null: all. */
  cols: number[] | null;
}

/** A file Windows, a second launch, a drop or the agent link opened. */
export interface OsOpen {
  name: string;
  opened: Opened | null;
  /** Already open in this document: show its tab. */
  existing: number | null;
  error: string | null;
}
export interface FileType {
  ext: string;
  on: boolean;
  text: boolean;
}
export interface AgentStatus {
  on: boolean;
  port: number | null;
}
export interface JoinSpec {
  keys: number[];
  other_keys: number[];
  /** The other table's columns to add (empty: all but its keys). */
  columns: number[];
  keep_first: boolean;
  other_header: boolean;
  match_case: boolean;
  trim: boolean;
  add_unmatched: boolean;
  /** Where the values go: new columns, or columns of this table (`targets`). */
  write: "new" | "overwrite" | "fill_blanks" | "nothing";
  targets: number[];
  keep_rows: "all" | "matched" | "unmatched";
  repeats: "first" | "last" | "every" | "joined" | "count" | "sum" | "min" | "max";
  separator: string;
  loose: boolean;
  not_found: string;
  place: "end" | "after_key";
  prefix: string;
  key_match: "exact" | "below" | "above";
}
export interface JoinPreview {
  titles: string[];
  /** New or updated columns. */
  columns: number[];
  rows: { kind: "matched" | "unmatched" | "blank" | "removed" | "extra" | "added"; row: number | null; values: string[]; changed: number[] }[];
  report: JoinReport;
}
export interface AppendSpec {
  by_name: boolean;
  keep_first: boolean;
  other_header: boolean;
  add_columns: boolean;
}
export interface JoinReport {
  matched: number;
  unmatched: number;
  added: number;
  repeated_keys: number;
  cols: number;
  dropped_cols: number;
  blank_keys: number;
  changed: number;
  removed: number;
  extra_rows: number;
  bad_keys: number;
}
export type Aggregate = "count" | "sum" | "average" | "min" | "max" | "first" | "join";
export interface Pivot {
  rows: number[];
  col: number;
  value: number;
  aggregate: Aggregate;
  separator: string;
  keep_first: boolean;
  read?: ReadOptions;
}
export interface Unpivot {
  ids: number[];
  values: number[];
  name_title: string;
  value_title: string;
  skip_blanks: boolean;
  keep_first: boolean;
}
export type TableOp =
  | { op: "pivot"; spec: Pivot }
  | { op: "unpivot"; spec: Unpivot }
  | { op: "subtotals"; spec: SubtotalSpec }
  | { op: "reshape"; spec: Reshape }
  | { op: "stack"; spec: Stack }
  | { op: "sample"; spec: Sample }
  | { op: "counts"; spec: CountsSpec }
  | { op: "compare"; other: number; spec: CompareSpec };
export interface InsightSpec {
  cols: number[];
  keep_first: boolean;
  sample: number;
  read: ReadOptions;
}
export interface Insight {
  col: number;
  /** Share of sampled cells filled, 0 to 1. */
  filled: number;
  distinct: number;
  /** Every filled sampled value differs. */
  unique: boolean;
  /** Bars are a histogram (else the most common values). */
  numbers: boolean;
  /** Heights, the tallest 1. */
  bars: number[];
  /** The pattern pill: the shape (9 digits, A letters) that makes up at least 90% of the filled sampled cells while some differ. */
  top_shape?: string | null;
  /** That shape's share of the filled sampled cells. */
  top_share?: number;
}
export interface Insights {
  cols: Insight[];
  sampled: number;
  rows: number;
  /** The data revision these figures are for. */
  revision?: number;
}
/** "Show the rows whose value in a column has (or has not) a shape": the pattern pill's click. */
export interface ShapeFilter {
  col: number;
  keep_first: boolean;
  shape: string;
  negate: boolean;
  include_blanks: boolean;
  /** The data revision the pill was worked out for: refused once the rows have changed. */
  revision?: number;
}
export interface ProfileSpec {
  col: number;
  keep_first: boolean;
  read: ReadOptions;
}
export interface ShapeCount {
  shape: string;
  count: number;
  example: string;
}
export interface Histogram {
  kind: "numbers" | "dates" | "lengths";
  low: number;
  high: number;
  counts: number[];
}
export interface Profile {
  col: number;
  rows: number;
  blanks: number;
  shapes: ShapeCount[];
  shape_kinds: number;
  histogram: Histogram | null;
  shortest: string | null;
  longest: string | null;
  min_len: number;
  max_len: number;
  rare_shapes: ShapeCount[];
  outliers: number;
  outlier_examples: string[];
  mean: number | null;
  sd: number | null;
}
/** The counts tab: each value of one to three columns with its count. */
export interface CountsSpec {
  cols: number[];
  keep_first: boolean;
}
export interface OpReport {
  /** Paste, fill, clear: hidden rows between the rows written, left alone. */
  hidden_rows?: number;
  info: DocInfo | null;
  cells: number;
  rows: number;
  groups: number;
  cols: number;
  skipped: number;
  join?: JoinReport | null;
  /** Cleaners: cells left as they were (not a date or number); keep
   *  newest/oldest: copies whose date could not be read. */
  left?: number;
  /** Cleaners: dates read the other way round from the chosen day order. */
  other_order?: number;
  /** Fill by example: the pattern used, in words. */
  pattern?: string | null;
  examples?: number;
  /** Computed columns: cells that came out as error codes. */
  errors?: number;
}

/** How values are put in quotes on Save As. */
export type Quoting = "keep" | "needed" | "all" | "text";
export interface SaveAsOptions {
  delimiter: string;
  quote: string | null;
  quoting: Quoting;
  encoding: string;
  bom: boolean;
  line_break: "CRLF" | "LF" | "CR";
  convert_line_breaks: boolean;
}

export interface SaveFlags {
  saveAs: boolean;
  overwriteChanged: boolean;
  allowLossy: boolean;
  options?: SaveAsOptions | null;
}

export type ImportFormat = "spreadsheet" | "json" | "xml" | "html" | "markdown" | "delimited" | "fixed_width" | "pattern";
export type Unmatched = "join" | "row" | "skip";
export interface ImportOptions {
  format: ImportFormat | null;
  part: number;
  raw_dates: boolean;
  record: string;
  /** Fixed width: column starts (characters from 0); empty: guessed. */
  starts?: number[];
  keep_spaces?: boolean;
  /** Log files: the line pattern; empty: guessed. */
  pattern?: string;
  unmatched?: Unmatched;
  /** Numbers shown with a zero-padding format (00123): true writes them padded, false/absent keeps the number. */
  pad_zeros?: boolean | null;
}
/** What the "exactly as written" import guards did: counts and column names only, never cell values. */
export interface ImportNotes {
  kept_big: number;
  lost_digits: number;
  padded: { name: string; count: number }[];
  padded_applied: number;
  padded_left: number;
  scanned_rows: number;
  all_scanned: boolean;
  /** The file to remember the zero-padding answer for (none: the clipboard or a private file). */
  remember_path: string | null;
  summary: string;
}
export interface SqliteOpened {
  info: DocInfo;
  note: string;
}
export interface ImportInfo {
  format: ImportFormat | null;
  parts: string[];
  hidden: boolean[];
  records: string[];
  starts?: number[];
  lines?: string[];
  pattern?: string;
}
export interface ImportStart {
  id: number;
  name: string;
  info: ImportInfo;
}
/** What Open did: a document, or a file for the import screen. */
export interface Opened {
  doc: DocInfo | null;
  import: ImportStart | null;
  /** A zip file holding several tables: pick which to open. */
  zip?: ZipList;
}
export interface ZipList {
  token: number;
  name: string;
  entries: { name: string; size: number }[];
}

export type ExportFormat = "json" | "json_lines" | "xml" | "html" | "markdown" | "template" | "xlsx" | "pdf" | "fixed_width" | "sql";
export interface ExportOptions {
  format: ExportFormat;
  header: boolean;
  arrays: boolean;
  pretty: boolean;
  root: string;
  record: string;
  before: string;
  row: string;
  after: string;
  sheet: string;
  numbers: boolean;
  /** Excel: made for Excel users (bold frozen titles with filter buttons,
   *  widths, real dates, currency and percents, IDs and ZIP codes as text). */
  look?: boolean;
  /** Excel: column widths in screen pixels (0: fit the values). */
  widths?: number[];
  /** Excel: rows with a height of their own, [shown row, pixels], rows increasing. */
  row_heights?: [number, number][];
  /** Excel: data rows per part (0: as many as a sheet holds). */
  part_rows?: number;
  /** Excel: parts as files of their own instead of sheets of one workbook. */
  split_files?: boolean;
  /** PDF: page setup. */
  page?: PageSetup;
  /** SQL: table name, name quoting, rows per INSERT, CREATE TABLE first. */
  table?: string;
  dialect?: "standard" | "mysql" | "sql_server";
  batch?: number;
  create?: boolean;
}
export interface ExportReport {
  rows: number;
  sheets: number;
  bytes: number;
  /** Files written when parts went to files of their own. */
  files?: string[];
}

export interface Backend {
  appInfo(): Promise<AppInfo>;
  /** Shows the Open dialog; null when cancelled. Spreadsheets and other
   *  formats come back as an import to finish on the import screen. */
  openFile(): Promise<Opened | null>;
  /** Shows the Import dialog; null when cancelled. */
  importFile(): Promise<ImportStart | null>;
  importText(text: string): Promise<ImportStart>;
  importInspect(id: number, format: ImportFormat | null): Promise<ImportInfo>;
  importPreview(id: number, options: ImportOptions): Promise<string[][]>;
  importNotes(id: number, options: ImportOptions): Promise<ImportNotes>;
  importFinish(id: number, options: ImportOptions): Promise<DocInfo>;
  /** Say what column `col` holds (Auto: guess again). Not an edit: nothing in the file changes. */
  setColumnType(doc: number, col: number, kind: ColumnType): Promise<DocInfo>;
  /** Put all the column types back (Undo, a saved view, a session). */
  setColumnTypes(doc: number, types: ColumnTypes): Promise<DocInfo>;
  importProgress(id: number): Promise<number | null>;
  importCancel(id: number): Promise<void>;
  importClose(id: number): Promise<void>;
  exportPreview(doc: number, options: ExportOptions): Promise<string>;
  /** Shows the Export dialog; null when cancelled. */
  exportFile(doc: number, options: ExportOptions): Promise<ExportReport | null>;
  /** Several tabs as the sheets of one Excel workbook (a Save dialog first). */
  /** Tabs as the sheets of one workbook; `widths`: each tab's on-screen
   *  column widths. */
  exportBook(docs: number[], options: ExportOptions, widths?: number[][]): Promise<ExportReport | null>;
  exportText(doc: number, options: ExportOptions): Promise<string>;
  /** Open the document's file again; `layout` null detects it again. */
  reopen(doc: number, lineByLine: boolean, layout?: Layout | null): Promise<DocInfo>;
  layoutChoices(doc: number): Promise<LayoutChoices>;
  previewLayout(doc: number, layout: Layout): Promise<string[][]>;
  newDocument(): Promise<DocInfo>;
  closeDocument(doc: number): Promise<void>;
  docInfo(doc: number): Promise<DocInfo>;
  /** Told the fresh info that follows a stale answer (the tab takes it as its state). */
  onFreshInfo(listener: (info: DocInfo) => void): void;
  viewport(doc: number, row: number, rows: number, col: number, cols: number, maxChars: number): Promise<ViewportData>;
  cellText(doc: number, row: number, col: number): Promise<{ text: string; truncated: boolean }>;
  setCells(doc: number, cells: [number, number, string][]): Promise<DocInfo>;
  /** `skip`: columns left as they are (hidden ones). */
  fillCells(doc: number, row: number, rows: number, col: number, cols: number, value: string, skip?: number[]): Promise<DocInfo>;
  /** One value into every cell of several blocks (Ctrl+click), one undo step. */
  fillAreas(doc: number, areas: import("./selection").Rect[], value: string, skip?: number[]): Promise<DocInfo>;
  /** Rows hidden between shown rows `row` and `row + rows - 1`. */
  hiddenBetween(doc: number, row: number, rows: number): Promise<number>;
  /** `skip`: columns left out (hidden ones). */
  copyCells(doc: number, row: number, rows: number, col: number, cols: number, skip?: number[]): Promise<string>;
  /** Move (or with `copy`, copy) a block of cells so its top-left lands on (toRow, toCol), one undo step. */
  moveCells(doc: number, row: number, rows: number, col: number, cols: number, toRow: number, toCol: number, copy: boolean): Promise<DocInfo>;
  change(doc: number, change: Change): Promise<DocInfo>;
  /** null when the Save As dialog was cancelled. */
  save(doc: number, flags: SaveFlags): Promise<DocInfo | null>;
  jobProgress(doc: number): Promise<JobInfo | null>;
  /** Stop the running long operation; with `label`, only if it is that one
   *  (another job that started since is left alone). */
  cancelJob(doc: number, label?: string): Promise<void>;
  sortRows(doc: number, spec: SortSpec): Promise<DocInfo>;
  setFilter(doc: number, spec: FilterSpec): Promise<DocInfo>;
  clearFilter(doc: number): Promise<DocInfo>;
  filterSpec(doc: number): Promise<FilterSpec | null>;
  filterValues(doc: number, col: number, spec: FilterSpec, search: string | null): Promise<ValueList>;
  findNext(doc: number, spec: FindSpec, from: [number, number] | null, backward: boolean): Promise<Found | null>;
  findAll(doc: number, spec: FindSpec): Promise<FindAll>;
  /** Count matches ("3 of 1,204"); a newer count or a change cancels it
   *  (kind "cancelled" or "busy"). */
  findCount(doc: number, spec: FindSpec, at: [number, number] | null): Promise<FindCount>;
  /** Matching cells in a window of the grid, to light them up. */
  findMarks(doc: number, spec: FindSpec, row: number, rows: number, col: number, cols: number): Promise<[number, number][]>;
  replacePreview(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport>;
  /** The preview's Replace all: applies exactly the previewed plan, or
   *  fails ("changed") if the search or the file changed since. */
  replaceConfirm(doc: number, spec: FindSpec, replacement: string, exclude?: [number, number][]): Promise<ReplaceReport>;
  /** Replace in one cell (Find's Replace button; not a recipe step). */
  replaceCell(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport>;
  /** Act on the rows with a match. */
  findRows(doc: number, spec: FindSpec, action: FindRowsAction, revision?: number): Promise<FindRowsReport>;
  /** Row notes that match a search. */
  findNotes(doc: number, pattern: Pattern): Promise<import("./notes").NoteMark[]>;
  /** Column titles a replace would change: [column, new title]. */
  replaceTitles(doc: number, spec: FindSpec, replacement: string): Promise<[number, string][]>;
  /** Show only the rows the last Replace all changed. */
  showChanged(doc: number, keepFirst: boolean): Promise<DocInfo>;
  /** Two columns of a tab as (find, replace with) pairs. */
  columnPairs(doc: number, findCol: number, withCol: number, skipFirst: boolean): Promise<[string, string][]>;
  /** Batch replace from a list: what would change (kept for confirm). */
  batchPreview(doc: number, spec: BatchReplace): Promise<ReplaceReport>;
  /** Apply exactly the previewed batch replace (one undo step). */
  batchConfirm(doc: number, spec: BatchReplace): Promise<ReplaceReport>;
  /** Stop a count still running (the find bar closed). */
  findCountCancel(doc: number): Promise<void>;
  /** Name a tab that has no file yet (also what Save suggests). */
  setDocTitle(doc: number, title: string): Promise<DocInfo>;
  /** A page of the last Replace all preview: every change, in order. */
  replacePage(doc: number, offset: number, count: number): Promise<ReplaceReport["preview"]>;
  replaceAll(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport>;
  /** Matches in a piece of text, as [start, end) UTF-16 offsets. */
  findInText(text: string, pattern: Pattern): Promise<[number, number][]>;
  /** Fill, Paste Special, duplicates, blanks, split/merge, sort columns. */
  dataOp(doc: number, op: DataOp): Promise<OpReport>;
  /** Pivot or unpivot into a new document. */
  newTable(doc: number, op: TableOp): Promise<DocInfo>;
  /** A new table, with what a comparison found. */
  tableReport(doc: number, op: TableOp): Promise<TableReport>;
  /** A pivot table's first rows and columns (live preview); `sample`
   *  reads only that many rows (a quick first look at a big file). */
  pivotPreview(doc: number, spec: PivotSpec, sample: number | null): Promise<PivotGrid>;
  /** The whole pivot table in a new tab. */
  pivotTable(doc: number, spec: PivotSpec): Promise<DocInfo>;
  /** The whole pivot table as tab-separated text. */
  pivotText(doc: number, spec: PivotSpec): Promise<string>;
  /** The source rows behind one pivot cell, in a new tab. */
  pivotDrill(doc: number, spec: PivotSpec, rowPath: string[], colPath: string[]): Promise<DocInfo>;
  /** The items of one field, for its pick list. */
  pivotItems(doc: number, field: PivotField, spec: PivotSpec): Promise<ItemList>;
  /** Forget pivot data kept for a tab. */
  pivotClose(doc: number): Promise<void>;
  /** Save a layout (Save dialog); its file name, or null when cancelled. */
  pivotSave(doc: number, name: string, spec: PivotSpec): Promise<string | null>;
  /** Open a saved layout fitted to the tab's columns; null when cancelled. */
  /** A saved layout fitted to the tab: its name, the layout, and the
   *  columns found by place because their titles were not (ask first). */
  pivotOpen(doc: number, keepFirst: boolean): Promise<[string, PivotSpec, PlaceSwap[]] | null>;
  /** The next shown row whose cell in `col` differs (up or down). */
  findChange(doc: number, row: number, col: number, up: boolean): Promise<number | null>;
  /** The row with the column's largest (or smallest) number. */
  findExtreme(doc: number, col: number, largest: boolean, keepFirst: boolean, read: ReadOptions): Promise<number | null>;
  /** Go To Special: the cells of an area of one kind (shown rows). */
  specialCells(doc: number, spec: SpecialSpec): Promise<SpecialCells>;
  /** Compare two columns: counts and the rows that differ. */
  compareCols(doc: number, spec: CompareCols): Promise<CompareColsReport>;
  /** Smallest and largest number of a column ("busy" while changing). */
  numberRange(doc: number, col: number, keepFirst: boolean, read: ReadOptions): Promise<[number, number] | null>;
  /** A value of the column that starts with what is typed. */
  typeahead(doc: number, col: number, prefix: string, keepFirst: boolean): Promise<string | null>;
  /** Shows the Save dialog for the new name; null when cancelled. */
  renameFile(doc: number): Promise<DocInfo | null>;
  /** Shows a folder dialog (not with `replace`); null when cancelled. */
  splitToFiles(doc: number, spec: SplitParts, replace: boolean): Promise<SplitReport | null>;
  /** Shows the Open dialog for several files; null when cancelled. */
  combineFiles(spec: Combine): Promise<DocInfo | null>;
  /** Show the file selected in File Explorer. */
  showInFolder(doc: number): Promise<void>;
  /** The first changes a cleaner would make. */
  cleanPreview(doc: number, spec: CleanSpec, now: number): Promise<CleanPreview>;
  /** Shows the Save dialog; the file's name, or null when cancelled. */
  cleanerSave(cleaner: Cleaner): Promise<string | null>;
  /** Shows the Open dialog; null when cancelled. */
  cleanerOpen(): Promise<Cleaner | null>;
  recentFiles(): Promise<RecentItem[]>;
  /** Remember the open tabs for next time (paths stay in the backend). */
  sessionSave(tabs: SessionTab[]): Promise<number>;
  /** The tabs open when the app last closed. */
  sessionList(): Promise<SessionItem[]>;
  /** Reopen a remembered tab, with where it was. */
  sessionOpen(index: number): Promise<{ opened: Opened; place: SessionPlace }>;
  sessionClear(): Promise<void>;
  /** Open tables picked from a zip file (tabs with no file). */
  zipOpen(token: number, names: string[]): Promise<DocInfo[]>;
  /** Protect these columns from changes (and no others). */
  setProtected(doc: number, cols: number[]): Promise<DocInfo>;
  /** Tidy column titles: what would change, [column, now, after]. */
  tidyTitlesPreview(doc: number, spec: TitleSpec): Promise<[number, string, string][]>;
  /** Tidy every column title (one undo step). */
  tidyTitles(doc: number, spec: TitleSpec): Promise<DocInfo>;
  openRecent(id: string): Promise<Opened>;
  /** Per tab: the folder telling same-named files apart ("" for the rest). */
  tabFolders(ids: number[]): Promise<string[]>;
  /** A column's name: its cell in the table's first row, shown or not. */
  firstRowCell(doc: number, col: number): Promise<string>;
  /** The first row's values (first 2,000 columns, cut to 200 characters). */
  firstRowNames(doc: number): Promise<string[]>;
  /** Rename a column: set its cell in the table's first row (one undo step). */
  setFirstRowCell(doc: number, col: number, value: string): Promise<DocInfo>;
  /** Copy column names: columns `cols`' names in full, one line split by
   *  `sep` (quoted where needed). */
  columnNamesLine(doc: number, cols: number[], sep: string): Promise<string>;
  /** Paste as column names: set first-row cells [column, name], one undo step. */
  setColumnNames(doc: number, names: [number, string][]): Promise<DocInfo>;
  folderList(doc: number): Promise<Listing>;
  folderOpen(doc: number, index: number): Promise<Opened>;
  viewSave(view: View): Promise<string | null>;
  viewOpen(): Promise<View | null>;
  setReadOnly(doc: number, on: boolean): Promise<DocInfo>;
  columnTypes(doc: number, skipFirst: boolean, read: ReadOptions): Promise<ColType[]>;
  columnSummary(doc: number, col: number, skipFirst: boolean, read: ReadOptions): Promise<Summary>;
  /** Keep a recovery copy of the document's unsaved changes. */
  autosave(doc: number): Promise<Autosaved>;
  /** Recovery copies left by a run that stopped without saving. */
  recoveryList(): Promise<RecoveryItem[]>;
  /** Keep earlier saves or not (the Options switch). */
  versionsKeep(on: boolean): Promise<void>;
  /** Delete every earlier save of every file; returns how many. */
  versionsClear(): Promise<number>;
  /** Open a listed recovery copy as a new unsaved tab. */
  recoveryOpen(index: number): Promise<DocInfo>;
  recoveryDiscard(index: number): Promise<void>;
  /** Earlier saves of the document's file. */
  versionsList(doc: number): Promise<VersionItem[]>;
  versionOpen(doc: number, index: number, label: string): Promise<DocInfo>;
  /** Likely clean-ups over the shown rows, each with its fix (a job). */
  cleanSuggestions(doc: number, skipFirst: boolean, read: ReadOptions): Promise<Suggestions>;
  /** Open a web or e-mail address with the system's handler (checked in the shell). */
  openLink(url: string): Promise<void>;
  /** Quick totals; a newer request cancels an older one (kind "cancelled"). */
  selectionTotals(doc: number, row: number, rows: number, col: number, cols: number, read: ReadOptions, distinct?: boolean): Promise<Totals>;
  /** Totals of several blocks (Ctrl+click), each cell once, no distinct count. */
  selectionTotalsAreas(doc: number, areas: import("./selection").Rect[], read: ReadOptions): Promise<Totals>;
  chartData(doc: number, spec: ChartSpec): Promise<ChartData>;
  highlights(doc: number, row: number, rows: number, col: number, cols: number, rules: Highlight[], skipFirst: boolean, read: ReadOptions): Promise<Mark[]>;
  /** Where Ctrl+arrow lands from a cell. */
  dataEdge(doc: number, row: number, col: number, dir: Dir): Promise<[number, number]>;
  /** The block of data around a cell, bounded by empty rows and columns
   *  (first row, last row, first column, last column); null when empty. */
  currentRegion(doc: number, row: number, col: number, first: number): Promise<[number, number, number, number] | null>;
  /** The shown row for a row number of the whole file (0-based) and whether
   *  that row itself is shown (otherwise the next shown one); null when
   *  no row is shown. */
  shownRowOf(doc: number, number: number): Promise<[number, boolean] | null>;
  /** Unsaved files (either window), files held, long jobs: for the quit question. */
  modifiedDocs(): Promise<ModifiedReport>;
  /** Quit, with the list the question showed (file and generation): "ask_again"
   *  when something else is unsaved now (nothing quits), "ignored" from a
   *  window that cannot quit. */
  quitApp(known: { id: number; generation: number }[]): Promise<string>;
  /** Move a tab to the second window (opened on demand). */
  windowMove(h: HandOver): Promise<void>;
  /** Take the tabs waiting for this window. */
  windowTake(): Promise<HandOver[]>;
  /** The second window hands a tab back to the main window. */
  windowMoveBack(h: HandOver): Promise<void>;
  /** The second window reports its tabs (and that it is alive). */
  windowSync(items: HandOver[]): Promise<void>;
  /** The second window made a document itself (a result tab): it holds it. */
  windowClaim(h: HandOver): Promise<void>;
  /** The second window closes; its tabs go back to the main window. */
  windowCloseSelf(): Promise<void>;
  onHandover(handler: () => void): void;
  onCloseRequested(handler: () => void): void;
  computePreview(doc: number, spec: ComputeSpec): Promise<ComputePreview>;
  /** The filter panel's counts (an older count still running is cancelled). */
  facetCounts(doc: number, cards: FacetCard[], spec: FilterSpec): Promise<Facets>;
  /** Free the filter panel's counts for a tab. */
  facetForget(doc: number): Promise<void>;
  /** A column's profile (shapes, histogram, odd values). */
  columnProfile(doc: number, spec: ProfileSpec): Promise<Profile>;
  /** The column insight strip, from a sample of the shown rows. */
  columnInsights(doc: number, spec: InsightSpec): Promise<Insights>;
  /** Run a formula typed in a cell; its results are one undo step. */
  cellFormula(doc: number, spec: CellFormula): Promise<FormulaReport>;
  /** The formula a cell's value came from (while the value is unchanged). */
  cellFormulaOf(doc: number, row: number, col: number): Promise<string | null>;
  /** Each distinct value of a column with its count (a job). */
  valueTally(doc: number, spec: TallySpec): Promise<ValueTally>;
  /** Group a column's values that sound or are spelled alike (a job:
   *  jobProgress and cancelJob apply). */
  lookalikeGroups(doc: number, spec: LookAlikeSpec): Promise<LookAlikes>;
  /** The first rows after a join, with counts for the whole table (a job). */
  joinPreview(doc: number, other: number | null, list: string | null, spec: JoinSpec, listId?: number | null): Promise<JoinPreview>;
  /** Keep a pasted list of keys for the Join screen; its number. */
  joinList(doc: number, text: string): Promise<number>;
  /** A tab's Join screen closed: the app lets go of what it kept. */
  joinClose(doc: number): Promise<void>;
  /** The computed-column screen closed: the app lets go of LOOKUP tables. */
  lookupClose(): Promise<void>;
  /** Columns that look like they hold private data (first 20,000 rows). */
  findPrivate(doc: number, header: boolean, read: ReadOptions, today: number): Promise<Finding[]>;
  /** Write a share-safe copy to a file the user picks (null: cancelled). */
  saveSafeCopy(doc: number, spec: SafeCopy, now: number): Promise<SafeCopyReport | null>;
  /** What changed since the file was opened or last saved. */
  changeList(doc: number): Promise<ChangeList>;
  /** Folder search: pick the folder, run, follow, stop, open a result. */
  pickSearchFolder(doc: number | null): Promise<string | null>;
  searchFolder(dir: string, spec: { pattern: Pattern; subfolders: boolean; types: string[] }): Promise<import("./FolderSearchDialog.svelte").FolderReport>;
  folderSearchProgress(): Promise<number | null>;
  folderSearchCancel(): Promise<void>;
  openFolderHit(path: string): Promise<Opened>;
  /** Cells in an area that differ from the file: [shown row, column]. */
  editedCells(doc: number, row: number, rows: number, col: number, cols: number): Promise<[number, number][]>;
  /** The list of changes as a CSV report the user names (null: cancelled). */
  saveChangeReport(doc: number): Promise<number | null>;
  /** Watched folder: choose the folder, recipe or results folder. */
  watchPick(which: "folder" | "recipe" | "out"): Promise<import("./watch").WatchStatus>;
  watchStatus(): Promise<import("./watch").WatchStatus>;
  watchStart(force: boolean, existing: boolean): Promise<import("./watch").WatchStatus>;
  watchStop(): Promise<import("./watch").WatchStatus>;
  /** Notes and flags: settings [key columns, header, count]. */
  notesInfo(doc: number): Promise<[number[], boolean, number]>;
  noteMarks(doc: number, row: number, rows: number): Promise<import("./notes").NoteMark[]>;
  noteSet(doc: number, row: number, flag: number, text: string): Promise<void>;
  notesSetKeys(doc: number, cols: number[], header: boolean): Promise<void>;
  /** Noted rows (first 5,000) and notes that match no row. */
  noteList(doc: number): Promise<[import("./notes").NoteMark[], number]>;
  notesShow(doc: number, flag: number): Promise<DocInfo>;
  /** Save the shown rows encrypted (Save dialog; null: cancelled). */
  saveEncrypted(doc: number, password: string, same?: boolean): Promise<number | null>;
  /** The encrypted file Save writes this tab to (opened from or saved to). */
  encryptedOrigin(doc: number): Promise<string | null>;
  /** Pick an encrypted file (null: cancelled), then open it. */
  pickEncrypted(): Promise<string | null>;
  openEncrypted(password: string): Promise<DocInfo>;
  /** Pick a database file: its path and tables (null: cancelled). */
  sqlitePick(): Promise<[string, import("./sqlite").DbTable[]] | null>;
  /** Open a table of the picked database in a new tab. */
  sqliteOpen(table: string): Promise<SqliteOpened>;
  /** Save as a table in a database (Save dialog; null: cancelled). */
  sqliteSave(doc: number, table: string, header: boolean, replace: boolean): Promise<number | null>;
  /** Compare with a newer copy in tab `other` (a job). */
  updatesFind(doc: number, other: number, spec: import("./updates").UpdateSpec): Promise<import("./updates").UpdateList>;
  /** Apply the ticked changes as one undo step. */
  updatesAccept(doc: number, choice: import("./updates").UpdateChoice): Promise<DocInfo>;
  updatesClose(doc: number): Promise<void>;
  /** Street range lookup from the index in tab `other`. */
  streetLookup(doc: number, other: number, cols: import("./streets").IndexCols, spec: import("./streets").StreetSpec): Promise<{ info: DocInfo; report: import("./streets").StreetReport }>;
  /** Match people: pairs that look like the same person (a job). */
  matchFind(doc: number, other: number | null, spec: import("./match").MatchSpec, bring: number[]): Promise<import("./match").MatchReport>;
  /** Add columns for accepted pairs [row id, other row, score]. */
  matchAccept(doc: number, accepted: [number, number, number][]): Promise<DocInfo>;
  matchClose(doc: number): Promise<void>;
  /** The steps done on a tab so far, as a recipe. */
  recipeGet(doc: number, header: boolean): Promise<import("./recipe").RecipeView>;
  /** Save the tab's steps as a recipe file (null: cancelled; else its path). */
  recipeSave(doc: number, header: boolean, name: string): Promise<string | null>;
  /** Pick and read a recipe file (null: cancelled). */
  recipeOpen(doc: number): Promise<import("./recipe").RecipeOpened | null>;
  /** Run a recipe on a tab (each step one undo step). */
  recipeRun(doc: number, recipe: import("./recipe").Recipe, now: number, force: boolean, answers?: [string, string][]): Promise<DocInfo>;
  /** The steps panel: the tab's steps and where the table is among them. */
  stepsView(doc: number): Promise<import("./recipe").StepsView>;
  /** Show the table as it was after step `k` (null: before the first). */
  stepsGoto(doc: number, k: number | null): Promise<DocInfo>;
  /** Change step `k`; the steps after it run again. */
  stepsChange(doc: number, k: number, change: import("./recipe").StepChange, now: number, force: boolean): Promise<DocInfo>;
  /** Make a text in step `k` a question asked each time the recipe runs. */
  stepsAsk(doc: number, k: number, path: string, name: string, prompt: string): Promise<import("./recipe").StepsView>;
  /** Run recipe on files: choose the files, a folder of them, the recipe or the results folder. */
  batchPick(which: "files" | "folder" | "recipe" | "out"): Promise<import("./batch").BatchStatus>;
  batchStatus(): Promise<import("./batch").BatchStatus>;
  /** `existing`: results already in the output folder are replaced, or the
   *  new ones get (2) and so on; without it, refused with kind
   *  "outputs_exist" when there are any. */
  batchStart(answers: [string, string][], force: boolean, existing?: "replace" | "number"): Promise<import("./batch").BatchStatus>;
  batchStop(): Promise<import("./batch").BatchStatus>;
  /** Label sheets to choose from. */
  labelSheets(): Promise<[string, import("./mail").LabelSheet][]>;
  /** The first few labels or letters, as lines. */
  mailPreview(doc: number, spec: import("./mail").MailSpec): Promise<string[][]>;
  /** Labels or letters as a PDF the user names (null: cancelled). */
  saveMail(doc: number, spec: import("./mail").MailSpec): Promise<import("./mail").MailReport | null>;
  /** Private file mode of a tab's file. */
  privateState(doc: number): Promise<PrivateState>;
  /** Put a tab's file (or its folder) in private file mode, or take it out. */
  setPrivate(doc: number, folder: boolean, on: boolean): Promise<PrivateState>;
  /** Earlier saves and recovery copies kept for the file (or its folder). */
  privateLeftovers(doc: number, folder: boolean): Promise<[number, number]>;
  /** Delete them (after the user said yes). */
  privateForget(doc: number, folder: boolean): Promise<number>;
  /** Forget the kept scramble key (its screens closed). */
  forgetScrambleKey(): Promise<void>;
  runChecks(doc: number, spec: CheckSpec): Promise<CheckReport>;
  /** Compare two open tabs by key (a job): the differences open as a new tab. */
  diffFiles(a: number, b: number, spec: DiffSpec): Promise<DiffReportOut>;
  /** The differences in a window of a tab that is in a diff (looked up by each row's key). */
  diffMarks(doc: number, row: number, rows: number, col: number, cols: number): Promise<DiffMarksOut>;
  diffNext(doc: number, row: number, backward: boolean): Promise<DiffJump | null>;
  diffClear(doc: number): Promise<void>;
  checkMarks(doc: number, row: number, rows: number, col: number, cols: number, spec: CheckSpec): Promise<CheckMark[]>;
  /** The next (previous) problem cell after a cell; `skipCols` are passed over (hidden columns). */
  nextProblem(doc: number, row: number, col: number, backward: boolean, skipCols: number[], spec: CheckSpec): Promise<JumpResult>;
  suggestRules(doc: number, col: number, keepFirst: boolean): Promise<RuleSuggestion[]>;
  /** The rules file beside the table, or null when there is none. */
  checksSidecarLoad(doc: number): Promise<SidecarRules | null>;
  /** Keep rules beside the file (an empty list removes the file); the file's name. */
  checksSidecarKeep(doc: number, checks: Check[], keepFirst: boolean): Promise<string>;
  /** What Replace all would make of the cells in a window (read-only; "busy" while a change runs). */
  replaceGhosts(doc: number, spec: FindSpec, replacement: string, row: number, rows: number, col: number, cols: number): Promise<GhostWindow>;
  /** How many cells Replace all would change (one count at a time per tab; cancelled by a newer one). */
  replaceCount(doc: number, spec: FindSpec, replacement: string): Promise<number>;
  replaceCountCancel(doc: number): Promise<void>;
  /** Shows the Save dialog; the file's name, or null when cancelled. */
  checksSave(checks: Check[]): Promise<string | null>;
  /** Shows the Open dialog; null when cancelled. */
  checksOpen(): Promise<Check[] | null>;
  /** Files opened from outside (Windows, a second launch, a drop). */
  takeOpened(): Promise<OsOpen[]>;
  onOpened(handler: () => void): void;
  onAgentGoto(handler: (g: { doc: number; row: number; col: number }) => void): void;
  fileTypes(): Promise<FileType[]>;
  setFileTypes(on: string[]): Promise<FileType[]>;
  openDefaultApps(): Promise<void>;
  agentStatus(): Promise<AgentStatus>;
  setAgentLink(on: boolean): Promise<AgentStatus>;
  logEvent(level: "info" | "warn" | "error", text: string): Promise<void>;
  openLogFolder(): Promise<void>;
}

export const inTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Listen for an app event in THIS window only. The default listener
 *  ("Any") also hears events addressed to other windows, so every event goes
 *  through here (a test fails on any other use of the event module). */
async function listenHere<T = void>(event: string, handler: (payload: T) => void): Promise<void> {
  const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  await getCurrentWebviewWindow().listen<T>(event, (e) => handler(e.payload));
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

/** Whether the slow-channel warning has been written to the log. */
let slowChannelLogged = false;

class TauriBackend implements Backend {
  appInfo() {
    return call<AppInfo>("app_info");
  }
  openFile() {
    return call<Opened | null>("open_file");
  }
  importFile() {
    return call<ImportStart | null>("import_file");
  }
  importText(text: string) {
    return call<ImportStart>("import_text", { text });
  }
  importInspect(id: number, format: ImportFormat | null) {
    return call<ImportInfo>("import_inspect", { id, format });
  }
  importPreview(id: number, options: ImportOptions) {
    return call<string[][]>("import_preview", { id, options });
  }
  importNotes(id: number, options: ImportOptions) {
    return call<ImportNotes>("import_notes", { id, options });
  }
  importFinish(id: number, options: ImportOptions) {
    return call<DocInfo>("import_finish", { id, options });
  }
  setColumnType(doc: number, col: number, kind: ColumnType) {
    return call<DocInfo>("set_column_type", { doc, col, kind });
  }
  setColumnTypes(doc: number, types: ColumnTypes) {
    return call<DocInfo>("set_column_types", { doc, types });
  }
  importProgress(id: number) {
    return call<number | null>("import_progress", { id });
  }
  importCancel(id: number) {
    return call<void>("import_cancel", { id });
  }
  importClose(id: number) {
    return call<void>("import_close", { id });
  }
  exportPreview(doc: number, options: ExportOptions) {
    return call<string>("export_preview", { doc, options });
  }
  exportBook(docs: number[], options: ExportOptions, widths: number[][] = []) {
    return call<ExportReport | null>("export_book", { docsIds: docs, options, widths });
  }
  exportFile(doc: number, options: ExportOptions) {
    return call<ExportReport | null>("export_file", { doc, options });
  }
  exportText(doc: number, options: ExportOptions) {
    return call<string>("export_text", { doc, options });
  }
  reopen(doc: number, lineByLine: boolean, layout: Layout | null = null) {
    return call<DocInfo>("reopen", { doc, lineByLine, layout });
  }
  layoutChoices(doc: number) {
    return call<LayoutChoices>("layout_choices", { doc });
  }
  previewLayout(doc: number, layout: Layout) {
    return call<string[][]>("preview_layout", { doc, layout });
  }
  newDocument() {
    return call<DocInfo>("new_document");
  }
  closeDocument(doc: number) {
    return call<void>("close_document", { doc });
  }
  private infoGuard = new InfoGuard((doc) => call<DocInfo>("doc_info", { doc }));
  docInfo(doc: number) {
    return this.infoGuard.get(doc);
  }
  onFreshInfo(listener: (info: DocInfo) => void) {
    this.infoGuard.onFresh = listener;
  }
  async viewport(doc: number, row: number, rows: number, col: number, cols: number, maxChars: number) {
    const reply = await call<ArrayBuffer | number[]>("viewport", { doc, row, rows, col, cols, maxChars });
    if (!(reply instanceof ArrayBuffer) && !slowChannelLogged) {
      slowChannelLogged = true;
      void this.logEvent("warn", "grid is using the slower fallback channel (fast channel blocked)").catch(() => {});
    }
    return decodeViewport(reply);
  }
  cellText(doc: number, row: number, col: number) {
    return call<{ text: string; truncated: boolean }>("cell_text", { doc, row, col });
  }
  setCells(doc: number, cells: [number, number, string][]) {
    return call<DocInfo>("set_cells", { doc, cells });
  }
  fillAreas(doc: number, areas: import("./selection").Rect[], value: string, skip?: number[]) {
    return call<DocInfo>("fill_areas", { doc, areas, value, skip: skip ?? null });
  }
  fillCells(doc: number, row: number, rows: number, col: number, cols: number, value: string, skip?: number[]) {
    return call<DocInfo>("fill_cells", { doc, row, rows, col, cols, value, skip: skip ?? null });
  }
  hiddenBetween(doc: number, row: number, rows: number) {
    return call<number>("hidden_between", { doc, row, rows });
  }
  copyCells(doc: number, row: number, rows: number, col: number, cols: number, skip?: number[]) {
    return call<string>("copy_cells", { doc, row, rows, col, cols, skip: skip ?? null });
  }
  moveCells(doc: number, row: number, rows: number, col: number, cols: number, toRow: number, toCol: number, copy: boolean) {
    return call<DocInfo>("move_cells", { doc, row, rows, col, cols, toRow, toCol, copy });
  }
  change(doc: number, change: Change) {
    return call<DocInfo>("change", { doc, change });
  }
  save(doc: number, f: SaveFlags) {
    return call<DocInfo | null>("save", {
      doc,
      saveAs: f.saveAs,
      overwriteChanged: f.overwriteChanged,
      allowLossy: f.allowLossy,
      options: f.options ?? null,
    });
  }
  jobProgress(doc: number) {
    return call<JobInfo | null>("job_progress", { doc });
  }
  cancelJob(doc: number, label?: string) {
    return call<void>("cancel_job", { doc, label: label ?? null });
  }
  sortRows(doc: number, spec: SortSpec) {
    return call<DocInfo>("sort_rows", { doc, spec });
  }
  setFilter(doc: number, spec: FilterSpec) {
    return call<DocInfo>("set_filter", { doc, spec });
  }
  clearFilter(doc: number) {
    return call<DocInfo>("clear_filter", { doc });
  }
  filterSpec(doc: number) {
    return call<FilterSpec | null>("filter_spec", { doc });
  }
  filterValues(doc: number, col: number, spec: FilterSpec, search: string | null) {
    return call<ValueList>("filter_values", { doc, col, spec, search });
  }
  findNext(doc: number, spec: FindSpec, from: [number, number] | null, backward: boolean) {
    return call<Found | null>("find_next", { doc, spec, from, backward });
  }
  findAll(doc: number, spec: FindSpec) {
    return call<FindAll>("find_all", { doc, spec });
  }
  findCount(doc: number, spec: FindSpec, at: [number, number] | null) {
    return call<FindCount>("find_count", { doc, spec, at });
  }
  findMarks(doc: number, spec: FindSpec, row: number, rows: number, col: number, cols: number) {
    return call<[number, number][]>("find_marks", { doc, spec, row, rows, col, cols });
  }
  replaceConfirm(doc: number, spec: FindSpec, replacement: string, exclude?: [number, number][]) {
    return call<ReplaceReport>("replace_confirm", { doc, spec, replacement, exclude: exclude ?? null });
  }
  replaceCell(doc: number, spec: FindSpec, replacement: string) {
    return call<ReplaceReport>("replace_cell", { doc, spec, replacement });
  }
  findRows(doc: number, spec: FindSpec, action: FindRowsAction, revision?: number) {
    return call<FindRowsReport>("find_rows", { doc, spec, action, revision: revision ?? null });
  }
  findNotes(doc: number, pattern: Pattern) {
    return call<import("./notes").NoteMark[]>("find_notes", { doc, pattern });
  }
  replaceTitles(doc: number, spec: FindSpec, replacement: string) {
    return call<[number, string][]>("replace_titles", { doc, spec, replacement });
  }
  showChanged(doc: number, keepFirst: boolean) {
    return call<DocInfo>("show_changed", { doc, keepFirst });
  }
  columnPairs(doc: number, findCol: number, withCol: number, skipFirst: boolean) {
    return call<[string, string][]>("column_pairs", { doc, findCol, withCol, skipFirst });
  }
  batchPreview(doc: number, spec: BatchReplace) {
    return call<ReplaceReport>("batch_preview", { doc, spec });
  }
  batchConfirm(doc: number, spec: BatchReplace) {
    return call<ReplaceReport>("batch_confirm", { doc, spec });
  }
  findCountCancel(doc: number) {
    return call<void>("find_count_cancel", { doc });
  }
  setDocTitle(doc: number, title: string) {
    return call<DocInfo>("set_doc_title", { doc, title });
  }
  replacePage(doc: number, offset: number, count: number) {
    return call<ReplaceReport["preview"]>("replace_page", { doc, offset, count });
  }
  replacePreview(doc: number, spec: FindSpec, replacement: string) {
    return call<ReplaceReport>("replace_preview", { doc, spec, replacement });
  }
  replaceAll(doc: number, spec: FindSpec, replacement: string) {
    return call<ReplaceReport>("replace_all", { doc, spec, replacement });
  }
  findInText(text: string, pattern: Pattern) {
    return call<[number, number][]>("find_in_text", { text, pattern });
  }
  dataOp(doc: number, op: DataOp) {
    return call<OpReport>("data_op", { doc, op });
  }
  newTable(doc: number, op: TableOp) {
    return call<DocInfo>("new_table", { doc, op });
  }
  tableReport(doc: number, op: TableOp) {
    return call<TableReport>("table_report", { doc, op });
  }
  pivotPreview(doc: number, spec: PivotSpec, sample: number | null) {
    return call<PivotGrid>("pivot_preview", { doc, spec, sample });
  }
  pivotTable(doc: number, spec: PivotSpec) {
    return call<DocInfo>("pivot_table", { doc, spec });
  }
  pivotText(doc: number, spec: PivotSpec) {
    return call<string>("pivot_text", { doc, spec });
  }
  pivotDrill(doc: number, spec: PivotSpec, rowPath: string[], colPath: string[]) {
    return call<DocInfo>("pivot_drill", { doc, spec, rowPath, colPath });
  }
  pivotItems(doc: number, field: PivotField, spec: PivotSpec) {
    return call<ItemList>("pivot_items", { doc, field, spec });
  }
  pivotClose(doc: number) {
    return call<void>("pivot_close", { doc });
  }
  pivotSave(doc: number, name: string, spec: PivotSpec) {
    return call<string | null>("pivot_save", { doc, name, spec });
  }
  pivotOpen(doc: number, keepFirst: boolean) {
    return call<[string, PivotSpec, PlaceSwap[]] | null>("pivot_open", { doc, keepFirst });
  }
  findChange(doc: number, row: number, col: number, up: boolean) {
    return call<number | null>("find_change", { doc, row, col, up });
  }
  findExtreme(doc: number, col: number, largest: boolean, keepFirst: boolean, read: ReadOptions) {
    return call<number | null>("find_extreme", { doc, col, largest, keepFirst, read });
  }
  specialCells(doc: number, spec: SpecialSpec) {
    return call<SpecialCells>("special_cells", { doc, spec });
  }
  compareCols(doc: number, spec: CompareCols) {
    return call<CompareColsReport>("compare_cols", { doc, spec });
  }
  numberRange(doc: number, col: number, keepFirst: boolean, read: ReadOptions) {
    return call<[number, number] | null>("number_range", { doc, col, keepFirst, read });
  }
  typeahead(doc: number, col: number, prefix: string, keepFirst: boolean) {
    return call<string | null>("typeahead", { doc, col, prefix, keepFirst });
  }
  renameFile(doc: number) {
    return call<DocInfo | null>("rename_file", { doc });
  }
  splitToFiles(doc: number, spec: SplitParts, replace: boolean) {
    return call<SplitReport | null>("split_to_files", { doc, spec, replace });
  }
  combineFiles(spec: Combine) {
    return call<DocInfo | null>("combine_files", { spec });
  }
  showInFolder(doc: number) {
    return call<void>("show_in_folder", { doc });
  }
  cleanPreview(doc: number, spec: CleanSpec, now: number) {
    return call<CleanPreview>("clean_preview", { doc, spec, now });
  }
  cleanerSave(cleaner: Cleaner) {
    return call<string | null>("cleaner_save", { cleaner });
  }
  cleanerOpen() {
    return call<Cleaner | null>("cleaner_open");
  }
  sessionSave(tabs: SessionTab[]) {
    return call<number>("session_save", { tabs });
  }
  sessionList() {
    return call<SessionItem[]>("session_list");
  }
  sessionOpen(index: number) {
    return call<{ opened: Opened; place: SessionPlace }>("session_open", { index });
  }
  sessionClear() {
    return call<void>("session_clear");
  }
  zipOpen(token: number, names: string[]) {
    return call<DocInfo[]>("zip_open", { token, names });
  }
  setProtected(doc: number, cols: number[]) {
    return call<DocInfo>("set_protected", { doc, cols });
  }
  tidyTitlesPreview(doc: number, spec: TitleSpec) {
    return call<[number, string, string][]>("tidy_titles_preview", { doc, spec });
  }
  tidyTitles(doc: number, spec: TitleSpec) {
    return call<DocInfo>("tidy_titles", { doc, spec });
  }
  recentFiles() {
    return call<RecentItem[]>("recent_files");
  }
  openRecent(id: string) {
    return call<Opened>("open_recent", { id });
  }
  tabFolders(ids: number[]) {
    return call<string[]>("tab_folders", { ids });
  }
  firstRowCell(doc: number, col: number) {
    return call<string>("first_row_cell", { doc, col });
  }
  firstRowNames(doc: number) {
    return call<string[]>("first_row_names", { doc });
  }
  setFirstRowCell(doc: number, col: number, value: string) {
    return call<DocInfo>("set_first_row_cell", { doc, col, value });
  }
  columnNamesLine(doc: number, cols: number[], sep: string) {
    return call<string>("column_names_line", { doc, cols, sep });
  }
  setColumnNames(doc: number, names: [number, string][]) {
    return call<DocInfo>("set_column_names", { doc, names });
  }
  folderList(doc: number) {
    return call<Listing>("folder_list", { doc });
  }
  folderOpen(doc: number, index: number) {
    return call<Opened>("folder_open", { doc, index });
  }
  viewSave(view: View) {
    return call<string | null>("view_save", { view });
  }
  viewOpen() {
    return call<View | null>("view_open");
  }
  setReadOnly(doc: number, on: boolean) {
    return call<DocInfo>("set_read_only", { doc, on });
  }
  columnTypes(doc: number, skipFirst: boolean, read: ReadOptions) {
    return call<ColType[]>("column_types", { doc, skipFirst, read });
  }
  columnSummary(doc: number, col: number, skipFirst: boolean, read: ReadOptions) {
    return call<Summary>("column_summary", { doc, col, skipFirst, read });
  }
  autosave(doc: number) {
    return call<Autosaved>("autosave", { doc });
  }
  recoveryList() {
    return call<RecoveryItem[]>("recovery_list");
  }
  recoveryOpen(index: number) {
    return call<DocInfo>("recovery_open", { index });
  }
  recoveryDiscard(index: number) {
    return call<void>("recovery_discard", { index });
  }
  versionsKeep(on: boolean) {
    return call<void>("versions_keep", { on });
  }
  versionsClear() {
    return call<number>("versions_clear", {});
  }
  versionsList(doc: number) {
    return call<VersionItem[]>("versions_list", { doc });
  }
  versionOpen(doc: number, index: number, label: string) {
    return call<DocInfo>("version_open", { doc, index, label });
  }
  cleanSuggestions(doc: number, skipFirst: boolean, read: ReadOptions) {
    return call<Suggestions>("clean_suggestions", { doc, skipFirst, read });
  }
  openLink(url: string) {
    return call<void>("open_link", { url });
  }
  selectionTotals(doc: number, row: number, rows: number, col: number, cols: number, read: ReadOptions, distinct = false) {
    return call<Totals>("selection_totals", { doc, row, rows, col, cols, read, distinct });
  }
  selectionTotalsAreas(doc: number, areas: import("./selection").Rect[], read: ReadOptions) {
    return call<Totals>("selection_totals_areas", { doc, areas, read });
  }
  chartData(doc: number, spec: ChartSpec) {
    return call<ChartData>("chart_data", { doc, spec });
  }
  highlights(doc: number, row: number, rows: number, col: number, cols: number, rules: Highlight[], skipFirst: boolean, read: ReadOptions) {
    return call<Mark[]>("highlights", { doc, row, rows, col, cols, rules, skipFirst, read });
  }
  dataEdge(doc: number, row: number, col: number, dir: Dir) {
    return call<[number, number]>("data_edge", { doc, row, col, dir });
  }
  currentRegion(doc: number, row: number, col: number, first: number) {
    return call<[number, number, number, number] | null>("current_region", { doc, row, col, first });
  }
  shownRowOf(doc: number, number: number) {
    return call<[number, boolean] | null>("shown_row_of", { doc, number });
  }
  modifiedDocs() {
    return call<ModifiedReport>("modified_docs");
  }
  quitApp(known: { id: number; generation: number }[]) {
    return call<string>("quit_app", { known });
  }
  windowMove(h: HandOver) {
    return call<void>("window_move", { h });
  }
  windowTake() {
    return call<HandOver[]>("window_take");
  }
  windowMoveBack(h: HandOver) {
    return call<void>("window_move_back", { h });
  }
  windowSync(items: HandOver[]) {
    return call<void>("window_sync", { items });
  }
  windowClaim(h: HandOver) {
    return call<void>("window_claim", { h });
  }
  windowCloseSelf() {
    return call<void>("window_close_self");
  }
  onHandover(handler: () => void) {
    void listenHere("handover", handler);
  }
  onCloseRequested(handler: () => void) {
    void listenHere("close-requested", handler);
  }
  facetCounts(doc: number, cards: FacetCard[], spec: FilterSpec) {
    return call<Facets>("facet_counts", { doc, cards, spec });
  }
  facetForget(doc: number) {
    return call<void>("facet_forget", { doc });
  }
  columnProfile(doc: number, spec: ProfileSpec) {
    return call<Profile>("column_profile", { doc, spec });
  }
  columnInsights(doc: number, spec: InsightSpec) {
    return call<Insights>("column_insights", { doc, spec });
  }
  cellFormula(doc: number, spec: CellFormula) {
    return call<FormulaReport>("cell_formula", { doc, spec });
  }
  cellFormulaOf(doc: number, row: number, col: number) {
    return call<string | null>("cell_formula_of", { doc, row, col });
  }
  computePreview(doc: number, spec: ComputeSpec) {
    return call<ComputePreview>("compute_preview", { doc, spec });
  }
  lookalikeGroups(doc: number, spec: LookAlikeSpec) {
    return call<LookAlikes>("lookalike_groups", { doc, spec });
  }
  valueTally(doc: number, spec: TallySpec) {
    return call<ValueTally>("value_tally", { doc, spec });
  }
  joinPreview(doc: number, other: number | null, list: string | null, spec: JoinSpec, listId: number | null = null) {
    return call<JoinPreview>("join_preview", { doc, other, list, listId, spec });
  }
  joinList(doc: number, text: string) {
    return call<number>("join_list", { doc, text });
  }
  joinClose(doc: number) {
    return call<void>("join_close", { doc });
  }
  lookupClose() {
    return call<void>("lookup_close");
  }
  findPrivate(doc: number, header: boolean, read: ReadOptions, today: number) {
    return call<Finding[]>("find_private", { doc, header, read, today });
  }
  saveSafeCopy(doc: number, spec: SafeCopy, now: number) {
    return call<SafeCopyReport | null>("save_safe_copy", { doc, spec, now });
  }
  changeList(doc: number) {
    return call<ChangeList>("change_list", { doc });
  }
  pickSearchFolder(doc: number | null) {
    return call<string | null>("pick_search_folder", { doc });
  }
  searchFolder(dir: string, spec: { pattern: Pattern; subfolders: boolean; types: string[] }) {
    return call<import("./FolderSearchDialog.svelte").FolderReport>("search_folder", { dir, spec });
  }
  folderSearchProgress() {
    return call<number | null>("folder_search_progress");
  }
  folderSearchCancel() {
    return call<void>("folder_search_cancel");
  }
  openFolderHit(path: string) {
    return call<Opened>("open_folder_hit", { path });
  }
  editedCells(doc: number, row: number, rows: number, col: number, cols: number) {
    return call<[number, number][]>("edited_cells", { doc, row, rows, col, cols });
  }
  saveChangeReport(doc: number) {
    return call<number | null>("save_change_report", { doc });
  }
  watchPick(which: "folder" | "recipe" | "out") {
    return call<import("./watch").WatchStatus>("watch_pick", { which });
  }
  watchStatus() {
    return call<import("./watch").WatchStatus>("watch_status");
  }
  watchStart(force: boolean, existing: boolean) {
    return call<import("./watch").WatchStatus>("watch_start", { force, existing });
  }
  watchStop() {
    return call<import("./watch").WatchStatus>("watch_stop");
  }
  notesInfo(doc: number) {
    return call<[number[], boolean, number]>("notes_info", { doc });
  }
  noteMarks(doc: number, row: number, rows: number) {
    return call<import("./notes").NoteMark[]>("note_marks", { doc, row, rows });
  }
  noteSet(doc: number, row: number, flag: number, text: string) {
    return call<void>("note_set", { doc, row, flag, text });
  }
  notesSetKeys(doc: number, cols: number[], header: boolean) {
    return call<void>("notes_set_keys", { doc, cols, header });
  }
  noteList(doc: number) {
    return call<[import("./notes").NoteMark[], number]>("note_list", { doc });
  }
  notesShow(doc: number, flag: number) {
    return call<DocInfo>("notes_show", { doc, flag });
  }
  saveEncrypted(doc: number, password: string, same = false) {
    return call<number | null>("save_encrypted", { doc, password, same });
  }
  encryptedOrigin(doc: number) {
    return call<string | null>("encrypted_origin", { doc });
  }
  pickEncrypted() {
    return call<string | null>("pick_encrypted");
  }
  openEncrypted(password: string) {
    return call<DocInfo>("open_encrypted", { password });
  }
  sqlitePick() {
    return call<[string, import("./sqlite").DbTable[]] | null>("sqlite_pick");
  }
  sqliteOpen(table: string) {
    return call<SqliteOpened>("sqlite_open", { table });
  }
  sqliteSave(doc: number, table: string, header: boolean, replace: boolean) {
    return call<number | null>("sqlite_save", { doc, table, header, replace });
  }
  updatesFind(doc: number, other: number, spec: import("./updates").UpdateSpec) {
    return call<import("./updates").UpdateList>("updates_find", { doc, other, spec });
  }
  updatesAccept(doc: number, choice: import("./updates").UpdateChoice) {
    return call<DocInfo>("updates_accept", { doc, choice });
  }
  updatesClose(doc: number) {
    return call<void>("updates_close", { doc });
  }
  streetLookup(doc: number, other: number, cols: import("./streets").IndexCols, spec: import("./streets").StreetSpec) {
    return call<{ info: DocInfo; report: import("./streets").StreetReport }>("street_lookup", { doc, other, cols, spec });
  }
  matchFind(doc: number, other: number | null, spec: import("./match").MatchSpec, bring: number[]) {
    return call<import("./match").MatchReport>("match_find", { doc, other, spec, bring });
  }
  matchAccept(doc: number, accepted: [number, number, number][]) {
    return call<DocInfo>("match_accept", { doc, accepted });
  }
  matchClose(doc: number) {
    return call<void>("match_close", { doc });
  }
  recipeGet(doc: number, header: boolean) {
    return call<import("./recipe").RecipeView>("recipe_get", { doc, header });
  }
  recipeSave(doc: number, header: boolean, name: string) {
    return call<string | null>("recipe_save", { doc, header, name });
  }
  recipeOpen(doc: number) {
    return call<import("./recipe").RecipeOpened | null>("recipe_open", { doc });
  }
  recipeRun(doc: number, recipe: import("./recipe").Recipe, now: number, force: boolean, answers?: [string, string][]) {
    return call<DocInfo>("recipe_run", { doc, recipe, now, force, answers: answers ?? null });
  }
  stepsView(doc: number) {
    return call<import("./recipe").StepsView>("steps_view", { doc });
  }
  stepsGoto(doc: number, k: number | null) {
    return call<DocInfo>("steps_goto", { doc, k });
  }
  stepsChange(doc: number, k: number, change: import("./recipe").StepChange, now: number, force: boolean) {
    return call<DocInfo>("steps_change", { doc, k, change, now, force });
  }
  stepsAsk(doc: number, k: number, path: string, name: string, prompt: string) {
    return call<import("./recipe").StepsView>("steps_ask", { doc, k, path, name, prompt });
  }
  batchPick(which: "files" | "folder" | "recipe" | "out") {
    return call<import("./batch").BatchStatus>("batch_pick", { which });
  }
  batchStatus() {
    return call<import("./batch").BatchStatus>("batch_status");
  }
  batchStart(answers: [string, string][], force: boolean, existing?: "replace" | "number") {
    return call<import("./batch").BatchStatus>("batch_start", { answers, force, existing: existing ?? null });
  }
  batchStop() {
    return call<import("./batch").BatchStatus>("batch_stop");
  }
  labelSheets() {
    return call<[string, import("./mail").LabelSheet][]>("label_sheets");
  }
  mailPreview(doc: number, spec: import("./mail").MailSpec) {
    return call<string[][]>("mail_preview", { doc, spec });
  }
  saveMail(doc: number, spec: import("./mail").MailSpec) {
    return call<import("./mail").MailReport | null>("save_mail", { doc, spec });
  }
  privateState(doc: number) {
    return call<PrivateState>("private_state", { doc });
  }
  setPrivate(doc: number, folder: boolean, on: boolean) {
    return call<PrivateState>("set_private", { doc, folder, on });
  }
  privateLeftovers(doc: number, folder: boolean) {
    return call<[number, number]>("private_leftovers", { doc, folder });
  }
  privateForget(doc: number, folder: boolean) {
    return call<number>("private_forget", { doc, folder });
  }
  forgetScrambleKey() {
    return call<void>("forget_scramble_key");
  }
  runChecks(doc: number, spec: CheckSpec) {
    return call<CheckReport>("run_checks", { doc, spec });
  }
  diffFiles(a: number, b: number, spec: DiffSpec) {
    return call<DiffReportOut>("diff_files", { a, b, spec });
  }
  diffMarks(doc: number, row: number, rows: number, col: number, cols: number) {
    return call<DiffMarksOut>("diff_marks", { doc, row, rows, col, cols });
  }
  diffNext(doc: number, row: number, backward: boolean) {
    return call<DiffJump | null>("diff_next", { doc, row, backward });
  }
  diffClear(doc: number) {
    return call<void>("diff_clear", { doc });
  }
  checkMarks(doc: number, row: number, rows: number, col: number, cols: number, spec: CheckSpec) {
    return call<CheckMark[]>("check_marks", { doc, row, rows, col, cols, spec });
  }
  nextProblem(doc: number, row: number, col: number, backward: boolean, skipCols: number[], spec: CheckSpec) {
    return call<JumpResult>("next_problem", { doc, row, col, backward, skipCols, spec });
  }
  suggestRules(doc: number, col: number, keepFirst: boolean) {
    return call<RuleSuggestion[]>("suggest_rules", { doc, col, keepFirst });
  }
  checksSidecarLoad(doc: number) {
    return call<SidecarRules | null>("checks_sidecar_load", { doc });
  }
  checksSidecarKeep(doc: number, checks: Check[], keepFirst: boolean) {
    return call<string>("checks_sidecar_keep", { doc, checks, keepFirst });
  }
  replaceGhosts(doc: number, spec: FindSpec, replacement: string, row: number, rows: number, col: number, cols: number) {
    return call<GhostWindow>("replace_ghosts", { doc, spec, replacement, row, rows, col, cols });
  }
  replaceCount(doc: number, spec: FindSpec, replacement: string) {
    return call<number>("replace_count", { doc, spec, replacement });
  }
  replaceCountCancel(doc: number) {
    return call<void>("replace_count_cancel", { doc });
  }
  checksSave(checks: Check[]) {
    return call<string | null>("checks_save", { checks });
  }
  checksOpen() {
    return call<Check[] | null>("checks_open");
  }
  takeOpened() {
    return call<OsOpen[]>("take_opened");
  }
  onOpened(handler: () => void) {
    void listenHere("opened", handler);
  }
  onAgentGoto(handler: (g: { doc: number; row: number; col: number }) => void) {
    void listenHere<{ doc: number; row: number; col: number }>("agent-goto", handler);
  }
  fileTypes() {
    return call<FileType[]>("file_types");
  }
  setFileTypes(on: string[]) {
    return call<FileType[]>("set_file_types", { on });
  }
  openDefaultApps() {
    return call<void>("open_default_apps");
  }
  agentStatus() {
    return call<AgentStatus>("agent_status");
  }
  setAgentLink(on: boolean) {
    return call<AgentStatus>("set_agent_link", { on });
  }
  logEvent(level: "info" | "warn" | "error", text: string) {
    return call<void>("log_event", { level, text });
  }
  openLogFolder() {
    return call<void>("open_log_folder");
  }
}

let instance: Backend | null = null;

/** Every method that changes what a file holds. While several blocks are
 *  selected (Ctrl+click) a call to any of them is refused unless the command
 *  running handles several blocks, whatever code made the call (a toolbar
 *  button, a panel, the editor). A test lists every method that returns a
 *  document's state and makes sure each is on this list or on the safe one. */
export const WRITE_METHODS: readonly string[] = [
  "setCells",
  "fillCells",
  "fillAreas",
  "moveCells",
  "change",
  "sortRows",
  "setFilter",
  "clearFilter",
  "replaceConfirm",
  "findRows",
  "replaceCell",
  "replaceTitles",
  "replaceAll",
  "batchConfirm",
  "dataOp",
  "tidyTitles",
  "setFirstRowCell",
  "setColumnNames",
  "updatesAccept",
  "matchAccept",
  "recipeRun",
  "stepsGoto",
  "stepsChange",
  "streetLookup",
];

/** The writes that MAY run while several blocks are selected, but only when the
 *  search they carry is over those blocks (scope kind "areas"): method name ->
 *  the position of its FindSpec among the arguments. Each is named here on
 *  purpose (Replace all, its confirm step, Replace of one cell, and Find's row actions) and has a
 *  test; every other write stays refused, with any scope. */
export const AREAS_SPEC_METHODS: Readonly<Record<string, number>> = {
  replaceAll: 1,
  replaceConfirm: 1,
  replaceCell: 1,
  findRows: 1,
};

function overBlocks(spec: unknown): boolean {
  const sc = (spec as { scope?: { kind?: string } } | null | undefined)?.scope;
  return sc?.kind === "areas";
}

/** Returns the reason a write to this file must not happen now, or null. The
 *  window installs it. */
export const writeGuard: { check: ((doc: number) => string | null) | null } = { check: null };

const WRITES = new Set(WRITE_METHODS);

function guarded(b: Backend): Backend {
  return new Proxy(b, {
    get(target, prop, receiver) {
      const v = Reflect.get(target, prop, receiver);
      if (typeof prop !== "string" || !WRITES.has(prop) || typeof v !== "function") return v;
      return (...args: unknown[]) => {
        const at = AREAS_SPEC_METHODS[prop];
        if (at !== undefined && overBlocks(args[at])) return (v as (...a: unknown[]) => unknown).apply(target, args);
        const why = writeGuard.check?.(args[0] as number) ?? null;
        if (why) return Promise.reject({ kind: "one_block", message: why });
        return (v as (...a: unknown[]) => unknown).apply(target, args);
      };
    },
  });
}

export function backend(): Backend {
  instance ??= guarded(inTauri() ? new TauriBackend() : new MockBackend());
  return instance;
}
