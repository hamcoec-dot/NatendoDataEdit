// Browser stand-in for the Rust backend: a generated document of a million
// rows with edits, row/column changes and undo. Used by `vite dev` outside
// the desktop shell and by tests. It is not the engine; it only has to
// behave the same at the command level.

import { mockDrill, mockItems, mockPivot, type ItemList, type PivotField, type PivotGrid, type PivotSpec } from "./pivot";
import type {
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
import type { Finding, PrivateKind, PrivateState, SafeCopy, SafeCopyReport, SplitReport, TableReport, Totals } from "./backend";
import type { Autosaved, RecoveryItem, Suggestions, VersionItem } from "./m9";
import type { AgentStatus, Check, CheckMark, CheckReport, CheckSpec, ComputePreview, ComputeSpec, FileType, JoinPreview, JoinSpec, OsOpen, TallySpec, ValueTally } from "./backend";
import { guessStarts, LOG_PRESETS, splitFixed, toJsPattern } from "./m8";
import { mockEval } from "./mockexpr";
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

interface MockDoc {
  info: DocInfo;
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
function regexOf(p: Pattern, global = false): RegExp {
  let body: string;
  if (p.kind === "regex") body = p.text;
  else {
    const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    if (p.kind === "plain") body = esc(p.text);
    else
      body = p.text
        .split(/(~[*?~]|\*|\?)/)
        .map((t) => (t === "*" ? "[\\s\\S]*" : t === "?" ? "[\\s\\S]" : t.startsWith("~") && t.length === 2 ? esc(t[1]!) : esc(t)))
        .join("");
  }
  if (p.whole_cell) body = `^(?:${body})$`;
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
    return { rows, sheets: o.format === "xlsx" ? Math.max(1, Math.ceil(rows / 1_048_575)) : 0, bytes: rows * 40 };
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
  async newDocument(): Promise<DocInfo> {
    return this.make("Untitled", 1, 1);
  }
  async closeDocument(doc: number): Promise<void> {
    this.docs.delete(doc);
  }
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
    d.info.modified = d.undo.length !== d.clean;
    d.info.column_changes = d.cols !== null;
    return { ...d.info };
  }

  private push(d: MockDoc, apply: () => () => void): DocInfo {
    const inverse = apply();
    d.undo.push({ apply, inverse });
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

  async setCells(doc: number, cells: [number, number, string][]): Promise<DocInfo> {
    const d = this.get(doc);
    if (cells.length > 100_000) throw err("too_large", "one change is limited to 100,000 cells");
    for (const [r] of cells) if (r >= this.rowCount(d)) throw err("out_of_range", "row or column out of range");
    const widest = Math.max(0, ...cells.map((c) => c[1] + 1));
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

  async fillCells(doc: number, row: number, rows: number, col: number, cols: number, value: string) {
    if (rows * cols > 100_000) throw err("too_large", "one change is limited to 100,000 cells");
    const cells: [number, number, string][] = [];
    for (let r = row; r < row + rows; r++) for (let c = col; c < col + cols; c++) cells.push([r, c, value]);
    return this.setCells(doc, cells);
  }

  async copyCells(doc: number, row: number, rows: number, col: number, cols: number) {
    const d = this.get(doc);
    if (rows * cols > 1_000_000) throw err("too_large", "copying is limited to 1,000,000 cells at once");
    const out: string[][] = [];
    for (let r = row; r < Math.min(row + rows, this.rowCount(d)); r++) {
      const line: string[] = [];
      for (let c = col; c < col + cols; c++) line.push(this.value(d, r, c));
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
    const d = this.get(doc);
    if (!d.info.path && !f.saveAs && d.info.name === "Untitled") {
      d.info.name = "Untitled.csv";
    }
    // Like the engine: saving clears the undo history.
    d.undo = [];
    d.redo = [];
    d.clean = 0;
    return this.refresh(d);
  }

  async jobProgress(): Promise<JobInfo | null> {
    return null;
  }
  async cancelJob(): Promise<void> {}

  async sortRows(doc: number, spec: SortSpec): Promise<DocInfo> {
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
    d.shown = shown;
    d.filter = spec;
    return this.refresh(d);
  }

  async clearFilter(doc: number): Promise<DocInfo> {
    const d = this.get(doc);
    d.shown = null;
    d.filter = null;
    return this.refresh(d);
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
  private area(d: MockDoc, spec: FindSpec): { r0: number; r1: number; cols: number[] } {
    const n = this.rowCount(d);
    const first = spec.skip_first && n > 0 ? 1 : 0;
    const all = Array.from({ length: this.colCount(d) }, (_, i) => i);
    switch (spec.scope.kind) {
      case "all":
        return { r0: first, r1: n, cols: all };
      case "columns":
        return { r0: first, r1: n, cols: spec.scope.cols.filter((c) => c < all.length) };
      case "range":
        return {
          r0: spec.scope.row,
          r1: Math.min(n, spec.scope.row + spec.scope.rows),
          cols: all.slice(spec.scope.col, spec.scope.col + spec.scope.cols),
        };
    }
  }

  async findNext(doc: number, spec: FindSpec, from: [number, number] | null, backward: boolean): Promise<Found | null> {
    const d = this.get(doc);
    if (!spec.pattern.text) throw err("bad_pattern", "the pattern is not valid: enter something to look for");
    const re = regexOf(spec.pattern);
    const { r0, r1, cols } = this.area(d, spec);
    const cells: [number, number][] = [];
    // Row-major list is too big for a million rows; walk instead.
    const total = (r1 - r0) * cols.length;
    if (total <= 0) return null;
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

  async findAll(doc: number, spec: FindSpec): Promise<FindAll> {
    const d = this.get(doc);
    const re = regexOf(spec.pattern);
    const { r0, r1, cols } = this.area(d, spec);
    const hits: FindAll["hits"] = [];
    let cells = 0;
    for (let r = r0; r < r1; r++)
      for (const c of cols) {
        const v = this.value(d, r, c);
        const m = re.exec(v);
        if (!m) continue;
        cells++;
        if (hits.length < 10_000) hits.push({ row: r, number: this.pos(d, r), col: c, text: v.replace(/[\r\n\t]/g, " ").slice(0, 200), start: [...v.slice(0, m.index)].length, len: [...m[0]].length });
      }
    return { hits, cells };
  }

  private planReplace(d: MockDoc, spec: FindSpec, rep: string) {
    const re = regexOf(spec.pattern, true);
    const { r0, r1, cols } = this.area(d, spec);
    const changes: [number, number, string][] = [];
    const preview: ReplaceReport["preview"] = [];
    let hits = 0;
    for (let r = r0; r < r1; r++)
      for (const c of cols) {
        const v = this.value(d, r, c);
        let n = 0;
        const out = v.replace(re, (m, ...rest) => {
          if (m === "" && v !== "") return m;
          n++;
          return spec.pattern.kind === "regex" ? rep.replace(/\$(\d)/g, (_, g) => String(rest[Number(g) - 1] ?? "")) : rep;
        });
        if (n && out !== v) {
          hits += n;
          changes.push([r, c, out]);
          if (preview.length < 50) preview.push({ row: r, number: this.pos(d, r), col: c, before: v, after: out });
        }
      }
    return { changes, preview, hits };
  }

  async replacePreview(doc: number, spec: FindSpec, replacement: string): Promise<ReplaceReport> {
    const p = this.planReplace(this.get(doc), spec, replacement);
    return { cells: p.changes.length, hits: p.hits, preview: p.preview, info: null };
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

  private planClean(d: MockDoc, spec: CleanSpec, maxRows: number) {
    const { r0, r1, cols } = this.area(d, { pattern: { text: "", kind: "plain", match_case: false, whole_cell: false }, scope: spec.scope, skip_first: spec.skip_first });
    const changes: [number, number, string][] = [];
    const preview: CleanPreview["changes"] = [];
    const end = Math.min(r1, r0 + maxRows);
    for (let r = r0; r < end; r++)
      for (const c of cols) {
        const v = this.value(d, r, c);
        const out = runCleaner(spec.cleaner, v, r - r0);
        if (out === v) continue;
        changes.push([r, c, out]);
        if (preview.length < 50) preview.push({ row: r, number: this.pos(d, r), col: c, before: v, after: out });
      }
    return { changes, preview, rows: end - r0, more: end < r1 };
  }

  async cleanPreview(doc: number, spec: CleanSpec, now: number): Promise<CleanPreview> {
    void now;
    const p = this.planClean(this.get(doc), spec, 20_000);
    return { changes: p.preview, rows: p.rows, changed: p.changes.length, left: 0, more: p.more };
  }

  private savedCleaner: Cleaner | null = null;

  async cleanerSave(cleaner: Cleaner): Promise<string | null> {
    this.savedCleaner = JSON.parse(JSON.stringify(cleaner)) as Cleaner;
    return `${cleaner.name || "cleaner"}.ndclean`;
  }

  // ---- Milestone 7 (simple stand-ins) ----

  private savedView: View | null = null;

  async recentFiles(): Promise<RecentItem[]> {
    return [{ index: 0, name: "sample-1M.csv", folder: "samples" }];
  }

  async openRecent(index: number): Promise<Opened> {
    void index;
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
    return this.openRecent(index);
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
    return [];
  }
  async recoveryOpen(): Promise<DocInfo> {
    throw err("bad_request", "list the recovery copies again");
  }
  async recoveryDiscard(): Promise<void> {}
  async versionsKeep(): Promise<void> {}
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
        const x = numberOf(v);
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

  async cleanerOpen(): Promise<Cleaner | null> {
    return this.savedCleaner ?? { version: 1, name: "Tidy names", steps: [{ tool: "spaces", how: "collapse" }, { tool: "case", to: "title" }] };
  }

  async dataOp(doc: number, op: DataOp): Promise<OpReport> {
    const d = this.get(doc);
    const rows = this.rowCount(d);
    const allCols = Array.from({ length: this.colCount(d) }, (_, i) => i);
    switch (op.op) {
      case "clean": {
        const p = this.planClean(d, op.spec, 100_000);
        if (p.more) throw err("too_large", "the browser preview cleans at most 100,000 rows");
        const info = p.changes.length ? await this.setCells(doc, p.changes) : this.refresh(d);
        return this.report(info, { cells: p.changes.length, left: 0 });
      }
      case "compute": {
        const names = allCols.map((c) => (op.spec.keep_first ? this.value(d, 0, c) : "") || `Column ${c + 1}`);
        const first = op.spec.keep_first ? 1 : 0;
        if (rows > 100_000) throw err("too_large", "the browser preview computes at most 100,000 rows");
        const vals: string[] = [];
        let errors = 0;
        for (let r = first; r < rows; r++) {
          const v = mockEval(op.spec.expr, names, allCols.map((c) => this.value(d, r, c)));
          if (v.startsWith("#")) errors++;
          vals.push(v);
        }
        let col = op.spec.target;
        if (col === null) {
          col = op.spec.at ?? allCols.length;
          await this.change(doc, { op: "insert_cols", at: col, count: 1 });
        }
        const cells: [number, number, string][] = vals.map((v, k) => [k + first, col, v]);
        if (op.spec.target === null && first) cells.push([0, col, op.spec.title || "Computed"]);
        const info = await this.setCells(doc, cells);
        return this.report(info, { cells: cells.length, cols: op.spec.target === null ? 1 : 0, errors });
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
        const need = op.spec.row + g.length - rows;
        if (need > 0) await this.change(doc, { op: "insert_rows", at: rows, count: need });
        const cells: [number, number, string][] = [];
        let skipped = 0;
        g.forEach((line, i) =>
          line.forEach((v, j) => {
            if (op.spec.skip_blanks && v === "") return;
            const r = op.spec.row + i;
            const c = op.spec.col + j;
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
        return this.report(info, { cells: cells.length, skipped });
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
        d.shown = keep;
        d.filter = { columns: [], keep_first: s.keep_first };
        return this.report(this.refresh(d), { rows: dupRows.length + leaders.size, groups: leaders.size });
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

  async tableReport(doc: number, op: TableOp): Promise<TableReport> {
    return { info: await this.newTable(doc, op), compare: null };
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

  async splitToFiles(): Promise<SplitReport | null> {
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
    return this.tableDoc(this.pivotLines(doc, spec));
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

  async pivotOpen(): Promise<[string, PivotSpec] | null> {
    throw err("invalid", "the browser preview cannot open files; the desktop app does");
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

  async quitApp(): Promise<void> {}
  onCloseRequested(): void {}

  async valueTally(doc: number, spec: TallySpec): Promise<ValueTally> {
    const d = this.get(doc);
    const counts = new Map<string, number>();
    let blanks = 0;
    const rows = Math.min(this.rowCount(d), 100_000);
    for (let r = spec.keep_first ? 1 : 0; r < rows; r++) {
      const v = this.value(d, r, spec.col);
      if (!v) blanks++;
      else counts.set(v, (counts.get(v) ?? 0) + 1);
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
    return { values: values.slice(0, 100_000), blanks, rows: rows - (spec.keep_first ? 1 : 0), complete: values.length <= 100_000, look_alikes };
  }

  private joinLists = new Map<number, string>();
  async joinList(_doc: number, text: string): Promise<number> {
    const id = this.joinLists.size + 1;
    this.joinLists.set(id, text);
    return id;
  }
  async lookupClose(): Promise<void> {}
  private privateDocs = new Map<number, PrivateState>();
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
    return { rows: Math.max(0, n - (spec.header ? 1 : 0)), cols: kept, changed: 0, bytes: 0 };
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
    return this.openRecent(0);
  }
  async saveChangeReport(doc: number): Promise<number | null> {
    const l = await this.changeList(doc);
    return l.cells_total + l.added_total + l.deleted_total;
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
  async sqliteOpen(): Promise<DocInfo> {
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
    return this.privateDocs.get(doc) ?? { file: false, folder: false };
  }
  async setPrivate(doc: number, folder: boolean, on: boolean): Promise<PrivateState> {
    const s = { ...(this.privateDocs.get(doc) ?? { file: false, folder: false }) };
    if (folder) s.folder = on;
    else s.file = on;
    this.privateDocs.set(doc, s);
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
  async checksSave(): Promise<string | null> {
    return "checks.json";
  }
  async checksOpen(): Promise<Check[] | null> {
    return [{ col: 0, kind: { type: "required" } }];
  }
  async takeOpened(): Promise<OsOpen[]> {
    return [];
  }
  onOpened(): void {}
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
    this.agent = { on, port: on ? 51234 : null };
    return { ...this.agent };
  }
  async logEvent(): Promise<void> {}
  async openLogFolder(): Promise<void> {
    throw err("invalid", "the browser preview has no log folder; the desktop app does");
  }
}
