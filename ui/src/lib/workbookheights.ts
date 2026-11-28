// A workbook's own row heights (gap #7), set on its tab in the step that
// makes the tab: not an undo step, and not a change to the file.
import type { Imported } from "./backend";
import { MAX_CUSTOM_ROWS, RowHeights } from "./rowheights";
import type { Tab } from "./tab.svelte";

const plural = (n: number, one: string): string => `${n.toLocaleString("en-US")} ${n === 1 ? one : `${one}s`}`;

/** Set `d`'s row heights on `t` and say so (empty when there were none).
 *  The title row's height is left out with `header` on (the titles have no
 *  row of their own in the grid; turning "First row is header" off later
 *  leaves that row at the plain height), and so is a height equal to the
 *  plain one, `plain` px (it would only use up the cap). */
export function applyWorkbookHeights(t: Tab, d: Imported, header: boolean, plain: number): string {
  const raw = d.row_heights ?? [];
  if (!raw.length) return "";
  const keep = raw.filter(([r, px]) => !(header && r === 0) && px !== plain);
  if (!keep.length) return "";
  const { heights } = RowHeights.fromView(keep, t.info.total_rows);
  if (heights.empty) return "";
  t.rowHeights.restore(heights.snapshot());
  // This is how the tab starts: the look Undo goes back to, not a step.
  t.baseline();
  const over = d.row_heights_over ?? 0;
  return ` Row heights from the workbook kept for ${plural(heights.count, "row")}${over ? ` (the first ${MAX_CUSTOM_ROWS.toLocaleString("en-US")} that have one; ${plural(over, "more row")} left at the plain height)` : ""}.`;
}
