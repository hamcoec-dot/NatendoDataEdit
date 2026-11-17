// Where a drag of columns, rows or a block of cells would drop them. Pure
// arithmetic, kept apart from the grid so it can be tested.

import type { Rect } from "./selection";

/** What the mouse does in the grid, for the shortcuts list (Help > Mouse
 *  and drag moves). */
export const MOUSE_MOVES: { label: string; keys: string }[] = [
  { label: "Select columns or rows", keys: "Click or drag the column headings or row numbers" },
  { label: "Move selected columns", keys: "Drag a selected column heading" },
  { label: "Move selected rows", keys: "Drag a selected row number" },
  { label: "Move the selected cells", keys: "Drag the selection's outline" },
  { label: "Copy the selected cells", keys: "Ctrl+drag the outline" },
  { label: "Stop a drag", keys: "Esc before letting go" },
  { label: "Resize a column (all selected ones)", keys: "Drag the line right of its heading" },
  { label: "Fit a column (all selected ones) to its text", keys: "Double-click that line" },
  { label: "Show hidden columns", keys: "Click the small arrow on the next heading" },
  { label: "Freeze rows or columns", keys: "Drag the bottom or right edge of the top-left corner" },
  { label: "Fill or continue a series", keys: "Drag the fill handle (Ctrl swaps)" },
  { label: "Fill down to the end of the data", keys: "Double-click the fill handle" },
  { label: "Clear the end of the selection", keys: "Drag the fill handle back inside it" },
  { label: "Jump to the edge of the data", keys: "Double-click the selection's edge on that side" },
  { label: "Open a column's menu (filter buttons off)", keys: "Point at its heading, click the arrow at the right" },
  { label: "Pick from a list of the column's values", keys: "Click the arrow on a list cell, or Alt+Down" },
  { label: "File tab menu (close others, reopen...)", keys: "Right-click the tab" },
  { label: "New file", keys: "Double-click the empty part of the tab bar" },
  { label: "Rename a tab (a saved file: rename the file)", keys: "Double-click the tab" },
  { label: "See a link's address", keys: "Point at it (Ctrl+click opens it)" },
  { label: "Put a file tab elsewhere", keys: "Drag the tab" },
  { label: "Rename a column", keys: "Double-click its heading" },
  { label: "Close a file tab", keys: "Middle-click the tab" },
  { label: "Open recent files, each in a new tab", keys: "Middle-click them in the recent files list" },
  { label: "Scroll sideways", keys: "Shift+wheel" },
  { label: "Zoom", keys: "Ctrl+wheel" },
];

/** The gap a pointer is nearest: before item `index` when it is in the
 *  first half of the item, after it otherwise. Gaps are numbered 0..count. */
export function gapAt(index: number, offset: number, size: number): number {
  return offset < size / 2 ? index : index + 1;
}

/** Moving `count` items at `at` into gap `gap` (0..total): the index they
 *  start at afterwards, or null when that is where they already are. `min`
 *  keeps items before it (the header row) where they are. */
export function dropIndex(at: number, count: number, gap: number, total: number, min = 0): number | null {
  if (count <= 0 || at < min || at + count > total) return null;
  const g = Math.min(Math.max(gap, min), total);
  if (g >= at && g <= at + count) return null;
  return g < at ? g : g - count;
}

/** Top-left cell a dragged block lands on: it keeps the grabbed cell under
 *  the pointer, clamped so the whole block stays inside the table (rows from
 *  `firstRow`). */
export function blockTarget(
  rect: Rect,
  grab: { row: number; col: number },
  over: { row: number; col: number },
  rows: number,
  cols: number,
  firstRow: number,
): { row: number; col: number } {
  const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi));
  return {
    row: clamp(rect.row + over.row - grab.row, firstRow, rows - rect.rows),
    col: clamp(rect.col + over.col - grab.col, 0, cols - rect.cols),
  };
}

/** Whether the point is on the outline of a box (within `slop` pixels of an
 *  edge, inside or out), where a block of cells can be picked up. */
export function onOutline(px: number, py: number, box: { x: number; y: number; w: number; h: number }, slop = 3): boolean {
  const inX = px >= box.x - slop && px <= box.x + box.w + slop;
  const inY = py >= box.y - slop && py <= box.y + box.h + slop;
  if (!inX || !inY) return false;
  const nearV = Math.abs(px - box.x) <= slop || Math.abs(px - (box.x + box.w)) <= slop;
  const nearH = Math.abs(py - box.y) <= slop || Math.abs(py - (box.y + box.h)) <= slop;
  return nearV || nearH;
}
