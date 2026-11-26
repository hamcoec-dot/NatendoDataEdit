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

/** Words that name a column ("id", "first_name", "Zip code"). */
const TITLE_WORD =
  /^(id|no|num|number|code|key|name|first|last|middle|full|title|type|kind|status|date|time|year|month|day|created|updated|email|e-mail|mail|phone|tel|mobile|fax|zip|zipcode|postal|postcode|address|street|city|town|county|state|country|region|precinct|district|ward|party|amount|total|price|cost|qty|quantity|count|sum|balance|fee|rate|percent|description|notes?|comments?|category|group|age|gender|sex|dob|birth|birthdate|value|label|url|website|company|organization|org|department|dept|account|user|username|login)$/i;

/** How sure the guess is: "yes" or "no" when the signs are clear,
 *  "unsure" otherwise (then `looksLikeHeader` is the guess). */
export function headerCertainty(rows: string[][]): "yes" | "no" | "unsure" {
  if (rows.length < 2) return "unsure";
  const titles = (rows[0] ?? []).map((v) => v.trim());
  // A number or a date in the first row: data. Years alone ("Name, 2023,
  // 2024") may be titles: decided below.
  const year = (v: string) => /^(19|20)\d\d$|^2100$/.test(v);
  const years = titles.some(numericLike) && titles.every((t) => !numericLike(t) || year(t));
  if (titles.some(numericLike) && !years) return "no";
  const filled = titles.length > 0 && titles.every((t) => t !== "");
  const unique = new Set(titles.map((t) => t.toLowerCase())).size === titles.length;
  if (!filled || !unique) return looksLikeHeader(rows) ? "unsure" : "no";
  const below = rows.slice(1);
  // Text (or a year) over a column of numbers or dates; under year titles
  // a column of years says nothing (the first row may be data too).
  for (let c = 0; c < titles.length; c++) {
    const values = below.map((r) => (r[c] ?? "").trim()).filter((v) => v !== "");
    if (years && values.some(year)) continue;
    if (values.length >= 2 && values.filter(numericLike).length / values.length >= 0.8) return "yes";
  }
  if (years) return "unsure";
  // Every title is or holds a column word ("first_name", "ZipCode"); being
  // written like one is not enough (surnames: McDonald, DeLuca).
  const wordy = (t: string) =>
    t
      .split(/[\s_.-]+|(?<=[a-z])(?=[A-Z])/)
      .filter(Boolean)
      .some((w) => TITLE_WORD.test(w));
  if (titles.every(wordy)) return "yes";
  return "unsure";
}

/**
 * Report title lines above the column titles (C4): the number of rows at the
 * top (1 to 5) that each fill fewer than half the columns, followed by a row
 * that looks like titles over the rows below it. 0 when there are none, or
 * when the first row already looks like the titles.
 */
export function titleLines(rows: string[][]): number {
  const width = Math.max(0, ...rows.map((r) => r.filter((v) => v.trim() !== "").length));
  if (width < 2) return 0;
  const sparse = (r: string[]) => r.filter((v) => v.trim() !== "").length * 2 < width;
  for (let k = 1; k <= 5 && k + 2 <= rows.length; k++) {
    if (!sparse(rows[k - 1]!)) return 0;
    const rest = rows.slice(k);
    if (!sparse(rest[0]!) && (looksLikeHeader(rest) || headerCertainty(rest) === "yes")) return k;
  }
  return 0;
}
