// The format painter: copy the look of some columns and give it to others.
// A look is the display format, alignment, wrapping, "cut off long text" and
// width of each column, and nothing else: no cell text, no titles, no hidden
// state, no column type, protection, totals, highlight rules or data checks.
// Pasting changes only the look (one look undo step), never the file.

import type { Align, Columns } from "./columns";
import { cleanFormat, type DisplayFormat } from "./m9";

/** One column's look. */
export interface ColLook {
  format: DisplayFormat | null;
  align: Align | null;
  wrap: boolean;
  clip: boolean;
  /** Width in pixels when shown. */
  width: number;
}

/** The copied look of one or more columns, in order. */
export interface CopiedLook {
  cols: ColLook[];
}

/** The most columns a look is copied from. */
export const MAX_LOOK_COLS = 16_384;

/** The look of `cols` (in that order; at most MAX_LOOK_COLS). */
export function copyLook(columns: Columns, cols: number[]): CopiedLook {
  return {
    cols: cols.slice(0, MAX_LOOK_COLS).map((c) => {
      const f = columns.formats.get(c);
      return {
        format: f ? structuredClone(f) : null,
        align: columns.aligns.get(c) ?? null,
        wrap: columns.wraps.has(c),
        clip: columns.clips.has(c),
        width: columns.shownWidth(c),
      };
    }),
  };
}

/** Give `look` to `targets` (each in turn: one copied column goes onto every
 *  target; several repeat across them in order, as Excel's painter does).
 *  A hidden target keeps hidden (its width is left). How many columns changed. */
export function pasteLook(columns: Columns, targets: number[], look: CopiedLook): number {
  if (!look.cols.length || !targets.length) return 0;
  const before = columns.key();
  const widths: [number, number][] = [];
  targets.forEach((c, k) => {
    const l = look.cols[k % look.cols.length]!;
    const f = l.format ? cleanFormat(l.format) : null;
    if (f) columns.formats.set(c, f);
    else columns.formats.delete(c);
    if (l.align) columns.aligns.set(c, l.align);
    else columns.aligns.delete(c);
    if (l.wrap) columns.wraps.add(c);
    else columns.wraps.delete(c);
    if (l.clip) columns.clips.add(c);
    else columns.clips.delete(c);
    if (!columns.isHidden(c)) widths.push([c, l.width]);
  });
  columns.setWidths(widths);
  return columns.key() === before ? 0 : targets.length;
}
