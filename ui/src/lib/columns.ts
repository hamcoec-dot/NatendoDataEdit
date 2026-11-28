// Column geometry for any number of columns (a broken file can have
// millions). Every column has the default width unless overridden; only the
// overrides are stored, so memory depends on how many columns were resized,
// not on the column count. Hidden columns have width 0. Display formats and
// wrapping (view only, never saved into the file) are kept per column here
// too, so they move with the columns when columns are inserted or deleted.

import type { DisplayFormat } from "./m9";
import type { TotalChoice } from "./totalsRow";

/** How a column lines up its text (automatic: numbers right, text left). */
export type Align = "left" | "center" | "right";

/** Everything here that Undo can put back. */
export interface ColSnap {
  defaultWidth: number;
  widths: [number, number][];
  hiddenWas: [number, number][];
  formats: [number, DisplayFormat][];
  wraps: number[];
  aligns?: [number, Align][];
  clips?: number[];
  totals?: [number, TotalChoice][];
}

export class Columns {
  /** Display formats by column. */
  formats = new Map<number, DisplayFormat>();
  /** Columns whose text wraps onto more lines. */
  wraps = new Set<number>();
  /** Alignment chosen per column (others: automatic). */
  aligns = new Map<number, Align>();
  /** Columns whose long text is cut at the column's edge instead of
   *  spilling into the empty cells beside it. */
  clips = new Set<number>();
  /** What the totals row shows per column (others: suggested). */
  totals = new Map<number, TotalChoice>();
  /** Widths of hidden columns before they were hidden, so showing them
   *  again gives them back (only those not at the default width). */
  private hiddenWas = new Map<number, number>();
  /** Sorted column indexes with a non-default width. */
  private keys: number[] = [];
  private widths = new Map<number, number>();
  /** prefix[i] = sum over keys[0..i) of (width - default). */
  private prefix: number[] = [0];

  constructor(
    public count: number,
    public defaultWidth: number,
  ) {}

  setCount(count: number): void {
    this.count = count;
  }

  setDefault(width: number): void {
    this.defaultWidth = width;
    this.rebuild();
  }

  width(c: number): number {
    return this.widths.get(c) ?? this.defaultWidth;
  }

  /** A width set by the user (undefined: the default). */
  customWidth(c: number): number | undefined {
    return this.widths.get(c);
  }

  isHidden(c: number): boolean {
    return this.widths.get(c) === 0;
  }

  /** The hidden columns, in order. */
  hiddenList(): number[] {
    const out: number[] = [];
    for (const [c, w] of this.widths) if (w === 0) out.push(c);
    return out.sort((a, b) => a - b);
  }

  setWidth(c: number, w: number): void {
    const width = Math.max(0, Math.round(w));
    const was = this.widths.get(c);
    if (width === 0 && was !== undefined && was !== 0) this.hiddenWas.set(c, was);
    else if (width !== 0) this.hiddenWas.delete(c);
    if (width === this.defaultWidth) {
      if (!this.widths.delete(c)) return;
    } else {
      this.widths.set(c, width);
    }
    this.rebuild();
  }

  /** The width a column has when shown (a hidden one: its width before it was hidden). */
  shownWidth(c: number): number {
    return this.isHidden(c) ? (this.hiddenWas.get(c) ?? this.defaultWidth) : this.width(c);
  }

  /** Set many widths at once (one rebuild: pasting a look onto 16,384 columns
   *  stays quick). Widths of 0 are not taken: this never hides a column. */
  setWidths(list: [number, number][]): void {
    let changed = false;
    for (const [c, w] of list) {
      const width = Math.max(1, Math.round(w));
      if (width === this.defaultWidth) changed = this.widths.delete(c) || changed;
      else if (this.widths.get(c) !== width) {
        this.widths.set(c, width);
        changed = true;
      }
      this.hiddenWas.delete(c);
    }
    if (changed) this.rebuild();
  }

  /** A short text that changes whenever `snapshot` would (cheap to compare:
   *  only the overrides, which are few unless many columns were resized). */
  key(): string {
    // Each distinct format once (a look pasted onto thousands of columns stays small).
    const seen = new Map<string, number>();
    const f: [number, number][] = [...this.formats].map(([c, x]) => {
      const j = JSON.stringify(x);
      let i = seen.get(j);
      if (i === undefined) seen.set(j, (i = seen.size));
      return [c, i];
    });
    return JSON.stringify([this.defaultWidth, [...this.widths], [...this.hiddenWas], [...seen.keys()], f, [...this.wraps], [...this.aligns], [...this.clips], [...this.totals]]);
  }

