// Cell cache for the grid: fixed-size blocks fetched from the engine on
// demand, least-recently-used blocks evicted. After an edit the cache is
// marked stale: old values keep showing (no flicker) until fresh blocks arrive.

import type { ViewportData } from "./binary";

export const BLOCK_ROWS = 64;
export const BLOCK_COLS = 32;

export type Fetcher = (
  row: number,
  rows: number,
  col: number,
  cols: number,
) => Promise<ViewportData>;

interface Block {
  gen: number;
  used: number;
  data: ViewportData;
}

export class CellCache {
  private blocks = new Map<string, Block>();
  /** Row numbers per block row (sent with every block while filtered). */
  private numbers = new Map<number, number[]>();
  private pending = new Map<string, number>();
  private gen = 0;
  private clock = 0;
  /** Last fetch error, for the status bar. */
  error: string | null = null;

  constructor(
    private fetcher: Fetcher,
    private onLoad: () => void,
    private maxBlocks = 400,
  ) {}

  private static key(br: number, bc: number): string {
    return `${br}:${bc}`;
  }

  /** Text of a cell, or undefined while it is loading. */
  get(row: number, col: number): string | undefined {
    const b = this.blocks.get(CellCache.key(Math.floor(row / BLOCK_ROWS), Math.floor(col / BLOCK_COLS)));
    if (!b) return undefined;
    b.used = this.clock;
    const d = b.data;
    const r = row - d.firstRow;
    const c = col - d.firstCol;
    if (r < 0 || c < 0 || c >= d.cols) return undefined;
    if (r >= d.rows) return ""; // past the end of the document when fetched
    return d.cells[r * d.cols + c];
  }

  /** With a filter on: the shown row's number in the whole document
   *  (0-based), or undefined while it is loading. */
  rowNumber(row: number): number | undefined {
    const br = Math.floor(row / BLOCK_ROWS);
    const nums = this.numbers.get(br);
    return nums?.[row - br * BLOCK_ROWS];
  }

  isTruncated(row: number, col: number): boolean {
    const b = this.blocks.get(CellCache.key(Math.floor(row / BLOCK_ROWS), Math.floor(col / BLOCK_COLS)));
    if (!b) return false;
    const d = b.data;
    const r = row - d.firstRow;
    const c = col - d.firstCol;
    if (r < 0 || c < 0 || r >= d.rows || c >= d.cols) return false;
    return d.truncated[r * d.cols + c] === 1;
  }

  /** Make sure rows [r0,r1) x cols [c0,c1) are loaded or loading. */
  request(r0: number, r1: number, c0: number, c1: number): void {
    this.clock++;
    if (r1 <= r0 || c1 <= c0) return;
    const br0 = Math.floor(r0 / BLOCK_ROWS);
    const br1 = Math.floor((r1 - 1) / BLOCK_ROWS);
    const bc0 = Math.floor(c0 / BLOCK_COLS);
    const bc1 = Math.floor((c1 - 1) / BLOCK_COLS);
    for (let br = br0; br <= br1; br++) {
      for (let bc = bc0; bc <= bc1; bc++) {
        const k = CellCache.key(br, bc);
        const b = this.blocks.get(k);
        if (b) b.used = this.clock;
        if ((b && b.gen === this.gen) || this.pending.get(k) === this.gen) continue;
        this.fetch(k, br, bc);
      }
    }
  }

  private fetch(k: string, br: number, bc: number): void {
    const gen = this.gen;
    this.pending.set(k, gen);
    this.fetcher(br * BLOCK_ROWS, BLOCK_ROWS, bc * BLOCK_COLS, BLOCK_COLS).then(
      (data) => {
        if (this.pending.get(k) === gen) this.pending.delete(k);
        const old = this.blocks.get(k);
        // Never let an older reply overwrite a newer one.
        if (old && old.gen > gen) return;
        this.blocks.set(k, { gen, used: this.clock, data });
        if (data.rowNumbers) this.numbers.set(br, data.rowNumbers);
        else this.numbers.delete(br);
        this.error = null;
        this.evict();
        this.onLoad();
      },
      (e: unknown) => {
        if (this.pending.get(k) === gen) this.pending.delete(k);
        this.error = e instanceof Error ? e.message : String((e as { message?: string })?.message ?? e);
        this.onLoad();
      },
    );
  }

  private evict(): void {
    if (this.blocks.size <= this.maxBlocks) return;
    const byAge = [...this.blocks.entries()].sort((a, b) => a[1].used - b[1].used);
    for (const [k] of byAge.slice(0, this.blocks.size - this.maxBlocks)) this.blocks.delete(k);
  }

  /** Data changed: refetch everything shown, keep old values meanwhile. */
  invalidate(): void {
    this.gen++;
  }

  /** Structure changed so much that old values would mislead: drop them. */
  clear(): void {
    this.gen++;
    this.blocks.clear();
    this.numbers.clear();
    this.pending.clear();
  }

  get size(): number {
    return this.blocks.size;
  }
}
