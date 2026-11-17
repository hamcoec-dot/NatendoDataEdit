// Column letters (A, B, ... Z, AA ...) and what Go To and the name box accept.

/** Column index (0-based) as spreadsheet letters: 0 = A, 25 = Z, 26 = AA. */
export function colLetters(c: number): string {
  let n = c + 1;
  let s = "";
  while (n > 0) {
    const m = (n - 1) % 26;
    s = String.fromCharCode(65 + m) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

/** Letters to a column index (0-based), or null. */
export function lettersCol(s: string): number | null {
  if (!/^[A-Za-z]{1,3}$/.test(s)) return null;
  let n = 0;
  for (const ch of s.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n - 1;
}

/** Where Go To should go: a row number as shown (1-based, of the whole
 *  file) and/or a column index (0-based). */
export interface GotoTarget {
  row: number | null;
  col: number | null;
}

/**
 * Read what was typed into the name box:
 * - "250" or "250, 3": row (and column number);
 * - "R250 C3", "R250C3", "r250": row and column as the name box shows them;
 * - "F200" or "C12" (column letters then row, as in Excel and Sheets), or
 *   just "F" (titles are tried first, so a column called "C2" wins);
 * - a column title, alone or followed by a row: "Last name 200".
 * `titles` are the column titles (lower case is fine); null when nothing fits.
 */
export function parseGoto(text: string, titles: string[]): GotoTarget | null {
  const s = text.trim();
  if (!s) return null;
  let m = /^r\s*(\d+)\s*[, ]?\s*(?:c\s*(\d+))?$/i.exec(s);
  if (m) return { row: Number(m[1]), col: m[2] ? Number(m[2]) - 1 : null };
  m = /^(\d+)(?:\s*[,; ]\s*(\d+))?$/.exec(s);
  if (m) return { row: Number(m[1]), col: m[2] ? Number(m[2]) - 1 : null };
  // A column title, then perhaps a row number.
  const low = s.toLowerCase();
  const byTitle = (name: string): number => titles.findIndex((t) => t.trim().toLowerCase() === name.trim());
  let c = byTitle(low);
  if (c >= 0) return { row: null, col: c };
  m = /^(.*?)[\s,:]+(\d+)$/.exec(low);
  if (m) {
    c = byTitle(m[1]!);
    if (c >= 0) return { row: Number(m[2]), col: c };
  }
  // Excel-style letters and a row: F200.
  m = /^([a-z]{1,3})\s*(\d+)$/i.exec(s);
  if (m) return { row: Number(m[2]), col: lettersCol(m[1]!) };
  const letters = lettersCol(s);
  if (letters !== null) return { row: null, col: letters };
  return null;
}