  /** The formats, sorted, with one copy of each distinct format shared by its columns. */
  private formatsCopy(): [number, DisplayFormat][] {
    const one = new Map<string, DisplayFormat>();
    return [...this.formats]
      .sort((a, b) => a[0] - b[0])
      .map(([c, f]) => {
        const j = JSON.stringify(f);
        let g = one.get(j);
        if (!g) one.set(j, (g = structuredClone(f)));
        return [c, g];
      });
  }

  /** A copy of the widths, hidden columns, formats and wrapping (for undo). */
  snapshot(): ColSnap {
    return {
      defaultWidth: this.defaultWidth,
      widths: [...this.widths].sort((a, b) => a[0] - b[0]),
      hiddenWas: [...this.hiddenWas].sort((a, b) => a[0] - b[0]),
      formats: this.formatsCopy(),
      wraps: [...this.wraps].sort((a, b) => a - b),
      aligns: [...this.aligns].sort((a, b) => a[0] - b[0]),
      clips: [...this.clips].sort((a, b) => a - b),
      totals: [...this.totals].sort((a, b) => a[0] - b[0]),
    };
  }

  /** Put back what `snapshot` copied. */
  restore(s: ColSnap): void {
    this.defaultWidth = s.defaultWidth;
    this.widths = new Map(s.widths);
    this.hiddenWas = new Map(s.hiddenWas);
    this.formats = new Map(s.formats.map(([c, f]) => [c, structuredClone(f)]));
    this.wraps = new Set(s.wraps);
    this.aligns = new Map(s.aligns ?? []);
    this.clips = new Set(s.clips ?? []);
    this.totals = new Map(s.totals ?? []);
    this.rebuild();
  }

  /** Forget all widths (used when columns are inserted, deleted or moved). */
  reset(): void {
    this.widths.clear();
    this.hiddenWas.clear();
    this.formats.clear();
    this.wraps.clear();
    this.aligns.clear();
    this.clips.clear();
    this.totals.clear();
    this.rebuild();
  }

  /** Shift overrides after `count` columns were inserted (>0) or removed (<0) at `at`. */
  shift(at: number, count: number): void {
    const next = new Map<number, number>();
    for (const [c, w] of this.widths) {
      if (c < at) next.set(c, w);
      else if (count > 0) next.set(c + count, w);
      else if (c >= at - count) next.set(c + count, w);
    }
    this.widths = next;
    const move = (c: number): number | null => (c < at ? c : count > 0 ? c + count : c >= at - count ? c + count : null);
    const formats = new Map<number, DisplayFormat>();
    for (const [c, f] of this.formats) {
      const to = move(c);
      if (to !== null) formats.set(to, f);
    }
    this.formats = formats;
    const wraps = new Set<number>();
    for (const c of this.wraps) {
      const to = move(c);
      if (to !== null) wraps.add(to);
    }
    this.wraps = wraps;
    const aligns = new Map<number, Align>();
    for (const [c, a] of this.aligns) {
      const to = move(c);
      if (to !== null) aligns.set(to, a);
    }
    this.aligns = aligns;
    const clips = new Set<number>();
    for (const c of this.clips) {
      const to = move(c);
      if (to !== null) clips.add(to);
    }
    this.clips = clips;
    const totals = new Map<number, TotalChoice>();
    for (const [c, t] of this.totals) {
      const to = move(c);
      if (to !== null) totals.set(to, t);
    }
    this.totals = totals;
    const was = new Map<number, number>();
    for (const [c, w] of this.hiddenWas) {
      const to = move(c);
      if (to !== null) was.set(to, w);
    }
    this.hiddenWas = was;
    this.rebuild();
  }

