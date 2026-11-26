import type { ImportFormat } from "./backend";

/** A dBase table's titles are always its field names: the import screen offers no choice about it. */
export function headerChoice(format: ImportFormat | null): boolean {
  return format !== "dbf";
}

/** Whether the imported table's first row holds titles. */
export function importHeader(format: ImportFormat | null, ticked: boolean): boolean {
  return headerChoice(format) ? ticked : true;
}

/** The line under the preview. `rows` counts the preview's rows; a dBase table says its records instead. */
export function previewLine(format: ImportFormat | null, rows: number, records: string | undefined): string {
  if (format === "dbf") return records ? `${records}.` : "Counting the records\u2026";
  return rows >= 100 ? "Showing the first 100 rows." : `${rows.toLocaleString()} rows.`;
}

// The encodings offered for a dBase (.dbf) table: [label sent to the engine, name shown].
export const DBF_ENCODINGS: [string, string][] = [
  ["", "Automatic"],
  ["utf-8", "UTF-8"],
  ["windows-1252", "Windows 1252 (Western)"],
  ["cp437", "DOS 437 (US)"],
  ["cp850", "DOS 850 (Western)"],
  ["windows-1250", "Windows 1250 (Central European)"],
  ["windows-1251", "Windows 1251 (Cyrillic)"],
  ["ibm866", "DOS 866 (Cyrillic)"],
  ["windows-1253", "Windows 1253 (Greek)"],
  ["windows-1254", "Windows 1254 (Turkish)"],
  ["windows-1255", "Windows 1255 (Hebrew)"],
  ["windows-1256", "Windows 1256 (Arabic)"],
  ["windows-1257", "Windows 1257 (Baltic)"],
  ["windows-874", "Windows 874 (Thai)"],
  ["macintosh", "Mac Roman"],
  ["shift_jis", "Japanese (Shift_JIS)"],
  ["gbk", "Chinese simplified (GBK)"],
  ["big5", "Chinese traditional (Big5)"],
  ["euc-kr", "Korean (EUC-KR)"],
];
