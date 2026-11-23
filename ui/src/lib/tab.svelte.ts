// One open document as the UI sees it: engine info, selection, view
// settings (header row, frozen panes, column widths) and its cell cache.

import type { Check, FindSpec } from "./backend";
import type { Highlight } from "./views";
import type { ColumnScale } from "./m10";
import type { SortSpec } from "./backend";
import { backend, type DocInfo, type FilterSpec } from "./backend";
import { CellCache } from "./cache";
import { Columns, type ColSnap } from "./columns";
import { RowHeights, type RowHeightSnap } from "./rowheights";
import { single, type Selection, rectOf, type Rect, type Cell, activeOf, MAX_AREAS, sameRect, rectInside } from "./selection";

/** Characters per cell fetched for display (the backend caps this too). */
export const DISPLAY_CHARS = 512;
/** Undo steps the engine keeps (engine MAX_UNDO). */
const MAX_UNDO = 10_000;
/** Undo steps kept for changes to how the table looks (widths and so on). */
const MAX_VIEW_STEPS = 1_000;
/** About this much memory may hold looks kept for undo; past it the oldest
 *  are let go (a 100,000-value check list is a few megabytes). */
const LOOK_BUDGET = 64 << 20;

/** How the table looks, beyond the file: what Undo puts back for a change
 *  that only touches the look (widths, hiding columns, display formats,
 *  wrapping, highlight rules, colour scales, banding, the group line, row
 *  height and data checks). Parts that did not change are shared with the
 *  look before, so keeping one per undo step costs little. */
export interface ViewLook {
  cols: ColSnap;
  highlights: Highlight[];
  scales: ColumnScale[];
  banded: boolean;
  divider: number | null;
  rowLines: number;
  /** Custom row heights (shared by reference while unchanged). */
  rows: RowHeightSnap;
  checks: Check[];
}
/** The look, plus the last sort (it changes with an engine undo step, so
 *  only engine steps put it back; the filter rules are read from the engine,
 *  which holds them). */
export interface ViewSnap {
  look: ViewLook;
  sort: SortSpec | null;
}
/** A change to the look alone, made at undo position `pos`. */
export interface LookStep {
  pos: number;
  before: ViewLook;
  after: ViewLook;
}

/** Two sets of custom row heights are the same (shared, or equal). */
function sameRows(a: RowHeightSnap, b: RowHeightSnap): boolean {
  if (a === b || (a.rows === b.rows && a.px === b.px)) return true;
  if (a.rows.length !== b.rows.length) return false;
  for (let i = 0; i < a.rows.length; i++) if (a.rows[i] !== b.rows[i] || a.px[i] !== b.px[i]) return false;
  return true;
}

/** A change to the rows that the heights must follow, set just before the
 *  change is made and used when the new state arrives (`update`). "reset":
 *  the rows cannot be followed exactly (some are hidden), so the heights go. */
export type RowMap = { kind: "insert"; at: number; count: number } | { kind: "delete"; at: number; count: number } | { kind: "move"; at: number; count: number; to: number } | { kind: "reset" };

/** Two looks hold the same values (parts shared or equal). */
function sameLook(a: ViewLook, b: ViewLook): boolean {
  return (
    a.banded === b.banded &&
    a.divider === b.divider &&
    a.rowLines === b.rowLines &&
    sameRows(a.rows, b.rows) &&
    (a.cols === b.cols || JSON.stringify(a.cols) === JSON.stringify(b.cols)) &&
    (a.highlights === b.highlights || JSON.stringify(a.highlights) === JSON.stringify(b.highlights)) &&
    (a.scales === b.scales || JSON.stringify(a.scales) === JSON.stringify(b.scales)) &&
    (a.checks === b.checks || JSON.stringify(a.checks) === JSON.stringify(b.checks))
  );
}

