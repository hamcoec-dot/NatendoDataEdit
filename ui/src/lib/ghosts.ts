// The ghost diff's fetching, kept out of the grid so it can be tested: what Replace all would change in
// the cells on screen is asked for once typing pauses (never one call per key), and an answer for a
// search, replacement, revision or window that is no longer the one on screen is dropped, never drawn.

import type { GhostWindow } from "./backend";

export type GhostMap = Map<string, { number: number; after: string; cut: boolean }>;

/** Typing waits this long before the window is asked about. */
export const GHOST_DEBOUNCE_MS = 100;

/** The ghost for a cell, unless the open Replace preview has that change unticked: unticked changes are kept
 *  by the row's number in the document ("number:col"), so a sort or filter while the preview is open cannot
 *  move a "leave this one" onto a different row. */
export function visibleGhost(ghosts: GhostMap, skip: ReadonlySet<string>, row: number, col: number) {
  const g = ghosts.get(`${row}:${col}`);
  return g && !skip.has(`${g.number}:${col}`) ? g : undefined;
}

export class GhostLoader {
  private base = "";
  private key = "";
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private o: {
      /** Put the ghosts of the window on screen (an empty map: none). */
      set: (m: GhostMap, partial: boolean) => void;
      /** Whether any ghosts are on screen now. */
      has: () => boolean;
      debounceMs?: number;
    },
  ) {}

  /** No ghosts wanted (nothing typed, toggle off, a write job running): drop what is shown and any pending ask. */
  off(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.base = this.key = "";
    if (this.o.has()) this.o.set(new Map(), false);
  }

  /**
   * Ghosts are wanted for `base` (tab, data revision, search and replacement) in the window `key` (base plus
   * the rows and columns on screen). A new base clears what is drawn at once; the window is asked about after
   * the debounce, and only the last key's answer is put on screen.
   */
  want(base: string, key: string, ask: () => Promise<GhostWindow[]>): void {
    if (key === this.key) return;
    if (base !== this.base) {
      this.base = base;
      if (this.o.has()) this.o.set(new Map(), false);
    }
    this.key = key;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      if (this.key !== key) return;
      ask()
        .then((all) => {
          if (this.key !== key) return; // stale: dropped, never drawn
          const m: GhostMap = new Map();
          let partial = false;
          for (const w of all) {
            partial ||= w.partial;
            for (const g of w.ghosts) m.set(`${g.row}:${g.col}`, { number: g.number, after: g.after, cut: g.cut });
          }
          this.o.set(m, partial);
        })
        .catch(() => {
          // A bad pattern or a file being changed: no ghosts; asked again on the next draw.
          if (this.key === key) this.key = "";
        });
    }, this.o.debounceMs ?? GHOST_DEBOUNCE_MS);
  }
}
