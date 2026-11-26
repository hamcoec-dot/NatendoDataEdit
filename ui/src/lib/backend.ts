// Typed access to the Rust side. In a plain browser (vite dev, tests) a stub
// backend with a generated document stands in, so the UI can be worked on
// without the desktop shell.
import { InfoGuard } from "./infoguard";
import type { GapReport, GapsSpec } from "./gaps";
import type { RelationKind, RelationsReport, RelationsSpec } from "./relations";
import type { SetsRun, SetsSpec } from "./sets";
import type { CleanMatrix, MatrixPreview, NamedCleaner } from "./matrix";
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
  /** A mark of the state the table is in (see the engine's step_mark). */
  step_mark?: number;
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
  /** The usual number of fields, and the file's rows with another number. */
  /** What the steps of a recipe just run have to say. */
  run_notes?: string[];
  usual_fields?: number;
  ragged_rows?: number;
  first_row_ragged?: boolean;
  /** Report notes: lines above and below the table (C4). */
  notes?: NoteCounts;
  long_quoted_cells: boolean;
  line_bound_from: number | null;
  error: string | null;
}

/** How many lines of report notes a tab has: lines that are not rows of the
 *  table (a report's title above it, a total line below it). */
export interface NoteCounts {
  /** Lines above the table that were in the file when it was opened (or saved). */
  above_file: number;
  /** Lines set aside above the table since ("Headers start here"). */
  above_set: number;
  /** Lines below the table that were in the file (after a save). */
  below_file: number;
  /** Lines set aside below the table since. */
  below_set: number;
  /** Saving writes them. */
  keep: boolean;
}

/** The Report notes panel: the lines themselves, read-only. */
export interface ReportNotes {
  above_file: string;
  above_file_lines: number;
  above_set: string;
  above_set_lines: number;
  below_file: string;
  below_file_lines: number;
  below_set: string;
  below_set_lines: number;
  keep: boolean;
  /** "Label: value" lines and lines of two fields. */
  info: [string, string][];
  /** The text was cut short for showing (Copy copies what is shown). */
  cut: boolean;
}

/** A row near the end that looks like a total line, and whether the rows above add up to it. */
export interface TotalRow {
  row: number;
  /** Rows from it to the end (blank lines after it go with it). */
  count: number;
  label: string;
  checks: { col: number; total: string; sum: string | null; matches: boolean }[];
  adds_up: boolean;
}

/** Which end of the table rows are set aside at. */
export type NotePlace = "head" | "foot";

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

/** A named moment in the undo history (a few bytes, no copy of the table). */
export interface Checkpoint {
  id: number;
  name: string;
  /** Made by the app before a risky change. */
  auto: boolean;
  /** The undo history still reaches it. */
  reachable: boolean;
  /** Undos (negative) or redos (positive) to get there; 0 when it is the state now or cannot be reached. */
  steps: number;
  /** Why "What changed since this?" cannot be answered ("" when it can). */
  since_note: string;
  /** The table is in exactly this state now. */
  current: boolean;
}