export class Tab {
  /** Rows with a height of their own (none: every row is the plain height). */
  rowHeights = new RowHeights();
  /** How the heights must follow the row change about to be made (see `RowMap`). */
  pendingRows: RowMap | null = null;
  /** Said with the next message: the heights were cleared (and why). */
  rowHeightsNote = "";
  info = $state<DocInfo>() as DocInfo;
  private _sel = $state<Selection>(single({ row: 0, col: 0 }));
  /** The selected block (the last one picked when several are selected).
   *  Putting a new selection here is picking one block again: the earlier
   *  blocks of a Ctrl+click selection go (use `setPrimary` to move the last
   *  block only, or `addArea` to add one). */
  get sel(): Selection {
    return this._sel;
  }
  set sel(v: Selection) {
    this._sel = v;
    if (this.extra.length) this.extra = [];
  }
  /** The blocks picked before the last one (Ctrl+click), oldest first. */
  extra = $state<Rect[]>([]);
  /** Move the last block (Shift+click, Shift+arrows, a Ctrl+drag) and keep the earlier ones. */
  setPrimary(v: Selection): void {
    this._sel = v;
  }
  /** Ctrl+click or Ctrl+drag: the block selected so far becomes an earlier
   *  block and `v` the new last one. A block already inside an earlier one
   *  adds nothing. False when the 10,000-block cap is reached. */
  addArea(v: Selection): boolean {
    const r = rectOf(this._sel);
    if (!this.extra.some((x) => sameRect(x, r) || rectInside(r, x))) {
      if (this.extra.length + 1 >= MAX_AREAS) return false;
      this.extra = [...this.extra, r];
    }
    this._sel = v;
    return true;
  }
  /** Every selected block: the earlier ones, then the last. */
  get areas(): Rect[] {
    return [...this.extra, rectOf(this._sel)];
  }
  /** The one selected block, or null when several blocks are selected: code
   *  that works on one block must handle null (refuse, or say "one block only"). */
  singleRect(): Rect | null {
    return this.extra.length ? null : rectOf(this._sel);
  }
  /** First row is shown as column titles. */
  header = $state(false);
  /** The user set the header switch, so the automatic guess stays out. */
  headerTouched = $state(false);
  /** The title-row question in the bar (null: not asked): how sure the
   *  guess applied is. */
  headerAsk = $state<"yes" | "no" | "unsure" | null>(null);
  /** How sure the guess made at open was (null: not guessed), so asking
   *  again words it the same way. */
  headerSure: "yes" | "no" | "unsure" | null = null;
  /** The question was closed without an answer (the guess is kept). */
  headerAskClosed = $state(false);
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
  /** The undo position the last sort made: its columns show a sort arrow
   *  until the sort is undone (or another one replaces it). */
  sortAt = $state<number | null>(null);

  /** The cells the last change made, for Undo to select (callers that
   *  select the result after the change put it here). */
  noteStepSel(sel: Selection): void {
    const w = this.stepWhat.get(this.info.history_pos);
    if (w) {
      w.sel = { ...sel };
      w.wide = false;
    }
  }

  /** The sort arrow a column shows, if any. */
  sortArrow(c: number): "up" | "down" | null {
    if (!this.sort || this.sortAt === null || this.info.history_pos < this.sortAt) return null;
    const k = this.sort.keys.find((x) => x.col === c);
    return k ? (k.descending ? "down" : "up") : null;
  }
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
  /** Cells Go To Special lit up ("row:col", shown rows); gone with the
   *  next change or change of shown rows. */
  picked = new Set<string>();
  /** The column insight strip's figures by column (View › Column insights). */
  insights = $state<{ byCol: Map<number, import("./backend").Insight>; sampled: number; rows: number } | null>(null);
  /** The steps panel shows the table as it was after an earlier step
   *  (it cannot be edited until it goes back to the latest). */
  peeking = $state(false);
  /** The filter panel's cards (kept with views). */
  facetCards = $state<{ col: number; kind: "values" | "numbers" | "dates"; range: [number, number] | null }[]>([]);
  /** A counts tab: the tab and columns it counts (a line can show its
   *  rows there). */
  countsOf: { source: number; cols: number[] } | null = null;
  /** A formula has been typed in a cell here, so F2 and the cell bar ask
   *  whether a cell's value came from one. */
  hasFormulas = false;
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
  /** The find bar's search while it is open (every match on screen is lit
   *  up), its version (bumped on each change) and the lit cells "row:col". */
  findSpec: FindSpec | null = null;
  findVersion = 0;
  /** How many cells match while the find bar is open (the status bar). */
  findCount = $state<number | null>(null);
  findHits = new Set<string>();

