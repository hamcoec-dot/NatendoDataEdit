// Browser stand-in for the Rust backend: a generated document of a million
// rows with edits, row/column changes and undo. Used by `vite dev` outside
// the desktop shell and by tests. It is not the engine; it only has to
// behave the same at the command level.

import type { GapReport, GapsSpec } from "./gaps";
import type { RelationsReport, RelationsSpec } from "./relations";
import { countsText, sliceName, type SetsRun, type SetsSpec } from "./sets";
import type { CleanMatrix, MatrixPreview, NamedCleaner } from "./matrix";
import { dateReadings } from "./dates";
import { COLUMNS_CHANGED } from "./ragged";
import { joinPreview } from "./repair";
import { mockDrill, mockItems, mockPivot, type ItemList, type PivotField, type PivotGrid, type PivotSpec } from "./pivot";
import { isIsoDate, shapeRegex } from "./lint";
import { shapeOf } from "./insights";
import type {
  JumpResult,
  RuleSuggestion,
  SidecarRules,
  AppInfo,
  Backend,
  Change,
  CmdError,
  Condition,
  DataOp,
  Dir,
  DocInfo,
  ExportOptions,
  ExportReport,
  FilterSpec,
  ImportFormat,
  ImportInfo,
  ImportOptions,
  ImportStart,
  Opened,
  FindAll,
  FindCount,
  FindRowsAction,
  BatchReplace,
  FindSpec,
  Found,
  JobInfo,
  Layout,
  LayoutChoices,
  OpReport,
  Pattern,
  ReplaceReport,
  Rule,
  SaveFlags,
  SortSpec,
  TableOp,
  ValueList,
} from "./backend";
import { encodeViewport, decodeViewport, type ViewportData } from "./binary";
import { parseTsv, toTsv } from "./tsv";
import { runCleaner, type Cleaner, type CleanPreview, type CleanSpec } from "./clean";
import type { ChartData, ChartSpec, ColType, Highlight, Listing, Mark, RecentItem, Summary, View } from "./views";
import type { Finding, PrivateKind, PrivateState, SafeCopy, SafeCopyReport, SplitReport, Written, TableReport, RaggedList, RowText, RowRepair, SameShape, DateScan, Totals } from "./backend";
import type { Autosaved, RecoveryItem, Suggestions, VersionItem } from "./m9";
import { tidyTitleList } from "./titles";
import { compareCells } from "./colcompare";
import { readNumberFor } from "./readers";
import { scanGremlins } from "./invisible";
import { disjoint, type Rect } from "./selection";
import type { CellFormula, FacetCard, Facets, FormulaReport, Insights, InsightSpec, Profile, ProfileSpec } from "./backend";
import type { CompareCols, CompareColsReport, TitleSpec, SessionItem, SessionPlace, SessionTab, SpecialCells, SpecialSpec, AgentStatus, Check, CheckMark, CheckReport, CheckSpec, ComputePreview, ComputeSpec, FileType, JoinPreview, JoinSpec, OsOpen, TallySpec, ValueTally, LookAlikeSpec, LookAlikes, HandOver, ModifiedReport } from "./backend";
import { guessStarts, LOG_PRESETS, splitFixed, toJsPattern } from "./m8";
import { mockEval } from "./mockexpr";
import { rewriteA1 } from "./cellformula";
import { stubProfile } from "./profile";
import { shapeMatches, stubInsights } from "./insights";
import { stubFacets } from "./facets";
import { previewOf, simulateJoin } from "./joins";

const FIRST = ["Ada", "Bo", "Cy", "Dee", "Eli", "Fay", "Gus", "Hal", "Ivy", "Jo"];
const LAST = ["Stone", "Rivera", "Okafor", "Lind", "Nakamura", "Quinn", "Varga", "Wells"];
const CITY = ["Dayton", "Lisbon", "Osaka", "Perth", "Quito", "Tromsø", "Kraków"];

function generated(row: number, col: number): string {
  if (row === 0) return ["id", "first_name", "last_name", "city", "amount", "joined", "notes"][col] ?? "";
  const r = row - 1;
  switch (col) {
    case 0:
      return String(r + 1);
    case 1:
      return FIRST[r % FIRST.length] ?? "";
    case 2:
      return LAST[(r * 7) % LAST.length] ?? "";
    case 3:
      return CITY[(r * 3) % CITY.length] ?? "";
    case 4:
      return ((r * 37) % 10_000 / 100).toFixed(2);
    case 5:
      return `20${String(10 + (r % 15)).padStart(2, "0")}-${String(1 + (r % 12)).padStart(2, "0")}-${String(1 + (r % 28)).padStart(2, "0")}`;
    case 6:
      return r % 11 === 0 ? "line one\nline two" : r % 5 === 0 ? "" : "ok";
    default:
      return "";
  }
}

/** Unique, rising marks for undo steps (the engine's step ids) and for checkpoints. */
let markSeq = 0;
const markOf = new WeakMap<object, number>();

interface MockDoc {
  info: DocInfo;
  /** What the data revision was worked out from (see refresh). */
  dataSig?: string;
  /** Protected columns (physical), passed over by every change. */
  protectedPhys?: Set<number>;
  /** Row order: original row numbers (>= 0) or inserted rows (< 0). */
  order: number[] | null;
  baseRows: number;
  cols: number[] | null; // column map over physical columns
  baseCols: number;
  nextCol: number;
  nextNew: number;
  edits: Map<string, string>;
  undo: { apply: () => () => void; inverse: () => void }[];
  redo: (() => () => void)[];
  /** Checkpoints: named moments, by mark (see `markOf`), with the cells as they were (the stand-in's way to compare). */
  checkpoints?: { id: number; name: string; mark: number; auto: boolean; snap?: Map<string, string>; order?: number[]; cols?: number[] }[];
  /** The mark of the state before the oldest undo step (new again after a save). */
  baseMark?: number;
  clean: number;
  /** Filter: ids of shown rows (new rows always show), and its rules. */
  shown: Set<number> | null;
  filter: FilterSpec | null;
  /** Positions of shown rows, rebuilt when `null`. */
  visible: number[] | null;
  /** Ids of rows the user hid. */
  hidden: Set<number> | null;
}

/** A JavaScript RegExp for a pattern (the browser stub only; the real app
 *  uses the engine's linear-time matcher). */
/** \t \n \r \\ \u{..} as the characters they name (Find's "Escapes"). */
export function unescapeText(t: string): string {
  return t.replace(/\\(t|n|r|\\|u\{([0-9a-fA-F]{1,6})\})/g, (m, k: string, hex?: string) => {
    if (hex) {
      const n = parseInt(hex, 16);
      return n <= 0x10ffff ? String.fromCodePoint(n) : m;
    }
    return k === "t" ? "\t" : k === "n" ? "\n" : k === "r" ? "\r" : "\\";
  });
}

let accentForms: Map<string, string> | null = null;
/** "e" → "eéèêë…": a letter's accented forms (the Latin blocks). */
function accentsOf(base: string): string {
  if (!accentForms) {
    accentForms = new Map();
    for (const [a, b] of [[0xc0, 0x24f], [0x1e00, 0x1eff]] as const)
      for (let u = a; u <= b; u++) {
        const c = String.fromCodePoint(u);
        const l = c.toLowerCase();
        const k = l.normalize("NFD")[0]!;
        if (k !== l && /[a-z]/.test(k)) accentForms.set(k, (accentForms.get(k) ?? "") + l);
      }
  }
  return accentForms.get(base) ?? "";
}
function accentClass(c: string, esc: (s: string) => string): string {
  const k = c.toLowerCase().normalize("NFD")[0]!;
  if (!/[a-z]/.test(k)) return esc(c);
  const upper = c !== c.toLowerCase();
  const forms = k + accentsOf(k);
  return `(?:[${upper ? forms.toUpperCase() : forms}]\\p{M}*)`;
}

/** Number, date and similar patterns: a test on the whole cell, shaped as a
 *  RegExp so the stub's searches and replaces work unchanged. */
class CellTest extends RegExp {
  constructor(private ok: (s: string) => boolean) {
    super("[^\\s\\S]");
  }
  override test(s: string): boolean {
    return this.ok(s);
  }
  override exec(s: string): RegExpExecArray | null {
    if (!this.ok(s)) return null;
    const m = [s] as unknown as RegExpExecArray;
    m.index = 0;
    m.input = s;
    return m;
  }
  override [Symbol.replace](s: string, rep: unknown): string {
    if (!this.ok(s)) return s;
    return typeof rep === "function" ? String((rep as (m: string, i: number, s: string) => string)(s, 0, s)) : String(rep);
  }
  override *[Symbol.matchAll](s: string): RegExpStringIterator<RegExpExecArray> {
    const m = this.exec(s);
    if (m) yield m;
  }
}

function mockNumber(s: string): number | null {
  let t = s.trim().replace(/[$€£,]/g, "");
  let neg = false;
  if (/^\(.*\)$/.test(t)) {
    neg = true;
    t = t.slice(1, -1);
  }
  if (t === "" || !/^[-+]?(\d+\.?\d*|\.\d+)$/.test(t)) return null;
  return neg ? -Number(t) : Number(t);
}
function mockDate(s: string, dayFirst = false): number | null {
  let t = s.trim();
  if (dayFirst) t = t.replace(/^(\d{1,2})\/(\d{1,2})\//, "$2/$1/");
  if (!/\d/.test(t)) return null;
  const ms = Date.parse(/^\d{4}-\d{2}-\d{2}$/.test(t) ? `${t}T00:00` : t);
  return Number.isNaN(ms) ? null : (ms - new Date(ms).getTimezoneOffset() * 60_000) / 86_400_000;
}
function similarKey(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
}
function editsWithin(a: string, b: string, max: number): boolean {
  const x = [...a];
  const y = [...b];
  if (Math.abs(x.length - y.length) > max) return false;
  let prev = Array.from({ length: y.length + 1 }, (_, j) => j);
  for (let i = 0; i < x.length; i++) {
    const cur = [i + 1];
    for (let j = 0; j < y.length; j++) cur.push(Math.min(prev[j]! + (x[i] === y[j] ? 0 : 1), prev[j + 1]! + 1, cur[j]! + 1));
    prev = cur;
  }
  return prev[y.length]! <= max;
}
function cellTestOf(p: Pattern): CellTest {
  if (p.kind === "similar") {
    const want = similarKey(p.text);
    if (!want) throw err("bad_pattern", "the pattern is not valid: enter a name or word to look for");
    const n = [...want].length;
    const max = n <= 4 ? 1 : n <= 10 ? 2 : 3;
    return new CellTest((s) => editsWithin(want, similarKey(s), max));
  }
  const date = p.kind === "date";
  const dmy = p.read?.date_order === "dmy";
  const read = date ? (s: string) => mockDate(s, dmy) : mockNumber;
  const val = (s: string): [number, boolean] => {
    const v = read(s);
    if (v === null) throw err("bad_pattern", `the pattern is not valid: "${s.trim()}" is not a ${date ? "date (try 1/5/2024 or 2024-01-05)" : "number"}`);
    return [v, date && Number.isInteger(v)];
  };
  const t = p.text.trim();
  let lo: [number, boolean] | null = null;
  let hi: [number, boolean] | null = null;
  if (t.startsWith(">=")) lo = [val(t.slice(2))[0], true];
  else if (t.startsWith("<=")) {
    const [v, day] = val(t.slice(2));
    hi = day ? [v + 1, false] : [v, true];
  } else if (t.startsWith(">")) {
    const [v, day] = val(t.slice(1));
    lo = day ? [v + 1, true] : [v, false];
  } else if (t.startsWith("<")) hi = [val(t.slice(1))[0], false];
  else if (t.includes("..")) {
    const [a, b] = t.split("..", 2) as [string, string];
    const [x] = val(a);
    const [y, day] = val(b);
    if (y < x) throw err("bad_pattern", "the pattern is not valid: the range ends before it starts");
    lo = [x, true];
    hi = day ? [y + 1, false] : [y, true];
  } else {
    const [v, day] = val(t);
    lo = [v, true];
    hi = day ? [v + 1, false] : [v, true];
  }
  return new CellTest((s) => {
    const v = read(s);
    if (v === null) return false;
    return (!lo || (lo[1] ? v >= lo[0] : v > lo[0])) && (!hi || (hi[1] ? v <= hi[0] : v < hi[0]));
  });
}

function keepCase(found: string, rep: string): string {
  const letters = [...found].filter((c) => /\p{L}/u.test(c));
  if (!letters.length || !rep) return rep;
  const up = (c: string) => c === c.toUpperCase() && c !== c.toLowerCase();
  const low = (c: string) => c === c.toLowerCase() && c !== c.toUpperCase();
  if (letters.length > 1 && letters.every(up)) return rep.toUpperCase();
  if (letters.every(low)) return rep.toLowerCase();
  if (up(letters[0]!) && letters.slice(1).every(low)) return rep.charAt(0).toUpperCase() + rep.slice(1);
  return rep;
}

function regexOf(p: Pattern, global = false): RegExp {
  if (p.kind === "number" || p.kind === "date" || p.kind === "similar") return cellTestOf(p);
  let body: string;
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const lit = (s: string) => (p.ignore_accents ? [...s].map((c) => accentClass(c, esc)).join("") : esc(s));
  const text = p.escapes && p.kind !== "regex" ? unescapeText(p.text) : p.text;
  if (p.kind === "regex") body = p.ignore_accents ? text.replace(/(?<!\\)[A-Za-z\u00c0-\u024f]/g, (c) => accentClass(c, esc)) : text;
  else {
    if (p.kind === "plain") body = lit(text);
    else
      body = text
        .split(/(~[*?~]|\*|\?)/)
        .map((t) => (t === "*" ? "[\\s\\S]*" : t === "?" ? "[\\s\\S]" : t.startsWith("~") && t.length === 2 ? lit(t[1]!) : lit(t)))
        .join("");
  }
  if (p.whole_cell) body = `^(?:${body})$`;
  else if (p.whole_word) body = `(?<![\\p{L}\\p{N}_])(?:${body})(?![\\p{L}\\p{N}_])`;
  return new RegExp(body, (p.match_case ? "" : "i") + (global ? "g" : "") + "u");
}

function numberOf(s: string): number | null {
  const t = s.trim().replace(/[$€£,]/g, "");
  if (t === "" || !/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(t)) return null;
  return Number(t);
}

function dayOf(s: string): number | null {
  const t = Date.parse(s.trim());
  return Number.isNaN(t) ? null : Math.floor(t / 86_400_000);
}

function testCond(c: Condition, v: string): boolean {
  const cmp = (op: string, a: number, b: number) =>
    ({ eq: a === b, ne: a !== b, gt: a > b, ge: a >= b, lt: a < b, le: a <= b })[op] ?? false;
  switch (c.type) {
    case "text": {
      const a = c.match_case ? v : v.toLowerCase();
      const b = c.match_case ? c.text : c.text.toLowerCase();
      return {
        equals: a === b,
        not_equals: a !== b,
        contains: a.includes(b),
        not_contains: !a.includes(b),
        begins_with: a.startsWith(b),
        ends_with: a.endsWith(b),
      }[c.op];
    }
    case "number": {
      const n = numberOf(v);
      return n !== null && cmp(c.op, n, c.value);
    }
    case "number_between": {
      const n = numberOf(v);
      return n !== null && n >= Math.min(c.low, c.high) && n <= Math.max(c.low, c.high);
    }
    case "date": {
      const d = dayOf(v);
      const w = dayOf(c.value);
      return d !== null && w !== null && cmp(c.op, d, w);
    }
    case "date_between": {
      const d = dayOf(v);
      const a = dayOf(c.low);
      const b = dayOf(c.high);
      return d !== null && a !== null && b !== null && d >= Math.min(a, b) && d <= Math.max(a, b);
    }
    case "top":
      // The browser stand-in keeps every number (the engine finds the cut-off).
      return numberOf(v) !== null;
    case "blank":
      return v === "";
    case "not_blank":
      return v !== "";
    case "pattern":
      return regexOf(c.pattern).test(v) !== !!c.negate;
  }
}

function testRule(r: Rule, v: string): boolean {
  if (r.type === "values") return v === "" ? r.blanks : r.values.includes(v);
  const a = testCond(r.first, v);
  if (!r.second) return a;
  return r.or ? a || testCond(r.second, v) : a && testCond(r.second, v);
}

function naturalCompare(a: string, b: string): number {
  const x = numberOf(a);
  const y = numberOf(b);
  if (x !== null && y !== null) return x - y;
  if (x !== null) return -1;
  if (y !== null) return 1;
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });
}

const err = (kind: string, message: string): CmdError => ({ kind, message, row: null });

/** A tiny stand-in for the engine's importers (browser preview only). */
function mockImport(text: string, format: ImportFormat, part: number): string[][] {
  if (format === "json") {
    const v: unknown = JSON.parse(text);
    const rows = Array.isArray(v) ? v : [v];
    if (rows.some((r) => r && typeof r === "object" && !Array.isArray(r))) {
      const names: string[] = [];
      for (const r of rows) if (r && typeof r === "object") for (const k of Object.keys(r)) if (!names.includes(k)) names.push(k);
      const cell = (x: unknown) => (x === null || x === undefined ? "" : typeof x === "object" ? JSON.stringify(x) : String(x));
      return [names, ...rows.map((r) => names.map((n) => cell((r as Record<string, unknown>)[n])))];
    }
    return rows.map((r) => (Array.isArray(r) ? r.map(String) : [String(r)]));
  }
  if (format === "markdown") {
    const lines = text.split(/\r?\n/).filter((l) => l.includes("|") && !/^\s*\|?\s*:?-+/.test(l));
    return lines.map((l) => l.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim()));
  }
  if (format === "fixed_width") {
    const lines = text.split(/\r?\n/);
    const starts = guessStarts(lines);
    return lines.filter((l, i, a) => l !== "" || i < a.length - 1).map((l) => splitFixed(l, starts));
  }
  if (format === "pattern") {
    const re = new RegExp(toJsPattern(LOG_PRESETS.at(-1)!.pattern));
    return [["line"], ...text.split(/\r?\n/).filter((l) => re.test(l) && l !== "").map((l) => [l])];
  }
  if (format === "spreadsheet") {
    return part === 0
      ? [["name", "joined", "score"], ["Ada", "2024-01-15", "9.5"], ["Bo", "2023-11-02", "7"], ["Cy", "2022-06-30", "8.25"]]
      : [["secret"], ["hidden sheet"]];
  }
  const sep = [",", "\t", ";", "|"].map((d) => [d, text.split("\n")[0]?.split(d).length ?? 0] as const).sort((a, b) => b[1] - a[1])[0]?.[0] ?? ",";
  return text
    .split(/\r?\n/)
    .filter((l, i, a) => l !== "" || i < a.length - 1)
    .map((l) => l.split(sep));
}

/** Undo steps that only change which rows show. */
const VIEW_STEPS = new WeakSet<() => () => void>();

export class MockBackend implements Backend {
  private docs = new Map<number, MockDoc>();
  private next = 1;
  private imports = new Map<number, { name: string; text: string; format: ImportFormat }>();
  private nextImport = 1;

  async appInfo(): Promise<AppInfo> {
    return { name: "Natendo Data Edit", version: "dev", engine_version: "browser stub" };
  }

  private make(name: string, rows: number, cols: number): DocInfo {
    const id = this.next++;
    const info: DocInfo = {
      id,
      name,
      path: null,
      rows,
      total_rows: rows,
      filtered: false,
      filter_cols: [],
      filter_on_deleted: false,
      cols,
      indexed: true,
      progress: 1,
      modified: false,
      can_undo: false,
      can_redo: false,
      history_pos: 0,
      column_types: { types: [] },
      typed: [],
      encoding: "UTF-8",
      bom: false,
      delimiter: ",",
      quote: '"',
      escape: null,
      merge: false,
      preamble_lines: 0,
      layout_confidence: 0.97,
      read_only: false,
      hidden_rows: 0,
      has_filter: false,
      line_break: "CRLF",
      mixed_line_breaks: false,
      lossy: false,
      invalid_bytes: false,
      line_by_line: false,
      column_changes: false,
      repair_count: 0,
      unclosed_quotes: false,
      long_quoted_cells: false,
      line_bound_from: null,
      error: null,
    };
    this.docs.set(id, {
      info,
      order: null,
      baseRows: rows,
      cols: null,
      baseCols: cols,
      nextCol: cols,
      nextNew: -1,
      edits: new Map(),
      undo: [],
      redo: [],
      clean: 0,
      shown: null,
      filter: null,
      visible: null,
      hidden: null,
    });
    return { ...info };
  }

  async openFile(): Promise<Opened | null> {
    // A made-up path (shown only) and a hesitant layout guess, so the
    // layout notice and dialog can be tried in the browser.
    const info = this.make("sample-1M.csv", 1_000_001, 7);
    const d = this.get(info.id);
    d.info.path = "C:/samples/sample-1M.csv";
    d.info.layout_confidence = 0.45;
    return { doc: { ...d.info }, import: null };
  }

  private startImport(name: string, text: string, format: ImportFormat): ImportStart {
    if (this.imports.size >= 4) throw err("too_many_imports", "close another import screen first");
    const id = this.nextImport++;
    this.imports.set(id, { name, text, format });
    return { id, name, info: this.inspect(text, format) };
  }
  private inspect(text: string, format: ImportFormat): ImportInfo {
    return {
      format,
      parts: format === "spreadsheet" ? ["People", "Notes"] : [],
      hidden: format === "spreadsheet" ? [false, true] : [],
      records: format === "xml" ? ["rows/row"] : [],
      starts: format === "fixed_width" ? guessStarts(text.split(/\r?\n/).slice(0, 2000)) : [],
      lines: format === "fixed_width" || format === "pattern" ? text.split(/\r?\n/).slice(0, 40) : [],
      pattern: format === "pattern" ? LOG_PRESETS.at(-1)!.pattern : "",
    };
  }
  private session(id: number) {
    const s = this.imports.get(id);
    if (!s) throw err("no_import", "that import was closed");
    return s;
  }
  async importFile(): Promise<ImportStart | null> {
    return this.startImport("people.xlsx", "", "spreadsheet");
  }
  async importText(text: string): Promise<ImportStart> {
    const t = text.trimStart();
    const format: ImportFormat = t.startsWith("[") || t.startsWith("{") ? "json" : /\n\s*\|?\s*:?-{3}/.test(text) ? "markdown" : "delimited";
    return this.startImport("Clipboard", text, format);
  }
  async importInspect(id: number, format: ImportFormat | null): Promise<ImportInfo> {
    const s = this.session(id);
    return this.inspect(s.text, format ?? s.format);
  }
  async importPreview(id: number, o: ImportOptions): Promise<string[][]> {
    const s = this.session(id);
    const f = o.format ?? s.format;
    if (f === "fixed_width" && o.starts?.length) {
      return s.text.split(/\r?\n/).slice(0, 100).map((l) => splitFixed(l, o.starts!, o.keep_spaces));
    }
    if (f === "pattern" && o.pattern) {
      let re: RegExp;
      try {
        re = new RegExp(toJsPattern(o.pattern));
      } catch (e) {
        throw err("bad_pattern", String(e));
      }
      const names = Array.from({ length: (new RegExp(`${re.source}|`).exec("")?.length ?? 1) - 1 }, (_, k) => `Column ${k + 1}`);
      const rows: string[][] = [names];
      for (const l of s.text.split(/\r?\n/).slice(0, 200)) {
        const m = re.exec(l);
        if (m) rows.push(m.slice(1).map((x) => x ?? ""));
        else if (o.unmatched === "row" && l) rows.push([l]);
        else if (o.unmatched !== "skip" && rows.length > 1 && l) rows[rows.length - 1]![rows[rows.length - 1]!.length - 1] += `\n${l}`;
      }
      return rows;
    }
    try {
      return mockImport(s.text, o.format ?? s.format, o.part).slice(0, 100);
    } catch (e) {
      throw err("invalid", `the text is not valid ${o.format ?? s.format}: ${String(e)}`);
    }
  }
  async importNotes(id: number, o: ImportOptions): Promise<import("./backend").ImportNotes> {
    const s = this.session(id);
    const none = { kept_big: 0, lost_digits: 0, padded: [], padded_applied: 0, padded_left: 0, scanned_rows: 0, all_scanned: true, remember_path: null, summary: "" };
    if ((o.format ?? s.format) !== "spreadsheet" || o.part !== 0) return none;
    const applied = o.pad_zeros ? 3 : 0;
    return {
      kept_big: 1,
      lost_digits: 0,
      padded: [{ name: "“zip”", count: 3 }],
      padded_applied: applied,
      padded_left: 0,
      scanned_rows: 4,
      all_scanned: true,
      remember_path: "C:\\Data\\people.xlsx",
      summary: `1 number of more than 15 digits kept exactly as the file stores them (“score”).${applied ? " 3 numbers written with leading zeros, as the spreadsheet shows them." : ""}`,
    };
  }
  async importFinish(id: number, o: ImportOptions): Promise<DocInfo> {
    const s = this.session(id);
    const rows = mockImport(s.text, o.format ?? s.format, o.part);
    return this.fromRows(s.name.replace(/\.[^.]*$/, ""), rows);
  }
  async importProgress(): Promise<number | null> {
    return null;
  }
  async importCancel(): Promise<void> {}
  async importClose(id: number): Promise<void> {
    this.imports.delete(id);
  }

  /** A new, unsaved document holding `rows`. */
  private fromRows(name: string, out: string[][]): DocInfo {
    const width = Math.max(1, ...out.map((r) => r.length));
    const info = this.make(name, Math.max(1, out.length), width);
    const nd = this.get(info.id);
    out.forEach((line, r) => {
      for (let c = 0; c < width; c++) nd.edits.set(`${r}:${c}`, line[c] ?? "");
    });
    nd.clean = -1;
    return this.refresh(nd);
  }

