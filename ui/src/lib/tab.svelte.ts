// One open document as the UI sees it: engine info, selection, view
// settings (header row, frozen panes, column widths) and its cell cache.

import type { Check } from "./backend";
import type { Highlight } from "./views";
import type { ColumnScale } from "./m10";
import type { SortSpec } from "./backend";
import { backend, type DocInfo, type FilterSpec } from "./backend";
import { CellCache } from "./cache";
import { Columns } from "./columns";
import { single, type Selection, rectOf, type Rect } from "./selection";

/** Characters per cell fetched for display (the backend caps this too). */
export const DISPLAY_CHARS = 512;
/** Undo steps the engine keeps (engine MAX_UNDO). */
const MAX_UNDO = 10_000;

export class Tab {
  info = $state<DocInfo>() as DocInfo;
  sel = $state<Selection>(single({ row: 0, col: 0 }));
  /** First row is shown as column titles. */
  header = $state(false);
  /** The user set the header switch, so the automatic guess stays out. */
  headerTouched = false;
  /** Data rows (after the header) and columns kept in view while scrolling. */
  frozenRows = $state(0);
  frozenCols = $state(0);
  /** Bumped whenever the grid must redraw. */
  version = $state(0);
  /** Notices about this file the user dismissed. */
  dismissed = $state(false);
  busy = $state<string | null>(null);
  /** Private file mode (the file itself, or its folder). */
  private = $state<{ file: boolean; folder: boolean }>({ file: false, folder: false });
  /** What the private data finder found (null: not looked yet). */
  findings: import("./backend").Finding[] | null = null;
  /** The user said not to warn again about this tab's private data. */
  warned = false;
  /** Progress of a long operation (0 to 1), while one runs. */
  progress = $state<number | null>(null);
  /** Filter arrows on the column headers. Shown from the start so a click on
   *  a header arrow opens the filter list; Ctrl+Shift+L hides or shows them. */
  filterButtons = $state(true);
  /** The filter rules as last applied (logical columns). */
  filter = $state<FilterSpec | null>(null);
  /** Highlight rules (kept with views; not in the file). */
  highlights = $state<Highlight[]>([]);
  /** The last sort run here (kept with views). */
  sort = $state<SortSpec | null>(null);
  /** Bumped when the highlight rules change. */
  highlightVersion = 0;
  /** Notes and flags on rows on screen (shown row -> flag and text). */
  rowNotes = new Map<number, { flag: number; text: string }>();
  /** Bumped when a note changes. */
  notesVersion = 0;
  /** Data checks (kept with views) and whether problems are marked. */
  checks = $state<Check[]>([]);
  checksMarked = $state(false);
  /** "Show my edits": cells that differ from the file get a corner mark. */
  showEdits = $state(false);
  /** Those cells on screen ("row:col"). */
  edits = new Set<string>();
  /** Bumped when the checks change. */
  checkVersion = 0;
  /** Alternating row colours (kept with views). */
  banded = $state(false);
  /** Numbers, dates and text in different colours (View › Show value types). */
  valueTypes = $state(false);
  /** A line under each group: where this column's value changes. */
  divider = $state<number | null>(null);
  /** Colour scales and data bars by column (kept with views). */
  scales = $state<ColumnScale[]>([]);
  /** Smallest and largest number of each scaled column, as last read. */
  ranges = new Map<number, [number, number] | null>();
  /** Lines of text per row (1 to MAX_ROW_LINES); taller rows show wrapped
   *  columns (kept with views). */
  rowLines = $state(1);
  /** Colours for cells on screen: "row:col" or "row:*" (whole row). */
  marks = new Map<string, number>();

  /** Set by the grid showing this tab: draw again now. */
  onRedraw: (() => void) | null = null;
  scrollX = 0;
  scrollY = 0;
  columns: Columns;
  cache: CellCache;

  constructor(info: DocInfo, defaultWidth: number) {
    this.info = info;
    this.columns = new Columns(info.cols, defaultWidth);
    this.cache = new CellCache(
      (row, rows, col, cols) => backend().viewport(this.info.id, row, rows, col, cols, DISPLAY_CHARS),
      () => this.bump(),
    );
  }

