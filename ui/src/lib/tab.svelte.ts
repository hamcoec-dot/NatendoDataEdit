// One open document as the UI sees it: engine info, selection, view
// settings (header row, frozen panes, column widths) and its cell cache.

import type { Check } from "./backend";
import type { Highlight } from "./views";
import type { ColumnScale } from "./m10";
import type { SortSpec } from "./backend";
import { backend, type DocInfo, type FilterSpec } from "./backend";
import { CellCache } from "./cache";
import { Columns, type ColSnap } from "./columns";
import { single, type Selection, rectOf, type Rect, type Cell, activeOf } from "./selection";

/** Characters per cell fetched for display (the backend caps this too). */
export const DISPLAY_CHARS = 512;
/** Undo steps the engine keeps (engine MAX_UNDO). */
const MAX_UNDO = 10_000;
/** Undo steps kept for changes to how the table looks (widths and so on). */
const MAX_VIEW_STEPS = 1_000;

/** How the table looks, beyond the file: what Undo puts back for a change
 *  that only touches the look (widths, hiding columns, display formats,
 *  wrapping, highlight rules, colour scales, banding, the group line, row
 *  height, data checks and the filter arrows). */
export interface ViewLook {
  cols: ColSnap;
  highlights: Highlight[];
  scales: ColumnScale[];
  banded: boolean;
  divider: number | null;
  rowLines: number;
  checks: Check[];
  filterButtons: boolean;
}
/** The look, plus the filter rules and last sort (those change with an
 *  engine undo step, so only engine steps put them back). */
export interface ViewSnap {
  look: ViewLook;
  filter: FilterSpec | null;
  sort: SortSpec | null;
}
/** A change to the look alone, made at undo position `pos`. */
export interface LookStep {
  pos: number;
  before: ViewLook;
  after: ViewLook;
}

export class Tab {
  info = $state<DocInfo>() as DocInfo;
  sel = $state<Selection>(single({ row: 0, col: 0 }));
  /** First row is shown as column titles. */
  header = $state(false);
  /** The user set the header switch, so the automatic guess stays out. */
  headerTouched = false;
  /** Undo position just after Rename column added a header row to a file
   *  without one (and switched "First row is header" on): undoing past it
   *  switches the setting off again. */
  headerAddedAt: number | null = null;
  /** Undo switched that setting off: redo switches it back on (until
   *  another change is made, which ends the redo). */
  headerOffByUndo = false;
  /** Data-check rules a rename rewrote, by the undo position just after the
   *  rename: undoing the rename puts the rules back as they were, redo
   *  rewrites them again (only while nobody changed the rules since). */
  refEdits: { at: number; before: Check[]; after: Check[]; applied: boolean }[] = [];
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

