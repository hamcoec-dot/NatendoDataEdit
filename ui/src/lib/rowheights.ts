// Per-row heights. Every row has the uniform height (the grid's plain row height) unless
// it has a custom one; only the custom rows are stored, so memory depends on
// how many rows were resized, not on the row count (a file can have millions).
//
// POSITIONS: rows here are DOCUMENT positions (row 0 is the file's first row,
// whether or not it is hidden or filtered out). The grid maps a shown row to its
// document row before it asks (`geom` in Grid.svelte); with no rows hidden that
// map is the identity. A hidden row keeps its height, so showing it again
// brings it back.
//
// Lookups are O(log k) in the number of custom rows. The arrays are replaced,
// never changed in place, so a snapshot (for Undo) is just the two references
// and an unchanged set is shared between any number of undo steps.

/** Smallest and largest height a row may be given, in pixels. */
export const MIN_ROW_PX = 14;
export const MAX_ROW_PX = 400;
/** Most rows with a custom height (more are refused with a message). */
export const MAX_CUSTOM_ROWS = 100_000;

/** What Undo keeps: shared by reference while nothing changes. */
export interface RowHeightSnap {
  readonly rows: readonly number[];
  readonly px: readonly number[];
  readonly sum: readonly number[];
}

const NONE: RowHeightSnap = { rows: [], px: [], sum: [0] };

export const clampRowPx = (px: number): number => Math.min(MAX_ROW_PX, Math.max(MIN_ROW_PX, Math.round(px)));

export class RowHeights {
  /** Sorted, unique document rows that have a custom height. */
  private rows: readonly number[] = [];
  /** Their heights, same order. */
  private px: readonly number[] = [];
  /** sum[i] = total height of the first i custom rows (length k + 1). */
  private sum: readonly number[] = [0];

  get empty(): boolean {
    return this.rows.length === 0;
  }

  get count(): number {
    return this.rows.length;
  }

  /** Bytes the stored heights take (for the undo budget). */
  get bytes(): number {
    return this.rows.length * 16;
  }

  /** How many custom rows lie before `row`. */
  private before(row: number): number {
    let lo = 0;
    let hi = this.rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.rows[mid]! < row) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  /** Index of `row` among the custom rows, or -1. */
  private find(row: number): number {
    const i = this.before(row);
    return this.rows[i] === row ? i : -1;
  }

  /** Custom height of a row (null: the uniform one). */
  custom(row: number): number | null {
    const i = this.find(row);
    return i < 0 ? null : this.px[i]!;
  }

  heightOf(row: number, base: number): number {
    const i = this.rows.length ? this.find(row) : -1;
    return i < 0 ? base : this.px[i]!;
  }

  /** Distance from the top of row 0 to the top of `row`. */
  offsetOf(row: number, base: number): number {
    if (!this.rows.length) return row * base;
    const k = this.before(row);
    return row * base + this.sum[k]! - k * base;
  }

  /** Height of the first `n` rows. */
  totalHeight(n: number, base: number): number {
    return this.offsetOf(n, base);
  }

