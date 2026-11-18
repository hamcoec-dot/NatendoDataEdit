// Column profile helpers: the expression that shows each line's rows (run
// as Filter by formula), labels for the histogram, and the browser stub's
// own small profile (the desktop app's comes from the engine).
import type { Histogram, Profile, ShapeCount } from "./backend";

/** Digits as 9, letters as A, the rest as it is (as the engine does). */
export function shapeOf(s: string): string {
  let out = "";
  let n = 0;
  for (const c of s) {
    if (n++ >= 40) return out + "…";
    out += /\p{N}/u.test(c) ? "9" : /\p{L}/u.test(c) ? "A" : c;
  }
  return out;
}

const q = (s: string): string => `"${s.replace(/"/g, '""')}"`;

/** The rows with this shape. */
export function shapeExpr(ref: string, shape: string): string {
  return shape.endsWith("…") ? `LEFT(SHAPE(${ref}), 40) = ${q(shape.slice(0, -1))}` : `SHAPE(${ref}) = ${q(shape)}`;
}

/** One bar's lower and upper edge (the last bar includes `high`). */
export function binEdges(h: Histogram, i: number): [number, number] {
  const w = (h.high - h.low) / h.counts.length;
  return [h.low + w * i, i === h.counts.length - 1 ? h.high : h.low + w * (i + 1)];
}

function dayText(d: number): string {
  return new Date(Math.round(d) * 86_400_000).toISOString().slice(0, 10);
}

const fmt = (x: number): string => (Math.abs(x) >= 1e6 || Number.isInteger(x) ? x.toLocaleString() : x.toLocaleString(undefined, { maximumFractionDigits: 2 }));

/** A bar's label, like "10 to 20" or "2024-01-01 to 2024-02-15". */
export function binLabel(h: Histogram, i: number): string {
  const [a, b] = binEdges(h, i);
  if (h.kind === "dates") return `${dayText(a)} to ${dayText(b)}`;
  if (h.kind === "lengths") return `${Math.ceil(a)} to ${Math.floor(b)} characters`;
  // The end bars also hold the few numbers beyond the range shown.
  if (i === 0) return `up to ${fmt(b)}`;
  if (i === h.counts.length - 1) return `${fmt(a)} and over`;
  return `${fmt(a)} to ${fmt(b)}`;
}

/** The rows in one bar. */
export function binExpr(ref: string, h: Histogram, i: number): string {
  const [a, b] = binEdges(h, i);
  const last = i === h.counts.length - 1;
  const v = h.kind === "lengths" ? `LEN(${ref})` : h.kind === "dates" ? `DATEVALUE(${ref})` : `VALUE(${ref})`;
  const lo = h.kind === "dates" ? `DATEVALUE(${q(dayText(a))})` : String(a);
  const hi = h.kind === "dates" ? `DATEVALUE(${q(dayText(b))})` : String(b);
  if (h.kind === "numbers" && i === 0) return `${v} < ${hi}`;
  if (h.kind === "numbers" && last) return `${v} >= ${lo}`;
  return `AND(${v} >= ${lo}, ${v} ${last ? "<=" : "<"} ${hi})`;
}

/** The rows of numbers far from the rest (more than four standard
 *  deviations from the mean). */
export function outlierExpr(ref: string, p: Profile): string | null {
  if (p.mean === null || p.sd === null || p.sd <= 0) return null;
  return `ABS(VALUE(${ref}) - (${p.mean})) > ${4 * p.sd}`;
}

/** The browser stub's profile of a list of cells. */
export function stubProfile(col: number, cells: string[]): Profile {
  const filled = cells.filter((v) => v.trim() !== "");
  const shapes = new Map<string, ShapeCount>();
  for (const v of filled) {
    const s = shapeOf(v);
    const e = shapes.get(s);
    if (e) e.count++;
    else shapes.set(s, { shape: s, count: 1, example: v });
  }
  const sorted = [...shapes.values()].sort((a, b) => b.count - a.count || a.shape.localeCompare(b.shape));
  const nums = filled.map((v) => Number(v.replace(/[$,]/g, ""))).filter((x) => !Number.isNaN(x));
  const numeric = filled.length > 0 && nums.length * 10 >= filled.length * 8;
  const lens = filled.map((v) => [...v].length);
  const vals = numeric ? nums : lens;
  const mean = nums.length > 1 ? nums.reduce((a, b) => a + b, 0) / nums.length : null;
  const sd = mean === null ? null : Math.sqrt(Math.max(0, nums.reduce((a, b) => a + b * b, 0) / nums.length - mean * mean));
  let low = vals.length ? Math.min(...vals) : 0;
  let high = vals.length ? Math.max(...vals) : 0;
  if (numeric && nums.length >= 50) {
    const s = [...nums].sort((a, b) => a - b);
    const at = (q: number) => s[Math.floor((s.length - 1) * q)]!;
    if (at(0.99) > at(0.01)) [low, high] = [at(0.01), at(0.99)];
  }
  const counts = new Array<number>(20).fill(0);
  for (const x of vals) counts[high > low ? Math.max(0, Math.min(19, Math.floor(((x - low) / (high - low)) * 20))) : 0]!++;
  const far = mean !== null && sd ? filled.filter((v) => Math.abs(Number(v) - mean) > 4 * sd) : [];
  const dominant = sorted.length > 0 && sorted[0]!.count * 2 >= filled.length;
  const rareCap = Math.max(1, Math.min(100, Math.floor(filled.length * 0.001)));
  const byLen = [...filled].sort((a, b) => [...a].length - [...b].length);
  return {
    col,
    rows: cells.length,
    blanks: cells.length - filled.length,
    shapes: sorted.slice(0, 12),
    shape_kinds: sorted.length,
    histogram: filled.length ? { kind: numeric ? "numbers" : "lengths", low, high, counts } : null,
    shortest: byLen[0] ?? null,
    longest: byLen[byLen.length - 1] ?? null,
    min_len: lens.length ? Math.min(...lens) : 0,
    max_len: lens.length ? Math.max(...lens) : 0,
    rare_shapes: dominant ? sorted.filter((s) => (s.count <= rareCap && s.count * 1000 <= filled.length) || (s.count <= 2 && s.count * 20 <= filled.length)).slice(0, 12) : [],
    outliers: far.length,
    outlier_examples: far.slice(0, 5),
    mean,
    sd,
  };
}