  get id(): number {
    return this.info.id;
  }

  /** Document row of the first data row (1 when the first row is a header). */
  get firstRow(): number {
    return this.header && this.info.rows > 0 ? 1 : 0;
  }

  get rect(): Rect {
    return rectOf(this.sel);
  }

  /** Column moves by the undo position they made, so undo and redo carry
   *  what the view keeps per column (widths, formats, wrapping, highlight
   *  rules, colour scales, checks, the filter and sort as last run) back and
   *  forth with the columns. As many as the engine keeps undo steps. */
  private colMoves: { pos: number; at: number; count: number; to: number }[] = [];
  /** What Cut (Ctrl+X) marked, waiting for Paste to move it; dropped by
   *  Esc, another cut or copy, or any change (`pos` is the undo position). */
  cut = $state<{ kind: "cols" | "rows" | "cells"; rect: Rect; pos: number; clip: string | null } | null>(null);
  /** Redo calls in flight: a rise of the undo position during one is a redo
   *  (replay the moves), otherwise a new change (forget undone moves). */
  redoing = 0;

  /** Columns were just moved (the change is already in `info`). */
  noteColMove(at: number, count: number, to: number): void {
    this.followMove(at, count, to);
    this.colMoves.push({ pos: this.info.history_pos, at, count, to });
    if (this.colMoves.length > MAX_UNDO) this.colMoves.shift();
  }

  /** Carry per-column view state along with a column move. */
  private followMove(at: number, count: number, to: number): void {
    if (count <= 0 || at === to) return;
    this.columns.move(at, count, to);
    const place = (c: number): number => {
      if (c >= at && c < at + count) return to + (c - at);
      if (to > at && c >= at + count && c < to + count) return c - count;
      if (to < at && c >= to && c < at) return c + count;
      return c;
    };
    if (this.highlights.length) {
      this.highlights = this.highlights.map((h) => ({ ...h, col: place(h.col) }));
      this.highlightVersion++;
    }
    if (this.scales.length) {
      this.scales = this.scales.map((s) => ({ ...s, col: place(s.col) }));
      this.ranges.clear();
    }
    if (this.checks.length) {
      this.checks = this.checks.map((k) => (k.col === null ? k : { ...k, col: place(k.col) }));
      this.checkVersion++;
    }
    if (this.filter) this.filter = { ...this.filter, columns: this.filter.columns.map((f) => ({ ...f, col: place(f.col) })) };
    if (this.sort) this.sort = { ...this.sort, keys: this.sort.keys.map((k) => ({ ...k, col: place(k.col) })) };
    if (this.divider !== null) this.divider = place(this.divider);
  }

  /** Take new engine info; refetch cells when the data may have changed. */
  update(info: DocInfo, dataChanged: boolean): void {
    const structural =
      info.cols !== this.info.cols || info.rows !== this.info.rows || info.filtered !== this.info.filtered;
    if (!info.has_filter) this.filter = null;
    const before = this.info.history_pos;
    const after = info.history_pos;
    if (after !== before) this.cut = null;
    if (after < before) {
      // Only undo lowers the position: take back every move undone, newest first.
      for (const m of this.colMoves.filter((x) => x.pos > after && x.pos <= before).reverse()) this.followMove(m.to, m.count, m.at);
    } else if (after > before && this.redoing > 0) {
      for (const m of this.colMoves.filter((x) => x.pos > before && x.pos <= after)) this.followMove(m.at, m.count, m.to);
    } else if (after > before) {
      // A new change: moves that could have been redone are gone.
      this.colMoves = this.colMoves.filter((x) => x.pos <= before);
    }
    this.info = info;
    this.columns.setCount(info.cols);
    if (dataChanged || structural) this.cache.invalidate();
    this.bump();
  }

  /** Something visible changed: redraw. */
  bump(): void {
    this.version++;
    this.onRedraw?.();
  }
}