  /** The grids showing this tab (two in a split view): draw again now. */
  redraws = new Set<() => void>();
  onRedraw = (): void => {
    for (const f of this.redraws) f();
  };
  scrollX = 0;
  scrollY = 0;
  /** View › Split: two panes of this tab, top and bottom or side by side;
   *  `at`: the first pane's share of the space. */
  split = $state<{ dir: "rows" | "cols"; at: number } | null>(null);
  /** Where the split view's second pane is scrolled to. */
  splitScroll = { scrollX: 0, scrollY: 0 };
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

  /** The last block picked, for drawing and gestures in the grid only (everything
   *  else asks `singleRect()` or `areas`). */
  get drawRect(): Rect {
    return rectOf(this._sel);
  }

  /** Column moves by the undo position they made, so undo and redo carry
   *  what the view keeps per column (widths, formats, wrapping, highlight
   *  rules, colour scales, checks, the filter and sort as last run) back and
   *  forth with the columns. As many as the engine keeps undo steps. */
  private colMoves: { pos: number; at: number; count: number; to: number }[] = [];
  /** What Cut (Ctrl+X) marked, waiting for Paste to move it; dropped by
   *  Esc, another cut or copy, or any change (`pos` is the undo position). */
  cut = $state<{ kind: "cols" | "rows" | "cells"; rect: Rect; pos: number; clip: string; inside: boolean } | null>(null);
  /** What Copy last put on the clipboard (a dashed outline until Esc or
   *  the next change). */
  copied = $state<Rect | null>(null);
  /** Changes to the look alone, oldest first (undone before the engine
   *  step that reached their position), and those undone, for redo. */
  lookSteps: LookStep[] = [];
  lookRedo: LookStep[] = [];
  /** Each engine step's look just before and after it, by the undo
   *  position it made: undo and redo put the widths, formats and so on
   *  back as they were (a deleted column comes back with its width). */
  stepViews = new Map<number, { before: ViewSnap; after: ViewSnap | null }>();
  /** What each engine step was and where (by the undo position it made):
   *  undo and redo select those cells again and say what was undone. */
  stepWhat = new Map<number, { what: string | null; sel: Selection; wide?: boolean }>();
  /** The next engine change is a tool over whole columns or the table
   *  (sort, filter, cleaning...): undoing it leaves the selection alone. */
  wideNext = false;
  /** What the next engine change is (set just before it is made). */
  whatNext: string | null = null;
  /** The label of the command running now, if any (set by the app). */
  static activeCommand: () => string | null = () => null;
  /** A look change made after undoing: the engine's redo is over at this
   *  position (as a new change would end it). */
  redoCut: number | null = null;
  /** Bumped when the look steps change (Undo and Redo enable). */
  undoVersion = $state(0);
  /** Saved views applied: their engine steps (from `start` to `end`) and
   *  look undo and redo as one. */
  groups: { start: number; end: number }[] = [];
  /** A saved view is being applied (its look joins its first engine step). */
  grouping = false;
  /** The look as last recorded, and the undo position then. */
  private lastView: ViewSnap | null = null;
  private lastPos = -1;
  /** What the parts of `lastView` were copied from (to tell cheaply whether
   *  they changed: these lists are replaced, never changed in place). */
  private lastRefs: { cols: string; highlights: unknown; scales: unknown; checks: unknown; sort: unknown } | null = null;
  /** Memory held by looks kept for undo (roughly), and each look's share
   *  (counted down once when a step holding it is let go; a look still
   *  shared by a later step is then undercounted, which only lets a little
   *  more be kept). */
  private lookBytes = 0;
  private lookSize = new WeakMap<ViewLook, number>();
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

