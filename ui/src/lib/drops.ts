// Several selected blocks dropped on another file's tab (gap #6): they must
// line up as for Copy, and go in packed as Copy packs them.
import { disjoint, type Rect } from "./selection";

/** Blocks to drop, in packing order; `side`: side by side (same rows). */
export type DropBlocks = { areas: Rect[]; side: boolean };

/** The blocks of a selection in the order Copy packs them, or null when
 *  several blocks do not line up (all the same rows, or all the same columns). */
export function lineUp(selected: readonly Rect[]): DropBlocks | null {
  const areas = disjoint(selected);
  if (areas.length <= 1) return areas.length ? { areas, side: true } : null;
  const sameRows = areas.every((a) => a.row === areas[0]!.row && a.rows === areas[0]!.rows);
  const sameCols = areas.every((a) => a.col === areas[0]!.col && a.cols === areas[0]!.cols);
  if (!sameRows && !sameCols) return null;
  return { areas: [...areas].sort((a, b) => (sameRows ? a.col - b.col : a.row - b.row)), side: sameRows };
}

/** Each block's rows, packed into one block: side by side, or one under the other. */
export function pack(parts: readonly string[][][], side: boolean): string[][] {
  if (parts.length === 1) return parts[0]!.map((r) => [...r]);
  return side ? parts[0]!.map((_, i) => parts.flatMap((g) => g[i] ?? [])) : parts.flat().map((r) => [...r]);
}
