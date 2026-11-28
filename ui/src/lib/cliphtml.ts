// The table a Copy also puts on the clipboard, for pasting into e-mail and Word (and read by Excel and Sheets in
// preference to the text). Values show as the grid shows them (the same display-format code), and Excel must not
// change what it gets: every cell that is not a plain number is marked as text (mso-number-format "\@"), so leading
// zeros, long IDs, "1-2" and values starting with = + - @ stay as they are (no dates, no formulas).

import { formatCell, type DisplayFormat } from "./m9";

/** Most cells put in the table (Outlook slows badly past this); above it only the text goes on the clipboard. */
export const MAX_HTML_CELLS = 20_000;

/** Characters a cell may hold in the table (the grid's edit limit). */
export const HTML_CELL_CHARS = 32_767;

/** A value Excel may keep as a number with no change: digits with an optional sign and point, no leading zero
 *  (except "0" itself or "0.5"), at most 15 significant digits (Excel's precision). */
export function excelKeepsAsNumber(s: string): boolean {
  if (!/^-?(0|[1-9]\d*)(\.\d+)?$/.test(s)) return false;
  return s.replace(/^-/, "").replace(".", "").replace(/^0+/, "").length <= 15;
}

/** Text as HTML: & < > " ' escaped, control characters other than tab and line break dropped, a line break as
 *  <br> (a tab stays a tab). */
export function escapeHtml(s: string): string {
  let out = "";
  for (const ch of s) {
    const c = ch.codePointAt(0)!;
    if (c === 0x0d) continue;
    if (c === 0x0a) out += "<br>";
    else if (c === 0x26) out += "&amp;";
    else if (c === 0x3c) out += "&lt;";
    else if (c === 0x3e) out += "&gt;";
    else if (c === 0x22) out += "&quot;";
    else if (c === 0x27) out += "&#39;";
    else if ((c < 0x20 && c !== 0x09) || c === 0x7f || (c >= 0x80 && c < 0xa0)) continue;
    else out += ch;
  }
  return out;
}

export interface HtmlTable {
  /** Column titles for a header row (Copy with column names, or the title row in the selection). */
  titles?: string[] | null;
  /** The rows, values as in the file. */
  rows: string[][];
  /** Each column's display format (undefined: none). */
  formats: (DisplayFormat | undefined)[];
  /** Columns set as Text or Id: always text, never numbers. */
  text: boolean[];
}

const BORDER = "border:1px solid #999;padding:2px 6px";
const AS_TEXT = "mso-number-format:'\\@'";

/** One cell: its shown value, and whether Excel may read it as a number (only a plain number in the file, shown
 *  plain or under a number, percent or currency format: nothing typed can become a formula or a date). */
export function htmlCell(value: string, f: DisplayFormat | undefined, textCol: boolean): { shown: string; number: boolean } {
  const shown = f ? formatCell(value, f) : value;
  const number = !textCol && excelKeepsAsNumber(value) && (!f || f.kind === "number" || f.kind === "percent" || f.kind === "currency");
  return { shown, number };
}

/** The whole document Windows puts on the clipboard as HTML. */
export function buildHtml(t: HtmlTable): string {
  const parts: string[] = ['<!DOCTYPE html><html><head><meta charset="utf-8"></head><body>', `<table style="border-collapse:collapse">`];
  if (t.titles) {
    parts.push("<tr>");
    for (const v of t.titles) parts.push(`<th style="${BORDER};${AS_TEXT};text-align:left">${escapeHtml(v)}</th>`);
    parts.push("</tr>");
  }
  for (const row of t.rows) {
    parts.push("<tr>");
    row.forEach((v, k) => {
      const { shown, number } = htmlCell(v, t.formats[k], t.text[k] ?? false);
      parts.push(number ? `<td style="${BORDER};text-align:right">${escapeHtml(shown)}</td>` : `<td style="${BORDER};${AS_TEXT}">${escapeHtml(shown)}</td>`);
    });
    parts.push("</tr>");
  }
  parts.push("</table></body></html>");
  return parts.join("");
}
