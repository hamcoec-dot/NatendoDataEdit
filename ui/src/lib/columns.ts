// Column geometry for any number of columns (a broken file can have
// millions). Every column has the default width unless overridden; only the
// overrides are stored, so memory depends on how many columns were resized,
// not on the column count. Hidden columns have width 0. Display formats and
// wrapping (view only, never saved into the file) are kept per column here
// too, so they move with the columns when columns are inserted or deleted.

import type { DisplayFormat } from "./m9";

export class Columns {
  /** Display formats by column. */
  formats = new Map<number, DisplayFormat>();
  /** Columns whose text wraps onto more lines. */
  wraps = new Set<number>();
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

  isHidden(c: number): boolean {
    return this.widths.get(c) === 0;
  }

  setWidth(c: number, w: number): void {
    const width = Math.max(0, Math.round(w));
    if (width === this.defaultWidth) {
      if (!this.widths.delete(c)) return;
    } else {
      this.widths.set(c, width);
    }
    this.rebuild();
  }

  /** Forget all widths (used when columns are inserted, deleted or moved). */
  reset(): void {
    this.widths.clear();
    this.formats.clear();
    this.wraps.clear();
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

  hiddenCount(): number {
    let n = 0;
    for (const w of this.widths.values()) if (w === 0) n++;
    return n;
  }

  showAll(): void {
    for (const [c, w] of [...this.widths]) if (w === 0) this.widths.delete(c);
    this.rebuild();
  }
}
