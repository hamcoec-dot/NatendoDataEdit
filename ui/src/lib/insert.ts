// "Insert rows…" / "Insert columns…": how many to add, checked before it
// reaches the engine (which has the same limits).

/** Most rows one insert adds (the engine refuses more). */
export const MAX_INSERT_ROWS = 100_000;
/** Most columns one insert adds (the engine refuses more). */
export const MAX_INSERT_COLS = 1_000;

/** A whole number from 1 to the limit, or a message saying what is wrong. */
export function insertCount(text: string, kind: "rows" | "cols"): number | string {
  const max = kind === "rows" ? MAX_INSERT_ROWS : MAX_INSERT_COLS;
  const t = text.trim().replace(/[,_ ]/g, "");
  if (!/^\d{1,9}$/.test(t)) return "Enter a whole number, such as 10.";
  const n = Number(t);
  if (n < 1) return "Enter at least 1.";
  if (n > max) return `At most ${max.toLocaleString("en-US")} ${kind === "rows" ? "rows" : "columns"} at a time.`;
  return n;
}
