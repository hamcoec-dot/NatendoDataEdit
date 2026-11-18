// Defaults and small helpers for the Save As options, Export, Import and
// Join/Append screens. The engine does the real work; these only fill in
// the screens and show previews.

import type { DocInfo, ExportOptions, ImportFormat, JoinReport, JoinSpec, Quoting, SaveAsOptions } from "./backend";
import { joinCounts } from "./joins";

/** Another open table, for join and append. */
export interface OtherTable {
  id: number;
  name: string;
  header: boolean;
  cols: number;
}

/** Encodings offered on Save As: engine names with a readable label. */
export const ENCODINGS: [string, string][] = [
  ["UTF-8", "Unicode (UTF-8)"],
  ["UTF-16LE", "Unicode (UTF-16, little-endian)"],
  ["UTF-16BE", "Unicode (UTF-16, big-endian)"],
  ["UTF-32LE", "Unicode (UTF-32, little-endian)"],
  ["UTF-32BE", "Unicode (UTF-32, big-endian)"],
  ["windows-1252", "Western European (Windows-1252)"],
  ["ISO-8859-15", "Western European (ISO-8859-15)"],
  ["windows-1250", "Central European (Windows-1250)"],
  ["windows-1251", "Cyrillic (Windows-1251)"],
  ["windows-1253", "Greek (Windows-1253)"],
  ["windows-1254", "Turkish (Windows-1254)"],
  ["Shift_JIS", "Japanese (Shift_JIS)"],
  ["EUC-KR", "Korean (EUC-KR)"],
  ["GBK", "Chinese Simplified (GBK)"],
  ["Big5", "Chinese Traditional (Big5)"],
];

/** Separators offered by name; anything else is typed. */
export const SEPARATORS: [string, string][] = [
  [",", "Comma ,"],
  [";", "Semicolon ;"],
  ["\t", "Tab"],
  ["|", "Pipe |"],
  [" ", "Space"],
];

export const QUOTINGS: [Quoting, string, string][] = [
  ["keep", "Keep as is", "Rows you did not change are written exactly as they were; changed cells get quotes only where needed."],
  ["needed", "Only where needed", "Quotes only around values holding the separator, a quote or a line break."],
  ["all", "Every value", "Quotes around every value, empty ones too."],
  ["text", "Text, not numbers", "Quotes around every value except plain numbers and empty cells."],
];

/** Save As starts from the file's own format. */
export function saveAsDefaults(info: DocInfo): SaveAsOptions {
  const lb = info.line_break === "LF" || info.line_break === "CR" ? info.line_break : "CRLF";
  return {
    delimiter: info.delimiter || ",",
    quote: info.quote ?? '"',
    quoting: "keep",
    encoding: info.encoding || "UTF-8",
    bom: info.bom,
    line_break: lb,
    convert_line_breaks: false,
  };
}

/** True when the save keeps untouched rows byte for byte. */
export function keepsBytes(o: SaveAsOptions, info: DocInfo): boolean {
  return (
    o.quoting === "keep" &&
    !o.convert_line_breaks &&
    o.delimiter === info.delimiter &&
    (o.quote ?? null) === (info.quote ?? null) &&
    o.encoding.toLowerCase() === (info.encoding || "").toLowerCase()
  );
}

/** A plain number: optional sign, digits with at most one point or comma,
 *  optional exponent (the engine's rule for "text, not numbers"). */
export function plainNumber(v: string): boolean {
  return /^[+-]?(\d+([.,]\d*)?|[.,]\d+)([eE][+-]?\d+)?$/.test(v);
}

/** One row written as Save As would write a changed row. */
export function writeRow(cells: string[], o: SaveAsOptions): string {
  const q = o.quote || null;
  return cells
    .map((v) => {
      const needs = q !== null && (v.includes(o.delimiter) || v.includes(q) || v.includes("\n") || v.includes("\r"));
      const quote =
        q !== null &&
        (o.quoting === "all" ? true : o.quoting === "text" ? needs || !(v === "" || plainNumber(v)) : needs);
      return quote ? q + v.split(q).join(q + q) + q : v;
    })
    .join(o.delimiter);
}

/** A readable name for a separator. */
export function separatorLabel(s: string): string {
  return SEPARATORS.find((x) => x[0] === s)?.[1] ?? JSON.stringify(s);
}

export function exportDefaults(header: boolean, sheet = "Sheet1"): ExportOptions {
  return {
    format: "xlsx",
    header,
    arrays: false,
    pretty: true,
    root: "rows",
    record: "row",
    before: "",
    row: "",
    after: "",
    sheet,
    numbers: true,
    look: true,
    part_rows: 0,
    split_files: false,
    table: sheet.replace(/[^A-Za-z0-9_]+/g, "_").replace(/^_+|_+$/g, "") || "data",
    dialect: "standard",
    batch: 500,
    create: true,
  };
}

/** A starting template naming every column. */
export function templateFor(names: string[]): string {
  return names.map((n) => `${n}: {${n.replace(/[{}]/g, "")}}`).join(", ") + "\n";
}

export const FORMAT_NAMES: Record<ImportFormat, string> = {
  spreadsheet: "Spreadsheet (Excel .xlsx, OpenDocument .ods)",
  json: "JSON",
  xml: "XML",
  html: "Web page table (HTML)",
  markdown: "Markdown table",
  delimited: "Delimited text (CSV, tab, ...)",
  fixed_width: "Fixed width (columns lined up with spaces)",
  pattern: "Log file (lines cut by a pattern)",
};

const n = (x: number, one: string, many = `${one}s`) => `${x.toLocaleString()} ${x === 1 ? one : many}`;

/** What a join or append did, in words. */
export function joinSummary(kind: "join" | "append", r: JoinReport, spec?: Pick<JoinSpec, "repeats" | "keep_rows" | "key_match">): string {
  if (kind === "append") {
    let s = `Added ${n(r.added, "row")}`;
    if (r.cols) s += ` and ${n(r.cols, "new column")}`;
    if (r.dropped_cols) s += `; ${n(r.dropped_cols, "column")} with no matching name left out`;
    return s + ".";
  }
  return joinCounts(r, spec ?? { repeats: "first", keep_rows: "all", key_match: "exact" });
}