  /** The active cell (where typing goes; see `selection.ts`). */
  get cell(): Cell {
    return activeOf(this.sel);
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
  cut = $state<{ kind: "cols" | "rows" | "cells"; rect: Rect; pos: number; clip: string; inside: boolean } | null>(null);
  /** Changes to the look alone, oldest first (undone before the engine
   *  step that reached their position), and those undone, for redo. */
  lookSteps: LookStep[] = [];
  lookRedo: LookStep[] = [];
  /** Each engine step's look just before and after it, by the undo
   *  position it made: undo and redo put the widths, formats and so on
   *  back as they were (a deleted column comes back with its width). */
  stepViews = new Map<number, { before: ViewSnap; after: ViewSnap | null }>();
  /** A look change made after undoing: the engine's redo is over at this
   *  position (as a new change would end it). */
  redoCut: number | null = null;
  /** Bumped when the look steps change (Undo and Redo enable). */
  undoVersion = $state(0);
  /** The look as last recorded, and the undo position then. */
  private lastView: ViewSnap | null = null;
  private lastKey = "";
  private lastPos = -1;
  /** Undo position of a new engine step waiting for its "after" look. */
  private pendingStep: number | null = null;

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

  /** The look now (copies). */
  snapView(): ViewSnap {
    return {
      look: {
        cols: this.columns.snapshot(),
        highlights: $state.snapshot(this.highlights) as Highlight[],
        scales: $state.snapshot(this.scales) as ColumnScale[],
        banded: this.banded,
        divider: this.divider,
        rowLines: this.rowLines,
        checks: $state.snapshot(this.checks) as Check[],
        filterButtons: this.filterButtons,
      },
      filter: this.filter ? ($state.snapshot(this.filter) as FilterSpec) : null,
      sort: this.sort ? ($state.snapshot(this.sort) as SortSpec) : null,
    };
  }

  /** Note a change to the look since last time as one undo step (called
   *  after each click or key, and before undo, redo and engine changes).
   *  After an engine step it only takes the look as it is now. */
  recordView(): void {
    const now = this.snapView();
    const key = JSON.stringify(now.look);
    const pos = this.info.history_pos;
    if (this.lastView === null || pos !== this.lastPos) {
      const s = this.pendingStep === pos ? this.stepViews.get(pos) : undefined;
      if (s) s.after = now;
      this.pendingStep = null;
    } else if (key !== this.lastKey) {
      this.lookSteps.push({ pos, before: this.lastView.look, after: now.look });
      if (this.lookSteps.length > MAX_VIEW_STEPS) this.lookSteps.shift();
      if (this.lookRedo.length || this.info.can_redo) this.redoCut = pos;
      this.lookRedo = [];
      this.undoVersion++;
    }
    this.lastView = now;
    this.lastKey = key;
    this.lastPos = pos;
  }

  /** Take the look as it is now without recording a step (after an undo
   *  or redo has put back what follows it, such as renamed rules). */
  baseline(): void {
    this.lastView = this.snapView();
    this.lastKey = JSON.stringify(this.lastView.look);
    this.lastPos = this.info.history_pos;
    this.pendingStep = null;
  }

  /** Put a look back. `engine`: for an engine undo or redo (the filter
   *  and sort come back too; data checks follow the rename rules instead). */
  restoreView(s: ViewSnap, engine: boolean): void {
    const l = s.look;
    this.columns.restore(l.cols);
    this.highlights = structuredClone(l.highlights);
    this.highlightVersion++;
    this.scales = structuredClone(l.scales);
    this.ranges.clear();
    this.banded = l.banded;
    this.divider = l.divider;
    this.rowLines = l.rowLines;
    this.filterButtons = l.filterButtons;
    if (engine) {
      this.filter = this.info.has_filter && s.filter ? structuredClone(s.filter) : null;
      this.sort = s.sort ? structuredClone(s.sort) : null;
    } else {
      this.checks = structuredClone(l.checks);
      this.checkVersion++;
    }
    this.lastView = this.snapView();
    this.lastKey = JSON.stringify(this.lastView.look);
    this.lastPos = this.info.history_pos;
    this.undoVersion++;
    this.bump();
  }

  /** Undo can take back a look change here, or an engine step. */
  get canUndo(): boolean {
    void this.undoVersion;
    return this.lookSteps.at(-1)?.pos === this.info.history_pos || this.info.can_undo;
  }

  get canRedo(): boolean {
    void this.undoVersion;
    return this.lookRedo.at(-1)?.pos === this.info.history_pos || (this.info.can_redo && this.redoCut !== this.info.history_pos);
  }

  /** Take new engine info; refetch cells when the data may have changed. */
  update(info: DocInfo, dataChanged: boolean): void {
    // A look change made just before this engine change is a step of its own.
    this.recordView();
    const structural =
      info.cols !== this.info.cols || info.rows !== this.info.rows || info.filtered !== this.info.filtered;
    if (!info.has_filter) this.filter = null;
    const before = this.info.history_pos;
    const after = info.history_pos;
    // Any change, and any change of which rows are shown (a filter), drops a
    // pending cut: its rows would now be other rows.
    if (dataChanged || after !== before || structural || info.has_filter !== this.info.has_filter || info.hidden_rows !== this.info.hidden_rows) this.cut = null;
    if (after < before) {
      // Only undo lowers the position: take back every move undone, newest first.
      for (const m of this.colMoves.filter((x) => x.pos > after && x.pos <= before).reverse()) this.followMove(m.to, m.count, m.at);
    } else if (after > before && this.redoing > 0) {
      for (const m of this.colMoves.filter((x) => x.pos > before && x.pos <= after)) this.followMove(m.at, m.count, m.to);
    } else if (after > before) {
      // A new change: moves that could have been redone are gone.
      this.colMoves = this.colMoves.filter((x) => x.pos <= before);
    }
    if (after > before && this.redoing === 0) {
      // A new change ends the redo of look changes too, and takes the look
      // before it (the look after it is taken once it has settled).
      this.lookRedo = [];
      this.redoCut = null;
      for (const k of [...this.stepViews.keys()]) if (k > before) this.stepViews.delete(k);
      if (after === before + 1 && this.lastView) {
        this.stepViews.set(after, { before: this.lastView, after: null });
        this.pendingStep = after;
      }
      if (this.stepViews.size > MAX_UNDO) this.stepViews.delete(this.stepViews.keys().next().value!);
    }
    if (after !== before) setTimeout(() => this.recordView(), 0);
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