  /** The look now. Only parts that changed since the last look are copied
   *  (and only they are turned into text to compare); the rest are shared,
   *  so a key press costs next to nothing however long the lists are.
   *  The same object comes back while nothing changed.
   *
   *  This relies on the highlight rules, colour scales, data checks and sort
   *  being REPLACED whenever they change (`t.checks = [...]`), never changed
   *  in place (no push, splice or `t.checks[i].x = ...`): a change in place
   *  would not be seen, so it would not be an undo step, and it would also
   *  change the copies kept for undo. Dialogs edit their own copies. */
  snapView(): ViewSnap {
    const last = this.lastView;
    const refs = this.lastRefs;
    const cols = this.columns.key();
    const same = (a: unknown, b: unknown): boolean => a === b;
    if (
      last &&
      refs &&
      refs.cols === cols &&
      same(refs.highlights, this.highlights) &&
      same(refs.scales, this.scales) &&
      same(refs.checks, this.checks) &&
      same(refs.sort, this.sort) &&
      last.look.banded === this.banded &&
      last.look.divider === this.divider &&
      last.look.rowLines === this.rowLines &&
      this.rowHeights.sameAs(last.look.rows)
    )
      return last;
    let bytes = 64;
    const copy = <T>(changed: boolean, keep: T | undefined, v: unknown): T => {
      if (!changed && keep !== undefined) return keep;
      const c = $state.snapshot(v) as T;
      bytes += JSON.stringify(c ?? null).length * 2;
      return c;
    };
    const colsChanged = !refs || refs.cols !== cols;
    const look: ViewLook = {
      cols: colsChanged || !last ? this.columns.snapshot() : last.look.cols,
      highlights: copy<Highlight[]>(!refs || !same(refs.highlights, this.highlights), last?.look.highlights, this.highlights),
      scales: copy<ColumnScale[]>(!refs || !same(refs.scales, this.scales), last?.look.scales, this.scales),
      banded: this.banded,
      divider: this.divider,
      rowLines: this.rowLines,
      rows: this.rowHeights.snapshot(),
      checks: copy<Check[]>(!refs || !same(refs.checks, this.checks), last?.look.checks, this.checks),
    };
    if (colsChanged) bytes += cols.length * 2;
    // The heights are shared by reference unless they changed: count them once.
    if (!last || !sameRows(last.look.rows, look.rows)) bytes += this.rowHeights.bytes;
    const sort = copy<SortSpec | null>(!refs || !same(refs.sort, this.sort), last?.sort, this.sort);
    this.lastRefs = { cols, highlights: this.highlights, scales: this.scales, checks: this.checks, sort: this.sort };
    this.lookSize.set(look, bytes);
    this.noteLookBytes(bytes);
    return { look, sort };
  }

  /** Count memory held by a new look; past the budget, let the oldest
   *  looks go (their steps can no longer put the look back). */
  private noteLookBytes(n: number): void {
    this.lookBytes += n;
    if (this.lookBytes <= LOOK_BUDGET) return;
    const keys = [...this.stepViews.keys()].sort((a, b) => a - b);
    const cut = keys[Math.floor(keys.length / 2)] ?? Infinity;
    for (const k of keys) if (k < cut) this.dropStepView(k);
    const half = Math.floor(this.lookSteps.length / 2);
    for (const s of this.lookSteps.slice(0, half)) this.releaseStep(s.before, s.after);
    this.lookSteps = this.lookSteps.slice(half);
    // Still over (the oldest looks were shared): start the count afresh.
    if (this.lookBytes > LOOK_BUDGET) this.lookBytes = Math.floor(LOOK_BUDGET / 2);
    this.undoVersion++;
  }

  /** Redo of look changes is over (a new change was made). */
  private clearLookRedo(): void {
    for (const s of this.lookRedo) this.releaseStep(s.before, s.after);
    this.lookRedo = [];
  }

  /** A kept look is let go: count its memory down (once). */
  private releaseStep(...looks: (ViewLook | undefined)[]): void {
    for (const l of looks) {
      if (!l) continue;
      const n = this.lookSize.get(l);
      if (n === undefined) continue;
      this.lookSize.delete(l);
      this.lookBytes = Math.max(0, this.lookBytes - n);
    }
  }

  private dropStepView(k: number): void {
    const s = this.stepViews.get(k);
    if (!s) return;
    this.stepViews.delete(k);
    this.releaseStep(s.before.look, s.after?.look);
  }

  /** Note a change to the look since last time as one undo step (called
   *  after each click or key, and before undo, redo and engine changes).
   *  After an engine step it only takes the look as it is now. */
  recordView(): void {
    const was = this.lastView;
    const now = this.snapView();
    const pos = this.info.history_pos;
    if (was === null || pos !== this.lastPos) {
      const s = this.pendingStep === pos ? this.stepViews.get(pos) : undefined;
      if (s) s.after = now;
      this.pendingStep = null;
    } else if (now !== was && !sameLook(now.look, was.look)) {
      this.lookSteps.push({ pos, before: was.look, after: now.look });
      if (this.lookSteps.length > MAX_VIEW_STEPS) {
        const old = this.lookSteps.shift()!;
        this.releaseStep(old.before, old.after);
      }
      if (this.lookRedo.length || this.info.can_redo) this.redoCut = pos;
      this.clearLookRedo();
      this.undoVersion++;
    }
    this.lastView = now;
    this.lastPos = pos;
  }

