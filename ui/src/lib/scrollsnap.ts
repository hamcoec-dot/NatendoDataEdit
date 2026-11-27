/** Scrolling to whole columns and rows, as spreadsheets do: when the grid
 *  scrolls forward to show the cursor, it stops on a column (or row)
 *  boundary, so the first column or row on screen is never cut. The wheel
 *  and the scrollbar still scroll by pixels; only the reveal snaps. */

/** One axis of the grid: where item `i` starts (from the start of item 0;
 *  `start(count)` is the full length) and which item holds position `p`. */
export interface Axis {
  start(i: number): number;
  at(p: number): number;
}

/** The scroll (measured from item `base`, the first one that scrolls) that
 *  shows item `i`, `size` long, at the far edge of a view `vis` long, landing
 *  on the first boundary that is far enough. Not past the item's own start,
 *  so an item longer than the view lands at its start. 0 when it fits. */
export function snapForward(axis: Axis, base: number, i: number, size: number, vis: number): number {
  const b0 = axis.start(base);
  const a = axis.start(i) - b0;
  const need = a + size - vis;
  if (need <= 0) return 0;
  const k = axis.at(b0 + need);
  let s = axis.start(k) - b0;
  if (s < need) s = axis.start(k + 1) - b0;
  return Math.min(s, a);
}

/** The furthest scroll: today's `content − vis + slack` (not below 0), or further to the
 *  boundary after the item cut by the far edge, so a snapped reveal of the
 *  last items is kept. When that item is longer than the view, today's max
 *  stands (a very wide last column must not leave a screen of blank space).
 *  The blank space past the end is at most that item or `slack`. */
export function snapMax(axis: Axis, base: number, count: number, vis: number, slack: number): number {
  const b0 = axis.start(base);
  const over = axis.start(count) - b0 - vis;
  const plain = Math.max(0, over + slack);
  if (over <= 0) return plain;
  const k = axis.at(b0 + over);
  const s = axis.start(k) - b0;
  if (s >= over) return Math.max(plain, s);
  const len = axis.start(k + 1) - axis.start(k);
  if (len > vis) return plain;
  return Math.max(plain, s + len);
}