  private exportRows(doc: number, o: ExportOptions, limit: number): { names: string[]; rows: string[][] } {
    const d = this.get(doc);
    const cols = this.colCount(d);
    const first = o.header ? 1 : 0;
    const names = Array.from({ length: cols }, (_, c) => (o.header ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
    const rows: string[][] = [];
    for (let r = first; r < Math.min(this.rowCount(d), first + limit); r++) rows.push(names.map((_, c) => this.value(d, r, c)));
    return { names, rows };
  }
  private exportString(doc: number, o: ExportOptions, limit: number): string {
    const { names, rows } = this.exportRows(doc, o, limit);
    const esc = (v: string) => v.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    switch (o.format) {
      case "json": {
        const v = o.arrays ? rows : rows.map((r) => Object.fromEntries(names.map((n, i) => [n, r[i] ?? ""])));
        return JSON.stringify(v, null, o.pretty ? 2 : undefined) + "\n";
      }
      case "json_lines": {
        const v = o.arrays ? rows : rows.map((r) => Object.fromEntries(names.map((n, i) => [n, r[i] ?? ""])));
        return v.map((x) => JSON.stringify(x) + "\n").join("");
      }
      case "xml":
        return `<?xml version="1.0" encoding="UTF-8"?>\n<${o.root}>${rows.map((r) => `<${o.record}>${names.map((n, i) => `<${n}>${esc(r[i] ?? "")}</${n}>`).join("")}</${o.record}>`).join("")}</${o.root}>\n`;
      case "html":
        return `<table>\n  <thead>\n    <tr>${names.map((n) => `<th>${esc(n)}</th>`).join("")}</tr>\n  </thead>\n  <tbody>\n${rows.map((r) => `    <tr>${r.map((v) => `<td>${esc(v).replace(/\n/g, "<br>")}</td>`).join("")}</tr>\n`).join("")}  </tbody>\n</table>\n`;
      case "markdown": {
        const line = (r: string[]) => `| ${r.map((v) => v.replace(/\|/g, "\\|").replace(/\n/g, "<br>")).join(" | ")} |\n`;
        return line(names) + `|${names.map(() => " --- |").join("")}\n` + rows.map(line).join("");
      }
      case "template": {
        const fill = (t: string, r: string[]) =>
          t.replace(/\{\{|\}\}|\{([^}]*)\}/g, (m, name: string | undefined) => {
            if (m === "{{") return "{";
            if (m === "}}") return "}";
            const n = name ?? "";
            const i = n.startsWith("#") ? Number(n.slice(1)) - 1 : names.indexOf(n);
            if (i < 0 || i >= names.length || Number.isNaN(i)) throw err("invalid", `the template names a column that is not there: {${n}}`);
            return r[i] ?? "";
          });
        return fill(o.before, []) + rows.map((r) => fill(o.row, r)).join("") + fill(o.after, []);
      }
      case "fixed_width": {
        const all = [names, ...rows];
        const w = names.map((_, k) => Math.max(...all.map((r) => Array.from(r[k] ?? "").length)));
        return all.map((r) => r.map((v, k) => v.padEnd(w[k]!)).join(" ").trimEnd() + "\r\n").join("");
      }
      case "sql": {
        const q = (v: string) => (v === "" ? "NULL" : `'${v.replace(/'/g, "''")}'`);
        return rows.map((r) => `INSERT INTO "${o.table ?? "data"}" VALUES (${r.map(q).join(", ")});
`).join("");
      }
      case "xlsx":
      case "pdf":
        return "";
    }
  }
  async exportPreview(doc: number, o: ExportOptions): Promise<string> {
    return this.exportString(doc, o, 20);
  }
  async exportFile(doc: number, o: ExportOptions): Promise<ExportReport | null> {
    const d = this.get(doc);
    const rows = Math.max(0, this.rowCount(d) - (o.header ? 1 : 0));
    if (o.format !== "xlsx") this.exportString(doc, o, 1); // template errors show
    const per = o.part_rows || 1_048_575;
    const parts = Math.max(1, Math.ceil(rows / per));
    const stem = d.info.name.replace(/\.[^.]*$/, "");
    if (o.format === "xlsx" && o.split_files && parts > 1) return { rows, sheets: parts, bytes: rows * 40, files: Array.from({ length: parts }, (_, i) => `${stem} (${i + 1} of ${parts}).xlsx`) };
    return { rows, sheets: o.format === "xlsx" ? parts : 0, bytes: rows * 40 };
  }
  async exportBook(docs: number[], o: ExportOptions, _widths: number[][] = []): Promise<ExportReport | null> {
    let rows = 0;
    for (const id of docs) rows += Math.max(0, this.rowCount(this.get(id)) - (o.header ? 1 : 0));
    return { rows, sheets: docs.length, bytes: rows * 40 };
  }
  async exportText(doc: number, o: ExportOptions): Promise<string> {
    if (o.format === "xlsx") throw err("invalid", "Excel files cannot go on the clipboard");
    return this.exportString(doc, o, 10_000);
  }
  async reopen(doc: number, lineByLine: boolean, layout: Layout | null = null): Promise<DocInfo> {
    const d = this.get(doc);
    const info = this.make(d.info.name, d.baseRows, d.baseCols);
    const nd = this.get(info.id);
    nd.info.line_by_line = lineByLine;
    nd.info.path = d.info.path;
    if (layout) {
      nd.info.delimiter = layout.delimiter;
      nd.info.quote = layout.quote;
      nd.info.escape = layout.escape;
      nd.info.merge = layout.merge;
      nd.info.preamble_lines = layout.skip_lines;
      nd.info.layout_confidence = null;
    }
    return { ...nd.info };
  }
  async layoutChoices(doc: number): Promise<LayoutChoices> {
    const d = this.get(doc);
    const comma: Layout = { delimiter: ",", quote: '"', escape: null, merge: false, skip_lines: 0 };
    const current: Layout = {
      delimiter: d.info.delimiter,
      quote: d.info.quote,
      escape: d.info.escape,
      merge: d.info.merge,
      skip_lines: d.info.preamble_lines,
    };
    const one: Layout = { ...comma, delimiter: ";" };
    return {
      confidence: d.info.layout_confidence ?? 0.97,
      sep_line: false,
      current,
      choices: [
        { layout: comma, columns: d.baseCols, preview: await this.previewLayout(doc, comma) },
        { layout: one, columns: 1, preview: await this.previewLayout(doc, one) },
      ],
      preamble: "",
    };
  }
  async previewLayout(doc: number, layout: Layout): Promise<string[][]> {
    const d = this.get(doc);
    if (!layout.delimiter || layout.delimiter.length > 4) throw err("bad_layout", "separator must be 1 to 4 characters");
    const rows: string[][] = [];
    for (let r = layout.skip_lines; r < Math.min(d.baseRows, layout.skip_lines + 8); r++) {
      const cells = Array.from({ length: d.baseCols }, (_, c) => generated(r, c).replace(/\n/g, " "));
      rows.push(layout.delimiter === "," ? cells.slice(0, 12) : [cells.join(",").slice(0, 40)]);
    }
    return rows;
  }
  /** Test hook: which rows of a document count as short and long. */
  fakeRagged = new Map<number, { short: number[]; long: number[] }>();
  markRagged(doc: number, short: number[], long: number[], usual: number): DocInfo {
    this.fakeRagged.set(doc, { short, long });
    const d = this.get(doc);
    Object.assign(d.info, { usual_fields: usual, ragged_rows: short.length + long.length });
    return this.refresh(d);
  }

  /** The stand-in's date radar: only slash dates, both parts 12 or under and different. */
  private twoWays(d: MockDoc, row: number, col: number): boolean {
    const m = /^(\d{1,2})\/(\d{1,2})\/\d{4}$/.exec(this.value(d, this.rowId(d, row), col).trim());
    return !!m && Number(m[1]) <= 12 && Number(m[2]) <= 12 && m[1] !== m[2];
  }
  async dateScan(doc: number, col: number, keepFirst: boolean, sample: number | null): Promise<DateScan> {
    const d = this.get(doc);
    const rows = Math.max(0, this.rowCount(d) - (keepFirst ? 1 : 0));
    const looked = Math.min(rows, sample ?? rows, 5_000);
    let ambiguous = 0;
    for (let r = keepFirst ? 1 : 0, k = 0; k < looked; r++, k++) if (this.twoWays(d, r, col)) ambiguous++;
    this.calls.push(`dateScan:${col}:${sample}`);
    return { looked: sample === null ? rows : looked, rows, blank: 0, iso: looked - ambiguous, written: 0, month_first_only: 0, day_first_only: 0, ambiguous, same_either: 0, compact: 0, compact_numbers: 0, not_date: 0, two_digit_year: 0, set_order: null, other_order: 0 };
  }
  async dateMarks(doc: number, col: number, keepFirst: boolean, row: number, rows: number): Promise<number[]> {
    const d = this.get(doc);
    const out: number[] = [];
    for (let r = Math.max(row, keepFirst ? 1 : 0); r < Math.min(row + rows, this.rowCount(d)); r++) if (this.twoWays(d, r, col)) out.push(r);
    return out;
  }
  async dateList(doc: number, col: number, keepFirst: boolean): Promise<[number, number, string][]> {
    const d = this.get(doc);
    const out: [number, number, string][] = [];
    for (let r = keepFirst ? 1 : 0; r < Math.min(this.rowCount(d), 5_000) && out.length < 500; r++) {
      if (!this.twoWays(d, r, col)) continue;
      out.push([r, r, this.value(d, this.rowId(d, r), col)]);
    }
    return out;
  }
  async dateFind(doc: number, col: number, keepFirst: boolean, from: number, forward: boolean): Promise<number | null> {
    const d = this.get(doc);
    const first = keepFirst ? 1 : 0;
    const total = this.rowCount(d) - first;
    for (let step = 1; step <= Math.min(total, 5_000); step++) {
      const r = first + ((((from - first + (forward ? step : -step)) % total) + total) % total);
      if (this.twoWays(d, r, col)) return r;
    }
    return null;
  }

  async raggedList(doc: number): Promise<RaggedList> {
    const d = this.get(doc);
    if (d.info.column_changes) throw err("invalid", COLUMNS_CHANGED);
    const m = this.fakeRagged.get(doc) ?? { short: [], long: [] };
    const ids = [...m.short, ...m.long].sort((a, b) => a - b);
    return { ids, total: ids.length };
  }
  /** The stand-in's repair bar: a marked row has one field fewer or more than the table. */
  private fields(d: MockDoc, doc: number, id: number): string[] {
    const m = this.fakeRagged.get(doc) ?? { short: [], long: [] };
    const row = Array.from({ length: d.info.cols }, (_, c) => this.value(d, id, c));
    if (m.short.includes(id)) return row.slice(0, -1);
    return m.long.includes(id) ? [...row, "extra"] : row;
  }
  async rowText(doc: number, row: number | null, rowId: number | null, keepFirst = true): Promise<RowText> {
    const d = this.get(doc);
    if (d.info.column_changes) throw err("invalid", COLUMNS_CHANGED);
    const id = rowId ?? this.rowId(d, row ?? 0);
    const fields = this.fields(d, doc, id);
    const shown = Array.from({ length: this.rowCount(d) }, (_, r) => r).find((r) => this.rowId(d, r) === id) ?? null;
    const near = (n: number) => ({ id: n, file_row: n + 1, edited: false, hidden: false, noted: false, note_keys: [], fields: this.fields(d, doc, n) });
    const next = near(id + 1);
    const first = keepFirst ? 1 : 0;
    return {
      id,
      row: shown,
      file_row: id + 1,
      revision: d.undo.length,
      edited: false,
      raw: fields.join(d.info.delimiter ?? ","),
      cut: false,
      fields,
      width: fields.length,
      usual: d.info.usual_fields ?? d.info.cols,
      line_by_line: false,
      no_join: "",
      quoted_as_text: [],
      next,
      no_next: "",
      previous: id - 1 >= first ? near(id - 1) : null,
      no_previous: id - 1 >= first ? "" : "the row before this one holds the column names",
      note_keys: [],
    };
  }
  async rowWidth(doc: number, row: number): Promise<[number, number]> {
    const d = this.get(doc);
    const usual = d.info.usual_fields || d.info.cols;
    return [this.fields(d, doc, this.rowId(d, row)).length - d.info.cols + usual, usual];
  }
  async repairRow(doc: number, rowId: number, revision: number, repair: RowRepair): Promise<DocInfo> {
    const d = this.get(doc);
    if (revision !== d.undo.length) throw err("invalid", "the table changed since the row was read; look again");
    this.calls.push(`repairRow:${rowId}:${JSON.stringify(repair)}`);
    const m = this.fakeRagged.get(doc) ?? { short: [], long: [] };
    const before = m;
    const after = { short: m.short.filter((r) => r !== rowId), long: m.long.filter((r) => r !== rowId) };
    return this.push(d, () => {
      this.fakeRagged.set(doc, after);
      return () => void this.fakeRagged.set(doc, before);
    });
  }

  /** The stand-in's "Fix all like this": the other long rows share a long row's shape. */
  async sameShape(doc: number, _keepFirst: boolean, fields: number, groups: number[]): Promise<SameShape> {
    const d = this.get(doc);
    if (fields === (d.info.usual_fields || d.info.cols)) throw err("invalid", "rows with the usual number of fields are never changed together");
    const m = this.fakeRagged.get(doc) ?? { short: [], long: [] };
    const ids = m.long.filter((id) => this.fields(d, doc, id).length === fields);
    return { revision: d.undo.length, total: ids.length, hidden: 0, first: ids.slice(0, 20).map((id) => ({ id, file_row: id + 1, cells: joinPreview(this.fields(d, doc, id), new Set(groups.flatMap((g, i) => (g > 1 ? [groups.slice(0, i).reduce((a, b) => a + b, 0)] : []))), ",") })) };
  }
  async fixAll(doc: number, keepFirst: boolean, revision: number, fields: number, groups: number[], skip: number[]): Promise<[number, DocInfo]> {
    const d = this.get(doc);
    if (revision !== d.undo.length) throw err("invalid", "the table changed since the row was read; look again");
    this.calls.push(`fixAll:${fields}:${JSON.stringify(groups)}:skip=${JSON.stringify(skip)}:${keepFirst}`);
    const before = this.fakeRagged.get(doc) ?? { short: [], long: [] };
    const fixed = new Set(before.long.filter((id) => !skip.includes(id) && this.fields(d, doc, id).length === fields));
    const after = { short: before.short, long: before.long.filter((id) => !fixed.has(id)) };
    const info = this.push(d, () => {
      this.fakeRagged.set(doc, after);
      return () => void this.fakeRagged.set(doc, before);
    });
    const left = skip.length ? [`${skip.length} ${skip.length === 1 ? "row you unticked is" : "rows you unticked are"} not in the recipe; running it fixes every row of ${fields} fields`] : [];
    return [fixed.size, { ...info, run_notes: left }];
  }
  async emptyExtraColumns(doc: number): Promise<[number, number]> {
    const m = this.fakeRagged.get(doc);
    return [m && !m.long.length ? 1 : 0, this.get(doc).undo.length];
  }
  async removeEmptyExtraColumns(doc: number): Promise<DocInfo> {
    this.calls.push("removeEmptyExtraColumns");
    return this.refresh(this.get(doc));
  }

  async newDocument(): Promise<DocInfo> {
    // As the backend's: one blank cell.
    const info = this.make("Untitled", 1, 1);
    const d = this.get(info.id);
    d.edits.set(`${this.rowId(d, 0)}:${this.physCol(d, 0)}`, "");
    return info;
  }
  async closeDocument(doc: number): Promise<void> {
    this.docs.delete(doc);
  }
  onFreshInfo(): void {}
  async docInfo(doc: number): Promise<DocInfo> {
    return this.refresh(this.get(doc));
  }

  private get(doc: number): MockDoc {
    const d = this.docs.get(doc);
    if (!d) throw err("no_document", "that document is not open");
    return d;
  }

  private totalRows(d: MockDoc): number {
    return d.order ? d.order.length : d.baseRows;
  }
  private visible(d: MockDoc): number[] | null {
    if (!d.shown && !d.hidden) return null;
    if (!d.visible) {
      const v: number[] = [];
      const n = this.totalRows(d);
      for (let p = 0; p < n; p++) {
        const id = d.order ? d.order[p]! : p;
        if (d.hidden?.has(id)) continue;
        if (!d.shown || id < 0 || d.shown.has(id)) v.push(p);
      }
      d.visible = v;
    }
    return d.visible;
  }
  private rowCount(d: MockDoc): number {
    return this.visible(d)?.length ?? this.totalRows(d);
  }
  /** Position in the whole document of shown row `row`. */
  /** Hidden rows between shown rows r0..r1 (exclusive). */
  private hiddenBetween_(d: MockDoc, r0: number, r1: number): number {
    if (r1 - r0 < 2 || (d.shown === null && d.hidden === null)) return 0;
    return Math.max(0, this.pos(d, r1 - 1) - this.pos(d, r0) + 1 - (r1 - r0));
  }

  private pos(d: MockDoc, row: number): number {
    return this.visible(d)?.[row] ?? row;
  }
  private valueAt(d: MockDoc, pos: number, col: number): string {
    const id = d.order ? (d.order[pos] ?? 0) : pos;
    const p = this.physCol(d, col);
    if (p < 0) return "";
    const e = d.edits.get(`${id}:${p}`);
    if (e !== undefined) return e;
    return id >= 0 && p < d.baseCols ? generated(id, p) : "";
  }
  private colCount(d: MockDoc): number {
    return d.cols ? d.cols.length : d.baseCols;
  }
  private rowId(d: MockDoc, row: number): number {
    const p = this.pos(d, row);
    return d.order ? (d.order[p] ?? 0) : p;
  }
  private physCol(d: MockDoc, col: number): number {
    return d.cols ? (d.cols[col] ?? -1) : col;
  }
  private value(d: MockDoc, row: number, col: number): string {
    const id = this.rowId(d, row);
    const p = this.physCol(d, col);
    if (p < 0) return "";
    const e = d.edits.get(`${id}:${p}`);
    if (e !== undefined) return e;
    return id >= 0 && p < d.baseCols ? generated(id, p) : "";
  }

  private refresh(d: MockDoc): DocInfo {
    d.visible = null;
    d.info.rows = this.rowCount(d);
    d.info.total_rows = this.totalRows(d);
    d.info.filtered = d.shown !== null || d.hidden !== null;
    d.info.hidden_rows = d.hidden?.size ?? 0;
    d.info.has_filter = d.shown !== null;
    d.info.filter_cols = d.filter?.columns.map((c) => c.col) ?? [];
    d.info.cols = this.colCount(d);
    d.info.can_undo = d.undo.length > 0;
    d.info.can_redo = d.redo.length > 0;
    d.info.history_pos = d.undo.length;
    d.info.step_mark = this.markNow(d);
    // Like the engine: steps that only change which rows show (a filter)
    // leave the document saved.
    const n = d.undo.length;
    d.info.modified =
      d.clean < 0 ||
      (d.clean <= n
        ? d.undo.slice(d.clean).some((s) => !VIEW_STEPS.has(s.apply))
        : d.clean - n > d.redo.length || d.redo.slice(d.redo.length - (d.clean - n)).some((a) => !VIEW_STEPS.has(a)));
    d.info.column_changes = d.cols !== null;
    // The data revision moves when a step that changes data is done or undone.
    const sig = `${d.undo.filter((s) => !VIEW_STEPS.has(s.apply)).length}/${d.redo.filter((a) => !VIEW_STEPS.has(a)).length}`;
    if (sig !== d.dataSig) {
      d.dataSig = sig;
      d.info.data_revision = (d.info.data_revision ?? 0) + 1;
    }
    return { ...d.info };
  }

  /** Turn a filter on, change it or clear it: one undo step (as the engine). */
  private setView(d: MockDoc, shown: Set<number> | null, filter: FilterSpec | null): DocInfo {
    if (shown === null && d.shown === null) return this.refresh(d);
    const apply = (): (() => void) => {
      const was = d.shown;
      const wasFilter = d.filter;
      d.shown = shown;
      d.filter = filter;
      return () => {
        d.shown = was;
        d.filter = wasFilter;
      };
    };
    VIEW_STEPS.add(apply);
    return this.push(d, apply);
  }

  private push(d: MockDoc, apply: () => () => void): DocInfo {
    // Like the app: no change while looking at an earlier recipe step.
    if (d.info.peeking) throw err("peeking", "looking at an earlier recipe step: go back to the latest step to change or save");
    const inverse = apply();
    d.undo.push({ apply, inverse });
    markOf.set(apply, ++markSeq);
    if (d.clean > d.undo.length - 1) d.clean = -1;
    d.redo = [];
    return this.refresh(d);
  }

  async viewport(doc: number, row: number, rows: number, col: number, cols: number, maxChars: number): Promise<ViewportData> {
    const d = this.get(doc);
    if (rows * cols > 20_000) throw err("too_large", "too many cells requested at once");
    const r1 = Math.min(row + rows, this.rowCount(d));
    const nr = Math.max(0, r1 - row);
    const cells: string[] = [];
    const truncated = new Uint8Array(nr * cols);
    for (let r = row; r < r1; r++)
      for (let c = col; c < col + cols; c++) {
        const v = this.value(d, r, c);
        if (v.length > maxChars) truncated[cells.length] = 1;
        cells.push(v.slice(0, maxChars));
      }
    const rowNumbers = d.shown || d.hidden ? Array.from({ length: nr }, (_, i) => this.pos(d, row + i)) : undefined;
    // Round-trip through the binary form so the decoder is exercised too.
    return decodeViewport(encodeViewport({ firstRow: row, firstCol: col, rows: nr, cols, cells, truncated, rowNumbers }));
  }

  async cellText(doc: number, row: number, col: number) {
    const d = this.get(doc);
    if (row >= this.rowCount(d)) throw err("out_of_range", "row or column out of range");
    return { text: this.value(d, row, col), truncated: false };
  }

  /** The column types, by physical column; `typed` is the same by shown column. */
  private applyTypes(d: MockDoc, types: import("./readers").ColumnTypes): DocInfo {
    const list = types.types.slice();
    while (list.length && list[list.length - 1] === "auto") list.pop();
    d.info.column_types = { types: list };
    d.info.typed = [];
    for (let c = 0; c < d.info.cols; c++) {
      const t = list[this.physCol(d, c)];
      if (t !== undefined && t !== "auto") d.info.typed.push([c, t]);
    }
    d.info.data_revision = (d.info.data_revision ?? 0) + 1;
    return { ...d.info };
  }
  async setColumnType(doc: number, col: number, kind: import("./readers").ColumnType): Promise<DocInfo> {
    const d = this.get(doc);
    if (col >= d.info.cols) throw err("out_of_range", "no such column");
    const list = d.info.column_types.types.slice();
    const p = this.physCol(d, col);
    while (list.length <= p) list.push("auto");
    list[p] = kind;
    return this.applyTypes(d, { types: list });
  }
  async setColumnTypes(doc: number, types: import("./readers").ColumnTypes): Promise<DocInfo> {
    return this.applyTypes(this.get(doc), types);
  }

  async setCells(doc: number, cells: [number, number, string][]): Promise<DocInfo> {
    if (cells.length > 100_000) throw err("too_large", "one change is limited to 100,000 cells");
    return this.writeCells(doc, cells);
  }

  /** Set cells as one undo step, any number (the engine's jobs).
   *  `guard`: pass over protected columns (a rename does not). */
  private writeCells(doc: number, cells: [number, number, string][], guard = true): DocInfo {
    const d = this.get(doc);
    d.info.protected_skipped = 0;
    if (guard && d.protectedPhys?.size) {
      const before = cells.length;
      cells = cells.filter(([, c]) => !d.protectedPhys!.has(this.physCol(d, c)));
      d.info.protected_skipped = before - cells.length;
      if (before && !cells.length) throw err("invalid", "those cells are in protected columns, so nothing changed (right-click a heading › Protect column to switch it off)");
    }
    const n = this.rowCount(d);
    let widest = 0;
    for (const [r, c] of cells) {
      if (r >= n) throw err("out_of_range", "row or column out of range");
      if (c + 1 > widest) widest = c + 1;
    }
    return this.push(d, () => {
      const grew = this.grow(d, widest);
      const old = cells.map(([r, c, v]) => {
        const k = `${this.rowId(d, r)}:${this.physCol(d, c)}`;
        const prev = d.edits.get(k);
        d.edits.set(k, v);
        return [k, prev] as const;
      });
      return () => {
        for (const [k, prev] of old.reverse()) {
          if (prev === undefined) d.edits.delete(k);
          else d.edits.set(k, prev);
        }
        grew();
      };
    });
  }

  /** Make the column count at least `width`; returns the inverse. */
  private grow(d: MockDoc, width: number): () => void {
    if (d.cols) {
      const map = d.cols;
      const before = map.length;
      while (map.length < width) map.push(d.nextCol++);
      return () => {
        map.length = before;
      };
    }
    const before = d.baseCols;
    d.baseCols = Math.max(d.baseCols, width);
    d.nextCol = Math.max(d.nextCol, d.baseCols);
    return () => {
      d.baseCols = before;
    };
  }

  async fillCells(doc: number, row: number, rows: number, col: number, cols: number, value: string, skip: number[] = []) {
    if (rows * cols > 100_000) throw err("too_large", "one change is limited to 100,000 cells");
    const hide = new Set(skip);
    const cells: [number, number, string][] = [];
    for (let r = row; r < row + rows; r++) for (let c = col; c < col + cols; c++) if (!hide.has(c)) cells.push([r, c, value]);
    return this.setCells(doc, cells);
  }

  async fillAreas(doc: number, areas: Rect[], value: string, skip: number[] = []) {
    const parts = disjoint(areas);
    const n = parts.reduce((a, r) => a + r.rows * r.cols, 0);
    if (n > 100_000) throw err("too_large", "one change is limited to 100,000 cells");
    const hide = new Set(skip);
    const cells: [number, number, string][] = [];
    for (const a of parts) for (let r = a.row; r < a.row + a.rows; r++) for (let c = a.col; c < a.col + a.cols; c++) if (!hide.has(c)) cells.push([r, c, value]);
    return this.setCells(doc, cells);
  }

  async hiddenBetween(doc: number, row: number, rows: number): Promise<number> {
    const d = this.get(doc);
    return this.hiddenBetween_(d, row, Math.min(row + rows, this.rowCount(d)));
  }

  async moveCells(doc: number, row: number, rows: number, col: number, cols: number, toRow: number, toCol: number, copy: boolean) {
    const d = this.get(doc);
    if (rows * cols * 2 > 100_000) throw err("too_large", "one change is limited to 100,000 cells");
    const fits = (r: number, c: number) => r >= 0 && c >= 0 && r + rows <= this.rowCount(d) && c + cols <= this.colCount(d);
    if (!fits(row, col) || !fits(toRow, toCol)) throw err("out_of_range", "row or column out of range");
    if (rows * cols === 0 || (row === toRow && col === toCol)) return this.refresh(d);
    const inside = (r: number, c: number) => r >= toRow && r < toRow + rows && c >= toCol && c < toCol + cols;
    const cells: [number, number, string][] = [];
    for (let r = row; r < row + rows; r++) for (let c = col; c < col + cols; c++) if (!copy && !inside(r, c)) cells.push([r, c, ""]);
    for (let i = 0; i < rows; i++) for (let j = 0; j < cols; j++) cells.push([toRow + i, toCol + j, this.value(d, row + i, col + j)]);
    return this.setCells(doc, cells);
  }

  async copyCells(doc: number, row: number, rows: number, col: number, cols: number, skip: number[] = []) {
    const d = this.get(doc);
    if (rows * cols > 1_000_000) throw err("too_large", "copying is limited to 1,000,000 cells at once");
    const hide = new Set(skip);
    const out: string[][] = [];
    for (let r = row; r < Math.min(row + rows, this.rowCount(d)); r++) {
      const line: string[] = [];
      for (let c = col; c < col + cols; c++) if (!hide.has(c)) line.push(this.value(d, r, c));
      out.push(line);
    }
    return toTsv(out);
  }

  private orderOf(d: MockDoc): number[] {
    d.order ??= Array.from({ length: d.baseRows }, (_, i) => i);
    return d.order;
  }
  private colsOf(d: MockDoc): number[] {
    d.cols ??= Array.from({ length: d.baseCols }, (_, i) => i);
    return d.cols;
  }

  async change(doc: number, c: Change): Promise<DocInfo> {
    const d = this.get(doc);
    const rows = this.rowCount(d);
    const cols = this.colCount(d);
    const range = () => err("out_of_range", "row or column out of range");
    const move = (list: number[], at: number, count: number, to: number) => {
      const items = list.splice(at, count);
      list.splice(to, 0, ...items);
    };
    if (c.op === "hide_rows" || c.op === "show_rows") {
      const before = d.hidden;
      const next = new Set(before ?? []);
      if (c.op === "hide_rows") {
        if (c.at + c.count > rows) throw range();
        for (let r = c.at; r < c.at + c.count; r++) next.add(this.rowId(d, r));
      } else if (c.which === "all") next.clear();
      else {
        const w = c.which.around;
        const p0 = w.at === 0 ? 0 : this.pos(d, w.at);
        const p1 = w.at + w.count >= rows ? this.totalRows(d) : this.pos(d, w.at + w.count - 1) + 1;
        for (let p = p0; p < p1; p++) next.delete(d.order ? d.order[p]! : p);
      }
      const after = next.size ? next : null;
      if ((after?.size ?? 0) === (before?.size ?? 0)) return this.refresh(d);
      return this.push(d, () => {
        d.hidden = after;
        d.visible = null;
        return () => {
          d.hidden = before;
          d.visible = null;
        };
      });
    }
    if (c.op === "delete_col_list") throw err("invalid", "the browser preview does not do this; the desktop app does");
    if (c.op === "shuffle_rows" || c.op === "flip_rows") {
      if (c.at + c.count > rows) throw range();
      const order = this.orderOf(d);
      const before = order.slice();
      const pos = Array.from({ length: c.count }, (_, i) => this.pos(d, c.at + i));
      const ids = pos.map((p) => order[p]!);
      if (c.op === "flip_rows") ids.reverse();
      else
        for (let i = ids.length - 1; i > 0; i--) {
          const j = Math.floor(Math.random() * (i + 1));
          [ids[i], ids[j]] = [ids[j]!, ids[i]!];
        }
      const after = before.slice();
      pos.forEach((p, i) => (after[p] = ids[i]!));
      return this.push(d, () => {
        d.order = after.slice();
        d.visible = null;
        return () => {
          d.order = before.slice();
          d.visible = null;
        };
      });
    }
    if (d.shown && (c.op === "insert_rows" || c.op === "delete_rows" || c.op === "move_rows"))
      throw err("filter_active", "turn the filter off to change rows (browser preview)");
    switch (c.op) {
      case "undo": {
        const step = d.undo.pop();
        if (step) {
          step.inverse();
          d.redo.push(step.apply);
        }
        return this.refresh(d);
      }
      case "redo": {
        const apply = d.redo.pop();
        if (apply) d.undo.push({ apply, inverse: apply() });
        return this.refresh(d);
      }
      case "insert_rows":
        if (c.at > rows) throw range();
        return this.push(d, () => {
          const ids = Array.from({ length: c.count }, () => d.nextNew--);
          this.orderOf(d).splice(c.at, 0, ...ids);
          return () => void this.orderOf(d).splice(c.at, c.count);
        });
      case "delete_rows":
        if (c.at + c.count > rows) throw range();
        return this.push(d, () => {
          const ids = this.orderOf(d).splice(c.at, c.count);
          return () => void this.orderOf(d).splice(c.at, 0, ...ids);
        });
      case "move_rows":
        if (c.at + c.count > rows || c.to + c.count > rows) throw range();
        return this.push(d, () => {
          move(this.orderOf(d), c.at, c.count, c.to);
          return () => move(this.orderOf(d), c.to, c.count, c.at);
        });
      case "insert_cols":
        if (c.at > cols) throw range();
        return this.push(d, () => {
          const ids = Array.from({ length: c.count }, () => d.nextCol++);
          this.colsOf(d).splice(c.at, 0, ...ids);
          return () => void this.colsOf(d).splice(c.at, c.count);
        });
      case "delete_cols":
        if (c.at + c.count > cols || c.count >= cols) throw range();
        return this.push(d, () => {
          const ids = this.colsOf(d).splice(c.at, c.count);
          return () => void this.colsOf(d).splice(c.at, 0, ...ids);
        });
      case "move_cols":
        if (c.at + c.count > cols || c.to + c.count > cols) throw range();
        return this.push(d, () => {
          move(this.colsOf(d), c.at, c.count, c.to);
          return () => move(this.colsOf(d), c.to, c.count, c.at);
        });
    }
  }

  async save(doc: number, f: SaveFlags): Promise<DocInfo | null> {
    if (this.get(doc).info.peeking) throw err("peeking", "looking at an earlier recipe step: go back to the latest step to change or save");
    const d = this.get(doc);
    if (!d.info.path && !f.saveAs && d.info.name === "Untitled") {
      d.info.name = "Untitled.csv";
    }
    // Like the engine: saving clears the undo history.
    d.undo = [];
    d.redo = [];
    d.baseMark = ++markSeq;
    d.clean = 0;
    return this.refresh(d);
  }

  async jobProgress(doc?: number): Promise<JobInfo | null> {
    const label = doc === undefined ? undefined : this.jobs.get(doc);
    return label ? { label, fraction: 0.5 } : null;
  }
  async cancelJob(doc?: number, label?: string): Promise<void> {
    const running = doc === undefined ? undefined : this.jobs.get(doc);
    const hit = running !== undefined && (label === undefined || label === running);
    this.callLog.push(`cancel:${label ?? "any"}:${hit ? "ran" : running ? "other" : "none"}`);
    if (hit && doc !== undefined) this.cancelled.add(doc);
  }

  // ---- key-aware diff (the same rules as the engine's diff.rs: shared fixture in diff.test.ts) ----
  private diffStates = new Map<number, { a: number; b: number; result: number; keys: number[]; keysB: number[]; spec: import("./backend").DiffSpec; aSide: Map<string, { kind: number; cols: number[] }>; bSide: Map<string, { kind: number; cols: number[] }>; rev: Record<number, number> }>();
  private diffNo = 0;
  async diffFiles(a: number, b: number, spec: import("./backend").DiffSpec): Promise<import("./backend").DiffReportOut> {
    this.calls.push("diffFiles");
    const da = this.get(a);
    const db = this.get(b);
    const norm = (v: string) => (spec.trim ? v.trim() : v).toString()[spec.match_case ? "toString" : "toLowerCase"]();
    const colsA = this.colCount(da);
    const colsB = this.colCount(db);
    const first = spec.keep_first ? 1 : 0;
    const titleOf = (d: MockDoc, c: number, n: number) => (spec.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`;
    void colsB;
    // A's column -> B's column: by title (repeated titles pair in order) or by position
    const titlesA = Array.from({ length: colsA }, (_, c) => titleOf(da, c, colsA));
    const titlesB = Array.from({ length: colsB }, (_, c) => titleOf(db, c, colsB));
    const queue = new Map<string, number[]>();
    titlesB.forEach((t, j) => queue.set(t.trim().toLowerCase(), [...(queue.get(t.trim().toLowerCase()) ?? []), j]));
    const map: (number | null)[] = titlesA.map((t, c) => (spec.keep_first ? (queue.get(t.trim().toLowerCase())?.shift() ?? null) : c < colsB ? c : null));
    if (spec.keep_first && map.every((m) => m === null)) throw err("invalid", "no column title of this table is in the other file; untick the first row is titles to pair columns by position");
    const keysB = spec.keys.map((k) => map[k] ?? -1);
    if (keysB.some((k) => k < 0)) throw err("invalid", `the key column ${titlesA[spec.keys.find((k) => map[k] === null) ?? 0]} is not in the other file`);
    const key = (d: MockDoc, r: number, cols: number[]) => cols.map((c) => norm(this.value(d, r, c))).join("\u001f");
    const blank = (d: MockDoc, r: number, cols: number[]) => cols.every((c) => (spec.trim ? this.value(d, r, c).trim() : this.value(d, r, c)) === "");
    // B rows by key (repeats in file order)
    const bRows = new Map<string, number[]>();
    for (let r = first; r < this.rowCount(db); r++) if (!blank(db, r, keysB)) bRows.set(key(db, r, keysB), [...(bRows.get(key(db, r, keysB)) ?? []), r]);
    const used = new Set<number>();
    const onlyB = titlesB.map((t, j) => ({ t, j })).filter(({ j }) => !map.includes(j));
    const rows: string[][] = [];
    const aSide = new Map<string, { kind: number; cols: number[] }>();
    const bSide = new Map<string, { kind: number; cols: number[] }>();
    const rep: import("./backend").DiffReport = { same: 0, changed: 0, added: 0, removed: 0, cells: 0, listed: 0, truncated: false, only_a: titlesA.filter((_, c) => map[c] === null), only_b: onlyB.map((x) => x.t), moved: map.filter((m, c) => m !== null && m !== c).length, repeated: 0, dup_a: [], dup_b: [], empty_a: 0, empty_b: 0 };
    const removedRows: string[][] = [];
    const changedRows: string[][] = [];
    const seenA = new Map<string, number>();
    for (let r = first; r < this.rowCount(da); r++) {
      const vals = Array.from({ length: colsA }, (_, c) => this.value(da, r, c));
      if (blank(da, r, spec.keys)) {
        rep.empty_a++;
        rep.removed++;
        removedRows.push(["removed", String(this.pos(da, r) + 1), "", ...vals, ...onlyB.map(() => ""), ""]);
        continue;
      }
      const k = key(da, r, spec.keys);
      seenA.set(k, (seenA.get(k) ?? 0) + 1);
      const cand = (bRows.get(k) ?? []).filter((x) => !used.has(x));
      const j = cand[0];
      if (j === undefined) {
        rep.removed++;
        if ((seenA.get(k) ?? 0) > 1) rep.repeated++;
        removedRows.push(["removed", String(this.pos(da, r) + 1), "", ...vals, ...onlyB.map(() => ""), ""]);
        aSide.set(k, { kind: 2, cols: [] });
        continue;
      }
      used.add(j);
      const newer = map.map((m) => (m === null ? "" : this.value(db, j, m)));
      const cols: number[] = [];
      const what: string[] = [];
      for (let c = 0; c < colsA; c++) if (map[c] !== null && vals[c] !== newer[c]) {
        cols.push(c);
        what.push(`${titlesA[c]}: ${vals[c]} → ${newer[c]}`);
      }
      if (!cols.length) {
        rep.same++;
        continue;
      }
      rep.changed++;
      rep.cells += cols.length;
      changedRows.push(["changed", String(this.pos(da, r) + 1), "", ...newer.map((v, c) => (map[c] === null ? (vals[c] ?? "") : v)), ...onlyB.map((x) => this.value(db, j!, x.j)), what.join("; ")]);
      aSide.set(k, { kind: 0, cols });
      bSide.set(k, { kind: 0, cols: cols.map((c) => map[c]!) });
    }
    const addedRows: string[][] = [];
    for (let r = first; r < this.rowCount(db); r++) {
      if (blank(db, r, keysB)) {
        rep.empty_b++;
        continue;
      }
      if (used.has(r)) continue;
      rep.added++;
      const vals = map.map((m) => (m === null ? "" : this.value(db, r, m)));
      addedRows.push(["added", "", "", ...vals, ...onlyB.map((x) => this.value(db, r, x.j)), ""]);
      bSide.set(key(db, r, keysB), { kind: 1, cols: [] });
    }
    for (const [k, n] of seenA) if (n > 1) rep.dup_a.push(k.split("\u001f").join(" | "));
    rep.dup_a = rep.dup_a.slice(0, 10);
    for (const [k, rs] of bRows) if (rs.length > 1 && rep.dup_b.length < 10) rep.dup_b.push(k.split("\u001f").join(" | "));
    rep.repeated = Math.max(rep.repeated, rep.dup_a.length + rep.dup_b.length);
    rows.push(["Change", "Row (first)", "Row (second)", ...titlesA, ...onlyB.map((x) => x.t), "What changed"], ...removedRows, ...changedRows, ...addedRows);
    rep.listed = rows.length - 1;
    const width = rows[0]!.length;
    const info = this.make("Untitled", rows.length, width);
    await this.setCells(info.id, rows.flatMap((r, ri) => r.map((v, c) => [ri, c, v] as [number, number, string])));
    const named = await this.setDocTitle(info.id, `Diff: ${da.info.name.replace(/\.[^. ]{1,5}$/, "")} vs ${db.info.name.replace(/\.[^. ]{1,5}$/, "")}`);
    const id = ++this.diffNo;
    this.diffStates.set(id, { a, b, result: info.id, keys: spec.keys, keysB, spec, aSide, bSide, rev: { [a]: da.info.data_revision ?? 0, [b]: db.info.data_revision ?? 0, [info.id]: this.get(info.id).info.data_revision ?? 0 } });
    return { info: named, report: rep, diff: id };
  }
  async diffMarks(doc: number, row: number, rows: number, col: number, cols: number): Promise<import("./backend").DiffMarksOut> {
    this.calls.push("diffMarks");
    const d = this.get(doc);
    const found = [...this.diffStates.entries()].filter(([, s]) => s.a === doc || s.b === doc || s.result === doc).sort((x, y) => y[0] - x[0])[0];
    if (!found) return { diff: null, out_of_date: false, marks: [] };
    const [id, s] = found;
    if ((d.info.data_revision ?? 0) !== s.rev[doc]) return { diff: id, out_of_date: true, marks: [] };
    const side = s.result === doc ? "result" : s.a === doc ? "a" : "b";
    const keys = side === "a" ? s.keys : side === "b" ? s.keysB : s.keys.map((k) => k + 3);
    const norm = (v: string) => (s.spec.trim ? v.trim() : v)[s.spec.match_case ? "toString" : "toLowerCase"]();
    const first = side === "result" || s.spec.keep_first ? 1 : 0;
    const marks: import("./backend").DiffMark[] = [];
    for (let r = Math.max(row, first); r < Math.min(row + rows, this.rowCount(d)); r++) {
      const parts = keys.map((c) => this.value(d, r, c));
      if (parts.every((p) => (s.spec.trim ? p.trim() : p) === "")) continue;
      const k = parts.map(norm).join("\u001f");
      const e = side === "a" ? s.aSide.get(k) : side === "b" ? s.bSide.get(k) : (s.aSide.get(k) ?? s.bSide.get(k));
      if (!e) continue;
      if (e.kind === 0) for (const c of e.cols) {
        const shown = side === "result" ? c + 3 : c;
        if (shown >= col && shown < col + cols) marks.push({ row: r, col: shown, kind: 0 });
      }
      else marks.push({ row: r, col: null, kind: e.kind });
    }
    return { diff: id, out_of_date: false, marks };
  }
  async diffNext(doc: number, row: number, backward: boolean): Promise<import("./backend").DiffJump | null> {
    this.calls.push("diffNext");
    const d = this.get(doc);
    const n = this.rowCount(d);
    const all: { row: number; col: number | null }[] = [];
    for (let r = 0; r < n; r += 500) for (const m of (await this.diffMarks(doc, r, 500, 0, 1000)).marks) all.push({ row: m.row, col: m.col });
    all.sort((x, y) => x.row - y.row || (x.col ?? -1) - (y.col ?? -1));
    const rowsHit = [...new Set(all.map((x) => x.row))];
    if (!rowsHit.length) return null;
    const next = backward ? [...rowsHit].reverse().find((r) => r < row) : rowsHit.find((r) => r > row);
    const hit = next ?? (backward ? rowsHit.at(-1)! : rowsHit[0]!);
    const cols = all.filter((x) => x.row === hit && x.col !== null).map((x) => x.col!);
    return { row: hit, col: cols.length ? Math.min(...cols) : null, wrapped: next === undefined };
  }
  async diffClear(doc: number): Promise<void> {
    this.calls.push("diffClear");
    for (const [id, s] of [...this.diffStates]) if (s.a === doc || s.b === doc || s.result === doc) this.diffStates.delete(id);
  }
  /** A sort only reorders the rows: a diff drawn on the tab still fits (as in the engine's shell). */
  private diffFollow(doc: number, before: number, after: number): void {
    for (const s of this.diffStates.values()) if (s.rev[doc] === before) s.rev[doc] = after;
  }
  async sortRows(doc: number, spec: SortSpec): Promise<DocInfo> {
    const before = this.get(doc).info.data_revision ?? 0;
    const info = await this.sortRowsInner(doc, spec);
    this.diffFollow(doc, before, info.data_revision ?? 0);
    return info;
  }
  async sortRowsInner(doc: number, spec: SortSpec): Promise<DocInfo> {
    const d = this.get(doc);
    const n = this.totalRows(d);
    const first = spec.keep_first && n > 0 ? 1 : 0;
    const positions = Array.from({ length: n - first }, (_, i) => i + first);
    const keys = spec.keys.map((k) => positions.map((p) => this.valueAt(d, p, k.col)));
    const idx = positions.map((_, i) => i);
    idx.sort((a, b) => {
      for (let k = 0; k < spec.keys.length; k++) {
        const x = keys[k]![a]!;
        const y = keys[k]![b]!;
        if (x === "" || y === "") {
          if (x !== y) return x === "" ? 1 : -1;
          continue;
        }
        const o = naturalCompare(x, y);
        if (o !== 0) return spec.keys[k]!.descending ? -o : o;
      }
      return a - b;
    });
    const before = this.orderOf(d).slice();
    const after = [...before.slice(0, first), ...idx.map((i) => before[positions[i]!]!)];
    return this.push(d, () => {
      d.order = after.slice();
      return () => {
        d.order = before.slice();
      };
    });
  }

  async setFilter(doc: number, spec: FilterSpec): Promise<DocInfo> {
    const d = this.get(doc);
    if (spec.columns.length === 0) return this.clearFilter(doc);
    const n = this.totalRows(d);
    const shown = new Set<number>();
    for (let p = 0; p < n; p++) {
      const id = d.order ? d.order[p]! : p;
      if ((p === 0 && spec.keep_first) || spec.columns.every((c) => testRule(c.rule, this.valueAt(d, p, c.col)))) shown.add(id);
    }
    return this.setView(d, shown, spec);
  }

  async clearFilter(doc: number): Promise<DocInfo> {
    return this.setView(this.get(doc), null, null);
  }

  async filterSpec(doc: number): Promise<FilterSpec | null> {
    return this.get(doc).filter;
  }

  async filterValues(doc: number, col: number, spec: FilterSpec, search: string | null): Promise<ValueList> {
    const d = this.get(doc);
    const n = this.totalRows(d);
    const counts = new Map<string, number>();
    let blanks = 0;
    let complete = true;
    const others = spec.columns.filter((c) => c.col !== col);
    for (let p = spec.keep_first ? 1 : 0; p < n; p++) {
      if (!others.every((c) => testRule(c.rule, this.valueAt(d, p, c.col)))) continue;
      const v = this.valueAt(d, p, col);
      if (v === "") blanks++;
      else if (search && !v.toLowerCase().includes(search.toLowerCase())) continue;
      else if (counts.has(v)) counts.set(v, counts.get(v)! + 1);
      else if (counts.size < 10_000) counts.set(v, 1);
      else complete = false;
    }
    const values = [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => naturalCompare(a.value, b.value));
    return { values, blanks, complete };
  }

  /** Cells a search covers: shown rows and logical columns. */
  private area(d: MockDoc, spec: FindSpec): { r0: number; r1: number; cols: number[]; member?: (r: number, c: number) => boolean } {
    const n = this.rowCount(d);
    const first = spec.skip_first && n > 0 ? 1 : 0;
    const skip = new Set(spec.skip_cols ?? []);
    const all = Array.from({ length: this.colCount(d) }, (_, i) => i).filter((c) => !skip.has(c));
    switch (spec.scope.kind) {
      case "all":
        return { r0: first, r1: n, cols: all };
      case "columns":
        return { r0: first, r1: n, cols: spec.scope.cols.filter((c) => all.includes(c)) };
      case "range": {
        const g = spec.scope;
        return {
          r0: spec.scope.row,
          r1: Math.min(n, spec.scope.row + spec.scope.rows),
          cols: all.filter((c) => c >= g.col && c < g.col + g.cols),
        };
      }
      case "areas": {
        // Several blocks: the bounding box, and a test for a cell in one of them
        // (the engine makes the blocks disjoint; here a cell is in or out).
        const blocks = spec.scope.areas.filter((b) => b.rows > 0 && b.cols > 0);
        if (blocks.length > 10_000) throw err("invalid", "Find works on up to 10,000 blocks at once");
        const inBlock = (r: number, c: number) => blocks.some((b) => r >= b.row && r < b.row + b.rows && c >= b.col && c < b.col + b.cols);
        const rows = blocks.map((b) => [Math.max(b.row, first), Math.min(n, b.row + b.rows)] as const).filter(([a, z]) => a < z);
        const cols = all.filter((c) => blocks.some((b) => c >= b.col && c < b.col + b.cols));
        if (!rows.length || !cols.length) return { r0: 0, r1: 0, cols: [], member: () => false };
        return { r0: Math.min(...rows.map((x) => x[0])), r1: Math.max(...rows.map((x) => x[1])), cols, member: (r, c) => r >= first && inBlock(r, c) };
      }
    }
  }

  async findNext(doc: number, spec: FindSpec, from: [number, number] | null, backward: boolean): Promise<Found | null> {
    const d = this.get(doc);
    if (!spec.pattern.text) throw err("bad_pattern", "the pattern is not valid: enter something to look for");
    const re = regexOf(spec.pattern);
    const { r0, r1, cols, member } = this.area(d, spec);
    const cells: [number, number][] = [];
    // Row-major list is too big for a million rows; walk instead.
    const total = (r1 - r0) * cols.length;
    if (total <= 0) return null;
    if (member && !spec.by_columns) {
      // Several blocks: the matches in reading order, then the next after `from`.
      const list: [number, number][] = [];
      for (let r = r0; r < r1; r++) for (const c of cols) if (member(r, c) && re.test(this.value(d, r, c))) list.push([r, c]);
      if (!list.length) return null;
      const outside = from && !(from[0] >= r0 && from[0] < r1);
      const f = outside ? null : from;
      const after = (r: number, c: number) => (f ? r > f[0] || (r === f[0] && c > f[1]) : true);
      const before = (r: number, c: number) => (f ? r < f[0] || (r === f[0] && c < f[1]) : true);
      const pick = backward ? [...list].reverse().find(([r, c]) => before(r, c)) : list.find(([r, c]) => after(r, c));
      const hit = pick ?? (backward ? list[list.length - 1]! : list[0]!);
      return { row: hit[0], col: hit[1], wrapped: !pick && !!f };
    }
    if (spec.by_columns) {
      // Down each column, then the next column.
      const list: [number, number][] = [];
      for (const c of cols) for (let r = r0; r < r1; r++) if ((!member || member(r, c)) && re.test(this.value(d, r, c))) list.push([r, c]);
      if (!list.length) return null;
      const key = (r: number, c: number) => cols.indexOf(c) * (r1 + 1) + r;
      const here = from ? key(from[0], from[1]) : backward ? Infinity : -1;
      const pick = backward ? [...list].reverse().find(([r, c]) => key(r, c) < here) : list.find(([r, c]) => key(r, c) > here);
      const hit = pick ?? (backward ? list[list.length - 1]! : list[0]!);
      return { row: hit[0], col: hit[1], wrapped: !pick };
    }
    const startRow = from && from[0] >= r0 && from[0] < r1 ? from[0] : backward ? r1 - 1 : r0;
    const k0 = from ? cols.findIndex((c) => c >= from[1]) : 0;
    let i = (startRow - r0) * cols.length + Math.max(0, k0 === -1 ? cols.length - 1 : k0);
    void cells;
    for (let step = 1; step <= total; step++) {
      const j = (((i + (backward ? -step : step)) % total) + total) % total;
      const r = r0 + Math.floor(j / cols.length);
      const c = cols[j % cols.length]!;
      if (re.test(this.value(d, r, c))) {
        const wrapped = backward ? j > i : j < i;
        return { row: r, col: c, wrapped };
      }
    }
    i = 0;
    return null;
  }

  async findCount(doc: number, spec: FindSpec, at: [number, number] | null): Promise<FindCount> {
    const d = this.get(doc);
    if (!spec.pattern.text) throw err("bad_pattern", "the pattern is not valid: enter something to look for");
    const re = regexOf(spec.pattern);
    const { r0, r1, cols, member } = this.area(d, spec);
    const out: FindCount = { cells: 0, before: 0, here: false };
    for (let r = r0; r < r1; r++)
      for (let k = 0; k < cols.length; k++) {
        const c = cols[k]!;
        if (member && !member(r, c)) continue;
        if (!re.test(this.value(d, r, c))) continue;
        out.cells++;
        const earlier = spec.by_columns ? c < (at?.[1] ?? 0) || (c === at?.[1] && r < at[0]) : at && (r < at[0] || (r === at[0] && c < at[1]));
        if (at && earlier) out.before++;
        else if (at && r === at[0] && c === at[1]) out.here = true;
      }
    // Hidden rows too: counted, and before the cell when above it.
    for (const [pos, c] of this.hiddenMatches(d, spec, re)) {
      out.cells++;
      if (at && pos < this.pos(d, at[0])) out.before++;
    }
    return out;
  }

  /** Matches in hidden rows (document position, column), when asked for. */
  private hiddenMatches(d: MockDoc, spec: FindSpec, re: RegExp): [number, number][] {
    const vis = this.visible(d);
    if (!spec.hidden_rows || !vis) return [];
    const shown = new Set(vis);
    const { cols, member } = this.area(d, spec);
    // (With several blocks the browser preview does not search hidden rows.)
    if (member) return [];
    const out: [number, number][] = [];
    for (let p = spec.skip_first ? 1 : 0; p < this.totalRows(d); p++) if (!shown.has(p)) for (const c of cols) if (re.test(this.valueAt(d, p, c))) out.push([p, c]);
    return out;
  }

  /** The ghost diff: the same routine as Replace all (`planReplace`), for the cells of a window. */
  async replaceGhosts(doc: number, spec: FindSpec, replacement: string, row: number, rows: number, col: number, cols: number): Promise<import("./backend").GhostWindow> {
    this.calls.push("replaceGhosts");
    const d = this.get(doc);
    if (!spec.pattern.text) return { ghosts: [], partial: false };
    const plan = this.planReplace(d, spec, replacement);
    const hidden = new Set(spec.hidden_cols ?? []);
    const ghosts = plan.changes
      .filter(([r, c]) => r >= row && r < row + rows && c >= col && c < col + cols && !hidden.has(c))
      .map(([r, c, out]) => ({ row: r, number: this.pos(d, r), col: c, after: out.slice(0, 20_000), cut: out.length > 20_000 }));
    return { ghosts, partial: false };
  }
  /** Tests: held back by a promise to make the count slow. */
  countGate: Promise<void> | null = null;
  private countSeq = new Map<number, number>();
  async replaceCount(doc: number, spec: FindSpec, replacement: string): Promise<number> {
    this.calls.push("replaceCount");
    const seq = (this.countSeq.get(doc) ?? 0) + 1;
    this.countSeq.set(doc, seq);
    if (this.countGate) await this.countGate;
    if (this.countSeq.get(doc) !== seq) throw err("cancelled", "a newer count started");
    return this.planReplace(this.get(doc), spec, replacement).changes.length;
  }
  async replaceCountCancel(doc: number): Promise<void> {
    this.calls.push("replaceCountCancel");
    this.countSeq.set(doc, (this.countSeq.get(doc) ?? 0) + 1);
  }
  async findMarks(doc: number, spec: FindSpec, row: number, rows: number, col: number, cols: number): Promise<[number, number][]> {
    const d = this.get(doc);
    if (!spec.pattern.text) return [];
    const re = regexOf(spec.pattern);
    const a = this.area(d, spec);
    const out: [number, number][] = [];
    for (let r = Math.max(row, a.r0); r < Math.min(row + rows, a.r1); r++)
      for (const c of a.cols) if (c >= col && c < col + cols && (!a.member || a.member(r, c)) && re.test(this.value(d, r, c))) out.push([r, c]);
    return out;
  }

  async findAll(doc: number, spec: FindSpec): Promise<FindAll> {
    const d = this.get(doc);
    const re = regexOf(spec.pattern);
    const { r0, r1, cols, member } = this.area(d, spec);
    const hits: FindAll["hits"] = [];
    let cells = 0;
    for (let r = r0; r < r1; r++)
      for (const c of cols) {
        if (member && !member(r, c)) continue;
        const v = this.value(d, r, c);
        const m = re.exec(v);
        if (!m) continue;
        cells++;
        if (hits.length < 10_000) hits.push({ row: r, number: this.pos(d, r), col: c, text: v.replace(/[\r\n\t]/g, " ").slice(0, 200), start: [...v.slice(0, m.index)].length, len: [...m[0]].length });
      }
    const hidden = this.hiddenMatches(d, spec, re);
    if (hidden.length) {
      const vis = this.visible(d) ?? [];
      for (const [pos, c] of hidden) {
        const v = this.valueAt(d, pos, c);
        const m = re.exec(v)!;
        const near = Math.max(0, vis.findIndex((x) => x > pos));
        hits.push({ row: near === 0 && vis[0]! > pos ? 0 : near > 0 ? near - 1 : vis.length - 1, number: pos, col: c, text: v.replace(/[\r\n\t]/g, " ").slice(0, 200), start: [...v.slice(0, m.index)].length, len: [...m[0]].length, hidden: true });
      }
      hits.sort((a, b) => a.number - b.number || a.col - b.col);
      cells += hidden.length;
    }
    const per = new Map<number, number>();
    for (const r of this.matchingCells(d, spec)) per.set(r[1], (per.get(r[1]) ?? 0) + 1);
    return { hits, cells, per_col: [...per].sort((a, b) => a[0] - b[0]) };
  }

  private *matchingCells(d: MockDoc, spec: FindSpec): Generator<[number, number]> {
    const re = regexOf(spec.pattern);
    const { r0, r1, cols, member } = this.area(d, spec);
    for (let r = r0; r < r1; r++) for (const c of cols) if ((!member || member(r, c)) && re.test(this.value(d, r, c))) yield [r, c];
  }

  private planReplace(d: MockDoc, spec: FindSpec, rep: string) {
    const re = regexOf(spec.pattern, true);
    if (spec.pattern.escapes || spec.pattern.kind === "regex") rep = unescapeText(rep);
    const { r0, r1, cols, member } = this.area(d, spec);
    const changes: [number, number, string][] = [];
    const preview: ReplaceReport["preview"] = [];
    const hiddenCols = new Set(spec.hidden_cols ?? []);
    let hits = 0;
    let hiddenCells = 0;
    for (let r = r0; r < r1; r++)
      for (const c of cols) {
        if (member && !member(r, c)) continue;
        const v = this.value(d, r, c);
        let n = 0;
        const out = v.replace(re, (m, ...rest) => {
          if (m === "" && v !== "") return m;
          n++;
          const piece = spec.pattern.kind === "regex" ? rep.replace(/\$(\d)/g, (_, g) => String(rest[Number(g) - 1] ?? "")) : rep;
          return spec.pattern.preserve_case ? keepCase(m, piece) : piece;
        });
        if (n && out !== v) {
          hits += n;
          changes.push([r, c, out]);
          if (hiddenCols.has(c)) hiddenCells++;
          if (preview.length < 50) preview.push({ row: r, number: this.pos(d, r), col: c, before: v, after: out, hidden: hiddenCols.has(c) });
        }
      }
    return { changes, preview, hits, hiddenCells };
  }

  private previewKey = new Map<number, string>();
  async replaceConfirm(doc: number, spec: FindSpec, replacement: string, exclude: [number, number][] = []): Promise<ReplaceReport> {
    const key = JSON.stringify([spec, replacement]);
    const ok = key === this.previewKey.get(doc);
    this.previewKey.delete(doc);
    if (!ok) throw err("changed", "the search or the file changed since the preview; nothing was replaced. Look at the changes again (Replace all…)");
    const p = this.planReplace(this.get(doc), spec, replacement);
    const d = this.get(doc);
    const skip = new Set(exclude.map(([n, c]) => `${n}:${c}`));
    const changes = p.changes.filter(([r, c]) => !skip.has(`${this.pos(d, r)}:${c}`));
    this.changedRows = [...new Set(changes.map(([r]) => this.rowId(this.get(doc), r)))];
    const info = changes.length ? await this.setCells(doc, changes) : this.refresh(this.get(doc));
    return { cells: changes.length, hits: p.hits, preview: [], info };
  }

  async replaceCell(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport> {
    return this.replaceAll(doc, spec, replacement);
  }

  private matchingRows(d: MockDoc, spec: FindSpec): number[] {
    const re = regexOf(spec.pattern);
    const { r0, r1, cols, member } = this.area(d, spec);
    const out: number[] = [];
    for (let r = r0; r < r1; r++) if (cols.some((c) => (!member || member(r, c)) && re.test(this.value(d, r, c)))) out.push(r);
    return out;
  }

  async findRows(doc: number, spec: FindSpec, action: FindRowsAction): Promise<import("./backend").FindRowsReport> {
    const d = this.get(doc);
    const rows = this.matchingRows(d, spec);
    const done = (info: DocInfo, recolored = 0, left_out_cols = 0) => ({ info, recolored, left_out_cols });
    switch (action.kind) {
      case "copy": {
        const skip = new Set(spec.skip_cols ?? []);
        const cols = Array.from({ length: this.colCount(d) }, (_, c) => c).filter((c) => !skip.has(c));
        const line = (r: number) => cols.map((c) => this.value(d, r, c));
        const lines = [...(spec.skip_first ? [line(0)] : []), ...rows.map(line)];
        const info = this.make("Untitled", Math.max(1, lines.length), cols.length);
        await this.setCells(info.id, lines.flatMap((l, r) => l.map((v, c) => [r, c, v] as [number, number, string])));
        return done(await this.setDocTitle(info.id, `Matches (${d.info.name.replace(/\.[^. ]{1,5}$/, "")})`), 0, this.colCount(d) - cols.length);
      }
      case "flag": {
        let recolored = 0;
        const m = this.notes.get(doc);
        for (const r of rows) {
          const old = m?.get(r);
          if (old && old.flag && old.flag !== action.flag) recolored++;
          await this.noteSet(doc, r, action.flag, old?.text ?? "");
        }
        return done(this.refresh(d), recolored);
      }
      case "delete":
        return done(this.removeRows(d, rows));
      case "show":
        return done(this.showOnly(d, new Set(rows.map((r) => this.rowId(d, r))), spec.skip_first));
    }
  }

  async findNotes(doc: number, pattern: Pattern): Promise<import("./notes").NoteMark[]> {
    const re = regexOf(pattern);
    const [all] = await this.noteList(doc);
    return all.filter((n) => re.test(n.text));
  }

  async replaceTitles(doc: number, spec: FindSpec, replacement: string): Promise<[number, string][]> {
    const d = this.get(doc);
    const re = regexOf(spec.pattern, true);
    const out: [number, string][] = [];
    const cols = spec.scope.kind === "columns" ? spec.scope.cols : Array.from({ length: this.colCount(d) }, (_, i) => i);
    for (const c of cols) {
      const t = this.value(d, 0, c);
      const n = t.replace(re, replacement);
      if (n !== t) out.push([c, n]);
    }
    return out;
  }

  private batchKey = "";
  private changedRows: number[] = [];

  async showChanged(doc: number, keepFirst: boolean): Promise<DocInfo> {
    const d = this.get(doc);
    if (!this.changedRows.length) throw err("invalid", "nothing was replaced yet");
    return this.showOnly(d, new Set(this.changedRows), keepFirst);
  }

  /** Hide every shown row but these (ids) as one undo step. */
  private showOnly(d: MockDoc, keep: Set<number>, keepFirst: boolean): DocInfo {
    const before = d.hidden;
    const next = new Set(before ?? []);
    for (let r = keepFirst ? 1 : 0; r < this.rowCount(d); r++) {
      const id = this.rowId(d, r);
      if (!keep.has(id)) next.add(id);
    }
    return this.push(d, () => {
      d.hidden = next.size ? next : null;
      d.visible = null;
      return () => {
        d.hidden = before;
        d.visible = null;
      };
    });
  }

  async columnPairs(doc: number, findCol: number, withCol: number, skipFirst: boolean): Promise<[string, string][]> {
    const d = this.get(doc);
    const out: [string, string][] = [];
    for (let r = skipFirst ? 1 : 0; r < this.rowCount(d); r++) {
      const a = this.value(d, r, findCol);
      if (a) out.push([a, this.value(d, r, withCol)]);
    }
    return out;
  }
  private batchPlan(d: MockDoc, spec: BatchReplace): [number, number, string][] {
    const { r0, r1, cols } = this.area(d, { pattern: { text: "x", kind: "plain", match_case: false, whole_cell: false }, scope: spec.scope, skip_first: spec.skip_first, skip_cols: spec.skip_cols });
    const key = (s: string) => (spec.match_case ? s : s.toLowerCase());
    const map = new Map<string, string>();
    for (const [a, b] of spec.pairs) if (a && !map.has(key(a))) map.set(key(a), b);
    const finds = [...spec.pairs].filter(([a]) => a).sort((x, y) => y[0].length - x[0].length);
    const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const re = finds.length ? new RegExp(finds.map(([a]) => esc(a)).join("|"), spec.match_case ? "g" : "gi") : null;
    const out: [number, number, string][] = [];
    for (let r = r0; r < r1; r++)
      for (const c of cols) {
        const v = this.value(d, r, c);
        let n: string | undefined;
        if (spec.whole_cell) n = map.get(key(v));
        else if (re) n = v.replace(re, (m) => map.get(key(m)) ?? m);
        if (n !== undefined && n !== v) out.push([r, c, n]);
      }
    return out;
  }

  async batchPreview(doc: number, spec: BatchReplace): Promise<ReplaceReport> {
    const d = this.get(doc);
    const plan = this.batchPlan(d, spec);
    this.batchKey = JSON.stringify(spec);
    this.lastReplace = plan.map(([r, c, after]) => ({ row: r, number: this.pos(d, r), col: c, before: this.value(d, r, c), after }));
    return { cells: plan.length, hits: plan.length, preview: this.lastReplace.slice(0, 50), info: null };
  }

  async batchConfirm(doc: number, spec: BatchReplace): Promise<ReplaceReport> {
    const ok = JSON.stringify(spec) === this.batchKey;
    this.batchKey = "";
    if (!ok) throw err("changed", "the list or the file changed since the preview; nothing was replaced. Look at the changes again");
    const plan = this.batchPlan(this.get(doc), spec);
    this.changedRows = [...new Set(plan.map(([r]) => this.rowId(this.get(doc), r)))];
    const info = plan.length ? await this.setCells(doc, plan) : this.refresh(this.get(doc));
    return { cells: plan.length, hits: plan.length, preview: [], info };
  }

  async findCountCancel(): Promise<void> {}

  async setDocTitle(doc: number, title: string): Promise<DocInfo> {
    const d = this.get(doc);
    if (d.info.path) throw err("has_path", "this tab is a saved file: use Rename file to rename it");
    const t = title.replace(/[\u0000-\u001f]/g, " ").trim().replace(/\s+/g, " ").slice(0, 120);
    d.info.name = t || "Untitled";
    return this.refresh(d);
  }

  private lastReplace: ReplaceReport["preview"] = [];
  async replacePage(_doc: number, offset: number, count: number): Promise<ReplaceReport["preview"]> {
    return this.lastReplace.slice(offset, offset + Math.min(count, 500));
  }

  async replacePreview(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport> {
    const p = this.planReplace(this.get(doc), spec, replacement);
    const d = this.get(doc);
    this.previewKey.set(doc, JSON.stringify([spec, replacement]));
    this.lastReplace = p.changes.slice(0, 100_000).map(([r, c, after]) => ({ row: r, number: this.pos(d, r), col: c, before: this.value(d, r, c), after, hidden: (spec.hidden_cols ?? []).includes(c) }));
    return { cells: p.changes.length, hits: p.hits, preview: p.preview, info: null, hidden_cells: p.hiddenCells };
  }

  async replaceAll(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport> {
    const p = this.planReplace(this.get(doc), spec, replacement);
    if (p.changes.length > 100_000) throw err("too_large", "the browser preview replaces at most 100,000 cells");
    const info = p.changes.length ? await this.setCells(doc, p.changes) : this.refresh(this.get(doc));
    return { cells: p.changes.length, hits: p.hits, preview: [], info };
  }

  async findInText(text: string, pattern: Pattern): Promise<[number, number][]> {
    const out: [number, number][] = [];
    for (const m of text.matchAll(regexOf(pattern, true))) {
      if (m[0] === "") continue;
      out.push([m.index, m.index + m[0].length]);
    }
    return out;
  }

  // ---- Milestone 4b (simple versions of the engine's operations) ----

  private report(info: DocInfo, x: Partial<OpReport> = {}): OpReport {
    return { info, cells: 0, rows: 0, groups: 0, cols: 0, skipped: 0, ...x };
  }

  /** Remove shown rows `rows` (shown-row numbers) as one undo step. */
  private removeRows(d: MockDoc, rows: number[]): DocInfo {
    const positions = rows.map((r) => this.pos(d, r)).sort((a, b) => a - b);
    return this.push(d, () => {
      const order = this.orderOf(d);
      const gone = positions.map((p) => order[p]!);
      for (let i = positions.length - 1; i >= 0; i--) order.splice(positions[i]!, 1);
      d.visible = null;
      return () => {
        positions.forEach((p, i) => order.splice(p, 0, gone[i]!));
        d.visible = null;
      };
    });
  }

  private keyOf(d: MockDoc, r: number, cols: number[], matchCase: boolean, trim: boolean): string {
    return JSON.stringify(cols.map((c) => {
      let v = this.value(d, r, c);
      if (trim) v = v.trim();
      return matchCase ? v : v.toLowerCase();
    }));
  }

  private seriesValue(start: string[], mode: { kind: string; step?: number; stop?: number | null }, i: number): string | null {
    const n = start.length;
    const nums = start.map((v) => (/^[-+]?\d+(\.\d+)?$/.test(v.trim()) ? Number(v) : null));
    const copy = () => start[((i % n) + n) % n]!;
    if (mode.kind === "copy") return copy();
    if (mode.kind === "linear" || mode.kind === "growth") {
      const a = nums[0];
      if (a === null || a === undefined) throw err("invalid", "a series needs a number in the first cell");
      const x = mode.kind === "linear" ? a + (mode.step ?? 1) * i : a * (mode.step ?? 1) ** i;
      if (mode.stop !== null && mode.stop !== undefined && ((mode.step ?? 1) >= 0 ? x > mode.stop : x < mode.stop)) return null;
      return String(Math.round(x * 1e9) / 1e9);
    }
    if (mode.kind === "date") {
      const t = Date.parse(start[0] ?? "");
      if (Number.isNaN(t)) throw err("invalid", "a date series needs a date in the first cell");
      return new Date(t + i * (mode.step ?? 1) * 86_400_000).toISOString().slice(0, 10);
    }
    const allNums = nums.every((x) => x !== null);
    const swap = mode.kind === "auto_swap";
    if (allNums && n >= 2 && !swap) {
      const step = (nums[n - 1]! - nums[0]!) / (n - 1);
      return String(Math.round((nums[0]! + step * i) * 1e9) / 1e9);
    }
    if (allNums && n === 1 && swap) return String(nums[0]! + i);
    const m = start.map((v) => /^(.*?)(\d+)(\D*)$/.exec(v));
    if (!swap && m.every((x) => x && x[1] === m[0]![1] && x[3] === m[0]![3])) {
      const first = Number(m[0]![2]);
      const step = n >= 2 ? Number(m[n - 1]![2]) - first : 1;
      return `${m[0]![1]}${Math.abs(first + step * i)}${m[0]![3]}`;
    }
    return copy();
  }

  /** The date review step in the stand-in: slash dates and year-month-day only (null: left as typed). */
  private reviewedDate(v: string, ambiguous: "leave" | "mdy" | "dmy"): string | null {
    if (/^\d{4}-\d{2}-\d{2}$/.test(v.trim()) || !v.trim()) return v;
    const two = dateReadings(v);
    if (!two) return null;
    const real = (iso: string) => Number(iso.slice(5, 7)) <= 12;
    if (real(two.mdy) && real(two.dmy) && two.mdy !== two.dmy) return ambiguous === "leave" ? null : two[ambiguous];
    return real(two.mdy) ? two.mdy : real(two.dmy) ? two.dmy : null;
  }

  private planClean(d: MockDoc, spec: CleanSpec, maxRows: number) {
    const step = spec.cleaner.steps[0];
    const review = step?.tool === "dates" && step.review ? step : null;
    let left = 0;
    const { r0, r1, cols } = this.area(d, { pattern: { text: "", kind: "plain", match_case: false, whole_cell: false }, scope: spec.scope, skip_first: spec.skip_first });
    const changes: [number, number, string][] = [];
    const preview: CleanPreview["changes"] = [];
    const end = Math.min(r1, r0 + maxRows);
    for (let r = r0; r < end; r++)
      for (const c of cols) {
        const v = this.value(d, r, c);
        let out: string;
        if (review) {
          const date = this.reviewedDate(v, review.ambiguous ?? "leave");
          if (date === null) left++;
          out = date ?? v;
        } else out = runCleaner(spec.cleaner, v, r - r0);
        if (out === v) continue;
        changes.push([r, c, out]);
        if (preview.length < 50) preview.push({ row: r, number: this.pos(d, r), col: c, before: v, after: out });
      }
    return { changes, preview, rows: end - r0, more: end < r1, left };
  }

  async cleanPreview(doc: number, spec: CleanSpec, now: number): Promise<CleanPreview> {
    void now;
    const p = this.planClean(this.get(doc), spec, 20_000);
    return { changes: p.preview, rows: p.rows, changed: p.changes.length, left: p.left, more: p.more };
  }

  async cleanPreviewPage(doc: number, spec: CleanSpec, now: number, offset: number, want: number): Promise<CleanPreview> {
    void now;
    const p = this.planClean(this.get(doc), spec, 1_000_000_000);
    const homoglyph_cells = p.changes.filter(([r, c]) => scanGremlins(this.value(this.get(doc), r, c)).some((g) => g.kind === "homoglyph")).length;
    return { changes: p.preview.slice(offset, offset + want), rows: p.rows, changed: p.changes.length, left: p.left, more: false, homoglyph_cells };
  }

  /** The browser stand-in's X-ray: every cell of the shown data rows through the same character table. */
  async xrayReport(doc: number, cols: number[], keepFirst: boolean): Promise<import("./backend").XrayReport> {
    const d = this.get(doc);
    const use = cols.length ? cols : Array.from({ length: d.info.cols }, (_, c) => c);
    const kinds = new Map<string, number>();
    const perCol = new Map<number, { cells: number; kinds: Map<string, number> }>();
    let cells = 0;
    const rows = new Set<number>();
    for (let r = keepFirst ? 1 : 0; r < this.rowCount(d); r++) {
      for (const c of use) {
        const found = scanGremlins(this.value(d, r, c));
        if (!found.length) continue;
        cells++;
        rows.add(r);
        const mine = perCol.get(c) ?? { cells: 0, kinds: new Map<string, number>() };
        mine.cells++;
        for (const k of new Set(found.map((g) => g.kind))) {
          kinds.set(k, (kinds.get(k) ?? 0) + 1);
          mine.kinds.set(k, (mine.kinds.get(k) ?? 0) + 1);
        }
        perCol.set(c, mine);
      }
    }
    return {
      looked: (this.rowCount(d) - (keepFirst ? 1 : 0)) * use.length,
      cells,
      rows: rows.size,
      kinds: [...kinds.entries()].sort((a, b) => b[1] - a[1]),
      columns: [...perCol.entries()].map(([col, m]) => ({ col, cells: m.cells, kinds: [...m.kinds.entries()] as [string, number][] })).sort((a, b) => b.cells - a.cells),
    };
  }
  // ---- Encoding (stand-ins: a made-up file whose rows read differently in each encoding) ----
  private static readonly ENCODINGS: import("./backend").EncodingChoice[] = [
    { label: "utf-8", name: "UTF-8", direct: true },
    { label: "windows-1252", name: "Windows-1252 / Latin-1", direct: true },
    { label: "utf-16le", name: "UTF-16 LE", direct: false },
    { label: "utf-16be", name: "UTF-16 BE", direct: false },
    { label: "utf-32le", name: "UTF-32 LE", direct: false },
    { label: "utf-32be", name: "UTF-32 BE", direct: false },
    { label: "shift_jis", name: "Shift_JIS", direct: false },
    { label: "gbk", name: "GBK", direct: false },
    { label: "big5", name: "Big5", direct: false },
  ];
  /** The name the engine gives an encoding (what the status bar shows). */
  private static realName(label: string): string {
    const m: Record<string, string> = { "utf-8": "UTF-8", "utf-16le": "UTF-16LE", "utf-16be": "UTF-16BE", "utf-32le": "UTF-32LE", "utf-32be": "UTF-32BE", shift_jis: "Shift_JIS", gbk: "GBK", big5: "Big5" };
    return m[label] ?? label;
  }
  private encodingSteps = new WeakSet<object>();
  async encodingChoices(): Promise<import("./backend").EncodingChoice[]> {
    return MockBackend.ENCODINGS.map((c) => ({ ...c }));
  }
  async encodingPreview(doc: number, encoding: string, rows: number): Promise<string[][]> {
    this.get(doc);
    const sample: Record<string, string[][]> = {
      "utf-8": [["name", "city"], ["Jos\u00e9", "Z\u00fcrich"], ["Ren\u00e9e", "Li\u00e8ge"]],
      "windows-1252": [["name", "city"], ["Jos\u00c3\u00a9", "Z\u00c3\u00bcrich"], ["Ren\u00c3\u00a9e", "Li\u00c3\u00a8ge"]],
    };
    const known = MockBackend.ENCODINGS.some((c) => c.label === encoding);
    if (!known) throw err("invalid", "that encoding is not known");
    return (sample[encoding] ?? [["name", "city"], ["Jos\ufffd\ufffd", "Z\ufffdrich"], ["\ufffd\ufffd", "Li\ufffdge"]]).slice(0, rows);
  }
  async encodingApply(doc: number, encoding: string): Promise<DocInfo> {
    const d = this.get(doc);
    const to = MockBackend.ENCODINGS.find((c) => c.label === encoding);
    if (!to) throw err("invalid", "that encoding is not known");
    if (!to.direct) throw err("invalid", `${to.name} cannot be read in place: reopen the file with it instead`);
    if (d.undo.some((s) => !this.encodingSteps.has(s.apply))) throw err("invalid", "Save or undo your changes first: they were made while the file was read as another encoding");
    if (d.info.peeking) throw err("peeking", "looking at an earlier recipe step: go back to the latest step to change or save");
    if (d.info.encoding === MockBackend.realName(to.label)) return { ...d.info };
    const apply = () => {
      const old = d.info.encoding;
      d.info.encoding = MockBackend.realName(to.label);
      return () => {
        d.info.encoding = old;
      };
    };
    this.encodingSteps.add(apply);
    return this.push(d, apply);
  }
  async reopenEncoding(doc: number, encoding: string, lineByLine: boolean, layout: Layout | null = null): Promise<DocInfo> {
    const d = this.get(doc);
    if (d.info.modified) throw err("modified", "Save or undo your changes first: reopening reads the file again from disk.");
    const to = MockBackend.ENCODINGS.find((c) => c.label === encoding);
    if (!to) throw err("invalid", "that encoding is not known");
    const info = await this.reopen(doc, lineByLine, layout);
    this.get(info.id).info.encoding = MockBackend.realName(to.label);
    return { ...this.get(info.id).info };
  }
  async scanEncodings(doc: number, _keepFirst: boolean): Promise<import("./backend").EncodingScan> {
    const d = this.get(doc);
    const looked = Math.max(0, d.info.rows - 1);
    return { looked, ascii: Math.max(0, looked - 3), utf8: 1, invalid: 1, double_encoded: 1, mixed: true, rows: 2 };
  }
  async encodingShowRows(doc: number, _keepFirst: boolean): Promise<DocInfo> {
    return { ...this.get(doc).info };
  }

  async xrayShow(doc: number): Promise<DocInfo> {
    return { ...this.get(doc).info };
  }

  private savedCleaner: Cleaner | null = null;

  async cleanerSave(cleaner: Cleaner): Promise<string | null> {
    this.savedCleaner = JSON.parse(JSON.stringify(cleaner)) as Cleaner;
    return `${cleaner.name || "cleaner"}.ndclean`;
  }

  // ---- Milestone 7 (simple stand-ins) ----

  private savedView: View | null = null;

  /** Made-up recent files: two share a name and a folder name, as
   *  2024\Data\voters.csv and 2025\Data\voters.csv would. */
  private recent: [string, string][] = [
    ["r1", "C:/samples/sample-1M.csv"],
    ["r2", "C:/Elections/2025/Data/voters.csv"],
    ["r3", "C:/Elections/2024/Data/voters.csv"],
    ["r4", "C:/precincts.csv"],
  ];
  private openAt(path: string): number | null {
    for (const d of this.docs.values()) if (d.info.path === path) return d.info.id;
    return null;
  }

  // Pick up where you left off: kept in the stub's memory (by path).
  private lastTabs: { path: string; view: unknown; row: number; col: number; active: boolean }[] = [];
  async sessionSave(tabs: SessionTab[]): Promise<number> {
    this.calls.push("sessionSave");
    this.lastTabs = tabs.flatMap((t) => {
      const p = this.docs.get(t.doc)?.info.path;
      return p ? [{ path: p, view: t.view, row: t.row, col: t.col, active: t.active }] : [];
    });
    return this.lastTabs.length;
  }
  async sessionList(): Promise<SessionItem[]> {
    this.calls.push("sessionList");
    return this.lastTabs.map((s, index) => ({ index, name: s.path.split("/").at(-1)!, folder: "C:", changed: false, missing: false, open: this.openAt(s.path) !== null, active: s.active }));
  }
  /** Tests: last time's tabs, and a file Windows passes while the
   *  `n`th of them reopens. */
  seedSession(paths: string[], active: number, launchDuring: { n: number; path: string } | null = null): void {
    this.lastTabs = paths.map((path, i) => ({ path, view: null, row: 0, col: 0, active: i === active }));
    this.launchDuring = launchDuring;
    this.reopened = 0;
  }
  private launchDuring: { n: number; path: string } | null = null;
  private reopened = 0;
  async sessionOpen(index: number): Promise<{ opened: Opened; place: SessionPlace }> {
    if (this.launchDuring && ++this.reopened === this.launchDuring.n) {
      const path = this.launchDuring.path;
      this.launchDuring = null;
      this.osOpen(path);
    }
    const s = this.lastTabs[index];
    if (!s) throw err("invalid", "that file is no longer in the list");
    const open = this.openAt(s.path);
    let opened: Opened;
    if (open !== null) opened = { doc: { ...this.get(open).info }, import: null };
    else {
      const info = this.make(s.path.split("/").at(-1)!, 2_001, 6);
      this.get(info.id).info.path = s.path;
      opened = { doc: { ...this.get(info.id).info }, import: null };
    }
    return { opened, place: { view: s.view, row: s.row, col: s.col, changed: false } };
  }
  async tidyTitlesPreview(doc: number, spec: TitleSpec): Promise<[number, string, string][]> {
    const d = this.get(doc);
    const titles = Array.from({ length: this.colCount(d) }, (_, c) => this.value(d, 0, c));
    const after = tidyTitleList(titles, spec);
    return titles.flatMap((t, c) => (t !== after[c] ? [[c, t, after[c]!] as [number, string, string]] : []));
  }
  async tidyTitles(doc: number, spec: TitleSpec): Promise<DocInfo> {
    const plan = await this.tidyTitlesPreview(doc, spec);
    if (!plan.length) return { ...this.get(doc).info };
    return this.writeCells(doc, plan.map(([c, , v]) => [0, c, v]), false);
  }
  async setProtected(doc: number, cols: number[]): Promise<DocInfo> {
    const d = this.get(doc);
    d.protectedPhys = new Set(cols.filter((c) => c < this.colCount(d)).map((c) => this.physCol(d, c)));
    d.info.protected = [...new Set(cols)].filter((c) => c < this.colCount(d)).sort((a, b) => a - b);
    return { ...d.info };
  }
  async zipOpen(_token: number, names: string[]): Promise<DocInfo[]> {
    return names.map((n) => {
      const info = this.make(`export.zip › ${n.split("/").at(-1)}`, 120, 4);
      return { ...this.get(info.id).info };
    });
  }
  async sessionClear(): Promise<void> {
    this.lastTabs = [];
  }

  async recentFiles(): Promise<RecentItem[]> {
    // As the backend does: a folder's own name, or two levels where two
    // files of the same name need telling apart.
    const name = (p: string) => p.split("/").at(-1)!;
    return this.recent.map(([id, p]) => {
      const parts = p.split("/").slice(1, -1);
      const twin = this.recent.some(([j, q]) => j !== id && name(q) === name(p));
      return { id, name: name(p), folder: (parts.length ? parts : ["C:"]).slice(twin ? -2 : -1).join("\\"), open_doc: this.openAt(p) };
    });
  }

  // The stub has no hidden first row: shown row 0 is the first row.
  async firstRowCell(doc: number, col: number): Promise<string> {
    return (await this.cellText(doc, 0, col)).text;
  }
  async firstRowNames(doc: number): Promise<string[]> {
    const d = this.get(doc);
    const v = await this.viewport(doc, 0, 1, 0, Math.min(d.info.cols, 20000), 200);
    return v.cells.slice(0, v.cols);
  }
  async setFirstRowCell(doc: number, col: number, value: string): Promise<DocInfo> {
    return this.writeCells(doc, [[0, col, value]], false);
  }
  async columnNamesLine(doc: number, cols: number[], sep: string): Promise<string> {
    const d = this.get(doc);
    const v = await this.viewport(doc, 0, 1, 0, this.colCount(d), 100_000);
    const q = (s: string) => (s.includes(sep) || /["\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
    return cols.map((c) => q(c < v.cols ? (v.cells[c] ?? "") : "")).join(sep);
  }
  async setColumnNames(doc: number, names: [number, string][]): Promise<DocInfo> {
    return this.writeCells(doc, names.map(([c, v]) => [0, c, v] as [number, number, string]), false);
  }

  async tabFolders(ids: number[]): Promise<string[]> {
    const paths = ids.map((id) => this.docs.get(id)?.info.path ?? null);
    const name = (p: string) => p.split("/").at(-1)!.toLowerCase();
    return paths.map((p) => (p && paths.filter((q) => q && name(q) === name(p)).length > 1 ? p.split("/").slice(-3, -1).join("\\") : ""));
  }

  async openRecent(id: string): Promise<Opened> {
    const p = this.recent.find(([i]) => i === id)?.[1];
    if (!p) throw err("bad_request", "that recent file is no longer listed");
    const open = this.openAt(p);
    if (open !== null) return { doc: { ...this.get(open).info }, import: null };
    if (p.endsWith("sample-1M.csv")) return this.openSample();
    const info = this.make(p.split("/").at(-1)!, 2_001, 6);
    this.get(info.id).info.path = p;
    return { doc: { ...this.get(info.id).info }, import: null };
  }

  private async openSample(): Promise<Opened> {
    const o = await this.openFile();
    if (!o) throw err("error", "nothing to open");
    return o;
  }

  async folderList(doc: number): Promise<Listing> {
    void doc;
    return { folder: "samples", items: [{ index: 0, name: "sample-1M.csv", size: 120_000_000, table: true }, { index: 1, name: "notes.txt", size: 2_048, table: true }], more: false };
  }

  async folderOpen(doc: number, index: number): Promise<Opened> {
    void doc;
    void index;
    return this.openSample();
  }

  async viewSave(view: View): Promise<string | null> {
    this.savedView = JSON.parse(JSON.stringify(view)) as View;
    return `${view.name || "view"}.ndview`;
  }

  async viewOpen(): Promise<View | null> {
    return this.savedView;
  }

  async setReadOnly(doc: number, on: boolean): Promise<DocInfo> {
    const d = this.get(doc);
    d.info.read_only = on;
    return this.refresh(d);
  }

  private kindOf(v: string): keyof Summary["mix"] {
    const t = v.trim();
    if (!t) return "blank";
    if (numberOf(t) !== null) return "number";
    if (/^(yes|no|true|false|y|n)$/i.test(t)) return "yes_no";
    if (!Number.isNaN(Date.parse(t))) return "date";
    return "text";
  }

  async columnTypes(doc: number, skipFirst: boolean): Promise<ColType[]> {
    const d = this.get(doc);
    const rows = Math.min(this.rowCount(d), 500);
    return Array.from({ length: this.colCount(d) }, (_, c) => {
      const mix = { blank: 0, number: 0, date: 0, yes_no: 0, text: 0 };
      for (let r = skipFirst ? 1 : 0; r < rows; r++) mix[this.kindOf(this.value(d, r, c))]++;
      const filled = mix.number + mix.date + mix.yes_no + mix.text;
      if (!filled) return "empty";
      const k = (["number", "date", "yes_no", "text"] as const).find((x) => mix[x] * 100 >= filled * 95);
      return k ?? "mixed";
    });
  }

  // Browser preview: no disk, so no recovery copies or earlier saves.
  async autosave(): Promise<Autosaved> {
    return { kind: "off" };
  }
  async recoveryList(): Promise<RecoveryItem[]> {
    this.calls.push("recoveryList");
    return [];
  }
  async recoveryOpen(): Promise<DocInfo> {
    throw err("bad_request", "list the recovery copies again");
  }
  async recoveryDiscard(): Promise<void> {}
  async versionsKeep(): Promise<void> {
    this.calls.push("versionsKeep");
  }
  async versionsClear(): Promise<number> {
    return 0;
  }
  async versionsList(): Promise<VersionItem[]> {
    return [];
  }
  async versionOpen(): Promise<DocInfo> {
    throw err("bad_request", "list the earlier saves again");
  }

  async cleanSuggestions(doc: number, skipFirst: boolean): Promise<Suggestions> {
    const d = this.get(doc);
    // The browser stub looks at the first 20,000 rows for spaces, repeated
    // and blank rows (the engine also checks numbers and dates).
    const rows = Math.min(this.rowCount(d), 20_000);
    const cols = Math.min(this.colCount(d), 200);
    const spaced = new Map<number, { n: number; ex: string[]; double: boolean }>();
    const seen = new Set<string>();
    let dupes = 0;
    let blank = 0;
    for (let r = skipFirst ? 1 : 0; r < rows; r++) {
      const vals: string[] = [];
      for (let c = 0; c < cols; c++) {
        const v = this.value(d, r, c);
        vals.push(v);
        if (v.trim() && (v.trim() !== v || v.includes("  "))) {
          const s = spaced.get(c) ?? { n: 0, ex: [], double: false };
          s.n++;
          s.double ||= v.includes("  ");
          if (s.ex.length < 3) s.ex.push(v);
          spaced.set(c, s);
        }
      }
      if (vals.every((v) => !v.trim())) blank++;
      else {
        const k = vals.join("\u0000");
        if (seen.has(k)) dupes++;
        else seen.add(k);
      }
    }
    const items: Suggestions["items"] = [...spaced].map(([col, s]) => ({
      kind: "spaces" as const,
      col,
      count: s.n,
      examples: s.ex,
      fix: { fix: "clean" as const, cols: [col], step: { tool: "spaces" as const, how: s.double ? ("collapse" as const) : ("trim" as const) } },
    }));
    if (dupes) items.push({ kind: "duplicate_rows", col: null, count: dupes, examples: [], fix: { fix: "remove_duplicates" } });
    if (blank) items.push({ kind: "blank_rows", col: null, count: blank, examples: [], fix: { fix: "remove_blank_rows" } });
    return { items, rows: rows - (skipFirst ? 1 : 0), partial: rows < this.rowCount(d) };
  }

  async openLink(url: string): Promise<void> {
    // Browser preview: a new tab stands in for the system's browser.
    window.open(url, "_blank", "noopener,noreferrer");
  }

  async selectionTotals(doc: number, row: number, rows: number, col: number, cols: number): Promise<Totals> {
    const d = this.get(doc);
    const t: Totals = { cells: rows * cols, filled: 0, numbers: 0, sum: null, average: null, min: null, max: null, too_large: false, chars: 0, distinct: null, distinct_exact: true };
    const seen = new Set<string>();
    // The browser stub reads at most 200,000 cells (the engine reads all).
    let budget = 200_000;
    let sum = 0;
    for (let r = row; r < row + rows && budget > 0; r++) {
      for (let c = col; c < col + cols && budget > 0; c++, budget--) {
        const raw = this.value(d, r, c);
        t.chars += [...raw].length;
        const v = raw.trim();
        if (!v) continue;
        seen.add(v);
        t.filled++;
        // A column typed Text or Id is never read as numbers (the engine does the same).
        const x = readNumberFor(d.info.typed, c, v) === null ? null : numberOf(v);
        if (x === null) continue;
        t.numbers++;
        sum += x;
        t.min = Math.min(t.min ?? x, x);
        t.max = Math.max(t.max ?? x, x);
      }
    }
    t.distinct = seen.size;
    if (t.numbers && !Number.isFinite(sum)) t.too_large = true;
    else if (t.numbers) {
      t.sum = sum;
      t.average = sum / t.numbers;
    }
    return t;
  }

  async selectionTotalsAreas(doc: number, areas: Rect[]): Promise<Totals> {
    const d = this.get(doc);
    const parts = disjoint(areas);
    const t: Totals = { cells: 0, filled: 0, numbers: 0, sum: null, average: null, min: null, max: null, too_large: false, chars: 0, distinct: null, distinct_exact: true };
    let sum = 0;
    let budget = 200_000;
    for (const a of parts) {
      t.cells += a.rows * a.cols;
      for (let r = a.row; r < a.row + a.rows && budget > 0; r++) {
        for (let c = a.col; c < a.col + a.cols && budget > 0; c++, budget--) {
          const raw = this.value(d, r, c);
          t.chars += [...raw].length;
          const v = raw.trim();
          if (!v) continue;
          t.filled++;
          const x = numberOf(v);
          if (x === null) continue;
          t.numbers++;
          sum += x;
          t.min = Math.min(t.min ?? x, x);
          t.max = Math.max(t.max ?? x, x);
        }
      }
    }
    if (t.numbers) {
      t.sum = sum;
      t.average = sum / t.numbers;
    }
    return t;
  }

  async columnSummary(doc: number, col: number, skipFirst: boolean): Promise<Summary> {
    const d = this.get(doc);
    const rows = Math.min(this.rowCount(d), 100_000);
    const mix = { blank: 0, number: 0, date: 0, yes_no: 0, text: 0 };
    const counts = new Map<string, number>();
    let sum = 0;
    let n = 0;
    let min = Infinity;
    let max = -Infinity;
    for (let r = skipFirst ? 1 : 0; r < rows; r++) {
      const v = this.value(d, r, col);
      mix[this.kindOf(v)]++;
      counts.set(v, (counts.get(v) ?? 0) + 1);
      const x = numberOf(v);
      if (x !== null) {
        sum += x;
        n++;
        min = Math.min(min, x);
        max = Math.max(max, x);
      }
    }
    const top = [...counts].sort((a, b) => b[1] - a[1]).slice(0, 20).map(([value, count]) => ({ value, cut: false, count }));
    const kinds = (await this.columnTypes(doc, skipFirst))[col] ?? "empty";
    return { col, rows: rows - (skipFirst ? 1 : 0), mix, kind: kinds, distinct: counts.size, distinct_capped: false, top_partial: false, sum: n ? sum : null, mean: n ? sum / n : null, min: n ? min : null, max: n ? max : null, min_len: 0, max_len: 0, top };
  }

  async chartData(doc: number, spec: ChartSpec): Promise<ChartData> {
    const d = this.get(doc);
    const rows = Math.min(this.rowCount(d), 2_000);
    const from = spec.skip_first ? 1 : 0;
    if (spec.kind === "count" || spec.kind === "pie") {
      const m = new Map<string, number>();
      for (let r = from; r < rows; r++) {
        const k = this.value(d, r, spec.label ?? 0);
        const v = spec.kind === "count" ? 1 : (numberOf(this.value(d, r, spec.values[0] ?? 0)) ?? 0);
        m.set(k, (m.get(k) ?? 0) + v);
      }
      const list = [...m].sort((a, b) => b[1] - a[1]).slice(0, 50);
      return { labels: list.map((x) => x[0]), series: [list.map((x) => x[1])], per_point: 1, rows: rows - from, other: m.size > 50 };
    }
    const labels: string[] = [];
    const series = spec.values.map(() => [] as (number | null)[]);
    for (let r = from; r < rows; r++) {
      labels.push(spec.label === null ? String(r + 1) : this.value(d, r, spec.label));
      spec.values.forEach((c, k) => series[k]!.push(numberOf(this.value(d, r, c))));
    }
    return { labels, series, per_point: 1, rows: rows - from, other: false };
  }

  async highlights(doc: number, row: number, rows: number, col: number, cols: number, rules: Highlight[], skipFirst: boolean): Promise<Mark[]> {
    const d = this.get(doc);
    const out: Mark[] = [];
    const end = Math.min(this.rowCount(d), row + rows);
    const test = (h: Highlight, v: string): boolean => {
      if (h.rule.type === "values") return v === "" ? !!h.rule.blanks : h.rule.values.includes(v);
      const c = h.rule.first;
      if (c.type === "blank") return v === "";
      if (c.type === "not_blank") return v !== "";
      if (c.type === "number") {
        const x = numberOf(v);
        if (x === null) return false;
        return { eq: x === c.value, ne: x !== c.value, gt: x > c.value, ge: x >= c.value, lt: x < c.value, le: x <= c.value }[c.op];
      }
      if (c.type === "text") return v.toLowerCase().includes(c.text.toLowerCase());
      return false;
    };
    for (let r = Math.max(row, skipFirst ? 1 : 0); r < end; r++) {
      let rowDone = false;
      const done = new Set<number>();
      for (const h of rules) {
        const v = this.value(d, r, h.col);
        if (h.whole_row) {
          if (!rowDone && test(h, v)) {
            out.push({ row: r, col: null, color: h.color });
            rowDone = true;
          }
        } else if (h.col >= col && h.col < col + cols && !done.has(h.col) && test(h, v)) {
          out.push({ row: r, col: h.col, color: h.color });
          done.add(h.col);
        }
      }
    }
    return out;
  }

  /** Comparisons of lists: the stand-in compares trimmed, small-letter
   *  texts (no loose keys). */
  private compared = new Map<number, { tabs: number[]; at: string; spec: SetsSpec; run: SetsRun; masks: number[][] }>();
  private comparisons = 0;

  /** What a table holds now, to tell whether it changed. */
  private stateOf(tabs: number[]): string {
    return tabs.map((id) => JSON.stringify([this.get(id).info.data_revision ?? 0, this.rowCount(this.get(id))])).join();
  }

  async setsRun(_owner: number, tabs: number[], spec: SetsSpec): Promise<SetsRun> {
    const keys = tabs.map((id, i) => {
      const d = this.get(id);
      const l = spec.lists[i]!;
      if (this.rowCount(d) > 200_000) throw err("too_large", "the browser preview compares at most 200,000 rows");
      const out: (string | null)[] = [];
      for (let r = l.header ? 1 : 0; r < this.rowCount(d); r++) {
        const parts = l.cols.map((c) => this.value(d, r, c));
        if (parts.every((p) => !p.trim())) out.push(null);
        else out.push(JSON.stringify(parts.map((p) => (spec.trim ? p.trim() : p)).map((p) => (spec.match_case ? p : p.toLowerCase()))));
      }
      return out;
    });
    const mask = new Map<string, number>();
    keys.forEach((list, i) => list.forEach((k) => k !== null && mask.set(k, (mask.get(k) ?? 0) | (1 << i))));
    const masks = keys.map((list) => list.map((k) => (k === null ? 0 : mask.get(k)!)));
    const slices = Array.from({ length: (1 << tabs.length) - 1 }, (_, j) => ({ mask: j + 1, keys: [...mask.values()].filter((m) => m === j + 1).length, rows: masks.map((l) => l.filter((m) => m === j + 1).length) }));
    const id = ++this.comparisons;
    const stems = tabs.map((t) => this.get(t).info.name.replace(/\.[A-Za-z0-9]{1,5}$/, ""));
    const names = stems.map((n, i) => (stems.filter((x) => x === n).length > 1 ? `${n} [list ${String.fromCharCode(65 + i)}]` : n));
    const run: SetsRun = { id, names, report: { slices, lists: masks.map((l, i) => ({ rows: l.length, keys: new Set(keys[i]!.filter((k) => k !== null)).size, blank: l.filter((m) => !m).length })) } };
    this.compared.set(id, { tabs, at: this.stateOf(tabs), spec, run, masks });
    return JSON.parse(JSON.stringify(run)) as SetsRun;
  }

  private kept(id: number) {
    const c = this.compared.get(id);
    if (!c || c.tabs.some((t) => !this.docs.has(t)) || c.at !== this.stateOf(c.tabs)) throw err("changed", "a list changed since it was compared: run the comparison again");
    return c;
  }

  private setsDoc(title: string, out: string[][]): DocInfo {
    const info = this.make("Untitled", out.length, out[0]!.length);
    const nd = this.get(info.id);
    out.forEach((line, r) => line.forEach((v, c) => nd.edits.set(`${r}:${c}`, v)));
    nd.clean = -1;
    nd.info.name = title;
    return this.refresh(nd);
  }

  async setsOpen(id: number, mask: number, list: number, keys: boolean): Promise<DocInfo> {
    const c = this.kept(id);
    const d = this.get(c.tabs[list]!);
    const l = c.spec.lists[list]!;
    const cols = keys ? l.cols : Array.from({ length: this.colCount(d) }, (_, i) => i);
    const first = l.header ? 1 : 0;
    const rows = [...(first ? [0] : []), ...c.masks[list]!.flatMap((m, i) => (m === mask ? [i + first] : []))];
    let out = rows.map((r) => cols.map((col) => this.value(d, r, col)));
    if (keys) out = out.filter((line, i) => out.findIndex((x) => JSON.stringify(x).toLowerCase() === JSON.stringify(line).toLowerCase()) === i);
    return this.setsDoc(`${sliceName(mask, c.run.names)} (compared)`, out);
  }

  async setsTable(id: number): Promise<DocInfo> {
    const c = this.kept(id);
    return this.setsDoc("Compared lists", countsText(c.run, false).trimEnd().split("\n").slice(0, 1 + c.run.report.slices.filter((s) => s.keys).length).map((x) => x.split("\t")));
  }

  async setsClose(id: number): Promise<void> {
    this.compared.delete(id);
  }

  /** The browser preview's relationship scan: dependencies only, over every
   *  row (the desktop app does one to one, blanks and sampling). */
  private relations(d: ReturnType<MockBackend["get"]>, spec: RelationsSpec): { report: RelationsReport; odd: Map<string, number[]> } {
    const first = spec.header ? 1 : 0;
    const rows = this.rowCount(d) - first;
    if (rows > 100_000) throw err("too_large", "the browser preview scans at most 100,000 rows");
    const data = spec.cols.map((c) => Array.from({ length: rows }, (_, r) => this.value(d, r + first, c).trim()));
    const report: RelationsReport = { findings: [], left_out: [], mostly_one: [], rows, sampled: rows, unchecked: 0 };
    const odd = new Map<string, number[]>();
    spec.cols.forEach((a, i) => {
      spec.cols.forEach((b, j) => {
        if (i === j) return;
        const groups = new Map<string, number[]>();
        data[i]!.forEach((v, r) => v && data[j]![r] && (groups.get(v) ?? groups.set(v, []).get(v)!).push(r));
        if (groups.size < 2) return;
        let coverage = 0;
        let agreeing = 0;
        let values = 0;
        let broken = 0;
        const out: number[] = [];
        for (const g of groups.values()) {
          if (g.length < 2) continue;
          const n = new Map<string, number>();
          for (const r of g) n.set(data[j]![r]!, (n.get(data[j]![r]!) ?? 0) + 1);
          const [best, most] = [...n].sort((x, y) => y[1] - x[1])[0]!;
          coverage += g.length;
          agreeing += most;
          values++;
          if (most < g.length) broken++;
          out.push(...g.filter((r) => data[j]![r] !== best).map((r) => r + first));
        }
        const percent = coverage ? (agreeing * 100) / coverage : 0;
        if (percent < spec.min_percent || new Set(data[j]!).size < 2 || new Set(data[j]!).size > groups.size) return;
        report.findings.push({ kind: "depends", a, b, there: { coverage, agreeing, values, broken_values: broken, percent }, back: null, exact: true, few_rows: false, few_per_value: false });
        odd.set(`${a}:${b}`, out);
      });
    });
    return { report, odd };
  }

  private lastRelations = new Map<number, ReturnType<MockBackend["relations"]> & { at: number }>();

  async findRelations(doc: number, spec: RelationsSpec): Promise<RelationsReport> {
    const d = this.get(doc);
    const found = this.relations(d, spec);
    this.lastRelations.set(doc, { ...found, at: d.info.data_revision ?? 0 });
    return found.report;
  }

  async cleanerOpenNamed(): Promise<NamedCleaner | null> {
    const cleaner = await this.cleanerOpen();
    return cleaner && { cleaner, file: "tidy-names.ndclean" };
  }

  async cleanMatrixPreview(doc: number, spec: CleanMatrix): Promise<MatrixPreview> {
    return this.matrixRun(this.get(doc), spec).preview;
  }

  private matrixRun(d: ReturnType<MockBackend["get"]>, spec: CleanMatrix): { preview: MatrixPreview; cells: [number, number, string][] } {
    const rows = this.rowCount(d);
    if (rows > 100_000) throw err("too_large", "the browser preview cleans at most 100,000 rows");
    const protectedCols = d.info.protected ?? [];
    const preview: MatrixPreview = { revision: d.info.data_revision ?? 0, columns: [], examples: [] };
    const cells: [number, number, string][] = [];
    for (const e of [...spec.entries].sort((a, b) => a.col - b.col)) {
      const column = { col: e.col, changed: 0, protected: 0, left: 0, other_order: 0, homoglyph_cells: 0 };
      let shown = 0;
      for (let r = spec.skip_first ? 1 : 0; r < rows; r++) {
        const before = this.value(d, r, e.col);
        const after = runCleaner({ version: 1, name: "", steps: e.steps }, before, r);
        if (after === before) continue;
        if (protectedCols.includes(e.col)) column.protected++;
        else {
          column.changed++;
          cells.push([r, e.col, after]);
        }
        if (!protectedCols.includes(e.col) && shown++ < 3) preview.examples.push({ row: r, number: r, col: e.col, before, after });
      }
      preview.columns.push(column);
    }
    return { preview, cells };
  }

  async cleanerOpen(): Promise<Cleaner | null> {
    return this.savedCleaner ?? { version: 1, name: "Tidy names", steps: [{ tool: "spaces", how: "collapse" }, { tool: "case", to: "title" }] };
  }

  async dataOp(doc: number, op: DataOp): Promise<OpReport> {
    const d = this.get(doc);
    const rows = this.rowCount(d);
    const allCols = Array.from({ length: this.colCount(d) }, (_, i) => i);
    switch (op.op) {
      case "clean": {
        const first = op.spec.cleaner.steps[0];
        const p = this.planClean(d, op.spec, first?.tool === "dates" && first.review ? 1_000_000_000 : 100_000);
        if (p.more) throw err("too_large", "the browser preview cleans at most 100,000 rows");
        const changes = [...p.changes.filter(([r, c]) => !op.typed?.some((t) => t[0] === r && t[1] === c)), ...(op.typed ?? [])];
        const info = changes.length ? await this.setCells(doc, changes) : this.refresh(d);
        return this.report(info, { cells: changes.length, left: p.left });
      }
      case "compute": {
        const names = allCols.map((c) => (op.spec.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
        const first = op.spec.keep_first ? 1 : 0;
        if (rows > 100_000) throw err("too_large", "the browser preview computes at most 100,000 rows");
        const vals: [number, string][] = [];
        let errors = 0;
        for (let r = first; r < rows; r++) {
          const v = mockEval(op.spec.expr, names, allCols.map((c) => this.value(d, r, c)));
          if (v.startsWith("#")) {
            errors++;
            // Changing a column, an error keeps the cell's value.
            if (op.spec.target !== null) continue;
          }
          vals.push([r, v]);
        }
        let col = op.spec.target;
        if (col === null) {
          col = op.spec.at ?? allCols.length;
          await this.change(doc, { op: "insert_cols", at: col, count: 1 });
        }
        const cells: [number, number, string][] = vals.map(([r, v]) => [r, col, v]);
        if (op.spec.target === null && first) cells.push([0, col, op.spec.title || "Computed"]);
        const info = await this.setCells(doc, cells);
        return this.report(info, { cells: cells.length, cols: op.spec.target === null ? 1 : 0, errors });
      }
      case "show_shape": {
        // The rows shown now whose value has (or has not) the shape; blanks only when asked.
        const s = op.spec;
        if (s.revision !== undefined && s.revision !== (d.info.data_revision ?? 0)) throw err("changed", "the table changed since the pattern was worked out; try again");
        const keep = new Set<number>();
        let shown = 0;
        for (let r = s.keep_first ? 1 : 0; r < rows; r++) {
          const v = this.value(d, r, s.col);
          const show = !v.trim() ? s.include_blanks : shapeMatches(v, s.shape) !== s.negate;
          if (show) {
            keep.add(this.rowId(d, r));
            shown++;
          }
        }
        if (s.keep_first && rows > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: s.keep_first }), { rows: shown });
      }
      case "clean_matrix": {
        if (op.revision !== undefined && op.revision !== (d.info.data_revision ?? 0)) throw err("invalid", "the table changed: preview again");
        const found = this.matrixRun(d, op.spec);
        const info = found.cells.length ? await this.setCells(doc, found.cells) : this.refresh(d);
        return this.report(info, { cells: found.cells.length, left: 0 });
      }
      case "show_relation_exceptions": {
        const last = this.lastRelations.get(doc);
        const found = last?.at === (d.info.data_revision ?? 0) ? last.odd.get(`${op.a}:${op.b}`) : undefined;
        if (!found) throw err("invalid", "the table changed since the scan: scan again");
        const keep = new Set<number>(found.map((r) => this.rowId(d, r)));
        if (rows > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: true }), { rows: found.length });
      }
      case "show_gap_rows": {
        const found = this.gaps(doc, op.spec).beside;
        const keep = new Set<number>(found.map((r) => this.rowId(d, r)));
        if (op.spec.keep_first && rows > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: op.spec.keep_first }), { rows: found.length });
      }
      case "show_ragged": {
        // The stand-in has no file rows of other widths: tests name them.
        if (d.info.column_changes) throw err("invalid", COLUMNS_CHANGED);
        const marks = this.fakeRagged.get(doc) ?? { short: [], long: [] };
        const pick = op.pick ?? "all";
        const ids = [...(pick === "long" ? [] : marks.short), ...(pick === "short" ? [] : marks.long)];
        const keep = new Set<number>(ids.map((r) => this.rowId(d, r)));
        const shown = keep.size;
        if (op.keep_first && rows > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: op.keep_first }), { rows: shown, groups: pick === "long" ? 0 : marks.short.length });
      }
      case "show_problems":
      case "sort_problems":
      case "check_column":
      case "split_name":
      case "fill_example":
      case "count_repeats":
      case "blank_repeats":
      case "breaks":
      case "delete_hidden":
      case "arrange":
        throw err("invalid", "the browser preview does not do this; the desktop app does");
      case "map_values": {
        const s = op.spec;
        const pairs = [...s.pairs];
        if (op.from) {
          const o = this.get(op.from.doc);
          for (let r = op.from.header ? 1 : 0; r < this.rowCount(o); r++) pairs.push([this.value(o, r, op.from.old), this.value(o, r, op.from.new)]);
        }
        const norm = (v: string) => {
          const t = s.trim ? v.trim() : v;
          return s.match_case ? t : t.toLowerCase();
        };
        const map = new Map<string, string>();
        for (const [a, b] of pairs) if (!map.has(norm(a))) map.set(norm(a), b);
        if (!map.size) throw err("invalid", "the list has no values");
        const cells: [number, number, string][] = [];
        for (let r = s.keep_first ? 1 : 0; r < Math.min(rows, 100_000); r++)
          for (const c of s.cols) {
            const v = this.value(d, r, c);
            const n = map.get(norm(v));
            if (n !== undefined && n !== v) cells.push([r, c, n]);
          }
        const info = cells.length ? await this.setCells(doc, cells) : this.refresh(d);
        return this.report(info, { cells: cells.length });
      }
      case "group_calc": {
        const s = op.spec;
        const lim = Math.min(rows, 100_000);
        const start = s.keep_first ? 1 : 0;
        const groups = new Map<string, number[]>();
        for (let r = start; r < lim; r++) {
          const k = JSON.stringify(s.keys.map((c) => (s.match_case ? this.value(d, r, c) : this.value(d, r, c).toLowerCase())));
          let g = groups.get(k);
          if (!g) groups.set(k, (g = []));
          g.push(r);
        }
        const out = new Map<number, string>();
        const num = (r: number) => (s.value === null ? NaN : Number(this.value(d, r, s.value).trim() || "x"));
        for (const rs of groups.values()) {
          const nums = rs.map(num).filter(Number.isFinite);
          const sum = nums.reduce((a, b) => a + b, 0);
          rs.forEach((r, i) => {
            let v = "";
            const x = num(r);
            if (s.func === "count") v = String(rs.length);
            else if (s.func === "running") v = String(i + 1);
            else if (s.func === "sum") v = nums.length ? String(sum) : "";
            else if (s.func === "average") v = nums.length ? String(sum / nums.length) : "";
            else if (s.func === "min") v = nums.length ? String(Math.min(...nums)) : "";
            else if (s.func === "max") v = nums.length ? String(Math.max(...nums)) : "";
            else if (s.func === "first") v = s.value === null ? "" : this.value(d, rs[0]!, s.value);
            else if (s.func === "last") v = s.value === null ? "" : this.value(d, rs[rs.length - 1]!, s.value);
            else if (s.func === "percent") v = Number.isFinite(x) && sum ? String(Number(((x / sum) * 100).toPrecision(15))) : "";
            else if (Number.isFinite(x)) v = String(1 + nums.filter((y) => (s.func === "rank" ? y > x : y < x)).length);
            out.set(r, v);
          });
        }
        const at = s.at ?? allCols.length;
        await this.change(doc, { op: "insert_cols", at, count: 1 });
        const cells: [number, number, string][] = [];
        if (start) cells.push([0, at, s.title || "Group figure"]);
        for (const [r, v] of out) if (v) cells.push([r, at, v]);
        const info = await this.setCells(doc, cells);
        return this.report(info, { cols: 1, cells: cells.length, groups: groups.size });
      }
      case "formula_filter": {
        const s = op.spec;
        const cols = Array.from({ length: this.colCount(d) }, (_, i) => i);
        const names = cols.map((c) => (s.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
        const keep = new Set<number>();
        let errors = 0;
        for (let r = s.keep_first ? 1 : 0; r < this.rowCount(d); r++) {
          let v: string;
          try {
            v = mockEval(s.expr, names, cols.map((c) => this.value(d, r, c)));
          } catch (e) {
            throw err("invalid", e instanceof Error ? e.message : String(e));
          }
          if (v.startsWith("#")) errors++;
          if (v === "TRUE") keep.add(this.rowId(d, r));
        }
        const rows = keep.size;
        if (s.keep_first && this.rowCount(d) > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: s.keep_first }), { rows, errors });
      }
      case "compare_show": {
        const rep = await this.compareCols(doc, op.spec);
        const keep = new Set(rep.rows.map((r) => this.rowId(d, r)));
        if (op.spec.keep_first && this.rowCount(d) > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: op.spec.keep_first }), { rows: rep.rows.length });
      }
      case "compare_column": {
        const s = op.spec;
        const at = Math.max(s.a, s.b) + 1;
        const title = (c: number) => this.value(d, 0, c) || `Column ${c + 1}`;
        const labels = { same: "Same", different: "Different", first: "Only in first", second: "Only in second" } as const;
        const vals: string[] = [];
        for (let r = s.keep_first ? 1 : 0; r < this.rowCount(d); r++) vals.push(labels[compareCells(this.value(d, r, s.a), this.value(d, r, s.b), s)]);
        await this.change(doc, { op: "insert_cols", at, count: 1 });
        const cells: [number, number, string][] = [];
        if (s.keep_first) cells.push([0, at, s.title || `${title(s.a)} vs ${title(s.b)}`]);
        vals.forEach((v, i) => cells.push([i + (s.keep_first ? 1 : 0), at, v]));
        const info = this.writeCells(doc, cells);
        return this.report(info, { cols: 1, cells: cells.length });
      }
      case "fill_special": {
        const found = await this.specialCells(doc, op.spec);
        if (found.total > 5_000_000) throw err("invalid", `${found.total} cells were found; one fill changes at most 5000000. Pick fewer columns or rows.`);
        const info = this.writeCells(doc, found.cells.map(([r, c]) => [r, c, op.value] as [number, number, string]));
        const first = found.cells[0]?.[0] ?? 0;
        const last = found.cells[found.cells.length - 1]?.[0] ?? 0;
        return this.report(info, { cells: found.total, hidden_rows: found.total ? this.hiddenBetween_(d, first, last + 1) : 0 });
      }
      case "fill_rect": {
        const s = op.spec;
        // No size limit here (as the engine): one undo step, any size.
        const cells: [number, number, string][] = [];
        const skip = new Set(s.skip_cols ?? []);
        for (let r = s.row; r < s.row + s.rows; r++) for (let c = s.col; c < s.col + s.cols; c++) if (!skip.has(c)) cells.push([r, c, s.value]);
        const info = this.writeCells(doc, cells);
        return this.report(info, { cells: cells.length, hidden_rows: this.hiddenBetween_(d, s.row, s.row + s.rows) });
      }
      case "shift_cells": {
        // The stub moves cells inside the table only (the desktop app adds
        // rows or columns for what is pushed past the end).
        const s = op.spec;
        if (d.shown !== null || d.hidden !== null) throw err("invalid", "cells can't be shifted while a filter is on or rows are hidden; show every row first");
        const nc = this.colCount(d);
        const cells: [number, number, string][] = [];
        const last = Math.min(rows, s.row + 100_000);
        if (s.shift === "up" || s.shift === "down") {
          for (let c = s.col; c < s.col + s.cols; c++) {
            const old = Array.from({ length: last - s.row }, (_, i) => this.value(d, s.row + i, c));
            for (let i = 0; i < old.length; i++) {
              const from = s.shift === "up" ? i + s.rows : i - s.rows;
              const v = from >= 0 && from < old.length ? old[from]! : "";
              if (v !== old[i]) cells.push([s.row + i, c, v]);
            }
          }
        } else {
          for (let r = s.row; r < s.row + s.rows; r++) {
            const old = Array.from({ length: nc - s.col }, (_, i) => this.value(d, r, s.col + i));
            for (let i = 0; i < old.length; i++) {
              const from = s.shift === "left" ? i + s.cols : i - s.cols;
              const v = from >= 0 && from < old.length ? old[from]! : "";
              if (v !== old[i]) cells.push([r, s.col + i, v]);
            }
          }
        }
        const info = cells.length ? this.writeCells(doc, cells) : this.refresh(d);
        return this.report(info, { cells: cells.length });
      }
      case "extract": {
        const s = op.spec;
        const pats: Record<string, RegExp> = {
          number: /[-+]?\d(?:[\d,]*\d)?(?:\.\d+)?/,
          numbers: /[-+]?\d(?:[\d,]*\d)?(?:\.\d+)?/g,
          email: /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/,
          zip: /\b\d{5}(?:-\d{4})?\b/,
          brackets: /\(([^()]*)\)|\[([^[\]]*)\]/,
          unit: /(?:\b(?:apt|apartment|unit|ste|suite|lot|rm|room|bldg|building|fl|floor)\b\.?|#)\s*#?\s*([A-Za-z]?-?\d[A-Za-z0-9-]*|[A-Za-z]\b)/i,
          phone: /(?:\+?\d{1,3}[\s.-]?)?(?:\(\d{3}\)|\d{3})[\s.-]?\d{3}[\s.-]?\d{4}\b/,
        };
        const tidy = (t: string) => t.split(/\s+/).filter(Boolean).join(" ").replace(/[\s,;-]+$/, "");
        const at = s.col + 1;
        await this.change(doc, { op: "insert_cols", at, count: 1 });
        const cells: [number, number, string][] = [];
        if (s.keep_first && rows > 0) cells.push([0, at, s.title || `${this.value(d, 0, s.col)} part`]);
        for (let r = s.keep_first ? 1 : 0; r < Math.min(rows, 100_000); r++) {
          const v = this.value(d, r, s.col);
          let piece: string | null = null;
          let rest = v;
          if (s.what === "first_word" || s.what === "last_word") {
            const w = v.split(/\s+/).filter(Boolean);
            if (w.length) {
              piece = s.what === "first_word" ? w[0]! : w[w.length - 1]!;
              rest = (s.what === "first_word" ? w.slice(1) : w.slice(0, -1)).join(" ");
            }
          } else if (s.what === "numbers") {
            const all = v.match(pats.numbers!);
            if (all) {
              piece = all.join(" ");
              rest = tidy(v.replace(pats.numbers!, " "));
            }
          } else {
            const m = pats[s.what]!.exec(v);
            if (m) {
              piece = (m.slice(1).find((x) => x !== undefined) ?? m[0]).trim();
              rest = tidy(`${v.slice(0, m.index)} ${v.slice(m.index + m[0].length)}`);
            }
          }
          if (piece === null) continue;
          cells.push([r, at, piece]);
          if (s.remove && rest !== v) cells.push([r, s.col, rest]);
        }
        const info = await this.setCells(doc, cells);
        return this.report(info, { cols: 1, cells: cells.length });
      }
      case "fill_blanks": {
        const s = op.spec;
        const changes: [number, number, string][] = [];
        const lim = Math.min(rows, 100_000);
        for (const c of s.cols) {
          let last: string | null = s.from.kind === "value" ? s.from.text : null;
          const order = Array.from({ length: lim - (s.keep_first ? 1 : 0) }, (_, i) => i + (s.keep_first ? 1 : 0));
          if (s.from.kind === "below") order.reverse();
          for (const r of order) {
            const v = this.value(d, r, c);
            const empty = s.spaces ? !v.trim() : !v;
            if (empty) {
              if (last !== null) changes.push([r, c, last]);
            } else if (s.from.kind !== "value") last = v;
          }
        }
        const info = changes.length ? await this.setCells(doc, changes) : this.refresh(d);
        return this.report(info, { rows: new Set(changes.map((x) => x[0])).size, groups: changes.length });
      }
      case "join": {
        const mine = this.table(d);
        const list = op.list_id != null ? (this.joinLists.get(op.list_id) ?? null) : (op.list ?? null);
        const sim = simulateJoin(mine, list !== null ? null : this.table(this.get(op.other ?? -1)), op.spec, list);
        const s = op.spec;
        const r = sim.report;
        if (r.changed + r.removed + r.extra_rows + r.added === 0 && !(s.keep_first && sim.insertN > 0))
          throw err("invalid", s.keep_rows === "all" ? "no keys matched and there is nothing to add" : "no rows would change");
        if (sim.insertN > 0) await this.change(doc, { op: "insert_cols", at: sim.insertAt, count: sim.insertN });
        const cells: [number, number, string][] = [];
        const start = s.keep_first && mine.length ? 1 : 0;
        if (start && sim.insertN > 0) for (let k = 0; k < sim.insertN; k++) cells.push([0, sim.insertAt + k, sim.titles[sim.insertAt + k] ?? ""]);
        if (sim.append.length) {
          const at = this.rowCount(d);
          await this.change(doc, { op: "insert_rows", at, count: sim.append.length });
          sim.append.forEach((line, i) => line.forEach((v, c) => v !== "" && cells.push([at + i, c, v])));
        }
        for (let i = sim.rows.length - 1; i >= 0; i--) {
          const o = sim.rows[i]!;
          const p = start + i;
          if (o.kind === "removed") {
            await this.change(doc, { op: "delete_rows", at: p, count: 1 });
            // Positions below moved up by one.
            for (const c of cells) if (c[0] > p) c[0]--;
            continue;
          }
          if (o.extra.length) {
            await this.change(doc, { op: "insert_rows", at: p + 1, count: o.extra.length });
            for (const c of cells) if (c[0] > p) c[0] += o.extra.length;
            o.extra.forEach((line, k) => line.forEach((v, c) => v !== "" && cells.push([p + 1 + k, c, v])));
          }
          for (const [c, v] of o.set) cells.push([p, c, v]);
        }
        const info = cells.length ? await this.setCells(doc, cells) : this.refresh(d);
        return { info, cells: cells.length, rows: r.added + r.extra_rows + r.removed, groups: 0, cols: sim.insertN, skipped: 0, join: r };
      }
      case "append": {
        const o = this.get(op.other);
        const s = op.spec;
        const oRows = this.rowCount(o);
        const oCols = this.colCount(o);
        const mine = allCols.map((c) => (s.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
        const target: (number | null)[] = [];
        const cells: [number, number, string][] = [];
        let next = allCols.length;
        let dropped = 0;
        for (let c = 0; c < oCols; c++) {
          if (!s.by_name) {
            target.push(c);
            continue;
          }
          const t = (s.other_header ? this.value(o, 0, c) : "") || `Column ${c + 1}`;
          const hit = mine.findIndex((m) => m === t || m.trim().toLowerCase() === t.trim().toLowerCase());
          if (hit >= 0) target.push(hit);
          else if (s.add_columns) {
            if (s.keep_first) cells.push([0, next, t]);
            target.push(next++);
          } else {
            target.push(null);
            dropped++;
          }
        }
        const from = s.other_header ? 1 : 0;
        const n = Math.max(0, oRows - from);
        if (n === 0) throw err("invalid", "the other table has no rows to add");
        await this.change(doc, { op: "insert_rows", at: rows, count: n });
        for (let r = 0; r < n; r++)
          for (let c = 0; c < oCols; c++) {
            const t = target[c];
            const v = this.value(o, from + r, c);
            if (t !== null && t !== undefined && v !== "") cells.push([rows + r, t, v]);
          }
        const info = await this.setCells(doc, cells.length ? cells : [[rows, 0, this.value(o, from, 0)]]);
        return { info, cells: cells.length, rows: n, groups: 0, cols: next - allCols.length, skipped: 0, join: { matched: 0, unmatched: 0, added: n, repeated_keys: 0, cols: next - allCols.length, dropped_cols: dropped, blank_keys: 0, changed: 0, removed: 0, extra_rows: 0, bad_keys: 0 } };
      }
      case "paste": {
        let g = parseTsv(op.text);
        if (op.spec.transpose) {
          const w = Math.max(0, ...g.map((r) => r.length));
          g = Array.from({ length: w }, (_, c) => g.map((r) => r[c] ?? ""));
        }
        if (op.spec.stop_at_end) g = g.slice(0, Math.max(0, rows - op.spec.row));
        const need = op.spec.row + g.length - rows;
        if (need > 0) await this.change(doc, { op: "insert_rows", at: rows, count: need });
        // Pasted column j goes to the j-th column not skipped (hidden ones).
        const skipCols = op.spec.skip_cols ?? [];
        const target: number[] = [];
        for (let c = op.spec.col; target.length < Math.max(0, ...g.map((x) => x.length)); c++) if (!skipCols.includes(c)) target.push(c);
        const cells: [number, number, string][] = [];
        let skipped = 0;
        g.forEach((line, i) =>
          line.forEach((v, j) => {
            if (op.spec.skip_blanks && v === "") return;
            const r = op.spec.row + i;
            const c = target[j]!;
            if (op.spec.op === "none") return void cells.push([r, c, v]);
            const s = numberOf(v);
            if (v.trim() === "") return;
            if (s === null) return void skipped++;
            const cur = this.value(d, r, c);
            const x = cur.trim() === "" ? 0 : numberOf(cur);
            if (x === null || (op.spec.op === "divide" && s === 0)) return void skipped++;
            const out = { add: x + s, subtract: x - s, multiply: x * s, divide: x / s }[op.spec.op];
            cells.push([r, c, String(Math.round(out * 1e9) / 1e9)]);
          }),
        );
        const info = cells.length ? await this.setCells(doc, cells) : this.refresh(d);
        return this.report(info, { cells: cells.length, skipped, hidden_rows: this.hiddenBetween_(d, op.spec.row, Math.min(op.spec.row + g.length, rows)) });
      }
      case "fill": {
        const f = op.spec;
        const vertical = f.dir === "down" || f.dir === "up";
        const cells: [number, number, string][] = [];
        const lines = vertical ? f.cols : f.rows;
        const len = vertical ? f.rows : f.cols;
        const max = f.dir === "down" ? rows - (f.row + f.rows) : f.dir === "up" ? f.row : f.dir === "left" ? f.col : f.count;
        const count = Math.min(f.count, max);
        for (let l = 0; l < lines; l++) {
          const start = Array.from({ length: len }, (_, k) => (vertical ? this.value(d, f.row + k, f.col + l) : this.value(d, f.row + l, f.col + k)));
          for (let k = 0; k < count; k++) {
            const i = f.dir === "down" || f.dir === "right" ? len + k : -1 - k;
            const v = this.seriesValue(start, f.mode, i);
            if (v === null) break;
            if (f.dir === "down") cells.push([f.row + f.rows + k, f.col + l, v]);
            else if (f.dir === "up") cells.push([f.row - 1 - k, f.col + l, v]);
            else if (f.dir === "right") cells.push([f.row + l, f.col + f.cols + k, v]);
            else cells.push([f.row + l, f.col - 1 - k, v]);
          }
        }
        const info = cells.length ? await this.setCells(doc, cells) : this.refresh(d);
        return this.report(info, { cells: cells.length });
      }
      case "show_duplicates":
      case "remove_duplicates": {
        const s = op.spec;
        const cols = s.cols.length ? s.cols : allCols;
        const first = new Map<string, number>();
        const dupRows: number[] = [];
        const leaders = new Set<number>();
        for (let r = s.keep_first ? 1 : 0; r < rows; r++) {
          const k = this.keyOf(d, r, cols, s.match_case, s.trim);
          const f = first.get(k);
          if (f === undefined) first.set(k, r);
          else {
            dupRows.push(r);
            leaders.add(f);
          }
        }
        if (op.op === "remove_duplicates" && ((s.keep && s.keep !== "first") || s.fill_blanks))
          throw err("invalid", "the browser preview keeps only the first copy; the desktop app does the rest");
        if (op.op === "remove_duplicates") {
          const info = dupRows.length ? this.removeRows(d, dupRows) : this.refresh(d);
          return this.report(info, { rows: dupRows.length, groups: leaders.size });
        }
        const keep = new Set([...dupRows, ...leaders].map((r) => this.rowId(d, r)));
        if (s.keep_first && rows > 0) keep.add(this.rowId(d, 0));
        return this.report(this.setView(d, keep, { columns: [], keep_first: s.keep_first }), { rows: dupRows.length + leaders.size, groups: leaders.size });
      }
      case "remove_blank_rows": {
        const gone: number[] = [];
        for (let r = op.spec.keep_first ? 1 : 0; r < rows; r++)
          if (allCols.every((c) => (op.spec.spaces ? this.value(d, r, c).trim() : this.value(d, r, c)) === "")) gone.push(r);
        const info = gone.length ? this.removeRows(d, gone) : this.refresh(d);
        return this.report(info, { rows: gone.length });
      }
      case "remove_blank_cols": {
        const n = this.totalRows(d);
        const blank = allCols.filter((c) => {
          for (let p = op.spec.keep_first ? 1 : 0; p < n; p++) {
            const v = this.valueAt(d, p, c);
            if ((op.spec.spaces ? v.trim() : v) !== "") return false;
          }
          return true;
        });
        if (blank.length === allCols.length) blank.shift();
        let info = this.refresh(d);
        for (const c of [...blank].reverse()) info = await this.change(doc, { op: "delete_cols", at: c, count: 1 });
        return this.report(info, { cols: blank.length });
      }
      case "split_column": {
        const s = op.spec;
        if (s.mode && s.mode !== "text") throw err("invalid", "the browser preview splits only at a separator; the desktop app does the rest");
        if (!s.separator) throw err("invalid", "choose a separator to split at");
        const parts: string[][] = [];
        for (let r = 0; r < rows; r++) {
          const v = this.value(d, r, s.col);
          parts.push(r === 0 && s.keep_first ? [v] : v.split(s.separator).map((x) => (s.trim ? x.trim() : x)));
        }
        const most = Math.max(0, ...parts.map((p) => p.length));
        if (most < 2) return this.report(this.refresh(d));
        const add = s.keep_original ? most : most - 1;
        await this.change(doc, { op: "insert_cols", at: s.col + 1, count: add });
        const cells: [number, number, string][] = [];
        parts.forEach((p, r) => {
          if (r === 0 && s.keep_first) {
            for (let k = s.keep_original ? 0 : 1; k < most; k++) cells.push([0, s.col + 1 + k - (s.keep_original ? 0 : 1), `${p[0]} ${k + 1}`]);
            return;
          }
          if (p.length < 2) return;
          p.forEach((x, k) => {
            if (!s.keep_original && k === 0) cells.push([r, s.col, x]);
            else cells.push([r, s.col + 1 + k - (s.keep_original ? 0 : 1), x]);
          });
        });
        const info = await this.setCells(doc, cells);
        return this.report(info, { cols: add, cells: cells.length });
      }
      case "merge_columns": {
        const s = op.spec;
        if (s.count < 2 || s.col + s.count > allCols.length) throw err("invalid", "choose two or more neighbouring columns to merge");
        const cells: [number, number, string][] = [];
        for (let r = 0; r < rows; r++) {
          const vals = Array.from({ length: s.count }, (_, k) => this.value(d, r, s.col + k)).filter((v) => !s.skip_blanks || v !== "");
          cells.push([r, s.col, vals.join(s.separator)]);
        }
        await this.setCells(doc, cells);
        const info = await this.change(doc, { op: "delete_cols", at: s.col + 1, count: s.count - 1 });
        return this.report(info, { cols: s.count - 1, cells: cells.length });
      }
      case "split_rows": {
        const s = op.spec;
        if (!s.separator) throw err("invalid", "choose a separator to split at");
        let added = 0;
        for (let r = rows - 1; r >= (s.keep_first ? 1 : 0); r--) {
          const parts = this.value(d, r, s.col).split(s.separator).map((x) => (s.trim ? x.trim() : x)).filter((x) => !s.skip_empty || x !== "");
          if (parts.length < 2) continue;
          const line = allCols.map((c) => this.value(d, r, c));
          await this.change(doc, { op: "insert_rows", at: r + 1, count: parts.length - 1 });
          const cells: [number, number, string][] = [[r, s.col, parts[0]!]];
          parts.slice(1).forEach((p, i) => line.forEach((v, c) => cells.push([r + 1 + i, c, c === s.col ? p : v])));
          await this.setCells(doc, cells);
          added += parts.length - 1;
        }
        return this.report(this.refresh(d), { rows: added });
      }
      case "merge_rows": {
        const s = op.spec;
        if (!s.keys.length) throw err("invalid", "choose the column(s) that identify a row");
        const groups = new Map<string, number[]>();
        for (let r = s.keep_first ? 1 : 0; r < rows; r++) {
          const k = this.keyOf(d, r, s.keys, s.match_case, s.trim);
          groups.set(k, [...(groups.get(k) ?? []), r]);
        }
        const cells: [number, number, string][] = [];
        const gone: number[] = [];
        let count = 0;
        for (const members of groups.values()) {
          if (members.length < 2) continue;
          count++;
          for (const c of allCols) {
            if (s.keys.includes(c)) continue;
            const vals: string[] = [];
            for (const r of members) {
              const v = this.value(d, r, c);
              if (v !== "" && !(s.distinct && vals.includes(v))) vals.push(v);
            }
            cells.push([members[0]!, c, vals.join(s.separator)]);
          }
          gone.push(...members.slice(1));
        }
        if (cells.length) await this.setCells(doc, cells);
        const info = gone.length ? this.removeRows(d, gone) : this.refresh(d);
        return this.report(info, { rows: gone.length, groups: count });
      }
      case "sort_columns": {
        const end = op.count ? op.col + op.count : allCols.length;
        const seg = allCols.slice(op.col, end).sort((a, b) => naturalCompare(this.value(d, 0, a), this.value(d, 0, b)) * (op.descending ? -1 : 1));
        const before = this.colsOf(d).slice();
        const after = [...before.slice(0, op.col), ...seg.map((c) => before[c]!), ...before.slice(end)];
        const moved = after.some((x, i) => x !== before[i]);
        const info = moved
          ? this.push(d, () => {
              d.cols = after.slice();
              return () => {
                d.cols = before.slice();
              };
            })
          : this.refresh(d);
        return this.report(info, { cols: moved ? seg.length : 0 });
      }
    }
  }

  private gridTab(out: string[][], name: string): DocInfo {
    const info = this.make(name, out.length, Math.max(1, ...out.map((l) => l.length)));
    const nd = this.get(info.id);
    out.forEach((line, r) => line.forEach((v, c) => nd.edits.set(`${r}:${c}`, v)));
    nd.clean = -1;
    return this.refresh(nd);
  }
  async tableReport(doc: number, op: TableOp): Promise<TableReport> {
    if (op.op !== "sample") return { info: await this.newTable(doc, op), compare: null };
    // A seeded sample (a simple stand-in for the app's method) and its record.
    const d = this.get(doc);
    const s = op.spec;
    const first = s.keep_first ? 1 : 0;
    const rows = this.rowCount(d);
    const cols = this.colCount(d);
    let state = s.seed || 1;
    const rand = () => (state = (state * 1103515245 + 12345) % 2147483648) / 2147483648;
    const groups = new Map<string, number[]>();
    for (let r = first; r < rows; r++) {
      const g = s.per != null ? this.value(d, r, s.per) : "(all rows)";
      groups.set(g, [...(groups.get(g) ?? []), r]);
    }
    const picked: number[] = [];
    const counts: [string, number, number][] = [];
    for (const [g, members] of groups) {
      const k = s.size.kind === "percent" ? Math.round((members.length * s.size.p) / 100) : s.size.kind === "rows" ? Math.min(s.size.n, members.length) : 0;
      const pool = [...members];
      for (let i = 0; i < k; i++) {
        const j = i + Math.floor(rand() * (pool.length - i));
        [pool[i], pool[j]] = [pool[j]!, pool[i]!];
      }
      picked.push(...pool.slice(0, k));
      counts.push([g, members.length, k]);
    }
    if (!s.drawn_order) picked.sort((a, b) => a - b);
    const line = (r: number) => [...(s.row_numbers ? [String(r + 1)] : []), ...Array.from({ length: cols }, (_, c) => this.value(d, r, c))];
    const out = [...(first ? [[...(s.row_numbers ? ["Row"] : []), ...Array.from({ length: cols }, (_, c) => this.value(d, 0, c))]] : []), ...picked.map(line)];
    const stem = d.info.name.replace(/\.[^.]*$/, "");
    const info = this.gridTab(out, `Sample (${stem}, seed ${s.seed})`);
    const record = this.gridTab(
      [
        ["What", "Value", "Rows"],
        ["Drawn from", d.info.name, String(rows - first)],
        ["File SHA-256", "(not saved to a file)", ""],
        ["Seed", String(s.seed), ""],
        ...(s.per != null ? [["Per group of", this.value(d, 0, s.per), ""]] : []),
        ["Method", `${s.size.kind === "percent" ? `${s.size.p}% of the rows` : `${s.size.kind === "rows" ? s.size.n : 0} rows`} at random${s.per != null ? " from each group" : ""}, seed ${s.seed}`, ""],
        ["Rows taken", "", String(picked.length)],
        ...counts.map(([g, n, k]) => ["Group", g || "(blank)", `${k} of ${n}`]),
      ],
      `Sample record (${stem})`,
    );
    return { info, compare: null, record };
  }

  async findChange(doc: number, row: number, col: number, up: boolean): Promise<number | null> {
    const d = this.get(doc);
    const here = this.value(d, row, col);
    const n = Math.min(this.rowCount(d), 200_000);
    for (let r = up ? row - 1 : row + 1; up ? r >= 0 : r < n; r += up ? -1 : 1) if (this.value(d, r, col) !== here) return r;
    return null;
  }

  async findExtreme(doc: number, col: number, largest: boolean, keepFirst: boolean): Promise<number | null> {
    const d = this.get(doc);
    let best: [number, number] | null = null;
    for (let r = keepFirst ? 1 : 0; r < Math.min(this.rowCount(d), 200_000); r++) {
      const x = numberOf(this.value(d, r, col));
      if (x !== null && (!best || (largest ? x > best[0] : x < best[0]))) best = [x, r];
    }
    return best ? best[1] : null;
  }

  /** The last audit asked for (the browser tests look at it). */
  lastGapSpec: GapsSpec | null = null;

  async gapAudit(doc: number, spec: GapsSpec): Promise<GapReport> {
    this.lastGapSpec = JSON.parse(JSON.stringify(spec)) as GapsSpec;
    return this.gaps(doc, spec).report;
  }

  /** The browser preview's audit: one column, the last run of digits, the
   *  most common text around it. Series, shapes and a typed range are ignored
   *  here (the desktop app does them). */
  private gaps(doc: number, spec: GapsSpec): { report: GapReport; beside: number[] } {
    const d = this.get(doc);
    const cells: { around: string; digits: string; row: number }[] = [];
    let blank = 0;
    let other = 0;
    const first = spec.keep_first ? 1 : 0;
    for (let r = first; r < this.rowCount(d); r++) {
      const v = this.value(d, r, spec.col).trim();
      const m = /^(.*?)([0-9]{1,15})([^0-9]*)$/.exec(v);
      if (!v) blank++;
      else if (!m) other++;
      else cells.push({ around: JSON.stringify([m[1], m[3]]), digits: m[2]!, row: r });
    }
    const count = new Map<string, number>();
    for (const c of cells) count.set(c.around, (count.get(c.around) ?? 0) + 1);
    const best = [...count].sort((a, b) => b[1] - a[1])[0]?.[0];
    const own = cells.filter((c) => c.around === best);
    const [prefix, suffix] = JSON.parse(best ?? '["",""]') as [string, string];
    const width = Math.max(0, ...own.filter((c) => c.digits.length > 1 && c.digits.startsWith("0")).map((c) => c.digits.length));
    const show = (n: number) => `${prefix}${String(n).padStart(width, "0")}${suffix}`;
    const sorted = [...own].sort((a, b) => Number(a.digits) - Number(b.digits));
    const ranges: GapReport["ranges"] = [];
    const beside = new Set<number>();
    let missing = 0;
    for (let i = 1; i < sorted.length; i++) {
      const a = Number(sorted[i - 1]!.digits);
      const b = Number(sorted[i]!.digits);
      if (b - a < 2) continue;
      ranges.push({ series: [], from: show(a + 1), to: show(b - 1), count: String(b - a - 1) });
      missing += b - a - 1;
      for (const c of sorted) if (Number(c.digits) === a || Number(c.digits) === b) beside.add(c.row);
    }
    const distinct = new Set(own.map((c) => Number(c.digits))).size;
    const pattern = best === undefined ? null : { prefix, suffix, width };
    const label = `${prefix}${"#".repeat(Math.max(1, width))}${suffix}`;
    const report: GapReport = {
      run: 0,
      pattern,
      patterns: pattern ? [{ pattern, label, count: own.length }] : [],
      pattern_total: count.size,
      too_many_shapes: false,
      dates: spec.dates ? { ambiguous: 0, ambiguous_rows: [], two_digit: 0, two_digit_rows: [], weekend: 0, weekend_rows: [], off_step: 0, off_step_rows: [] } : null,
      shown: this.rowCount(d) - first,
      rows: own.length,
      distinct,
      blank,
      other_shape: other + cells.length - own.length,
      other_rows: [],
      outside: 0,
      drops: own.filter((c, i) => i > 0 && Number(c.digits) < Number(own[i - 1]!.digits)).length,
      drop_rows: [],
      added: 0,
      duplicate_values: 0,
      duplicate_extra: own.length - distinct,
      duplicates: [],
      series: own.length ? [{ key: [], first: show(Number(sorted[0]!.digits)), last: show(Number(sorted.at(-1)!.digits)), rows: own.length, distinct, missing: String(missing), too_few: own.length < 2 }] : [],
      series_total: own.length ? 1 : 0,
      ranges,
      range_total: ranges.length,
      missing: String(missing),
      looks_like_amounts: false,
    };
    return { report, beside: [...beside] };
  }

  async compareCols(doc: number, spec: CompareCols): Promise<CompareColsReport> {
    const d = this.get(doc);
    const rep: CompareColsReport = { same: 0, different: 0, only_first: 0, only_second: 0, rows: [] };
    if (spec.a === spec.b) throw err("invalid", "pick two different columns");
    for (let r = spec.keep_first ? 1 : 0; r < this.rowCount(d); r++) {
      const k = compareCells(this.value(d, r, spec.a), this.value(d, r, spec.b), spec);
      if (k === "same") rep.same++;
      else {
        if (k === "different") rep.different++;
        else if (k === "first") rep.only_first++;
        else rep.only_second++;
        rep.rows.push(r);
      }
    }
    return rep;
  }

  async specialCells(doc: number, spec: SpecialSpec): Promise<SpecialCells> {
    const d = this.get(doc);
    const k = spec.kind;
    const against = k.kind === "row_diff" ? k.against : -1;
    const cols = [...new Set(spec.cols)].filter((c) => c < this.colCount(d) && c !== against).sort((a, b) => a - b);
    const out: SpecialCells = { cells: [], total: 0 };
    const end = Math.min(this.rowCount(d), spec.row + spec.rows);
    const loose = (s: string) => s.split(/\s+/).filter(Boolean).join(" ").toLowerCase();
    // Changed or failing cells: from the stub's change list or checks.
    let marks: Set<string> | null = null;
    if (k.kind === "edited" || k.kind === "problems") {
      const c0 = Math.min(...cols);
      const c1 = Math.max(...cols) + 1;
      const list =
        k.kind === "edited"
          ? (await this.editedCells(doc, spec.row, end - spec.row, c0, c1 - c0)).map(([r, c]) => `${r}:${c}`)
          : (await this.checkMarks(doc, spec.row, end - spec.row, c0, c1 - c0, k.checks)).flatMap((m) => (m.col === null ? cols.map((c) => `${m.row}:${c}`) : [`${m.row}:${m.col}`]));
      marks = new Set(list);
    }
    for (let r = spec.row; r < end; r++) {
      for (const c of cols) {
        if (marks) {
          if (marks.has(`${r}:${c}`)) {
            out.total++;
            out.cells.push([r, c]);
          }
          continue;
        }
        const v = this.value(d, r, c);
        const blank = v.trim() === "";
        let hit: boolean;
        if (k.kind === "blanks") hit = blank;
        else if (k.kind === "numbers") hit = numberOf(v) !== null;
        else if (k.kind === "text") hit = !blank && numberOf(v) === null;
        else if (k.kind !== "row_diff") hit = false;
        else {
          const o = this.value(d, r, k.against);
          hit = k.exact ? v !== o : v !== o && loose(v) !== loose(o);
        }
        if (hit) {
          out.total++;
          if (out.cells.length < 5_000_000) out.cells.push([r, c]);
        }
      }
    }
    return out;
  }

  async numberRange(doc: number, col: number, keepFirst: boolean): Promise<[number, number] | null> {
    const d = this.get(doc);
    let lo = Infinity;
    let hi = -Infinity;
    for (let r = keepFirst ? 1 : 0; r < Math.min(this.rowCount(d), 200_000); r++) {
      const x = numberOf(this.value(d, r, col));
      if (x !== null) {
        lo = Math.min(lo, x);
        hi = Math.max(hi, x);
      }
    }
    return lo <= hi ? [lo, hi] : null;
  }

  async typeahead(doc: number, col: number, prefix: string, keepFirst: boolean): Promise<string | null> {
    if (!prefix) return null;
    const d = this.get(doc);
    const p = prefix.toLowerCase();
    let best: string | null = null;
    for (let r = keepFirst ? 1 : 0; r < Math.min(this.rowCount(d), 20_000); r++) {
      const v = this.value(d, r, col).trim();
      const k = v.toLowerCase();
      if (k.startsWith(p) && k.length > p.length && (best === null || k < best.toLowerCase())) best = v;
    }
    return best;
  }

  async renameFile(): Promise<DocInfo | null> {
    throw err("invalid", "the browser preview has no files to rename; the desktop app does");
  }

  async splitToFiles(_doc?: number, spec?: { guard?: boolean }): Promise<SplitReport | null> {
    this.calls.push(`splitToFiles:guard=${spec?.guard === true}`);
    throw err("invalid", "the browser preview cannot write files; the desktop app does");
  }

  async combineFiles(): Promise<DocInfo | null> {
    throw err("invalid", "the browser preview cannot read files; the desktop app does");
  }

  async showInFolder(): Promise<void> {
    throw err("invalid", "the browser preview has no folders; the desktop app does");
  }

  async newTable(doc: number, op: TableOp): Promise<DocInfo> {
    if (op.op === "subtotals") throw err("invalid", "the browser preview does not make subtotals; the desktop app does");
    if (op.op === "gaps") {
      const r = this.gaps(doc, op.spec).report;
      const out = [["Missing", r.missing, ""], ["", "", ""], ["From", "To", "Missing"], ...r.ranges.map((g) => [g.from, g.to, g.count])];
      const info = this.make("Untitled", out.length, 3);
      const nd = this.get(info.id);
      out.forEach((line, i) => line.forEach((v, c) => nd.edits.set(`${i}:${c}`, v)));
      nd.clean = -1;
      return this.refresh(nd);
    }
    if (op.op === "counts") {
      const d = this.get(doc);
      const s = op.spec;
      const tally = new Map<string, number>();
      let n = 0;
      for (let r = s.keep_first ? 1 : 0; r < this.rowCount(d); r++) {
        const k = JSON.stringify(s.cols.map((c) => this.value(d, r, c)));
        tally.set(k, (tally.get(k) ?? 0) + 1);
        n++;
      }
      const names = s.cols.map((c) => (s.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
      const lines = [...tally].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
      const out = [[...names, "Count", "Percent"], ...lines.map(([k, c]) => [...(JSON.parse(k) as string[]), String(c), `${((c * 100) / Math.max(1, n)).toFixed(1)}%`])];
      const info = this.make("Untitled", out.length, out[0]!.length);
      const nd = this.get(info.id);
      out.forEach((line, r) => line.forEach((v, c) => nd.edits.set(`${r}:${c}`, v)));
      nd.clean = -1;
      return this.refresh(nd);
    }
    if (op.op !== "pivot" && op.op !== "unpivot") throw err("invalid", "the browser preview does not do this; the desktop app does");
    const d = this.get(doc);
    const rows = this.rowCount(d);
    const title = (c: number, header: boolean) => (header ? this.value(d, 0, c) : "") || `Column ${c + 1}`;
    const out: string[][] = [];
    if (op.op === "pivot") {
      const s = op.spec;
      const rowKeys = new Map<string, string[]>();
      const colKeys: string[] = [];
      const cells = new Map<string, string[]>();
      for (let r = s.keep_first ? 1 : 0; r < rows; r++) {
        const rk = s.rows.map((c) => this.value(d, r, c));
        const k = JSON.stringify(rk);
        rowKeys.set(k, rk);
        const ck = this.value(d, r, s.col);
        if (!colKeys.includes(ck)) colKeys.push(ck);
        const key = JSON.stringify([k, ck]);
        cells.set(key, [...(cells.get(key) ?? []), this.value(d, r, s.value)]);
      }
      out.push([...s.rows.map((c) => title(c, s.keep_first)), ...colKeys.map((k) => k || "(blank)")]);
      for (const [k, rk] of rowKeys) {
        const line = [...rk];
        for (const ck of colKeys) {
          const vals = cells.get(JSON.stringify([k, ck]));
          if (!vals) {
            line.push("");
            continue;
          }
          const nums = vals.map(numberOf).filter((x): x is number => x !== null);
          const agg = {
            count: String(vals.filter((v) => v !== "").length),
            sum: nums.length ? String(nums.reduce((a, b) => a + b, 0)) : "",
            average: nums.length ? String(nums.reduce((a, b) => a + b, 0) / nums.length) : "",
            min: nums.length ? String(Math.min(...nums)) : "",
            max: nums.length ? String(Math.max(...nums)) : "",
            first: vals[0] ?? "",
            join: vals.filter((v) => v !== "").join(s.separator),
          }[s.aggregate];
          line.push(agg);
        }
        out.push(line);
      }
    } else {
      const s = op.spec;
      const all = Array.from({ length: this.colCount(d) }, (_, i) => i);
      const values = s.values.length ? s.values : all.filter((c) => !s.ids.includes(c));
      out.push([...s.ids.map((c) => title(c, s.keep_first)), s.name_title || "Name", s.value_title || "Value"]);
      for (let r = s.keep_first ? 1 : 0; r < rows; r++)
        for (const c of values) {
          const v = this.value(d, r, c);
          if (s.skip_blanks && v === "") continue;
          out.push([...s.ids.map((i) => this.value(d, r, i)), title(c, s.keep_first), v]);
        }
    }
    const width = Math.max(1, ...out.map((r) => r.length));
    if (out.length * width > 500_000) throw err("too_large", "the browser preview builds tables of at most 500,000 cells");
    const info = this.make("Untitled", out.length, width);
    const nd = this.get(info.id);
    out.forEach((line, r) => {
      for (let c = 0; c < width; c++) nd.edits.set(`${r}:${c}`, line[c] ?? "");
    });
    nd.clean = -1;
    return this.refresh(nd);
  }

  private pivotData(doc: number, keepFirst: boolean): { data: string[][]; titles: string[] } {
    const d = this.get(doc);
    const rows = this.rowCount(d);
    const cols = this.colCount(d);
    const titles = Array.from({ length: cols }, (_, c) => (keepFirst ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
    const data: string[][] = [];
    for (let r = keepFirst ? 1 : 0; r < Math.min(rows, 200_000); r++) data.push(Array.from({ length: cols }, (_, c) => this.value(d, r, c)));
    return { data, titles };
  }

  async pivotPreview(doc: number, spec: PivotSpec, sample: number | null): Promise<PivotGrid> {
    const { data, titles } = this.pivotData(doc, spec.keep_first);
    return mockPivot(data, titles, spec, sample);
  }

  private tableDoc(out: string[][]): DocInfo {
    const width = Math.max(1, ...out.map((r) => r.length));
    if (out.length * width > 500_000) throw err("too_large", "the browser preview builds tables of at most 500,000 cells");
    const info = this.make("Untitled", out.length, width);
    const nd = this.get(info.id);
    out.forEach((line, r) => {
      for (let c = 0; c < width; c++) nd.edits.set(`${r}:${c}`, line[c] ?? "");
    });
    nd.clean = -1;
    return this.refresh(nd);
  }

  private pivotLines(doc: number, spec: PivotSpec): string[][] {
    const { data, titles } = this.pivotData(doc, spec.keep_first);
    const g = mockPivot(data, titles, spec, null);
    const head = g.head.map((h) => h.flatMap((c) => [c.text, ...Array(Math.max(0, c.span - 1)).fill("")]));
    return [...head, ...g.rows.map((r) => [...r.labels, ...r.cells])];
  }

  async pivotTable(doc: number, spec: PivotSpec): Promise<DocInfo> {
    const info = await this.tableDoc(this.pivotLines(doc, spec));
    const { titles } = this.pivotData(doc, spec.keep_first);
    const names = (f: PivotField[]) => f.map((x) => titles[x.col] ?? `column ${x.col + 1}`).join(", ");
    const [r, c] = [names(spec.rows), names(spec.cols)];
    const what = r && c ? ` – ${r} by ${c}` : r ? ` – ${r}` : c ? ` – by ${c}` : "";
    const stem = this.get(doc).info.name.replace(/\.[^. ]{1,5}$/, "");
    return this.setDocTitle(info.id, `Pivot${what} (${stem})`);
  }

  async pivotText(doc: number, spec: PivotSpec): Promise<string> {
    return this.pivotLines(doc, spec).map((l) => l.join(String.fromCharCode(9))).join(String.fromCharCode(13, 10)) + String.fromCharCode(13, 10);
  }

  async pivotDrill(doc: number, spec: PivotSpec, rowPath: string[], colPath: string[]): Promise<DocInfo> {
    const { data, titles } = this.pivotData(doc, spec.keep_first);
    return this.tableDoc([titles, ...mockDrill(data, spec, rowPath, colPath)]);
  }

  async pivotItems(doc: number, field: PivotField, spec: PivotSpec): Promise<ItemList> {
    const { data } = this.pivotData(doc, spec.keep_first);
    return mockItems(data, field.col, spec);
  }

  async pivotClose(): Promise<void> {}

  async pivotSave(): Promise<string | null> {
    throw err("invalid", "the browser preview cannot save files; the desktop app does");
  }

  /** The stub has no files: it hands back a made-up layout "by city" that
   *  counts rows by column 4, saved when that column was called "city". A
   *  file whose column 4 has another name gets it offered by place. */
  async pivotOpen(doc: number, keepFirst: boolean): Promise<[string, PivotSpec, import("./backend").PlaceSwap[]] | null> {
    const { newSpec, newField, newValue } = await import("./pivot");
    const spec = newSpec(keepFirst);
    spec.rows = [newField(3)];
    spec.values = [newValue(0, "count")];
    const now = keepFirst ? (await this.cellText(doc, 0, 3)).text : "";
    return ["by city", spec, now === "city" ? [] : [{ was: "city", col: 3, now }]];
  }

  async shownRowOf(doc: number, number: number): Promise<[number, boolean] | null> {
    const d = this.get(doc);
    const vis = this.visible(d);
    if (!vis) {
      const n = this.totalRows(d);
      return n > 0 ? [Math.min(number, n - 1), number < n] : null;
    }
    if (vis.length === 0) return null;
    let lo = 0;
    let hi = vis.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (vis[mid]! < number) lo = mid + 1;
      else hi = mid;
    }
    return lo < vis.length && vis[lo] === number ? [lo, true] : [Math.min(lo, vis.length - 1), false];
  }

  async currentRegion(doc: number, row: number, col: number, first: number): Promise<[number, number, number, number] | null> {
    const d = this.get(doc);
    const n = this.rowCount(d);
    const w = this.colCount(d);
    const empty = (r: number, c: number) => r < first || c < 0 || r >= n || c >= w || this.value(d, r, c) === "";
    if (row >= n || col >= w) throw { kind: "error", message: "out of range", row: null };
    if (empty(row, col)) return null;
    let [r0, r1, c0, c1] = [row, row, col, col];
    const runEnd = (r: number, c: number, step: 1 | -1) => {
      while (!empty(r + step, c)) r += step;
      return r;
    };
    const colHas = (c: number) => {
      for (let r = r0; r <= r1; r++) if (!empty(r, c)) return true;
      return false;
    };
    for (;;) {
      let grew = false;
      let to = r1;
      for (let c = c0; c <= c1; c++) if (!empty(r1 + 1, c)) to = Math.max(to, runEnd(r1 + 1, c, 1));
      if (to > r1) [r1, grew] = [to, true];
      to = r0;
      for (let c = c0; c <= c1; c++) if (!empty(r0 - 1, c)) to = Math.min(to, runEnd(r0 - 1, c, -1));
      if (to < r0) [r0, grew] = [to, true];
      if (c1 + 1 < w && colHas(c1 + 1)) [c1, grew] = [c1 + 1, true];
      if (c0 > 0 && colHas(c0 - 1)) [c0, grew] = [c0 - 1, true];
      if (!grew) return [r0, r1, c0, c1];
    }
  }

  async dataEdge(doc: number, row: number, col: number, dir: Dir): Promise<[number, number]> {
    const d = this.get(doc);
    const rows = this.rowCount(d);
    const cols = this.colCount(d);
    const filled = (r: number, c: number) => this.value(d, r, c) !== "";
    if (dir === "up" || dir === "down") {
      const step = dir === "down" ? 1 : -1;
      const last = dir === "down" ? rows - 1 : 0;
      let r = row + step;
      if (r < 0 || r >= rows) return [row, col];
      if (filled(row, col) && filled(r, col)) {
        while (r !== last && filled(r + step, col)) r += step;
      } else {
        while (r !== last && !filled(r, col)) r += step;
      }
      return [r, col];
    }
    const step = dir === "right" ? 1 : -1;
    const last = dir === "right" ? cols - 1 : 0;
    let c = col + step;
    if (c < 0 || c >= cols) return [row, col];
    if (filled(row, col) && filled(row, c)) {
      while (c !== last && filled(row, c + step)) c += step;
    } else {
      while (c !== last && !filled(row, c)) c += step;
    }
    return [row, c];
  }

  /** Tests: which of the start-up and session calls the page made. */
  calls: string[] = [];
  /** Tests: tabs "moved" to the second window, and waiting for this one. */
  moved: HandOver[] = [];
  waiting: HandOver[] = [];
  private onHandoverHandler: (() => void) | null = null;
  async modifiedDocs(): Promise<ModifiedReport> {
    const docs = [...this.docs.values()]
      .filter((d) => d.info.modified)
      .map((d) => ({ id: d.info.id, name: d.info.name, generation: d.info.history_pos }));
    return { docs, busy: [], jobs: [] };
  }
  async quitApp(known: { id: number; generation: number }[]): Promise<string> {
    this.calls.push(`quitApp:${known.map((k) => k.id).join(",")}`);
    return "exiting";
  }
  async windowMove(h: HandOver): Promise<void> {
    this.calls.push(`windowMove:${h.doc}`);
    this.moved.push(h);
  }
  async windowTake(): Promise<HandOver[]> {
    this.calls.push("windowTake");
    return this.waiting.splice(0);
  }
  async windowMoveBack(h: HandOver): Promise<void> {
    this.calls.push(`windowMoveBack:${h.doc}`);
  }
  async windowSync(items: HandOver[]): Promise<void> {
    this.calls.push(`windowSync:${items.length}`);
  }
  async windowClaim(h: HandOver): Promise<void> {
    this.calls.push(`windowClaim:${h.doc}`);
  }
  async windowCloseSelf(): Promise<void> {
    this.calls.push("windowCloseSelf");
  }
  onHandover(f?: () => void): void {
    this.onHandoverHandler = f ?? null;
  }
  /** Tests: a tab arrives from the other window. */
  handoverArrives(h: HandOver): void {
    this.waiting.push(h);
    this.onHandoverHandler?.();
  }
  onCloseRequested(): void {}

  /** Documents with a counting or grouping job running: like the backend, a
   *  second one is refused until the first ends. */
  private jobs = new Map<number, string>();
  private cancelled = new Set<number>();
  /** Tests: what the editor asked of the file, in order ("tally:start:2",
   *  "group:end:2", "cancel:Looking for look-alikes:ran", ...). */
  callLog: string[] = [];
  private async oneJob<T>(doc: number, label: string, tag: string, f: () => Promise<T>): Promise<T> {
    if (this.jobs.has(doc)) {
      this.callLog.push(`${tag}:refused`);
      throw err("busy", "another operation is still running on this file");
    }
    this.jobs.set(doc, label);
    this.callLog.push(`${tag}:start`);
    try {
      // The search takes a moment, so a test can change something meanwhile.
      await new Promise((r) => setTimeout(r, label === "Looking for look-alikes" ? 300 : 30));
      if (this.cancelled.delete(doc)) throw err("cancelled", "cancelled");
      return await f();
    } finally {
      this.jobs.delete(doc);
      this.callLog.push(`${tag}:end`);
    }
  }
  async valueTally(doc: number, spec: TallySpec): Promise<ValueTally> {
    return this.oneJob(doc, "Counting values", `tally:${spec.col}`, () => this.valueTallyNow(doc, spec));
  }
  private async valueTallyNow(doc: number, spec: TallySpec): Promise<ValueTally> {
    const d = this.get(doc);
    const counts = new Map<string, number>();
    let blanks = 0;
    const rows = Math.min(this.rowCount(d), 100_000);
    for (let r = spec.keep_first ? 1 : 0; r < rows; r++) {
      const v = this.value(d, r, spec.col);
      if (!v) blanks++;
      else counts.set(v, (counts.get(v) ?? 0) + 1);
      if (spec.limit != null && counts.size > spec.limit) break;
    }
    const values = [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
    const key = (v: string) => v.toLowerCase().normalize("NFD").replace(/[^\p{L}\p{N}]/gu, "");
    const by = new Map<string, number[]>();
    values.forEach((v, i) => {
      const k = key(v.value);
      if (!k) return;
      let g = by.get(k);
      if (!g) by.set(k, (g = []));
      g.push(i);
    });
    const look_alikes = [...by.values()].filter((m) => m.length > 1).map((members) => ({ members })).sort((a, b) => a.members[0]! - b.members[0]!);
    return { values: values.slice(0, 100_000), blanks, rows: rows - (spec.keep_first ? 1 : 0), complete: values.length <= 100_000 && (spec.limit == null || values.length <= spec.limit), look_alikes };
  }

  /** A stand-in for the engine's grouping: a rough sound key, and spelling
   *  within one slip for values of 4 or more letters. */
  async lookalikeGroups(doc: number, spec: LookAlikeSpec): Promise<LookAlikes> {
    return this.oneJob(doc, "Looking for look-alikes", `group:${spec.col}`, () => this.lookalikeGroupsNow(doc, spec));
  }
  private async lookalikeGroupsNow(doc: number, spec: LookAlikeSpec): Promise<LookAlikes> {
    const t = await this.valueTallyNow(doc, { col: spec.col, keep_first: spec.keep_first });
    const fold = (v: string) => v.toLowerCase().normalize("NFD").replace(/[^\p{L}\p{N}]/gu, "");
    const sound = (v: string) => {
      const w = fold(v).replace(/\d/g, "");
      const digits = fold(v).replace(/\D/g, "");
      const k = w.replace(/^kn|^gn|^pn|^wr/, (m) => m[1]!).replace(/ph/g, "f").replace(/th/g, "0").replace(/[cq]/g, "k").replace(/ck/g, "k").replace(/v/g, "f").replace(/z/g, "s").replace(/(.)\1+/g, "$1");
      return k.slice(0, 1) + k.slice(1).replace(/[aeiouyhw]/g, "") + digits;
    };
    const near = (a: string, b: string): boolean => {
      if (a === b) return true;
      if (Math.abs(a.length - b.length) > 1) return false;
      let i = 0;
      while (i < a.length && i < b.length && a[i] === b[i]) i++;
      const sub = a.slice(i + 1) === b.slice(i + 1);
      const del = a.slice(i + 1) === b.slice(i);
      const ins = a.slice(i) === b.slice(i + 1);
      const swap = a[i] === b[i + 1] && a[i + 1] === b[i] && a.slice(i + 2) === b.slice(i + 2);
      return sub || del || ins || swap;
    };
    const vals = t.values;
    const taken = new Set<number>();
    const groups: { value: string; count: number }[][] = [];
    const folded = vals.map((v) => fold(v.value));
    const sounds = vals.map((v) => sound(v.value));
    // Sound buckets, so a column of thousands of values stays quick.
    const bySoundKey = new Map<string, number[]>();
    if (spec.method !== "spelling") {
      sounds.forEach((k, i) => {
        if (folded[i]!.length < 3 || k.replace(/\d/g, "").length < 2) return;
        const b = bySoundKey.get(k);
        if (b) b.push(i);
        else bySoundKey.set(k, [i]);
      });
    }
    for (let h = 0; h < vals.length; h++) {
      if (taken.has(h)) continue;
      const hk = folded[h]!;
      const members = [h];
      const candidates = new Set<number>();
      for (const j of bySoundKey.get(sounds[h]!) ?? []) if (j > h) candidates.add(j);
      if (spec.method !== "sounds" && hk.length >= 4) for (let j = h + 1; j < vals.length; j++) if (folded[j]!.length >= 4 && Math.abs(folded[j]!.length - hk.length) <= 1) candidates.add(j);
      for (const j of [...candidates].sort((a, b2) => a - b2)) {
        if (taken.has(j)) continue;
        const jk = folded[j]!;
        const bySound = spec.method !== "spelling" && sounds[h] === sounds[j];
        const bySpelling = spec.method !== "sounds" && hk.length >= 4 && jk.length >= 4 && hk.replace(/\D/g, "") === jk.replace(/\D/g, "") && near(hk, jk);
        if (bySound || bySpelling) members.push(j);
      }
      if (members.length < 2) continue;
      members.forEach((m) => taken.add(m));
      groups.push(members.map((m) => vals[m]!));
    }
    return { groups, values: vals.length, complete: t.complete, too_long: 0, crowded: 0 };
  }

  private joinLists = new Map<number, string>();
  async joinList(_doc: number, text: string): Promise<number> {
    const id = this.joinLists.size + 1;
    this.joinLists.set(id, text);
    return id;
  }
  async lookupClose(): Promise<void> {}
  // Kept by file (as the backend does), so a file opened again stays private.
  private privateDocs = new Map<string, PrivateState>();
  private privateKey(doc: number): string {
    return this.docs.get(doc)?.info.path ?? `#${doc}`;
  }
  async findPrivate(doc: number, header: boolean): Promise<Finding[]> {
    const d = this.get(doc);
    const n = this.rowCount(d);
    const cols = d.info.cols;
    const out: Finding[] = [];
    for (let c = 0; c < cols; c++) {
      const title = header && n ? this.value(d, 0, c) : `Column ${c + 1}`;
      const t = title.toLowerCase().replace(/[^a-z0-9]/g, "");
      let kind: PrivateKind | null = /^ssn|socialsecurity/.test(t) ? "ssn" : /birth|^dob$/.test(t) ? "birth_date" : /email/.test(t) ? "email" : /phone|mobile/.test(t) ? "phone" : /licen|^dl$/.test(t) ? "licence" : /address|street/.test(t) ? "address" : null;
      const byTitle = kind !== null && header;
      let looked = 0;
      let hits = 0;
      for (let r = header ? 1 : 0; r < Math.min(n, 2000); r++) {
        const v = this.value(d, r, c).trim();
        if (!v) continue;
        looked++;
        if (/^\d{3}-\d{2}-\d{4}$/.test(v)) {
          hits++;
          kind ??= "ssn";
        } else if (/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(v)) {
          hits++;
          kind ??= "email";
        }
      }
      if (kind && (byTitle || hits * 10 >= looked * 6)) out.push({ col: c, kind, title, looked, hits, by_title: byTitle });
    }
    return out;
  }
  async saveSafeCopy(doc: number, spec: SafeCopy): Promise<SafeCopyReport | null> {
    const d = this.get(doc);
    const n = this.rowCount(d);
    const kept = Array.from({ length: d.info.cols }, (_, c) => spec.columns[c] ?? { action: "keep" }).filter((a) => a.action !== "drop").length;
    if (!kept) throw err("invalid", "keep at least one column");
    this.calls.push(`saveSafeCopy:guard=${spec.guard !== false}`);
    return { rows: Math.max(0, n - (spec.header ? 1 : 0)), cols: kept, changed: 0, bytes: 0, guarded: spec.guard !== false ? 2 : 0 };
  }
  /** Cell edits that differ from the generated file (stand-in for the
   *  engine's full comparison). */
  private changedCells(d: MockDoc): import("./backend").CellChange[] {
    const out: import("./backend").CellChange[] = [];
    const cols = this.colCount(d);
    const total = this.totalRows(d);
    for (let pos = 0; pos < total && out.length < 2000; pos++) {
      const id = d.order ? (d.order[pos] ?? 0) : pos;
      if (id < 0) continue;
      for (let c = 0; c < cols; c++) {
        const p = this.physCol(d, c);
        const e = d.edits.get(`${id}:${p}`);
        if (e === undefined) continue;
        const old = p < d.baseCols ? generated(id, p) : "";
        if (old !== e) out.push({ row: pos, file_row: id, col: c, old, new: e });
      }
    }
    return out;
  }
  async changeList(doc: number): Promise<import("./backend").ChangeList> {
    const d = this.get(doc);
    const cells = this.changedCells(d);
    const added = d.order ? d.order.flatMap((id, pos) => (id < 0 ? [pos] : [])) : [];
    const present = new Set(d.order ?? []);
    const deleted = d.order ? Array.from({ length: d.baseRows }, (_, i) => i).filter((i) => !present.has(i)) : [];
    const kept = (d.order ?? []).filter((id) => id >= 0);
    return {
      cells,
      cells_total: cells.length,
      added_rows: added.slice(0, 10000),
      added_total: added.length,
      deleted_rows: deleted.slice(0, 10000),
      deleted_total: deleted.length,
      added_cols: d.cols ? d.cols.flatMap((p, c) => (p >= d.baseCols ? [c] : [])) : [],
      deleted_cols: d.cols ? Array.from({ length: d.baseCols }, (_, p) => p).filter((p) => !d.cols!.includes(p)) : [],
      moved_cols: !!d.cols && d.cols.filter((p) => p < d.baseCols).some((p, i, a) => i > 0 && p < a[i - 1]!),
      reordered: kept.some((id, i) => i > 0 && id < kept[i - 1]!),
    };
  }
  async editedCells(doc: number, row: number, rows: number, col: number, cols: number): Promise<[number, number][]> {
    const d = this.get(doc);
    const out: [number, number][] = [];
    const n = this.rowCount(d);
    for (let r = row; r < Math.min(n, row + rows); r++) {
      const id = this.rowId(d, r);
      for (let c = col; c < Math.min(this.colCount(d), col + cols); c++) {
        const p = this.physCol(d, c);
        if (p < 0) continue;
        const e = d.edits.get(`${id}:${p}`);
        const old = id >= 0 && p < d.baseCols ? generated(id, p) : "";
        if ((e !== undefined && e !== old) || (id < 0 && this.value(d, r, c) !== "")) out.push([r, c]);
      }
    }
    return out;
  }
  async pickSearchFolder(): Promise<string | null> {
    return "C:\\samples";
  }
  async searchFolder(dir: string, spec: { pattern: { text: string } }): Promise<import("./FolderSearchDialog.svelte").FolderReport> {
    const t = spec.pattern.text;
    const hits = [0, 1, 2].map((i) => ({ path: `${dir}\\sample-${i + 1}.csv`, row: 3 + i * 7, col: 1, text: `before ${t} after`, start: 7, len: [...t].length }));
    return { hits, cells: hits.length, files: 4, files_with_hits: 3, skipped: [[`${dir}\\broken.csv`, "the file could not be read"]], too_many_files: false };
  }
  async folderSearchProgress(): Promise<number | null> {
    return null;
  }
  async folderSearchCancel(): Promise<void> {}
  async openFolderHit(): Promise<Opened> {
    return this.openSample();
  }
  async saveChangeReport(doc: number, guard = true): Promise<Written | null> {
    const l = await this.changeList(doc);
    this.calls.push(`saveChangeReport:guard=${guard}`);
    return { entries: l.cells_total + l.added_total + l.deleted_total, guarded: guard ? 2 : 0 };
  }
  private watch: import("./watch").WatchStatus = { running: false, folder: null, recipe: null, out: null, done: 0, failed: 0, log: [] };
  async watchPick(which: "folder" | "recipe" | "out"): Promise<import("./watch").WatchStatus> {
    if (which === "folder") this.watch.folder = "C:\\Data\\Inbox";
    else if (which === "out") this.watch.out = "C:\\Data\\Done";
    else this.watch.recipe = "C:\\Data\\monthly.recipe.json";
    return { ...this.watch };
  }
  async watchStatus(): Promise<import("./watch").WatchStatus> {
    return { ...this.watch };
  }
  async watchStart(): Promise<import("./watch").WatchStatus> {
    this.watch.running = true;
    this.watch.log = ["12:00:00 UTC  Watching C:\\Data\\Inbox (2 step recipe)", ...this.watch.log];
    return { ...this.watch };
  }
  async watchStop(): Promise<import("./watch").WatchStatus> {
    this.watch.running = false;
    return { ...this.watch };
  }
  private notes = new Map<number, Map<number, { flag: number; text: string }>>();
  async notesInfo(doc: number): Promise<[number[], boolean, number]> {
    return [[0], true, this.notes.get(doc)?.size ?? 0];
  }
  async noteMarks(doc: number, row: number, rows: number): Promise<import("./notes").NoteMark[]> {
    const m = this.notes.get(doc);
    if (!m) return [];
    return [...m].filter(([r]) => r >= row && r < row + rows).map(([r, n]) => ({ row: r, ...n }));
  }
  async noteSet(doc: number, row: number, flag: number, text: string): Promise<void> {
    if (!this.notes.has(doc)) this.notes.set(doc, new Map());
    const m = this.notes.get(doc)!;
    if (!flag && !text.trim()) m.delete(row);
    else m.set(row, { flag, text });
  }
  async notesSetKeys(): Promise<void> {}
  async noteList(doc: number): Promise<[import("./notes").NoteMark[], number]> {
    return [[...(this.notes.get(doc) ?? [])].map(([r, n]) => ({ row: r, ...n })), 0];
  }
  async notesShow(doc: number): Promise<DocInfo> {
    return { ...this.get(doc).info };
  }
  encryptedFrom = new Map<number, string>();
  async encryptedOrigin(doc: number): Promise<string | null> {
    return this.encryptedFrom.get(doc) ?? null;
  }
  async saveEncrypted(doc: number): Promise<number | null> {
    return this.get(doc).info.rows;
  }
  async pickEncrypted(): Promise<string | null> {
    return "C:\\Data\\voters.ndenc";
  }
  async openEncrypted(): Promise<DocInfo> {
    throw err("invalid", "the browser preview cannot open encrypted files");
  }
  async sqlitePick(): Promise<[string, import("./sqlite").DbTable[]] | null> {
    return ["C:\\Data\\county.sqlite", [{ name: "people", rows: 12, view: false }, { name: "precincts", rows: 3, view: false }, { name: "active", rows: null, view: true }]];
  }
  async sqliteOpen(): Promise<import("./backend").SqliteOpened> {
    throw err("invalid", "the browser preview cannot read databases");
  }
  async sqliteSave(doc: number): Promise<number | null> {
    return Math.max(0, this.get(doc).info.rows - 1);
  }
  async updatesFind(doc: number): Promise<import("./updates").UpdateList> {
    this.get(doc);
    return {
      cells: [{ row: 1, col: 1, old: "Ann", new: "Anne" }, { row: 4, col: 3, old: "4", new: "7" }],
      cells_total: 2,
      added: [["9", "Dee", "Long"]],
      added_total: 1,
      removed: [6],
      removed_total: 1,
      repeated_keys: 0,
      here_only: [],
      newer_only: ["email"],
      moved: 1,
    };
  }
  async updatesAccept(doc: number): Promise<DocInfo> {
    return { ...this.get(doc).info };
  }
  async updatesClose(): Promise<void> {}
  async streetLookup(doc: number): Promise<{ info: DocInfo; report: import("./streets").StreetReport }> {
    return { info: { ...this.get(doc).info }, report: { found: 3, no_street: 1, no_range: 0, overlaps: 0, no_number: 0, bad_ranges: 0 } };
  }
  async matchFind(doc: number): Promise<import("./match").MatchReport> {
    this.get(doc);
    return {
      listed: [
        { row: 1, id: 1, other_row: 4, score: 97.5, parts: [1, 0.95], here: ["william", "smith"], there: ["bill", "smyth"] },
        { row: 3, id: 3, other_row: 9, score: 86, parts: [0.85, 0.9], here: ["j", "lee"], there: ["john", "leigh"] },
      ],
      total: 2,
      rows_matched: 2,
      compared: 40,
      windowed: 0,
    };
  }
  async matchAccept(doc: number): Promise<DocInfo> {
    return { ...this.get(doc).info };
  }
  async matchClose(): Promise<void> {}
  async recipeGet(doc: number, header: boolean): Promise<import("./recipe").RecipeView> {
    this.get(doc);
    return {
      recipe: { version: 1, name: "", columns: header ? ["id", "name"] : [], header, steps: [{ op: "delete_cols", at: 2, count: 1 }, { op: "sort", spec: {} }], left_out: ["a paste"] },
      described: ["Delete column 3", "Sort by column 1"],
      holds_values: [false, false],
    };
  }
  async recipeSave(doc: number, header: boolean, name: string): Promise<string | null> {
    await this.recipeGet(doc, header);
    return `C:\\Data\\${name || "recipe"}.recipe.json`;
  }
  async recipeOpen(doc: number): Promise<import("./recipe").RecipeOpened | null> {
    const v = await this.recipeGet(doc, true);
    v.recipe.steps.push({ op: "clean", spec: { cleaner: { version: 1, name: "", steps: [{ tool: "scramble", loose: false }] } } });
    v.described.push("Clean: Scramble with a secret phrase");
    v.holds_values.push(true);
    return { ...v, path: "C:\\Data\\monthly.recipe.json", problems: ['column 2 is "Name", not "name"'] };
  }
  async recipeRun(doc: number): Promise<DocInfo> {
    return { ...this.get(doc).info };
  }
  /** The browser preview's steps panel: a list only (no replay). */
  private mockSteps = new Map<number, { rows: import("./recipe").StepRow[]; at: number; questions: import("./recipe").Question[] }>();
  private stepsOf(doc: number) {
    let s = this.mockSteps.get(doc);
    if (!s) {
      const row = (text: string, op: import("./recipe").RecipeStep, texts: [string, string][] = []): import("./recipe").StepRow => ({ kind: "step", text, done: true, op, texts });
      s = {
        rows: [
          row("Delete 1 column(s) from column 3", { op: "delete_cols", at: 2, count: 1 }),
          row('Rename column 2 to "county"', { op: "rename_column", col: 1, name: "county" }, [["name", "county"]]),
          row("Sort by column 1", { op: "sort", spec: { keys: [{ col: 0 }], keep_first: true } }),
          { kind: "left_out", text: "a paste", done: true, op: null, texts: [] },
        ],
        at: 4,
        questions: [],
      };
      this.mockSteps.set(doc, s);
    }
    return s;
  }
  async stepsView(doc: number): Promise<import("./recipe").StepsView> {
    this.get(doc);
    const s = this.stepsOf(doc);
    return { steps: s.rows.map((r, i) => ({ ...r, done: i < s.at })), at: s.at, hand_changes: false, questions: s.questions };
  }
  private markNow(d: MockDoc): number {
    d.baseMark ??= ++markSeq;
    const top = d.undo[d.undo.length - 1];
    return top ? (markOf.get(top.apply) ?? 0) : d.baseMark;
  }
  /** Undos (negative) or redos (positive) to reach a mark, or null when the history does not. */
  private reach(d: MockDoc, mark: number): number | null {
    const at = d.undo.map((s) => markOf.get(s.apply)).lastIndexOf(mark);
    if (at >= 0) return -(d.undo.length - 1 - at);
    if (mark === (d.baseMark ??= ++markSeq)) return -d.undo.length;
    const r = [...d.redo].reverse().findIndex((a) => markOf.get(a) === mark);
    return r >= 0 ? r + 1 : null;
  }
  private cpList(d: MockDoc): import("./backend").Checkpoint[] {
    const now = this.markNow(d);
    return (d.checkpoints ?? []).map((c) => {
      const r = this.reach(d, c.mark);
      const since_note =
        r === null
          ? "No longer reachable: the undo history was trimmed past it, a newer change replaced it, or the file was saved."
          : r === 0
            ? "The table is in this state now: nothing has changed since."
            : r > 0
              ? "This checkpoint is ahead of the table (you went back past it): go to it first, then compare."
              : "";
      return { id: c.id, name: c.name, auto: c.auto, reachable: r !== null, steps: r ?? 0, current: c.mark === now, since_note };
    });
  }
  async checkpointList(doc: number): Promise<import("./backend").Checkpoint[]> {
    return this.cpList(this.get(doc));
  }
  async checkpointAdd(doc: number, name: string): Promise<import("./backend").Checkpoint[]> {
    const d = this.get(doc);
    const n = name.trim().slice(0, 100);
    if (!n) throw err("invalid", "give the checkpoint a name");
    const mark = this.markNow(d);
    const list = (d.checkpoints ??= []);
    const auto = list.find((c) => c.mark === mark && c.auto);
    if (auto) Object.assign(auto, { auto: false, name: n });
    else list.push({ id: ++markSeq, name: n, mark, auto: false, snap: new Map(d.edits), order: [...this.orderOf(d)], cols: [...this.colsOf(d)] });
    return this.cpList(d);
  }
  /** What differs now from a checkpoint (the stand-in compares the edits, rows and columns it kept). */
  private sinceView(d: MockDoc, checkpoint: number, header: boolean): import("./backend").SinceView {
    const c = (d.checkpoints ?? []).find((x) => x.id === checkpoint);
    if (!c) throw err("invalid", "there is no such checkpoint");
    const r = this.reach(d, c.mark);
    if (r === null) throw err("invalid", "That checkpoint can no longer be reached: the undo history was trimmed past it, a newer change replaced it, or the file was saved.");
    if (r > 0) throw err("invalid", "That checkpoint is ahead of the table (you went back past it): go to it first, then compare.");
    const order = this.orderOf(d);
    const cols = this.colsOf(d);
    const thenOrder = c.order ?? order;
    const thenCols = c.cols ?? cols;
    const value = (edits: Map<string, string> | undefined, id: number, p: number) => edits?.get(`${id}:${p}`) ?? (id >= 0 && p < d.baseCols ? generated(id, p) : "");
    const nowSet = new Set(order);
    const thenSet = new Set(thenOrder);
    const kept = thenOrder.filter((id) => nowSet.has(id)).sort((a, b) => a - b);
    const line = (edits: Map<string, string> | undefined, id: number, map: number[]) => map.map((p) => value(edits, id, p)).join(" | ");
    type Line = import("./backend").SinceCell;
    const at = (id: number) => order.indexOf(id);
    const cells: Line[] = [];
    for (const id of kept) for (const p of cols.filter((x) => thenCols.includes(x))) {
      const old = value(c.snap, id, p);
      const now = value(d.edits, id, p);
      if (old !== now) cells.push({ kind: "cell", id, row: at(id), col: cols.indexOf(p), removed_col: null, old, new: now });
    }
    const gone: Line[] = thenOrder.filter((id) => !nowSet.has(id)).sort((a, b) => a - b).map((id) => ({ kind: "row_removed", id, row: null, col: null, removed_col: null, old: line(c.snap, id, thenCols), new: "(taken out)" }));
    const made: Line[] = order.filter((id) => !thenSet.has(id)).sort((a, b) => a - b).map((id) => ({ kind: "row_added", id, row: at(id), col: null, removed_col: null, old: "(new)", new: line(d.edits, id, cols) }));
    const removed_cols = thenCols.map((p, i) => ({ at: i, p })).filter((x) => !cols.includes(x.p)).map((x) => ({ at: x.at, p: x.p, title: header ? value(c.snap, thenOrder[0]!, x.p) : "" }));
    const lost: Line[] = removed_cols.flatMap((x, i) => kept.filter((id) => !(header && id === thenOrder[0])).map((id): Line => ({ kind: "col_removed", id, row: at(id), col: null, removed_col: i, old: value(c.snap, id, x.p), new: "(column taken out)" })));
    const all = [...cells, ...gone, ...made, ...lost];
    const added_cols = cols.map((p, i) => ({ p, i })).filter((x) => !thenCols.includes(x.p)).map((x) => x.i);
    const a = thenCols.filter((p) => cols.includes(p));
    const b = cols.filter((p) => thenCols.includes(p));
    const moved_cols = b.filter((p, i) => a[i] !== p).map((p) => cols.indexOf(p));
    return {
      totals: {
        steps: -r,
        rows_changed: new Set(cells.map((x) => x.id)).size,
        cells_changed: cells.length,
        rows_added: made.length,
        rows_removed: gone.length,
        cols_added: added_cols.length,
        cols_removed: removed_cols.length,
        cols_moved: moved_cols.length > 0,
        reordered: thenOrder.filter((id) => nowSet.has(id)).join() !== order.filter((id) => thenSet.has(id)).join(),
        added_cols,
        removed_cols: removed_cols.map((x) => ({ at: x.at, title: x.title })),
        moved_cols,
        entries: all.length,
      },
      cells: all,
    };
  }
  async checkpointSince(doc: number, checkpoint: number, header: boolean, offset: number, want: number): Promise<import("./backend").SinceView> {
    const v = this.sinceView(this.get(doc), checkpoint, header);
    return { totals: v.totals, cells: v.cells.slice(offset, offset + want) };
  }
  async checkpointSinceReport(doc: number, checkpoint: number, header: boolean, guard = true): Promise<Written | null> {
    this.calls.push(`checkpointSinceReport:guard=${guard}`);
    return { entries: this.sinceView(this.get(doc), checkpoint, header).totals.entries, guarded: guard ? 2 : 0 };
  }
  async checkpointRemove(doc: number, checkpoint: number): Promise<import("./backend").Checkpoint[]> {
    const d = this.get(doc);
    d.checkpoints = (d.checkpoints ?? []).filter((c) => c.id !== checkpoint);
    return this.cpList(d);
  }
  async checkpointRestore(doc: number, checkpoint: number): Promise<DocInfo> {
    const d = this.get(doc);
    const c = (d.checkpoints ?? []).find((x) => x.id === checkpoint);
    if (!c) throw err("invalid", "there is no such checkpoint");
    const leaving = this.markNow(d);
    const n = this.reach(d, c.mark);
    if (n === null) throw err("invalid", "that checkpoint can no longer be reached: the undo history was trimmed past it, a newer change replaced it, or the file was saved");
    for (let i = 0; i < Math.abs(n); i++) {
      if (n < 0) {
        const step = d.undo.pop();
        if (step) {
          step.inverse();
          d.redo.push(step.apply);
        }
      } else {
        const apply = d.redo.pop();
        if (apply) d.undo.push({ apply, inverse: apply() });
      }
    }
    d.info.peeking = false;
    if (leaving !== c.mark && !(d.checkpoints ?? []).some((x) => x.mark === leaving)) d.checkpoints!.push({ id: ++markSeq, name: `Before restoring \u201c${c.name}\u201d`, mark: leaving, auto: true });
    return this.refresh(d);
  }
  async stepsGoto(doc: number, k: number | null): Promise<DocInfo> {
    const s = this.stepsOf(doc);
    s.at = k === null ? 0 : Math.min(k + 1, s.rows.length);
    const d = this.get(doc);
    d.info.peeking = s.at < s.rows.length;
    return { ...d.info };
  }
  async stepsChange(doc: number, k: number, change: import("./recipe").StepChange, _now: number, force: boolean): Promise<DocInfo> {
    const s = this.stepsOf(doc);
    const r = s.rows;
    if (!force && r.slice(k).some((x) => x.kind === "left_out")) throw err("steps_lost", `going back would undo step ${r.findIndex((x) => x.kind === "left_out") + 1} (a paste) cannot be done again`);
    if (change.kind === "remove") r.splice(k, 1);
    else if (change.kind === "toggle") r[k]!.kind = r[k]!.kind === "off" ? "step" : "off";
    else if (change.kind === "move_up" && k > 0) [r[k - 1], r[k]] = [r[k]!, r[k - 1]!];
    else if (change.kind === "move_down" && k + 1 < r.length) [r[k + 1], r[k]] = [r[k]!, r[k + 1]!];
    else if (change.kind === "replace") r[k]!.op = change.op;
    s.at = r.length;
    return { ...this.get(doc).info };
  }
  async stepsAsk(doc: number, k: number, path: string, name: string, prompt: string): Promise<import("./recipe").StepsView> {
    const s = this.stepsOf(doc);
    const row = s.rows[k]!;
    const t = row.texts.find(([p]) => p === path);
    if (!t) throw err("invalid", "that value is not in the step");
    s.questions.push({ name, prompt, default: t[1] });
    t[1] = `{{${name}}}`;
    return this.stepsView(doc);
  }
  private mockBatch: import("./batch").BatchStatus = { files: [], recipe: null, steps: [], questions: [], out: null, running: false, done: 0, failed: 0, current: null, results: [] };
  async batchPick(which: "files" | "folder" | "recipe" | "out"): Promise<import("./batch").BatchStatus> {
    const b = this.mockBatch;
    if (which === "files" || which === "folder") b.files = ["C:\\Data\\Hamilton.csv", "C:\\Data\\Knox.csv", "C:\\Data\\Bradley.csv"];
    if (which === "recipe") {
      b.recipe = "C:\\Data\\monthly.recipe.json";
      b.steps = ["Delete 1 column(s) from column 3", 'Replace "TBD" with {{county}}'];
      b.questions = [{ name: "county", prompt: "Which county?", default: "Hamilton" }];
    }
    if (which === "out") b.out = "C:\\Data\\Results";
    return { ...b };
  }
  async batchStatus(): Promise<import("./batch").BatchStatus> {
    return { ...this.mockBatch };
  }
  async batchStart(_answers: [string, string][], _force: boolean, existing?: "replace" | "number"): Promise<import("./batch").BatchStatus> {
    const b = this.mockBatch;
    // Like the app: results already in the folder are replaced only when asked.
    const there = b.results.filter((r) => r.ok).length;
    if (there && !existing) throw err("outputs_exist", `${there} files in ${b.out} already have these names`);
    b.results = b.files.map((f, i) => {
      const name = f.split("\\").pop() ?? f;
      return i === 2 ? { name, ok: false, rows_in: 0, rows_out: 0, message: 'its columns differ from the recipe\'s (column 2 is "Town", not "county")' } : { name, ok: true, rows_in: 1200 + i * 310, rows_out: 1180 + i * 300, message: `written to ${b.out}\\${name}` };
    });
    b.done = b.results.filter((r) => r.ok).length;
    b.failed = b.results.length - b.done;
    return { ...b };
  }
  async batchStop(): Promise<import("./batch").BatchStatus> {
    return { ...this.mockBatch };
  }
  async labelSheets(): Promise<[string, import("./mail").LabelSheet][]> {
    const { DEFAULT_SHEET } = await import("./mail");
    return [
      ["Address labels, 1 x 2 5/8 in, 30 per sheet", DEFAULT_SHEET],
      ["Shipping labels, 2 x 4 in, 10 per sheet", { ...DEFAULT_SHEET, across: 2, down: 5, width: 4, height: 2, left: 0.15625, gap_across: 0.1875 }],
    ];
  }
  async mailPreview(doc: number, spec: import("./mail").MailSpec): Promise<string[][]> {
    const { fillTemplate, unknownFields } = await import("./mail");
    const d = this.get(doc);
    const n = this.rowCount(d);
    const titles = Array.from({ length: d.info.cols }, (_, c) => (spec.header && n ? this.value(d, 0, c) : `Column ${c + 1}`));
    const bad = unknownFields(spec.template, titles);
    if (bad.length) throw err("invalid", `there is no column called "${bad[0]}"`);
    const out: string[][] = [];
    for (let r = spec.header ? 1 : 0; r < n && out.length < 6; r++) {
      const row = Array.from({ length: d.info.cols }, (_, c) => this.value(d, r, c));
      out.push(fillTemplate(spec.template, titles, row, spec.skip_blank_lines));
    }
    return out;
  }
  async saveMail(doc: number, spec: import("./mail").MailSpec): Promise<import("./mail").MailReport | null> {
    const d = this.get(doc);
    const rows = Math.max(0, this.rowCount(d) - (spec.header ? 1 : 0));
    const per = spec.sheet.across * spec.sheet.down;
    return { rows, pages: spec.kind === "labels" ? Math.ceil((rows + spec.skip) / per) : rows, bytes: 0, cut: 0 };
  }
  async privateState(doc: number): Promise<PrivateState> {
    return this.privateDocs.get(this.privateKey(doc)) ?? { file: false, folder: false };
  }
  async setPrivate(doc: number, folder: boolean, on: boolean): Promise<PrivateState> {
    const s = { ...(this.privateDocs.get(this.privateKey(doc)) ?? { file: false, folder: false }) };
    if (folder) s.folder = on;
    else s.file = on;
    this.privateDocs.set(this.privateKey(doc), s);
    return s;
  }
  async privateLeftovers(_doc: number, _folder: boolean): Promise<[number, number]> {
    return [3, 1];
  }
  async privateForget(_doc: number, _folder: boolean): Promise<number> {
    return 4;
  }
  async forgetScrambleKey(): Promise<void> {}
  async joinClose(_doc: number): Promise<void> {
    this.joinLists.clear();
  }

  async joinPreview(doc: number, other: number | null, list: string | null, spec: JoinSpec, listId: number | null = null): Promise<JoinPreview> {
    if (listId !== null) list = this.joinLists.get(listId) ?? null;
    const mine = this.table(this.get(doc));
    const sim = simulateJoin(mine, list !== null ? null : this.table(this.get(other ?? -1)), spec, list);
    return previewOf(mine, sim, spec);
  }

  /** Shown rows as text (the stub joins only the first 5,000 rows). */
  private table(d: MockDoc): string[][] {
    const cols = this.colCount(d);
    return Array.from({ length: Math.min(this.rowCount(d), 5000) }, (_, r) => Array.from({ length: cols }, (_, c) => this.value(d, r, c)));
  }

  async facetForget(): Promise<void> {}
  async facetCounts(doc: number, cards: FacetCard[], spec: FilterSpec): Promise<Facets> {
    const d = this.get(doc);
    const first = spec.keep_first ? 1 : 0;
    const rows: string[][] = [];
    const cols = this.colCount(d);
    for (let r = first; r < this.totalRows(d); r++) rows.push(Array.from({ length: cols }, (_, c) => this.valueAt(d, r, c)));
    return stubFacets(rows, cards, spec, testRule);
  }

  async columnInsights(doc: number, spec: InsightSpec): Promise<Insights> {
    const d = this.get(doc);
    const first = spec.keep_first ? 1 : 0;
    const end = Math.min(this.rowCount(d), first + spec.sample);
    return stubInsights(spec.cols, (c) => {
      const out: string[] = [];
      for (let r = first; r < end; r++) out.push(this.value(d, r, c));
      return out;
    }, d.info.data_revision ?? 0);
  }

  async columnProfile(doc: number, spec: ProfileSpec): Promise<Profile> {
    const d = this.get(doc);
    const cells: string[] = [];
    for (let r = spec.keep_first ? 1 : 0; r < this.rowCount(d); r++) cells.push(this.value(d, r, spec.col));
    return stubProfile(spec.col, cells);
  }

  private formulas = new Map<string, [string, string]>();
  async cellFormula(doc: number, spec: CellFormula): Promise<FormulaReport> {
    const d = this.get(doc);
    const ncols = this.colCount(d);
    const nrows = this.rowCount(d);
    const cols = Array.from({ length: ncols }, (_, i) => i);
    const names = cols.map((c) => (spec.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
    const src = spec.formula.replace(/^\s*=/, "");
    const targets = spec.targets.length ? spec.targets : [[spec.row, spec.col] as [number, number]];
    if (targets.length > 100_000) throw err("invalid", "a formula fills at most 100000 cells at once; for a whole column use Data › Computed column");
    const at = (r: number, c: number) => this.value(d, r, c);
    const probe = rewriteA1(src, [spec.row, spec.col], [spec.row, spec.col], [nrows, ncols], at);
    if (!probe) throw err("invalid", "a cell reference in the formula is outside the table");
    try {
      mockEval(probe.run, names, cols.map((c) => at(spec.row, c)));
    } catch (e) {
      throw err("invalid", e instanceof Error ? e.message : String(e));
    }
    const cells: [number, number, string][] = [];
    const kept: string[] = [];
    let errors = 0;
    let first: string | null = null;
    for (const [r, c] of targets) {
      const w = rewriteA1(src, [spec.row, spec.col], [r, c], [nrows, ncols], at);
      const v = w ? mockEval(w.run, names, cols.map((k) => at(r, k))) : "#REF!";
      if (v.startsWith("#")) {
        errors++;
        first ??= v;
        continue;
      }
      cells.push([r, c, v]);
      kept.push(`=${w!.keep}`);
    }
    const info = cells.length ? this.writeCells(doc, cells) : this.refresh(d);
    cells.forEach(([r, c, v], i) => this.formulas.set(`${doc}:${r}:${c}`, [kept[i]!, v]));
    return { info, written: cells.length, errors, first_error: first };
  }
  async cellFormulaOf(doc: number, row: number, col: number): Promise<string | null> {
    const f = this.formulas.get(`${doc}:${row}:${col}`);
    return f && this.value(this.get(doc), row, col) === f[1] ? f[0] : null;
  }

  async computePreview(doc: number, spec: ComputeSpec): Promise<ComputePreview> {
    const d = this.get(doc);
    const cols = Array.from({ length: this.colCount(d) }, (_, i) => i);
    const names = cols.map((c) => (spec.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
    const first = spec.keep_first ? 1 : 0;
    const out: ComputePreview = { rows: [], errors: 0 };
    try {
      for (let r = first; r < Math.min(this.rowCount(d), first + 20); r++) {
        const v = mockEval(spec.expr, names, cols.map((c) => this.value(d, r, c)));
        if (v.startsWith("#")) out.errors++;
        out.rows.push([r + 1, v]);
      }
    } catch (e) {
      throw err("invalid", e instanceof Error ? e.message : String(e));
    }
    return out;
  }
  private checkFails(d: MockDoc, c: Check, r: number, names: string[], seen: Map<string, number>): boolean {
    const k = c.kind;
    if (k.type === "expression") {
      const cols = Array.from({ length: this.colCount(d) }, (_, i) => i);
      try {
        return mockEval(k.expr, names, cols.map((x) => this.value(d, r, x))) !== "TRUE";
      } catch (e) {
        throw err("invalid", e instanceof Error ? e.message : String(e));
      }
    }
    const v = this.value(d, r, c.col ?? 0).trim();
    if (!v) return k.type === "required";
    switch (k.type) {
      case "number": {
        const n = numberOf(v);
        return n === null || (k.min !== null && n < k.min) || (k.max !== null && n > k.max) || (k.whole && !Number.isInteger(n));
      }
      case "date_iso":
        return !isIsoDate(v);
      case "date":
        return Number.isNaN(Date.parse(v)) || (!!k.min && v < k.min) || (!!k.max && v > k.max);
      case "length": {
        const n = Array.from(v).length;
        return (k.min !== null && n < k.min) || (k.max !== null && n > k.max);
      }
      case "list":
        return !k.values.some((x) => (k.match_case ? x === v : x.toLowerCase() === v.toLowerCase()));
      case "pattern":
        try {
          return !new RegExp(`^(?:${k.pattern.text})$`, k.pattern.match_case ? "u" : "iu").test(v);
        } catch {
          throw err("bad_pattern", "the pattern is not valid");
        }
      case "unique": {
        const key = k.match_case ? v : v.toLowerCase();
        return (seen.get(key) ?? 0) > 1;
      }
      default:
        return false;
    }
  }
  private names(d: MockDoc, header: boolean): string[] {
    return Array.from({ length: this.colCount(d) }, (_, c) => (header ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
  }
  private counts(d: MockDoc, spec: CheckSpec): Map<string, number>[] {
    return spec.checks.map((c) => {
      const m = new Map<string, number>();
      if (c.kind.type !== "unique") return m;
      for (let r = spec.keep_first ? 1 : 0; r < this.rowCount(d); r++) {
        const v = this.value(d, r, c.col ?? 0).trim();
        if (!v) continue;
        const key = c.kind.match_case ? v : v.toLowerCase();
        m.set(key, (m.get(key) ?? 0) + 1);
      }
      return m;
    });
  }
  async runChecks(doc: number, spec: CheckSpec): Promise<CheckReport> {
    const d = this.get(doc);
    const rows = this.rowCount(d);
    if (rows > 200_000) throw err("too_large", "the browser preview checks at most 200,000 rows");
    const names = this.names(d, spec.keep_first);
    const counts = this.counts(d, spec);
    const rep: CheckReport = { checked: 0, rows: 0, problems: 0, per_check: spec.checks.map(() => 0), listed: [] };
    for (let r = spec.keep_first ? 1 : 0; r < rows; r++) {
      rep.checked++;
      let bad = false;
      spec.checks.forEach((c, i) => {
        if (!this.checkFails(d, c, r, names, counts[i]!)) return;
        bad = true;
        rep.problems++;
        rep.per_check[i]!++;
        if (rep.listed.length < 1000) rep.listed.push({ row: r, col: c.kind.type === "expression" ? null : c.col, check: i, value: c.kind.type === "expression" ? "" : this.value(d, r, c.col ?? 0) });
      });
      if (bad) rep.rows++;
    }
    return rep;
  }
  async checkMarks(doc: number, row: number, rows: number, col: number, cols: number, spec: CheckSpec): Promise<CheckMark[]> {
    const d = this.get(doc);
    const names = this.names(d, spec.keep_first);
    const counts = this.counts(d, spec);
    const out: CheckMark[] = [];
    for (let r = Math.max(row, spec.keep_first ? 1 : 0); r < Math.min(this.rowCount(d), row + rows); r++) {
      spec.checks.forEach((c, i) => {
        const at = c.kind.type === "expression" ? null : c.col;
        if (at !== null && (at < col || at >= col + cols)) return;
        if (this.checkFails(d, c, r, names, counts[i]!)) out.push({ row: r, col: at, check: i });
      });
    }
    return out;
  }
  /** Rules kept beside a file, by document. */
  sidecars = new Map<number, { checks: Check[]; keep_first: boolean }>();
  async nextProblem(doc: number, row: number, col: number, backward: boolean, skipCols: number[], spec: CheckSpec): Promise<JumpResult> {
    this.calls.push("nextProblem");
    const d = this.get(doc);
    const n = this.rowCount(d);
    const marks = await this.checkMarks(doc, 0, n, 0, this.colCount(d), spec);
    const first = Array.from({ length: this.colCount(d) }, (_, i) => i).find((i) => !skipCols.includes(i));
    let hidden = 0;
    const cells: [number, number, number, boolean][] = [];
    for (const m of marks) {
      if (m.col !== null && skipCols.includes(m.col)) {
        hidden++;
        continue;
      }
      if (m.col === null && first === undefined) continue;
      cells.push([m.row, m.col ?? first!, m.check, m.col === null]);
    }
    cells.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
    const uniq = cells.filter((c, i) => i === 0 || c[0] !== cells[i - 1]![0] || c[1] !== cells[i - 1]![1]);
    const after = backward ? [...uniq].reverse().find((c) => c[0] < row || (c[0] === row && c[1] < col)) : uniq.find((c) => c[0] > row || (c[0] === row && c[1] > col));
    const hit = after ?? (backward ? uniq.at(-1) : uniq[0]);
    return { hit: hit ? { row: hit[0], col: hit[1], check: hit[2], row_rule: hit[3] } : null, wrapped: !after && !!hit, hidden_skipped: hidden };
  }
  async suggestRules(doc: number, col: number, keepFirst: boolean): Promise<RuleSuggestion[]> {
    this.calls.push("suggestRules");
    const d = this.get(doc);
    const vals: string[] = [];
    for (let r = keepFirst ? 1 : 0; r < this.rowCount(d); r++) vals.push(this.value(d, r, col));
    const filled = vals.filter((v) => v.trim());
    const out: RuleSuggestion[] = [];
    if (filled.length < 5) return out;
    if (filled.length === vals.length) out.push({ check: { col, kind: { type: "required" } }, why: `none of the ${vals.length} values looked at is blank`, share: 1 });
    const shapes = new Map<string, number>();
    for (const v of filled) shapes.set(shapeOf(v), (shapes.get(shapeOf(v)) ?? 0) + 1);
    const top = [...shapes].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))[0]!;
    if (top[1] / filled.length >= 0.99 && !top[0].endsWith("…") && /[9A]/.test(top[0])) {
      if (top[0] === "9999-99-99" && filled.every(isIsoDate)) out.push({ check: { col, name: "Date written YYYY-MM-DD", kind: { type: "date_iso" } }, why: `${filled.length} of ${filled.length} filled values are real dates written year-month-day`, share: 1 });
      else out.push({ check: { col, name: `Matches ${top[0]}`, kind: { type: "pattern", pattern: { text: shapeRegex(top[0]), kind: "regex", match_case: true } as never } }, why: `${top[1]} of ${filled.length} filled values are written ${top[0]}`, share: top[1] / filled.length });
    }
    if (new Set(filled).size === filled.length && filled.length >= 20) out.push({ check: { col, kind: { type: "unique", match_case: false } }, why: `all ${filled.length} filled values looked at are different`, share: 1 });
    return out;
  }
  async checksSidecarLoad(doc: number): Promise<SidecarRules | null> {
    this.calls.push("checksSidecarLoad");
    const s = this.sidecars.get(doc);
    return s ? { checks: s.checks, skipped: 0, keep_first: s.keep_first, file: "mock.csv.ndchecks" } : null;
  }
  async checksSidecarKeep(doc: number, checks: Check[], keepFirst: boolean): Promise<string> {
    this.calls.push("checksSidecarKeep");
    if (checks.length) this.sidecars.set(doc, { checks, keep_first: keepFirst });
    else this.sidecars.delete(doc);
    return "mock.csv.ndchecks";
  }
  async checksSave(): Promise<string | null> {
    return "checks.json";
  }
  async checksOpen(): Promise<Check[] | null> {
    return [{ col: 0, kind: { type: "required" } }];
  }
  // Files from Windows (tests): queued, then the window is told.
  private osQueue: string[] = [];
  private osListener: (() => void) | null = null;
  /** Tests: Windows passes `path` to the running app. */
  osOpen(path: string): void {
    this.osQueue.push(path);
    this.osListener?.();
  }
  async takeOpened(): Promise<OsOpen[]> {
    this.calls.push("takeOpened");
    return this.osQueue.splice(0).map((path) => {
      const name = path.split("/").at(-1)!;
      const open = this.openAt(path);
      if (open !== null) return { name, opened: null, existing: open, error: null };
      const info = this.make(name, 2_001, 6);
      this.get(info.id).info.path = path;
      return { name, opened: { doc: { ...this.get(info.id).info }, import: null }, existing: null, error: null };
    });
  }
  onOpened(f?: () => void): void {
    this.osListener = f ?? null;
  }
  onAgentGoto(): void {}
  private types: FileType[] = [
    ...["csv", "tsv", "tab", "psv", "ssv"].map((ext) => ({ ext, on: false, text: false })),
    ...["txt", "dat", "log", "prn"].map((ext) => ({ ext, on: false, text: true })),
  ];
  async fileTypes(): Promise<FileType[]> {
    return this.types.map((t) => ({ ...t }));
  }
  async setFileTypes(on: string[]): Promise<FileType[]> {
    this.types = this.types.map((t) => ({ ...t, on: on.includes(t.ext) }));
    return this.fileTypes();
  }
  async openDefaultApps(): Promise<void> {}
  private agent: AgentStatus = { on: false, port: null };
  async agentStatus(): Promise<AgentStatus> {
    return { ...this.agent };
  }
  async setAgentLink(on: boolean): Promise<AgentStatus> {
    this.calls.push("setAgentLink");
    this.agent = { on, port: on ? 51234 : null };
    return { ...this.agent };
  }
  async logEvent(): Promise<void> {}
  async openLogFolder(): Promise<void> {
    throw err("invalid", "the browser preview has no log folder; the desktop app does");
  }
}
