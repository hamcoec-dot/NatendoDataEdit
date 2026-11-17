// Typed access to the Rust side. In a plain browser (vite dev, tests) a stub
// backend with a generated document stands in, so the UI can be worked on
// without the desktop shell.
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
  /** A filter rule sits on a deleted column: rows stay hidden without a funnel. */
  filter_on_deleted: boolean;
  cols: number;
  indexed: boolean;
  progress: number;
  modified: boolean;
  can_undo: boolean;
  can_redo: boolean;
  history_pos: number;
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
  | { op: "insert_cols"; at: number; count: number }
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
export interface TableReport {
  info: DocInfo;
  compare: CompareReport | null;
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
export type PatternKind = "plain" | "wildcard" | "regex";
export interface Pattern {
  text: string;
  kind: PatternKind;
  match_case: boolean;
  whole_cell: boolean;
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
  | { kind: "range"; row: number; rows: number; col: number; cols: number };
export interface FindSpec {
  pattern: Pattern;
  scope: Scope;
  skip_first: boolean;
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
}
export interface FindAll {
  hits: Hit[];
  cells: number;
}
export interface ReplaceReport {
  cells: number;
  hits: number;
  preview: { row: number; number: number; col: number; before: string; after: string }[];
  info: DocInfo | null;
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
}
export type PasteOp = "none" | "add" | "subtract" | "multiply" | "divide";
export interface PasteSpec {
  row: number;
  col: number;
  op: PasteOp;
  skip_blanks: boolean;
  transpose: boolean;
  read?: ReadOptions;
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
}
export interface ValueTally {
  values: { value: string; count: number }[];
  blanks: number;
  rows: number;
  complete: boolean;
  /** Indexes into `values`; the first of each is the most common. */
  look_alikes: { members: number[] }[];
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
  | { op: "extract"; spec: Extract };

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
export interface ComputePreview {
  /** Row number (from 1) and result. */
  rows: [number, string][];
  errors: number;
}

export type CheckKind =
  | { type: "required" }
  | { type: "number"; min: number | null; max: number | null; whole: boolean }
  | { type: "date"; min: string | null; max: string | null }
  | { type: "length"; min: number | null; max: number | null }
  | { type: "list"; values: string[]; match_case: boolean }
  | { type: "pattern"; pattern: Pattern }
  | { type: "unique"; match_case: boolean }
  | { type: "expression"; expr: string };
export interface Check {
  /** Column (ignored for expressions). */
  col: number | null;
  kind: CheckKind;
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
  | { op: "compare"; other: number; spec: CompareSpec };
export interface OpReport {
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
}

export type ExportFormat = "json" | "xml" | "html" | "markdown" | "template" | "xlsx" | "pdf" | "fixed_width" | "sql";
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
  importFinish(id: number, options: ImportOptions): Promise<DocInfo>;
  importProgress(id: number): Promise<number | null>;
  importCancel(id: number): Promise<void>;
  importClose(id: number): Promise<void>;
  exportPreview(doc: number, options: ExportOptions): Promise<string>;
  /** Shows the Export dialog; null when cancelled. */
  exportFile(doc: number, options: ExportOptions): Promise<ExportReport | null>;
  exportText(doc: number, options: ExportOptions): Promise<string>;
  /** Open the document's file again; `layout` null detects it again. */
  reopen(doc: number, lineByLine: boolean, layout?: Layout | null): Promise<DocInfo>;
  layoutChoices(doc: number): Promise<LayoutChoices>;
  previewLayout(doc: number, layout: Layout): Promise<string[][]>;
  newDocument(): Promise<DocInfo>;
  closeDocument(doc: number): Promise<void>;
  docInfo(doc: number): Promise<DocInfo>;
  viewport(doc: number, row: number, rows: number, col: number, cols: number, maxChars: number): Promise<ViewportData>;
  cellText(doc: number, row: number, col: number): Promise<{ text: string; truncated: boolean }>;
  setCells(doc: number, cells: [number, number, string][]): Promise<DocInfo>;
  fillCells(doc: number, row: number, rows: number, col: number, cols: number, value: string): Promise<DocInfo>;
  copyCells(doc: number, row: number, rows: number, col: number, cols: number): Promise<string>;
  /** Move (or with `copy`, copy) a block of cells so its top-left lands on (toRow, toCol), one undo step. */
  moveCells(doc: number, row: number, rows: number, col: number, cols: number, toRow: number, toCol: number, copy: boolean): Promise<DocInfo>;
  change(doc: number, change: Change): Promise<DocInfo>;
  /** null when the Save As dialog was cancelled. */
  save(doc: number, flags: SaveFlags): Promise<DocInfo | null>;
  jobProgress(doc: number): Promise<JobInfo | null>;
  cancelJob(doc: number): Promise<void>;
  sortRows(doc: number, spec: SortSpec): Promise<DocInfo>;
  setFilter(doc: number, spec: FilterSpec): Promise<DocInfo>;
  clearFilter(doc: number): Promise<DocInfo>;
  filterSpec(doc: number): Promise<FilterSpec | null>;
  filterValues(doc: number, col: number, spec: FilterSpec, search: string | null): Promise<ValueList>;
  findNext(doc: number, spec: FindSpec, from: [number, number] | null, backward: boolean): Promise<Found | null>;
  findAll(doc: number, spec: FindSpec): Promise<FindAll>;
  replacePreview(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport>;
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
  pivotOpen(doc: number, keepFirst: boolean): Promise<[string, PivotSpec] | null>;
  /** The next shown row whose cell in `col` differs (up or down). */
  findChange(doc: number, row: number, col: number, up: boolean): Promise<number | null>;
  /** The row with the column's largest (or smallest) number. */
  findExtreme(doc: number, col: number, largest: boolean, keepFirst: boolean, read: ReadOptions): Promise<number | null>;
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
  openRecent(id: string): Promise<Opened>;
  /** Per tab: the folder telling same-named files apart ("" for the rest). */
  tabFolders(ids: number[]): Promise<string[]>;
  /** A column's name: its cell in the table's first row, shown or not. */
  firstRowCell(doc: number, col: number): Promise<string>;
  /** The first row's values (first 2,000 columns, cut to 200 characters). */
  firstRowNames(doc: number): Promise<string[]>;
  /** Rename a column: set its cell in the table's first row (one undo step). */
  setFirstRowCell(doc: number, col: number, value: string): Promise<DocInfo>;
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
  chartData(doc: number, spec: ChartSpec): Promise<ChartData>;
  highlights(doc: number, row: number, rows: number, col: number, cols: number, rules: Highlight[], skipFirst: boolean, read: ReadOptions): Promise<Mark[]>;
  /** Where Ctrl+arrow lands from a cell. */
  dataEdge(doc: number, row: number, col: number, dir: Dir): Promise<[number, number]>;
  quitApp(): Promise<void>;
  onCloseRequested(handler: () => void): void;
  computePreview(doc: number, spec: ComputeSpec): Promise<ComputePreview>;
  /** Each distinct value of a column with its count (a job). */
  valueTally(doc: number, spec: TallySpec): Promise<ValueTally>;
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
  sqliteOpen(table: string): Promise<DocInfo>;
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
  recipeRun(doc: number, recipe: import("./recipe").Recipe, now: number, force: boolean): Promise<DocInfo>;
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
  checkMarks(doc: number, row: number, rows: number, col: number, cols: number, spec: CheckSpec): Promise<CheckMark[]>;
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
  importFinish(id: number, options: ImportOptions) {
    return call<DocInfo>("import_finish", { id, options });
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
  docInfo(doc: number) {
    return call<DocInfo>("doc_info", { doc });
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
  fillCells(doc: number, row: number, rows: number, col: number, cols: number, value: string) {
    return call<DocInfo>("fill_cells", { doc, row, rows, col, cols, value });
  }
  copyCells(doc: number, row: number, rows: number, col: number, cols: number) {
    return call<string>("copy_cells", { doc, row, rows, col, cols });
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
  cancelJob(doc: number) {
    return call<void>("cancel_job", { doc });
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
    return call<[string, PivotSpec] | null>("pivot_open", { doc, keepFirst });
  }
  findChange(doc: number, row: number, col: number, up: boolean) {
    return call<number | null>("find_change", { doc, row, col, up });
  }
  findExtreme(doc: number, col: number, largest: boolean, keepFirst: boolean, read: ReadOptions) {
    return call<number | null>("find_extreme", { doc, col, largest, keepFirst, read });
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
  chartData(doc: number, spec: ChartSpec) {
    return call<ChartData>("chart_data", { doc, spec });
  }
  highlights(doc: number, row: number, rows: number, col: number, cols: number, rules: Highlight[], skipFirst: boolean, read: ReadOptions) {
    return call<Mark[]>("highlights", { doc, row, rows, col, cols, rules, skipFirst, read });
  }
  dataEdge(doc: number, row: number, col: number, dir: Dir) {
    return call<[number, number]>("data_edge", { doc, row, col, dir });
  }
  quitApp() {
    return call<void>("quit_app");
  }
  onCloseRequested(handler: () => void) {
    void import("@tauri-apps/api/event").then(({ listen }) => listen("close-requested", handler));
  }
  computePreview(doc: number, spec: ComputeSpec) {
    return call<ComputePreview>("compute_preview", { doc, spec });
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
    return call<DocInfo>("sqlite_open", { table });
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
  recipeRun(doc: number, recipe: import("./recipe").Recipe, now: number, force: boolean) {
    return call<DocInfo>("recipe_run", { doc, recipe, now, force });
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
  checkMarks(doc: number, row: number, rows: number, col: number, cols: number, spec: CheckSpec) {
    return call<CheckMark[]>("check_marks", { doc, row, rows, col, cols, spec });
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
    void import("@tauri-apps/api/event").then(({ listen }) => listen("opened", handler));
  }
  onAgentGoto(handler: (g: { doc: number; row: number; col: number }) => void) {
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<{ doc: number; row: number; col: number }>("agent-goto", (e) => handler(e.payload)),
    );
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

export function backend(): Backend {
  instance ??= inTauri() ? new TauriBackend() : new MockBackend();
  return instance;
}
