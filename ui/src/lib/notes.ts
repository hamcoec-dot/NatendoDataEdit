// Notes and flags on rows (Milestone 15).

export interface NoteMark {
  row: number;
  flag: number;
  text: string;
}

/** Flag number, name and colour (0: none). */
export const FLAG_NAMES: [number, string, string][] = [
  [0, "No flag", ""],
  [1, "Red", "#e5484d"],
  [2, "Orange", "#f76b15"],
  [3, "Yellow", "#e2b400"],
  [4, "Green", "#30a46c"],
  [5, "Blue", "#0090ff"],
  [6, "Purple", "#8e4ec6"],
];

/** A short status line for a row's note. */
export function noteSummary(n: { flag: number; text: string } | undefined): string {
  if (!n) return "";
  const flag = n.flag ? `${FLAG_NAMES[n.flag]?.[1] ?? ""} flag` : "";
  const text = n.text.length > 80 ? `${n.text.slice(0, 80)}…` : n.text;
  return [flag, text].filter(Boolean).join(": ");
}