  /** Take the look as it is now without recording a step (after an undo
   *  or redo has put back what follows it, such as renamed rules). */
  baseline(): void {
    this.lastView = this.snapView();
    this.lastPos = this.info.history_pos;
    this.pendingStep = null;
  }

  /** A saved view was applied from undo position `start`: its look and
   *  engine steps undo and redo as one. */
  endGroup(start: number): void {
    this.grouping = false;
    const pos = this.info.history_pos;
    const now = this.snapView();
    if (pos > start) {
      const s = this.stepViews.get(pos);
      if (s) s.after = now;
      this.groups = [...this.groups.filter((g) => g.end <= start), { start, end: pos }].slice(-MAX_VIEW_STEPS);
      this.lastView = now;
      this.lastPos = pos;
      this.pendingStep = null;
    }
  }

  /** After a save rewrote the file the engine's undo history is gone:
   *  drop the looks kept for its steps. */
  afterSave(): void {
    const pos = this.info.history_pos;
    this.stepViews.clear();
    this.groups = [];
    this.lookSteps = this.lookSteps.filter((s) => s.pos >= pos);
    this.lookBytes = 0;
    this.lookSize = new WeakMap();
    this.undoVersion++;
  }

  /** Put a look back. `engine`: for an engine undo or redo (the sort comes
   *  back too; data checks follow the rename rules instead). */
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
    // (A look from before heights existed has none.)
    this.rowHeights.restore(l.rows ?? { rows: [], px: [], sum: [0] });
    if (engine) {
      this.sort = s.sort ? structuredClone(s.sort) : null;
    } else {
      this.checks = structuredClone(l.checks);
      this.checkVersion++;
    }
    this.baseline();
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
    // A look change made just before this engine change is a step of its own
    // (while a saved view is applied, its look joins its first engine step).
    if (!this.grouping) this.recordView();
    const structural =
      info.cols !== this.info.cols || info.rows !== this.info.rows || info.filtered !== this.info.filtered;
    if (!info.has_filter) this.filter = null;
    const before = this.info.history_pos;
    const after = info.history_pos;
    // Several blocks stand for rows and columns as they were: other rows
    // (a filter, undo, redo) or a changed shape make them stale.
    if (this.extra.length && (structural || after < before || this.redoing > 0 || info.has_filter !== this.info.has_filter || info.hidden_rows !== this.info.hidden_rows)) this.extra = [];
    this.followRows(info);
    // Any change, and any change of which rows are shown (a filter), drops a
    // pending cut: its rows would now be other rows.
    if (dataChanged || after !== before || structural || info.has_filter !== this.info.has_filter || info.hidden_rows !== this.info.hidden_rows) {
      this.cut = null;
      this.copied = null;
      this.picked = new Set();
    }
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
      this.clearLookRedo();
      this.redoCut = null;
      if (this.groups.some((g) => g.end > before)) this.groups = this.groups.filter((g) => g.end <= before);
      for (const k of [...this.stepViews.keys()]) if (k > before) this.dropStepView(k);
      for (const k of [...this.stepWhat.keys()]) if (k > before) this.stepWhat.delete(k);
      // A sort that was undone is gone for good once something else is done.
      if (this.sortAt !== null && this.sortAt > before) this.sortAt = null;
      const cmd = Tab.activeCommand();
      const what = cmd ? `“${cmd}”` : this.whatNext;
      this.stepWhat.set(after, { what, sel: { ...this.sel }, wide: this.wideNext });
      if (this.stepWhat.size > MAX_UNDO) this.stepWhat.delete(this.stepWhat.keys().next().value!);
      if (after === before + 1 && this.lastView) {
        this.stepViews.set(after, { before: this.lastView, after: null });
        this.pendingStep = after;
      }
      if (this.stepViews.size > MAX_UNDO) this.dropStepView(this.stepViews.keys().next().value!);
    }
    if (after !== before) setTimeout(() => this.recordView(), 0);
    this.whatNext = null;
    this.wideNext = false;
    // Columns added or taken away: the sort's columns may be other ones now.
    if (info.cols !== this.info.cols) this.sortAt = null;
    this.info = info;
    this.columns.setCount(info.cols);
    if (dataChanged || structural) this.cache.invalidate();
    this.bump();
  }

  /** Custom row heights follow the rows they belong to, or go with a note when
   *  that cannot be done exactly. Undo and Redo put them back with the look. */
  private followRows(info: DocInfo): void {
    const pend = this.pendingRows;
    this.pendingRows = null;
    if (this.rowHeights.empty) return;
    const before = this.info.history_pos;
    const after = info.history_pos;
    if (after < before || this.redoing > 0) return;
    const added = info.total_rows !== this.info.total_rows;
    if (after === before && !added) return;
    if (pend && pend.kind !== "reset" && after > before) {
      if (pend.kind === "insert") this.rowHeights.insertRows(pend.at, pend.count);
      else if (pend.kind === "delete") this.rowHeights.deleteRows(pend.at, pend.count);
      else this.rowHeights.moveRows(pend.at, pend.count, pend.to);
      return;
    }
    if (added || (pend && pend.kind === "reset" && after > before)) {
      this.rowHeights.clear();
      this.rowHeightsNote = "The custom row heights were cleared because rows changed and the grid cannot tell which rows kept theirs (Ctrl+Z brings them back).";
    }
  }

  /** Something visible changed: redraw. */
  bump(): void {
    this.version++;
    this.onRedraw?.();
  }
}

