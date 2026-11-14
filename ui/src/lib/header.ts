// Guess whether a file's first row holds column titles or ordinary data.
// Only a guess for the "First row is header" switch: the file is never
// changed by it, and the user can flip the switch at any time.

const NUMBER = /^[-+]?(\d{1,3}(,\d{3})+|\d+)?([.,]\d+)?([eE][-+]?\d+)?%?$/;
const DATE = /^\d{1,4}[-/.]\d{1,2}[-/.]\d{1,4}([ T]\d{1,2}:\d{2}(:\d{2})?)?$/;

function numericLike(v: string): boolean {
  const t = v.trim().replace(/^[$€£¥]/, "");
  return t !== "" && /\d/.test(t) && (NUMBER.test(t) || DATE.test(t));
}

/**
 * `rows` is a sample from the top of the file (first row first). True when
 * the first row looks like titles:
 * - some column is numbers or dates below while its first cell is not, or
 * - every first-row cell is filled, unique and free of digits, and in at
 *   least half the columns that title never repeats below while the values
 *   below do contain digits or differ in kind.
 */
export function looksLikeHeader(rows: string[][]): boolean {
  if (rows.length < 2) return false;
  const first = rows[0]!;
  const width = Math.max(...rows.map((r) => r.length));
  if (width === 0) return false;
  const titles = first.map((v) => v.trim());
  const filled = titles.length >= width && titles.every((t) => t !== "");
  const unique = new Set(titles.map((t) => t.toLowerCase())).size === titles.length;
  if (!filled || !unique) return false;
  if (titles.some(numericLike)) return false;

  const below = rows.slice(1);
  let typed = 0;
  let distinct = 0;
  for (let c = 0; c < width; c++) {
    const values = below.map((r) => (r[c] ?? "").trim()).filter((v) => v !== "");
    if (values.length === 0) continue;
    const numeric = values.filter(numericLike).length;
    if (numeric / values.length >= 0.8) typed++;
    const title = titles[c] ?? "";
    const repeats = values.some((v) => v.toLowerCase() === title.toLowerCase());
    const digitsBelow = values.some((v) => /\d/.test(v));
    if (!repeats && (digitsBelow || values.some((v) => v.length > title.length * 2 + 8))) distinct++;
  }
  if (typed > 0) return true;
  return !titles.some((t) => /\d/.test(t)) && distinct * 2 >= width;
}
