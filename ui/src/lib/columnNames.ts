/**
 * Paste as column names: the names in copied text. The first line is read
 * as one row: split by tabs (copied from a table or this app), else by the
 * file's delimiter, else by commas; quoted names may hold the separator or
 * a line break. Text with no separator on several lines is a list, one
 * name per line.
 */
export function namesFromText(text: string, delimiter: string): string[] {
  const t = text.replace(/^﻿/, "");
  // The first record: up to the first line break outside quotes.
  let end = 0;
  for (let q = false; end < t.length; end++) {
    const ch = t[end];
    if (ch === '"') q = !q;
    else if (!q && (ch === "\n" || ch === "\r")) break;
  }
  const firstLine = t.slice(0, end);
  const sep = firstLine.includes("\t") ? "\t" : delimiter && firstLine.includes(delimiter) ? delimiter : firstLine.includes(",") ? "," : null;
  if (sep === null) {
    const lines = t.split(/\r?\n/).map((s) => s.trim());
    while (lines.length && lines.at(-1) === "") lines.pop();
    return lines.length > 1 ? lines : lines.length ? [lines[0]!] : [];
  }
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  let i = 0;
  for (; i < t.length; i++) {
    const ch = t[i]!;
    if (quoted) {
      if (ch === '"') {
        if (t[i + 1] === '"') {
          cur += '"';
          i++;
        } else quoted = false;
      } else cur += ch;
    } else if (ch === '"' && cur === "") quoted = true;
    else if (ch === sep) {
      out.push(cur);
      cur = "";
    } else if (ch === "\n" || ch === "\r") break;
    else cur += ch;
  }
  out.push(cur);
  return out;
}