/** A tab's look and place, to show it in another window: column widths and
 *  formats, highlight rules and colour scales, data checks, the sort and
 *  filter as the page knows them, frozen panes, the facet cards, the header
 *  setting, the selection, the scroll position, the split. (The rows
 *  themselves, the filter's effect and the undo history live in the engine and
 *  are shared by both windows.) Plain data: it goes through JSON. */
export interface HandTab {
  snap: ViewSnap;
  filter: FilterSpec | null;
  sortAt: number | null;
  frozenRows: number;
  frozenCols: number;
  facetCards: Tab["facetCards"];
  valueTypes: boolean;
  header: boolean;
  headerTouched: boolean;
  filterButtons: boolean;
  showEdits: boolean;
  split: Tab["split"];
  sel: Selection;
  scrollX: number;
  scrollY: number;
  splitScroll: { scrollX: number; scrollY: number };
}

export function exportHandTab(t: Tab): HandTab {
  return structuredClone(
    $state.snapshot({
      snap: t.snapView(),
      filter: t.filter,
      sortAt: t.sortAt,
      frozenRows: t.frozenRows,
      frozenCols: t.frozenCols,
      facetCards: t.facetCards,
      valueTypes: t.valueTypes,
      header: t.header,
      headerTouched: t.headerTouched,
      filterButtons: t.filterButtons,
      showEdits: t.showEdits,
      split: t.split,
      sel: t.sel,
      scrollX: t.scrollX,
      scrollY: t.scrollY,
      splitScroll: t.splitScroll,
    }),
  ) as HandTab;
}

/** Put a tab's look and place (from `exportHandTab`) on a new tab. */
export function importHandTab(t: Tab, h: HandTab): void {
  // The look, the data checks and the sort; the filter's rows are already
  // the engine's.
  t.restoreView(h.snap, false);
  t.sort = h.snap.sort ? structuredClone(h.snap.sort) : null;
  t.sortAt = h.sortAt;
  t.filter = h.filter ? structuredClone(h.filter) : null;
  t.frozenRows = h.frozenRows;
  t.frozenCols = h.frozenCols;
  t.facetCards = structuredClone(h.facetCards);
  t.valueTypes = h.valueTypes;
  t.header = h.header;
  t.headerTouched = h.headerTouched;
  t.filterButtons = h.filterButtons;
  t.showEdits = h.showEdits;
  t.split = h.split ? structuredClone(h.split) : null;
  t.sel = structuredClone(h.sel);
  t.scrollX = h.scrollX;
  t.scrollY = h.scrollY;
  t.splitScroll = { ...h.splitScroll };
  t.bump();
}