/** Counts of what changed since a checkpoint. */
export interface SinceTotals {
  steps: number;
  rows_changed: number;
  cells_changed: number;
  rows_added: number;
  rows_removed: number;
  cols_added: number;
  cols_removed: number;
  cols_moved: boolean;
  reordered: boolean;
  /** The columns added, by their position now. */
  added_cols: number[];
  /** The columns taken out: where they were, and their title then (empty without a title row). */
  removed_cols: { at: number; title: string }[];
  /** The columns in another place among those in both, by their position now. */
  moved_cols: number[];
  /** Lines of the whole list (what the pages go through). */
  entries: number;
}
/** One line of what changed since a checkpoint. */
export interface SinceCell {
  kind: "cell" | "row_removed" | "row_added" | "col_removed";
  /** The row's id: it follows the row through sorts and deletes. */
  id: number;
  /** Its position now in the whole table (null: not found). */
  row: number | null;
  /** Its column now (logical), for a changed cell. */
  col: number | null;
  /** For a cell of a column taken out: an index into `removed_cols`. */
  removed_col: number | null;
  old: string;
  new: string;
}
export interface SinceView {
  totals: SinceTotals;
  cells: SinceCell[];
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
/** One column's totals over the shown rows, for the totals row. */
export interface ColumnTotal {
  col: number;
  cells: number;
  filled: number;
  numbers: number;
  blank: number;
  sum: number | null;
  average: number | null;
  min: number | null;
  max: number | null;
  too_large: boolean;
  /** What to show until the person chooses: never "sum" for an identifier. */
  suggested: "sum" | "count";
}
export interface ColumnTotals {
  revision: number;
  /** Data rows shown (and so totalled). */
  rows: number;
  filtered: boolean;
  columns: ColumnTotal[];
}
/** What a look at one tab's file on disk found. */
export interface DiskEvent {
  doc: number;
  state: "same" | "changed" | "gone";
  token: string;
  modified: number | null;
}
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
/** How many dates of each kind a column holds. */
export interface DateScan {
  looked: number;
  rows: number;
  blank: number;
  iso: number;
  written: number;
  month_first_only: number;
  day_first_only: number;
  ambiguous: number;
  same_either: number;
  compact: number;
  compact_numbers: number;
  not_date: number;
  two_digit_year: number;
  set_order: "mdy" | "dmy" | null;
  other_order: number;
}
/** What goes between two rows joined into one (never the file's separator). */
export type Between = "space" | "line_break" | "nothing";
export type RowRepair =
  | { kind: "join"; groups: number[] }
  | { kind: "next"; between: Between }
  | { kind: "previous"; between: Between; header: boolean }
  | { kind: "quote_as_text" };
/** The next row of the file, as the repair bar shows it. */
export interface NextRow {
  id: number;
  file_row: number;
  edited: boolean;
  hidden: boolean;
  noted: boolean;
  /** The columns its note is matched on (empty without a note). */
  note_keys: number[];
  fields: string[];
}
/** A row as the repair bar shows it. */
export interface RowText {
  id: number;
  /** Where it is on screen (null: hidden or filtered out). */
  row: number | null;
  file_row: number;
  revision: number;
  edited: boolean;
  raw: string;
  cut: boolean;
  fields: string[];
  width: number;
  usual: number;
  line_by_line: boolean;
  no_join: string;
  quoted_as_text: string[];
  next: NextRow | null;
  no_next: string;
  previous: NextRow | null;
  no_previous: string;
  note_keys: number[];
}
/** One row "Fix all like this" would change: its cells afterwards. */
export interface ShapeRow {
  id: number;
  file_row: number;
  cells: string[];
}
/** The rows with the same number of fields as a repaired one. */
export interface SameShape {
  revision: number;
  total: number;
  hidden: number;
  first: ShapeRow[];
}
export interface RaggedList {
  /** Row ids in table order. */
  ids: number[];
  total: number;
}
export interface SplitParts {
  col: number | null;
  every: number;
  keep_first: boolean;
  /** CSV formula guard; off when left out. */
  guard?: boolean;
}
/** What a CSV report wrote. */
export interface Written {
  entries: number;
  /** Cells given an apostrophe by the formula guard. */
  guarded: number;
}
export interface SplitReport {
  guarded?: number;
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
/** The first rows of a picked file (its name only, never its folder). */
export interface FileHead {
  name: string;
  titles: string[];
  cols: number;
  rows_est: number;
  /** The whole file was read (rows_est is exact). */
  whole: boolean;
  preview: string[][];
}
/** Where each column of each file goes: an output column, or null (left out). */
export interface CombinePlan {
  titles: string[];
  files: (number | null)[][];
}
export type CombineWhy = { kind: "tidied" } | { kind: "similar"; score: number };
export interface CombineSuggestion {
  keep: number;
  merge: number;
  why: CombineWhy;
  keep_samples: string[];
  merge_samples: string[];
}
export interface CombineStart {
  token: number;
  files: FileHead[];
  plan: CombinePlan;
  suggestions: CombineSuggestion[];
}
export interface CombineChecked {
  plan: CombinePlan;
  suggestions: CombineSuggestion[];
}
export type CombineExtras = "add" | "leave_out";
export interface CombineFit extends CombineChecked {
  /** [file, output] the file has nothing for. */
  missing: [number, number][];
  /** [file, column] the map does not name. */
  extras: [number, number][];
  problems: { file: number; columns: number[]; what: string }[];
  file_name_column: boolean;
  extras_mode: CombineExtras;
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
  | { kind: "problems"; checks: CheckSpec }
  | { kind: "wrong_width"; header: boolean };
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
  /** Left out: every row. "shown": hidden rows stay where they are. */
  scope?: "all" | "shown";
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
  /** CSV formula guard; on when left out. */
  guard?: boolean;
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
  guarded?: number;
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
  | { op: "clean"; spec: CleanSpec; now: number; typed?: [number, number, string][] }
  | { op: "split_name"; spec: SplitName }
  | { op: "fill_example"; spec: ExampleSpec }
  | { op: "compute"; spec: ComputeSpec }
  | { op: "show_problems"; spec: CheckSpec; passing?: boolean }
  | { op: "show_ragged"; pick?: "all" | "short" | "long"; keep_first: boolean }
  | { op: "show_gap_rows"; spec: GapsSpec }
  | { op: "show_relation_exceptions"; kind: RelationKind; a: number; b: number }
  | { op: "clean_matrix"; spec: CleanMatrix; now: number; revision?: number }
  | { op: "show_shape"; spec: ShapeFilter }
  | { op: "sort_problems"; spec: CheckSpec; scope?: "all" | "shown" }
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
  /** A new column is filled in hidden rows too (left out: it is). */
  hidden?: boolean;
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
  | { type: "unique"; match_case: boolean; also?: number[] }
  | { type: "expression"; expr: string }
  /** The value must be one of `column`'s values in the open file called `table` (a foreign key). */
  | { type: "foreign_key"; table: string; column: string; match_case: boolean; trim: boolean; allow_blank: boolean };
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
/** How a count compares with the number given. */
export type Compare = "equals" | "at_least" | "at_most";
/** How close two figures must be to count as equal: by an amount, or by a share of the larger one (0.01 is 1%). */
export type Tolerance = { kind: "absolute"; value: number } | { kind: "relative"; value: number };
/** A rule about the table as a whole. */
export type TableCheckKind =
  | { type: "row_count"; compare: Compare; value: number }
  | { type: "row_count_same_as"; table: string }
  | { type: "no_blanks"; col: number }
  | { type: "distinct"; col: number; compare: Compare; value: number }
  /** A formula over whole-column figures (SUM([A]) = SUM([B])); a top-level = or <> compares within the tolerance. */
  | { type: "aggregate"; expr: string; tolerance?: Tolerance };
export interface TableCheck {
  name?: string | null;
  kind: TableCheckKind;
}
/** What a table check found (`ok` null: it did not run, `why` says why). */
export interface TableResult {
  check: number;
  name: string;
  ok: boolean | null;
  actual: string;
  expected: string;
  why?: string | null;
}
/** Rules read from a `.ndchecks` file beside the table. */
export interface SidecarRules {
  checks: Check[];
  table_checks?: TableCheck[];
  skipped: number;
  keep_first: boolean;
  file: string;
}
export interface CheckSpec {
  checks: Check[];
  keep_first: boolean;
  read: ReadOptions;
  now: number;
  /** Rules about the whole table, run with the rules. */
  table_checks?: TableCheck[];
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
  /** Rules that did not run (a "value must exist in" rule whose file is not open), and why. */
  not_run?: { check: number; why: string }[];
  /** The files "value must exist in" rules read: rows read, rows hidden (not read), revisions then. */
  tables?: { check: number; table: string; column: string; doc: number; revision: number; data_revision: number; rows: number; hidden: number }[];
  /** What each table check found. */
  table?: TableResult[];
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
  | { op: "gaps"; spec: GapsSpec }
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
  /** Write a processing receipt beside the saved file (null: no). */
  receipt?: ReceiptAsk | null;
}
/** A processing receipt asked for: full paths and full step settings are off unless ticked. */
export interface ReceiptAsk {
  full_paths: boolean;
  full_steps: boolean;
}
/** What checking a receipt found. */
export interface ReceiptReport {
  /** Every check passed and at least one file was compared. */
  ok: boolean;
  /** Files compared with the receipt. */
  compared: number;
  /** The one-line answer (the same as the command line's). */
  verdict: string;
  lines: { what: string; ok: boolean; why: string }[];
  summary: Record<string, unknown>;
}

/** One encoding of the picker. */
export interface EncodingChoice {
  /** The label the engine knows (utf-8, windows-1252, shift_jis ...). */
  label: string;
  name: string;
  /** The file can switch to it without reopening (ASCII-compatible). */
  direct: boolean;
}

/** What a scan of the raw row bytes found (counts only; never a value). */
export interface EncodingScan {
  looked: number;
  ascii: number;
  utf8: number;
  invalid: number;
  double_encoded: number;
  mixed: boolean;
  /** Rows that are invalid or double-encoded. */
  rows: number;
}

/** What an X-ray run found over the shown rows (counts and columns; never a value). */
export interface XrayReport {
  looked: number;
  cells: number;
  rows: number;
  kinds: [string, number][];
  columns: { col: number; cells: number; kinds: [string, number][] }[];
}

export type ImportFormat = "spreadsheet" | "json" | "xml" | "html" | "markdown" | "delimited" | "fixed_width" | "pattern" | "dbf";
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
  /** dBase tables. */
  dbf?: DbfOptions;
}
export interface DbfOptions {
  /** An encoding label; empty: from the .cpg file, the table's own code, or a guess. */
  encoding: string;
  fid: "off" | "from0" | "from1";
  show_deleted: boolean;
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
  /** dBase: the record count, said under the preview. */
  records?: string;
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
  /** Why this file opened (a shapefile opens its attribute table). */
  note?: string;
}
/** What Open did: a document, or a file for the import screen. */
export interface Opened {
  doc: DocInfo | null;
  import: ImportStart | null;
  /** A zip file holding several tables: pick which to open. */
  zip?: ZipList;
  /** A database or GeoPackage: its path and tables, to pick from. */
  sqlite?: [string, import("./sqlite").DbTable[]];
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
  dateScan(doc: number, col: number, keepFirst: boolean, sample: number | null): Promise<DateScan>;
  dateMarks(doc: number, col: number, keepFirst: boolean, row: number, rows: number): Promise<number[]>;
  dateFind(doc: number, col: number, keepFirst: boolean, from: number, forward: boolean): Promise<number | null>;
  /** The first 500 rows whose date can be read two ways: shown row, row number, the cell's text. */
  dateList(doc: number, col: number, keepFirst: boolean): Promise<[number, number, string][]>;
  /** Rows hidden by hand in the gap just above shown row `row` (the row count: after the last). */
  gapHidden(doc: number, row: number): Promise<number>;
  /** Why rows `at..at+count` cannot be set aside as report notes (null: they can). */
  setAsideCheck(doc: number, at: number, count: number, place: NotePlace): Promise<string | null>;
  /** Set rows aside above ("head") or below the table as report notes (one undo step). */
  setAside(doc: number, at: number, count: number, place: NotePlace): Promise<DocInfo>;
  reportNotes(doc: number): Promise<ReportNotes>;
  /** A total line among the last 5 rows (null: none, or rows are hidden), with its sums checked. */
  totalRow(doc: number, header: boolean, read: ReadOptions): Promise<TotalRow | null>;
  /** Put every report note back into the table (one undo step). */
  putBackNotes(doc: number): Promise<DocInfo>;
  /** Save the report notes, or leave them out (one undo step; nothing is lost). */
  keepNotes(doc: number, keep: boolean): Promise<DocInfo>;
  raggedList(doc: number, keepFirst: boolean): Promise<RaggedList>;
  rowText(doc: number, row: number | null, rowId: number | null, keepFirst: boolean): Promise<RowText>;
  sameShape(doc: number, keepFirst: boolean, fields: number, groups: number[]): Promise<SameShape>;
  /** "Fix all like this": the rows changed and the document's facts. */
  fixAll(doc: number, keepFirst: boolean, revision: number, fields: number, groups: number[], skip: number[]): Promise<[number, DocInfo]>;
  /** Empty, untitled columns past the usual number (0: none), and the revision looked at. */
  emptyExtraColumns(doc: number, keepFirst: boolean): Promise<[number, number]>;
  removeEmptyExtraColumns(doc: number, keepFirst: boolean, revision: number): Promise<DocInfo>;
  /** A shown row's fields and the usual number. */
  rowWidth(doc: number, row: number): Promise<[number, number]>;
  repairRow(doc: number, rowId: number, revision: number, repair: RowRepair): Promise<DocInfo>;
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
  gapAudit(doc: number, spec: GapsSpec): Promise<GapReport>;
  findRelations(doc: number, spec: RelationsSpec): Promise<RelationsReport>;
  setsRun(owner: number, tabs: number[], spec: SetsSpec): Promise<SetsRun>;
  setsOpen(id: number, mask: number, list: number, keys: boolean): Promise<DocInfo>;
  setsTable(id: number): Promise<DocInfo>;
  setsClose(id: number): Promise<void>;
  cleanMatrixPreview(doc: number, spec: CleanMatrix, now: number): Promise<MatrixPreview>;
  cleanerOpenNamed(): Promise<NamedCleaner | null>;
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
  /** Combine with a column map: pick the files and read their first rows (null: cancelled). */
  combinePick(spec: Combine): Promise<CombineStart | null>;
  combineCheck(token: number, plan: CombinePlan): Promise<CombineChecked>;
  combineRefresh(token: number): Promise<CombineStart>;
  /** Open the files fully, check they still fit the map, and combine into a new tab. */
  combineRun(token: number, plan: CombinePlan, source: boolean): Promise<DocInfo>;
  combineCancel(token: number): Promise<void>;
  /** Save the map (titles only); false: cancelled. */
  combineSaveMapping(token: number, plan: CombinePlan, extras: CombineExtras): Promise<boolean>;
  combineLoadMapping(token: number): Promise<CombineFit | null>;
  /** Show the file selected in File Explorer. */
  showInFolder(doc: number): Promise<void>;
  /** The first changes a cleaner would make. */
  cleanPreview(doc: number, spec: CleanSpec, now: number): Promise<CleanPreview>;
  /** A page of what a cleaner would change, with exact counts over everything it looks at (Sanitize). */
  cleanPreviewPage(doc: number, spec: CleanSpec, now: number, offset: number, want: number): Promise<CleanPreview>;
  /** Look for look-alike characters in the shown rows (counts and row numbers only). */
  xrayReport(doc: number, cols: number[], keepFirst: boolean): Promise<XrayReport>;
  /** The encodings the picker offers. */
  encodingChoices(): Promise<EncodingChoice[]>;
  /** The first rows of the file read as another encoding (a throwaway read; refused when the file changed on disk). */
  encodingPreview(doc: number, encoding: string, rows: number): Promise<string[][]>;
  /** Read the file as another ASCII-compatible encoding without reopening: one undo step. */
  encodingApply(doc: number, encoding: string): Promise<DocInfo>;
  /** The file opened again as another encoding: a new document (the old one stays until the caller closes it). */
  reopenEncoding(doc: number, encoding: string, lineByLine: boolean, layout?: Layout | null): Promise<DocInfo>;
  /** Look for rows whose bytes are in another encoding: a job with progress and Cancel. */
  scanEncodings(doc: number, keepFirst: boolean): Promise<EncodingScan>;
  /** Show only the rows the scan flagged. */
  encodingShowRows(doc: number, keepFirst: boolean): Promise<DocInfo>;
  /** Show only the rows an X-ray finds gremlins in. */
  xrayShow(doc: number, cols: number[], keepFirst: boolean): Promise<DocInfo>;
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
  /** The totals row (at most 256 columns at once). */
  columnTotals(doc: number, cols: number[], header: boolean, read: ReadOptions): Promise<ColumnTotals>;
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
  /** Look whether another program changed an open file (waits at most 2 s). */
  diskCheck(): Promise<DiskEvent[]>;
  /** The disk's version of the tab's file, and its differences from what the tab read, as two new tabs. */
  diskCompare(doc: number, header: boolean): Promise<[DocInfo, DocInfo]>;
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
  saveChangeReport(doc: number, guard: boolean): Promise<Written | null>;
  /** Watched folder: choose the folder, recipe or results folder. */
  watchPick(which: "folder" | "recipe" | "out"): Promise<import("./watch").WatchStatus>;
  watchStatus(): Promise<import("./watch").WatchStatus>;
  watchStart(force: boolean, existing: boolean, receipt?: ReceiptAsk | null): Promise<import("./watch").WatchStatus>;
  /** What the last save with a receipt said about it (once). */
  receiptNote(doc: number): Promise<string | null>;
  /** Tools > Verify a receipt: files are picked in the app's own dialogs. Null: cancelled. */
  receiptVerify(withFiles: boolean, chain: boolean): Promise<ReceiptReport | null>;
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
  /** Preview golden records from a one-list match's accepted pairs (a job). */
  goldenPlan(doc: number, accepted: [number, number, number][], spec: import("./golden").GoldenSpec): Promise<import("./golden").GoldenSummary>;
  /** A page of the preview's clusters. */
  goldenPage(doc: number, page: number): Promise<import("./golden").GoldenCluster[]>;
  /** The result as a new tab, and the folded rows as another with `away`. */
  goldenCreate(doc: number, away: boolean): Promise<import("./golden").GoldenMade>;
  goldenClose(doc: number): Promise<void>;
  /** Where to save a patch (Save dialog; null: cancelled). */
  patchPickSave(doc: number): Promise<string | null>;
  /** Write the differences from tab `a` to the newer tab `b` (a job on `a`). `plain`: the user chose no phrase. */
  patchMake(a: number, b: number, spec: import("./patch").PatchSpec, path: string, phrase: string | null, plain: boolean): Promise<import("./patch").PatchSaved>;
  /** Pick a patch file (null: cancelled). */
  patchPick(): Promise<string | null>;
  patchProtected(path: string): Promise<boolean>;
  /** The dry run of a patch file on a tab (a job). */
  patchPreview(doc: number, path: string, phrase: string | null): Promise<import("./patch").PatchPreview>;
  /** Apply the last dry run as one undo step. */
  patchApply(doc: number): Promise<import("./patch").PatchApplied & { info: DocInfo }>;
  /** Save the dry run's lines as CSV (Save dialog; null: cancelled). */
  patchReportSave(doc: number): Promise<string | null>;
  patchClose(doc: number): Promise<void>;
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
  /** The tab's checkpoints (named moments in the undo history), each with whether the history still reaches it. */
  checkpointList(doc: number): Promise<Checkpoint[]>;
  /** Name this moment: nothing is copied. */
  checkpointAdd(doc: number, name: string): Promise<Checkpoint[]>;
  checkpointRemove(doc: number, checkpoint: number): Promise<Checkpoint[]>;
  /** Go to the state a checkpoint names (one jump, by undo or redo). */
  checkpointRestore(doc: number, checkpoint: number): Promise<DocInfo>;
  /** What changed since a checkpoint: the counts and a page of the changed cells (a job with progress and Cancel). */
  checkpointSince(doc: number, checkpoint: number, header: boolean, offset: number, want: number): Promise<SinceView>;
  /** The same as a CSV report the user names (null: cancelled): the number of cells written. */
  checkpointSinceReport(doc: number, checkpoint: number, header: boolean, guard: boolean): Promise<Written | null>;
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
  batchStart(answers: [string, string][], force: boolean, existing?: "replace" | "number", receipt?: ReceiptAsk | null): Promise<import("./batch").BatchStatus>;
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
  checksSidecarKeep(doc: number, checks: Check[], keepFirst: boolean, tableChecks?: TableCheck[]): Promise<string>;
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
  dateScan(doc: number, col: number, keepFirst: boolean, sample: number | null) {
    return call<DateScan>("date_scan", { doc, col, keepFirst, sample });
  }
  gapHidden(doc: number, row: number) {
    return call<number>("gap_hidden", { doc, row });
  }
  setAsideCheck(doc: number, at: number, count: number, place: NotePlace) {
    return call<string | null>("set_aside_check", { doc, at, count, place });
  }
  setAside(doc: number, at: number, count: number, place: NotePlace) {
    return call<DocInfo>("set_aside", { doc, at, count, place });
  }
  reportNotes(doc: number) {
    return call<ReportNotes>("report_notes", { doc });
  }
  totalRow(doc: number, header: boolean, read: ReadOptions) {
    return call<TotalRow | null>("total_row", { doc, header, read });
  }
  putBackNotes(doc: number) {
    return call<DocInfo>("put_back_notes", { doc });
  }
  keepNotes(doc: number, keep: boolean) {
    return call<DocInfo>("keep_notes", { doc, keep });
  }
  dateList(doc: number, col: number, keepFirst: boolean) {
    return call<[number, number, string][]>("date_list", { doc, col, keepFirst });
  }
  dateMarks(doc: number, col: number, keepFirst: boolean, row: number, rows: number) {
    return call<number[]>("date_marks", { doc, col, keepFirst, row, rows });
  }
  dateFind(doc: number, col: number, keepFirst: boolean, from: number, forward: boolean) {
    return call<number | null>("date_find", { doc, col, keepFirst, from, forward });
  }
  raggedList(doc: number, keepFirst: boolean) {
    return call<RaggedList>("ragged_list", { doc, keepFirst });
  }
  rowText(doc: number, row: number | null, rowId: number | null, keepFirst: boolean) {
    return call<RowText>("row_text", { doc, row, rowId, keepFirst });
  }
  sameShape(doc: number, keepFirst: boolean, fields: number, groups: number[]) {
    return call<SameShape>("same_shape", { doc, keepFirst, fields, groups });
  }
  fixAll(doc: number, keepFirst: boolean, revision: number, fields: number, groups: number[], skip: number[]) {
    return call<[number, DocInfo]>("fix_all", { doc, keepFirst, revision, fields, groups, skip });
  }
  emptyExtraColumns(doc: number, keepFirst: boolean) {
    return call<[number, number]>("empty_extra_columns", { doc, keepFirst });
  }
  removeEmptyExtraColumns(doc: number, keepFirst: boolean, revision: number) {
    return call<DocInfo>("remove_empty_extra_columns", { doc, keepFirst, revision });
  }
  rowWidth(doc: number, row: number) {
    return call<[number, number]>("row_width", { doc, row });
  }
  repairRow(doc: number, rowId: number, revision: number, repair: RowRepair) {
    return call<DocInfo>("repair_row", { doc, rowId, revision, repair });
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
      receipt: f.receipt ?? null,
    });
  }
  receiptNote(doc: number) {
    return call<string | null>("receipt_note", { doc });
  }
  receiptVerify(withFiles: boolean, chain: boolean) {
    return call<ReceiptReport | null>("receipt_verify", { withFiles, chain });
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
  setsRun(owner: number, tabs: number[], spec: SetsSpec) {
    return call<SetsRun>("sets_run", { owner, tabs, spec });
  }
  setsOpen(id: number, mask: number, list: number, keys: boolean) {
    return call<DocInfo>("sets_open", { id, mask, list, keys });
  }
  setsTable(id: number) {
    return call<DocInfo>("sets_table", { id });
  }
  setsClose(id: number) {
    return call<void>("sets_close", { id });
  }
  findRelations(doc: number, spec: RelationsSpec) {
    return call<RelationsReport>("find_relations", { doc, spec });
  }
  gapAudit(doc: number, spec: GapsSpec) {
    return call<GapReport>("gap_audit", { doc, spec });
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
  combinePick(spec: Combine) {
    return call<CombineStart | null>("combine_pick", { spec });
  }
  combineCheck(token: number, plan: CombinePlan) {
    return call<CombineChecked>("combine_check", { token, plan });
  }
  combineRefresh(token: number) {
    return call<CombineStart>("combine_refresh", { token });
  }
  combineRun(token: number, plan: CombinePlan, source: boolean) {
    return call<DocInfo>("combine_run", { token, plan, source });
  }
  combineCancel(token: number) {
    return call<void>("combine_cancel", { token });
  }
  combineSaveMapping(token: number, plan: CombinePlan, extras: CombineExtras) {
    return call<boolean>("combine_save_mapping", { token, plan, extras });
  }
  combineLoadMapping(token: number) {
    return call<CombineFit | null>("combine_load_mapping", { token });
  }
  showInFolder(doc: number) {
    return call<void>("show_in_folder", { doc });
  }
  cleanPreviewPage(doc: number, spec: CleanSpec, now: number, offset: number, want: number) {
    return call<CleanPreview>("clean_preview_page", { doc, spec, now, offset, want });
  }
  encodingChoices() {
    return call<EncodingChoice[]>("encoding_choices", {});
  }
  encodingPreview(doc: number, encoding: string, rows: number) {
    return call<string[][]>("encoding_preview", { doc, encoding, rows });
  }
  encodingApply(doc: number, encoding: string) {
    return call<DocInfo>("encoding_apply", { doc, encoding });
  }
  reopenEncoding(doc: number, encoding: string, lineByLine: boolean, layout: Layout | null = null) {
    return call<DocInfo>("reopen_encoding", { doc, encoding, lineByLine, layout });
  }
  scanEncodings(doc: number, keepFirst: boolean) {
    return call<EncodingScan>("scan_encodings", { doc, keepFirst });
  }
  encodingShowRows(doc: number, keepFirst: boolean) {
    return call<DocInfo>("encoding_show_rows", { doc, keepFirst });
  }
  xrayReport(doc: number, cols: number[], keepFirst: boolean) {
    return call<XrayReport>("xray_report", { doc, cols, keepFirst });
  }
  xrayShow(doc: number, cols: number[], keepFirst: boolean) {
    return call<DocInfo>("xray_show", { doc, cols, keepFirst });
  }
  cleanPreview(doc: number, spec: CleanSpec, now: number) {
    return call<CleanPreview>("clean_preview", { doc, spec, now });
  }
  cleanerSave(cleaner: Cleaner) {
    return call<string | null>("cleaner_save", { cleaner });
  }
  cleanMatrixPreview(doc: number, spec: CleanMatrix, now: number) {
    return call<MatrixPreview>("clean_matrix_preview", { doc, spec, now });
  }
  cleanerOpenNamed() {
    return call<NamedCleaner | null>("cleaner_open_named");
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
  columnTotals(doc: number, cols: number[], header: boolean, read: ReadOptions) {
    return call<ColumnTotals>("column_totals", { doc, cols, header, read });
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
  diskCheck() {
    return call<DiskEvent[]>("disk_check");
  }
  diskCompare(doc: number, header: boolean) {
    return call<[DocInfo, DocInfo]>("disk_compare", { doc, header });
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
  saveChangeReport(doc: number, guard: boolean) {
    return call<Written | null>("save_change_report", { doc, guard });
  }
  watchPick(which: "folder" | "recipe" | "out") {
    return call<import("./watch").WatchStatus>("watch_pick", { which });
  }
  watchStatus() {
    return call<import("./watch").WatchStatus>("watch_status");
  }
  watchStart(force: boolean, existing: boolean, receipt?: ReceiptAsk | null) {
    return call<import("./watch").WatchStatus>("watch_start", { force, existing, receipt: receipt ?? null });
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
  goldenPlan(doc: number, accepted: [number, number, number][], spec: import("./golden").GoldenSpec) {
    return call<import("./golden").GoldenSummary>("golden_plan", { doc, accepted, spec });
  }
  goldenPage(doc: number, page: number) {
    return call<import("./golden").GoldenCluster[]>("golden_page", { doc, page });
  }
  goldenCreate(doc: number, away: boolean) {
    return call<import("./golden").GoldenMade>("golden_create", { doc, away });
  }
  goldenClose(doc: number) {
    return call<void>("golden_close", { doc });
  }
  patchPickSave(doc: number) {
    return call<string | null>("patch_pick_save", { doc });
  }
  patchMake(a: number, b: number, spec: import("./patch").PatchSpec, path: string, phrase: string | null, plain: boolean) {
    return call<import("./patch").PatchSaved>("patch_make", { a, b, spec, path, phrase, plain });
  }
  patchPick() {
    return call<string | null>("patch_pick");
  }
  patchProtected(path: string) {
    return call<boolean>("patch_protected", { path });
  }
  patchPreview(doc: number, path: string, phrase: string | null) {
    return call<import("./patch").PatchPreview>("patch_preview", { doc, path, phrase });
  }
  patchApply(doc: number) {
    return call<import("./patch").PatchApplied & { info: DocInfo }>("patch_apply", { doc });
  }
  patchReportSave(doc: number) {
    return call<string | null>("patch_report_save", { doc });
  }
  patchClose(doc: number) {
    return call<void>("patch_close", { doc });
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
  checkpointList(doc: number) {
    return call<Checkpoint[]>("checkpoint_list", { doc });
  }
  checkpointAdd(doc: number, name: string) {
    return call<Checkpoint[]>("checkpoint_add", { doc, name });
  }
  checkpointRemove(doc: number, checkpoint: number) {
    return call<Checkpoint[]>("checkpoint_remove", { doc, checkpoint });
  }
  checkpointRestore(doc: number, checkpoint: number) {
    return call<DocInfo>("checkpoint_restore", { doc, checkpoint });
  }
  checkpointSince(doc: number, checkpoint: number, header: boolean, offset: number, want: number) {
    return call<SinceView>("checkpoint_since", { doc, checkpoint, header, offset, want });
  }
  checkpointSinceReport(doc: number, checkpoint: number, header: boolean, guard: boolean) {
    return call<Written | null>("checkpoint_since_report", { doc, checkpoint, header, guard });
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
  batchStart(answers: [string, string][], force: boolean, existing?: "replace" | "number", receipt?: ReceiptAsk | null) {
    return call<import("./batch").BatchStatus>("batch_start", { answers, force, existing: existing ?? null, receipt: receipt ?? null });
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
  checksSidecarKeep(doc: number, checks: Check[], keepFirst: boolean, tableChecks?: TableCheck[]) {
    return call<string>("checks_sidecar_keep", { doc, checks, keepFirst, tableChecks: tableChecks ?? [] });
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
  "repairRow",
  "fixAll",
  "removeEmptyExtraColumns",
  "encodingApply",
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
  "updatesAccept", "patchApply",
  "matchAccept",
  "recipeRun",
  "stepsGoto",
  "checkpointRestore",
  "stepsChange",
  "streetLookup",
  "setAside",
  "putBackNotes",
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