  /** The row whose band holds distance `y` from the top of row 0 (y >= 0). */
  rowAt(y: number, base: number): number {
    if (y <= 0) return 0;
    if (!this.rows.length) return Math.floor(y / base);
    // The last custom row whose top is at or above y.
    let lo = 0;
    let hi = this.rows.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      const top = this.rows[mid]! * base + this.sum[mid]! - mid * base;
      if (top <= y) lo = mid + 1;
      else hi = mid;
    }
    const i = lo - 1;
    if (i < 0) return Math.floor(y / base);
    const top = this.rows[i]! * base + this.sum[i]! - i * base;
    const bottom = top + this.px[i]!;
    if (y < bottom) return this.rows[i]!;
    // Past that row: uniform rows until the next custom row.
    return this.rows[i]! + 1 + Math.floor((y - bottom) / base);
  }

  private commit(rows: number[], px: number[]): void {
    this.rows = rows;
    this.px = px;
    const sum = new Array<number>(rows.length + 1);
    sum[0] = 0;
    for (let i = 0; i < rows.length; i++) sum[i + 1] = sum[i]! + px[i]!;
    this.sum = sum;
  }

  /** Take already-built arrays as they are (they are never changed in place). */
  private adopt(rows: readonly number[], px: readonly number[], sum: readonly number[]): void {
    this.rows = rows;
    this.px = px;
    this.sum = sum;
  }

  /** Give these rows this height (clamped to 14..400). False when that would
   *  pass the cap: nothing changes. */
  set(rows: readonly number[], px: number): boolean {
    const h = clampRowPx(px);
    if (rows.length === 1) return this.setOne(rows[0]!, h);
    const map = new Map<number, number>();
    for (let i = 0; i < this.rows.length; i++) map.set(this.rows[i]!, this.px[i]!);
    for (const r of rows) map.set(r, h);
    if (map.size > MAX_CUSTOM_ROWS) return false;
    const keys = [...map.keys()].sort((a, b) => a - b);
    this.commit(
      keys,
      keys.map((k) => map.get(k)!),
    );
    return true;
  }

  /** One row: a binary search and a copy of the arrays, no map and no sort (a
   *  drag calls this on every pointer move with up to 100,000 heights). */
  private setOne(row: number, h: number): boolean {
    const i = this.before(row);
    const n = this.rows.length;
    if (this.rows[i] === row) {
      const old = this.px[i]!;
      if (old === h) return true;
      const px = this.px.slice();
      px[i] = h;
      const sum = this.sum.slice();
      const d = h - old;
      for (let j = i + 1; j <= n; j++) sum[j] = sum[j]! + d;
      this.px = px;
      this.sum = sum;
      return true;
    }
    if (n >= MAX_CUSTOM_ROWS) return false;
    const rows = this.rows.slice();
    const px = this.px.slice();
    rows.splice(i, 0, row);
    px.splice(i, 0, h);
    const sum = this.sum.slice();
    sum.splice(i + 1, 0, sum[i]! + h);
    for (let j = i + 2; j <= n + 1; j++) sum[j] = sum[j]! + h;
    this.rows = rows;
    this.px = px;
    this.sum = sum;
    return true;
  }

  /** Back to the uniform height for these rows. */
  reset(rows: readonly number[]): void {
    if (!this.rows.length) return;
    const drop = new Set(rows);
    const keep: number[] = [];
    const pk: number[] = [];
    for (let i = 0; i < this.rows.length; i++) {
      if (drop.has(this.rows[i]!)) continue;
      keep.push(this.rows[i]!);
      pk.push(this.px[i]!);
    }
    if (keep.length !== this.rows.length) this.commit(keep, pk);
  }

  clear(): void {
    if (this.rows.length) this.commit([], []);
  }

  /** `count` rows were inserted at `at`: rows from there on move down. */
  insertRows(at: number, count: number): void {
    if (!this.rows.length || count <= 0) return;
    const i = this.before(at);
    if (i === this.rows.length) return;
    this.commit(
      this.rows.map((r, j) => (j >= i ? r + count : r)),
      [...this.px],
    );
  }

  /** `count` rows at `at` were deleted: their heights go, later rows move up. */
  deleteRows(at: number, count: number): void {
    if (!this.rows.length || count <= 0) return;
    const keep: number[] = [];
    const pk: number[] = [];
    let changed = false;
    for (let i = 0; i < this.rows.length; i++) {
      const r = this.rows[i]!;
      if (r < at) {
        keep.push(r);
        pk.push(this.px[i]!);
      } else if (r >= at + count) {
        keep.push(r - count);
        pk.push(this.px[i]!);
        changed = true;
      } else changed = true;
    }
    if (changed) this.commit(keep, pk);
  }

  /** `count` rows at `at` were moved so they now start at `to` (the index
   *  they start at afterwards, as `dropIndex` says): their heights go with them. */
  moveRows(at: number, count: number, to: number): void {
    if (!this.rows.length || count <= 0 || at === to) return;
    const pairs: [number, number][] = this.rows.map((r, i) => {
      let n: number;
      if (r >= at && r < at + count) n = to + (r - at);
      else {
        const removed = r >= at + count ? r - count : r;
        n = removed >= to ? removed + count : removed;
      }
      return [n, this.px[i]!];
    });
    pairs.sort((a, b) => a[0] - b[0]);
    this.commit(
      pairs.map((p) => p[0]),
      pairs.map((p) => p[1]),
    );
  }

  /** The document rows that have a custom height, in order. */
  get places(): readonly number[] {
    return this.rows;
  }

  /** The heights in SHOWN positions: `shown[i]` is the shown row of the i-th
   *  custom row (`places`), null when it is hidden (left out). The shown rows
   *  keep the document order, so the result stays sorted. */
  project(shown: readonly (number | null)[]): RowHeights {
    const out = new RowHeights();
    const rows: number[] = [];
    const px: number[] = [];
    for (let i = 0; i < this.rows.length; i++) {
      const s = shown[i];
      if (s === null || s === undefined) continue;
      rows.push(s);
      px.push(this.px[i]!);
    }
    out.commit(rows, px);
    return out;
  }

  /** These document rows were deleted, as runs `[first, count]` in order
   *  (positions as before): their heights go, later rows move up by the
   *  rows deleted before them. One pass. */
  deleteRuns(runs: readonly (readonly [number, number])[]): void {
    if (!this.rows.length || !runs.length) return;
    const keep: number[] = [];
    const pk: number[] = [];
    let j = 0;
    let gone = 0;
    for (let i = 0; i < this.rows.length; i++) {
      const r = this.rows[i]!;
      while (j < runs.length && runs[j]![0] + runs[j]![1] <= r) gone += runs[j++]![1];
      if (j < runs.length && r >= runs[j]![0]) continue;
      keep.push(r - gone);
      pk.push(this.px[i]!);
    }
    this.commit(keep, pk);
  }

  snapshot(): RowHeightSnap {
    return this.rows.length ? { rows: this.rows, px: this.px, sum: this.sum } : NONE;
  }

  /** The snapshot's arrays are taken as they are (shared, not copied). */
  restore(s: RowHeightSnap): void {
    this.adopt(s.rows, s.px, s.sum);
  }

  /** The same snapshot (by reference) means nothing changed. */
  sameAs(s: RowHeightSnap): boolean {
    return s === NONE ? this.rows.length === 0 : s.rows === this.rows && s.px === this.px;
  }

  /** For the view file and the session: [row, px] pairs. */
  toView(): [number, number][] {
    return this.rows.map((r, i) => [r, this.px[i]!]);
  }

  /** From a view file or the session. Anything wrong is dropped, not fatal:
   *  non-integer or negative rows, rows past the file's end, duplicates,
   *  heights outside 14..400 (clamped), and everything past the cap. */
  static fromView(raw: unknown, rowCount: number): { heights: RowHeights; dropped: number } {
    const h = new RowHeights();
    if (!Array.isArray(raw)) return { heights: h, dropped: raw === undefined || raw === null ? 0 : 1 };
    const map = new Map<number, number>();
    let dropped = 0;
    for (const e of raw) {
      if (!Array.isArray(e) || e.length !== 2) {
        dropped++;
        continue;
      }
      const [r, p] = e as [unknown, unknown];
      if (typeof r !== "number" || typeof p !== "number" || !Number.isInteger(r) || r < 0 || r >= rowCount || !Number.isFinite(p)) {
        dropped++;
        continue;
      }
      if (map.size >= MAX_CUSTOM_ROWS && !map.has(r)) {
        dropped++;
        continue;
      }
      if (map.has(r)) dropped++;
      map.set(r, clampRowPx(p));
    }
    const keys = [...map.keys()].sort((a, b) => a - b);
    h.commit(
      keys,
      keys.map((k) => map.get(k)!),
    );
    return { heights: h, dropped };
  }
}
