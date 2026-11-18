// Compare two cells as Data › Compare two columns does (the engine's
// colcompare.rs), for the browser stub and tests.
import type { CompareCols } from "./backend";
import { readNumber } from "./m9";

export type CellCompare = "same" | "different" | "first" | "second";

export function compareCells(a: string, b: string, spec: Pick<CompareCols, "case" | "spaces" | "numbers">): CellCompare {
  const blank = (s: string) => (spec.spaces ? s === "" : s.trim() === "");
  if (blank(a) && blank(b)) return "same";
  if (blank(b)) return "first";
  if (blank(a)) return "second";
  if (a === b) return "same";
  if (spec.numbers) {
    const x = readNumber(a);
    const y = readNumber(b);
    if (x !== null && y !== null) return x === y ? "same" : "different";
  }
  const norm = (s: string) => {
    const t = spec.spaces ? s : s.split(/\s+/u).filter(Boolean).join(" ");
    return spec.case ? t : t.toLowerCase();
  };
  return norm(a) === norm(b) ? "same" : "different";
}