  /** Carry widths, formats and wrapping along after `count` columns at `at`
   *  were moved to start at `to`. */
  move(at: number, count: number, to: number): void {
    if (count <= 0 || at === to) return;
    const place = (c: number): number => {
      if (c >= at && c < at + count) return to + (c - at);
      if (to > at && c >= at + count && c < to + count) return c - count;
      if (to < at && c >= to && c < at) return c + count;
      return c;
    };
    this.widths = new Map([...this.widths].map(([c, w]) => [place(c), w]));
    this.formats = new Map([...this.formats].map(([c, f]) => [place(c), f]));
    this.wraps = new Set([...this.wraps].map(place));
    this.aligns = new Map([...this.aligns].map(([c, a]) => [place(c), a]));
    this.clips = new Set([...this.clips].map(place));
    this.totals = new Map([...this.totals].map(([c, t]) => [place(c), t]));
    this.hiddenWas = new Map([...this.hiddenWas].map(([c, w]) => [place(c), w]));
    this.rebuild();
  }

  private rebuild(): void {
    this.keys = [...this.widths.keys()].sort((a, b) => a - b);
    this.prefix = [0];
    let s = 0;
    for (const k of this.keys) {
      s += (this.widths.get(k) ?? this.defaultWidth) - this.defaultWidth;
      this.prefix.push(s);
    }
  }

  /** Number of overrides at indexes < c. */
  private before(c: number): number {
    let lo = 0;
    let hi = this.keys.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if ((this.keys[mid] ?? 0) < c) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  /** Left edge of column c (c may equal count for the right edge). */
  x(c: number): number {
    return c * this.defaultWidth + (this.prefix[this.before(c)] ?? 0);
  }

  total(): number {
    return this.x(this.count);
  }

  /** Column containing x (clamped to the valid range); -1 when there are none. */
  at(x: number): number {
    if (this.count <= 0) return -1;
    if (x <= 0) return this.firstVisible(0);
    let lo = 0;
    let hi = this.count - 1;
    // Largest c with x(c) <= x.
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (this.x(mid) <= x) lo = mid;
      else hi = mid - 1;
    }
    // Skip hidden columns that share the same left edge.
    while (lo < this.count - 1 && this.width(lo) === 0) lo++;
    return lo;
  }

  firstVisible(from: number): number {
    for (let c = Math.max(0, from); c < this.count; c++) if (this.width(c) > 0) return c;
    return -1;
  }

  /** Next visible column after c in direction dir (+1/-1), or c if none. */
  step(c: number, dir: 1 | -1, n = 1): number {
    let cur = c;
    let left = n;
    let probe = c;
    while (left > 0) {
      probe += dir;
      if (probe < 0 || probe >= this.count) break;
      if (this.width(probe) > 0) {
        cur = probe;
        left--;
      }
    }
    return cur;
  }

  /** Give columns at..at+count the same width, in one go. */
  setRange(at: number, count: number, w: number): void {
    const width = Math.max(0, Math.round(w));
    if (at <= 0 && count >= this.count) {
      // Every column: a new default, keeping hidden ones hidden (memory stays
      // small however many columns there are).
      for (const [c, v] of [...this.widths]) if (v !== 0) this.widths.delete(c);
      this.hiddenWas.clear();
      this.setDefault(width);
      return;
    }
    for (let c = at; c < at + count; c++) {
      // Hidden columns stay hidden (as with every column selected).
      if (this.widths.get(c) === 0) continue;
      if (width === this.defaultWidth) this.widths.delete(c);
      else this.widths.set(c, width);
    }
    this.rebuild();
  }

  /** Show the hidden columns in from..to (to not included). */
  showRange(from: number, to: number): void {
    for (const [c, w] of [...this.widths]) if (w === 0 && c >= from && c < to) this.unhide(c);
    this.rebuild();
  }

  private unhide(c: number): void {
    const was = this.hiddenWas.get(c);
    if (was === undefined) this.widths.delete(c);
    else this.widths.set(c, was);
    this.hiddenWas.delete(c);
  }

  /** Whether any column in from..to (to not included) is hidden. */
  hiddenIn(from: number, to: number): boolean {
    for (const [c, w] of this.widths) if (w === 0 && c >= from && c < to) return true;
    return false;
  }

  hiddenCount(): number {
    let n = 0;
    for (const w of this.widths.values()) if (w === 0) n++;
    return n;
  }

  showAll(): void {
    for (const [c, w] of [...this.widths]) if (w === 0) this.unhide(c);
    this.rebuild();
  }
}
