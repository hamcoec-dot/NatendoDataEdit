<script lang="ts">
  // Canvas grid. Draws only what is on screen (rows and columns are both
  // virtual), so a file with ten million rows draws as fast as one with ten.
  // The DOM holds just the canvas, two scrollbars and, while editing, one
  // text box over the active cell.
  import { onMount, tick, untrack } from "svelte";
  import type { Tab } from "./tab.svelte";
  import { prefs } from "./prefs.svelte";
  import { contains, type Cell, type Rect } from "./selection";
  import { blockTarget, dropIndex, gapAt, onOutline } from "./dragmove";
  import { backend, errorText, type Dir } from "./backend";
  import { PALETTE } from "./views";
  import { localDays } from "./m8";
  import { formatCell, linkOf, MAX_ROW_LINES, pickMatches, readNumber, tickOf, toggled, wrapLines } from "./m9";
  import { barSpan, completion, scaleColour, scalePos, valueType } from "./m10";
  /** Mark value for a data-check problem (drawn red). */
  const PROBLEM = 255;
  const PROBLEM_COLOR = "#e5484d";
  /** Corner mark on cells that differ from the file ("Show my edits"). */
  const EDIT_COLOR = "#f5a524";
  /** Row flag colours 1-6 (red, orange, yellow, green, blue, purple). */
  const FLAG_COLORS = ["", "#e5484d", "#f76b15", "#e2b400", "#30a46c", "#0090ff", "#8e4ec6"];

  interface Props {
    tab: Tab;
    /** Commit an edit of one cell (or, with `fill`, of every selected cell). */
    onCommit: (row: number, col: number, value: string, fill: boolean) => Promise<boolean>;
    onContext: (x: number, y: number, area: "cell" | "row" | "col" | "corner") => void;
    onMessage: (text: string) => void;
    /** The filter button on column `col`'s header was clicked (x, y: where
     *  to open its menu, in window coordinates). */
    onFilterButton?: (col: number, x: number, y: number) => void;
    /** The fill handle was dragged `count` rows or columns in `dir`; `swap`
     *  (Ctrl held) switches between copying and continuing a series. */
    onFill?: (dir: Dir, count: number, swap: boolean) => void;
    /** The fill handle was double-clicked: fill down as far as the data
     *  next to the selection goes. */
    onFillAuto?: () => void;
    /** Ctrl+click on a cell holding a web or e-mail address (checked). */
    onOpenLink?: (url: string) => void;
    /** Whole columns (rows) were dragged by their header: move `count` at
     *  `at` so they start at `to`. */
    onMoveCols?: (at: number, count: number, to: number) => void;
    onMoveRows?: (at: number, count: number, to: number) => void;
    /** The selected block was dragged by its outline to start at (row, col);
     *  `copy`: Ctrl was held, so copy it instead. */
    onMoveCells?: (from: Rect, row: number, col: number, copy: boolean) => void;
    readOnly: boolean;
  }
  let { tab, onCommit, onContext, onMessage, onFilterButton, onFill, onFillAuto, onOpenLink, onMoveCols, onMoveRows, onMoveCells, readOnly }: Props = $props();

  let host: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  /** Hidden text box that holds keyboard focus for the grid, so typed text
   *  (including IME input) and paste arrive as normal text events. */
  let sink: HTMLTextAreaElement;
  let editor = $state<HTMLTextAreaElement>();
  let width = $state(0);
  let height = $state(0);
  let editing = $state<{ row: number; col: number; value: string; x: number; y: number; w: number; h: number } | null>(null);
  let scrollTick = $state(0);

  const SCROLLBAR = 12;

  // ---- metrics -----------------------------------------------------------
  const fontPx = $derived(Math.round(prefs.data.fontSize * prefs.data.zoom));
  /** Height of the header row and of a one-line row. */
  const headH = $derived(Math.round(fontPx * 1.8) + 2);
  /** Extra height per extra line of text. */
  const textLineH = $derived(Math.round(fontPx * 1.35));
  const rowH = $derived(headH + (Math.max(1, Math.min(MAX_ROW_LINES, tab.rowLines)) - 1) * textLineH);
  const font = $derived(`${fontPx}px ${prefs.data.font}`);
  const headerFont = $derived(`600 ${fontPx}px ${prefs.data.font}`);
  let rowHeaderW = 56;

  function dataRows(): number {
    return Math.max(0, tab.info.rows - tab.firstRow);
  }
  function frozenRows(): number {
    return Math.min(tab.frozenRows, dataRows());
  }
  function frozenCols(): number {
    return Math.min(tab.frozenCols, tab.info.cols);
  }
  function bodyTop(): number {
    return headH + frozenRows() * rowH;
  }
  function bodyLeft(): number {
    return rowHeaderW + tab.columns.x(frozenCols());
  }
  function viewW(): number {
    return Math.max(0, width - SCROLLBAR);
  }
  function viewH(): number {
    return Math.max(0, height - SCROLLBAR);
  }
  function maxScrollY(): number {
    const content = (dataRows() - frozenRows()) * rowH;
    return Math.max(0, content - (viewH() - bodyTop()) + rowH);
  }
  function maxScrollX(): number {
    const content = tab.columns.total() - tab.columns.x(frozenCols());
    return Math.max(0, content - (viewW() - bodyLeft()) + 40);
  }
  function clampScroll(): void {
    tab.scrollY = Math.min(Math.max(0, tab.scrollY), maxScrollY());
    tab.scrollX = Math.min(Math.max(0, tab.scrollX), maxScrollX());
  }

  // ---- theme colours (read from CSS so themes and accents just work) ----
  let colors = { bg: "#fff", surface: "#fff", border: "#ddd", text: "#000", muted: "#666", accent: "#1f7a6d", header: "#f3f4f6" };
  function readColors(): void {
    const s = getComputedStyle(document.documentElement);
    const v = (n: string, d: string) => s.getPropertyValue(n).trim() || d;
    colors = {
      bg: v("--bg", "#f6f7f9"),
      surface: v("--surface", "#fff"),
      border: v("--grid-line", v("--border", "#ddd")),
      text: v("--text", "#000"),
      muted: v("--muted", "#666"),
      accent: v("--accent", "#1f7a6d"),
      header: v("--header", "#f0f2f5"),
    };
  }

  // ---- drawing -------------------------------------------------------------
  let frame = 0;
  export function redraw(): void {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      draw();
    });
  }

  $effect(() => {
    // Redraw on any change the grid depends on.
    void tab.version;
    void tab.sel;
    void tab.header;
    void tab.frozenRows;
    void tab.frozenCols;
    void tab.filterButtons;
    void tab.info;
    void tab.banded;
    void tab.valueTypes;
    void tab.divider;
    void tab.scales;
    void tab.rowLines;
    void listCols;
    void font;
    void prefs.data.gridLines;
    void width;
    void height;
    untrack(redraw);
  });

  $effect(() => {
    void prefs.data.theme;
    void prefs.data.accent;
    untrack(() => {
      requestAnimationFrame(() => {
        readColors();
        redraw();
      });
    });
  });

  const displayText = (s: string) =>
    s.length > 0 && /[\r\n\t]/.test(s) ? s.replace(/\r\n|\r|\n/g, " ↵ ").replace(/\t/g, " ") : s;

  function colTitle(c: number): string {
    if (tab.header && tab.info.rows > 0) {
      const t = tab.cache.get(0, c);
      if (t !== undefined && t !== "") return displayText(t);
    }
    return String(c + 1);
  }

  interface Span {
    start: number;
    end: number;
    /** Screen position of `start`. */
    pos: number;
  }

  function visibleCols(): { frozen: Span; body: Span } {
    const fc = frozenCols();
    const frozen = { start: 0, end: fc, pos: rowHeaderW };
    const base = tab.columns.x(fc);
    let start = fc;
    let end = fc;
    if (tab.info.cols > fc) {
      start = Math.max(fc, tab.columns.at(base + tab.scrollX));
      end = start;
      const right = base + tab.scrollX + (viewW() - bodyLeft());
      while (end < tab.info.cols && tab.columns.x(end) < right) end++;
    }
    return { frozen, body: { start, end, pos: bodyLeft() + tab.columns.x(start) - base - tab.scrollX } };
  }

  function visibleRows(): { frozen: Span; body: Span } {
    const off = tab.firstRow;
    const fr = frozenRows();
    const frozen = { start: off, end: off + fr, pos: headH };
    const first = off + fr + Math.floor(tab.scrollY / rowH);
    const count = Math.ceil((viewH() - bodyTop()) / rowH) + 1;
    const end = Math.min(tab.info.rows, first + count);
    return {
      frozen,
      body: { start: Math.min(first, end), end, pos: bodyTop() - (tab.scrollY % rowH) },
    };
  }

  function colX(c: number, cols: { frozen: Span; body: Span }): number {
    return c < cols.frozen.end
      ? rowHeaderW + tab.columns.x(c)
      : cols.body.pos + tab.columns.x(c) - tab.columns.x(cols.body.start);
  }

  function rowY(r: number, rows: { frozen: Span; body: Span }): number {
    return r < rows.frozen.end ? headH + (r - rows.frozen.start) * rowH : rows.body.pos + (r - rows.body.start) * rowH;
  }

  function draw(): void {
    if (!canvas || width <= 0 || height <= 0) return;
    const dpr = window.devicePixelRatio || 1;
    const w = viewW();
    const h = viewH();
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.font = headerFont;
    rowHeaderW = Math.max(48, Math.ceil(ctx.measureText("9".repeat(String(tab.info.total_rows ?? tab.info.rows).length)).width) + 24);
    clampScroll();

    const rows = visibleRows();
    const cols = visibleCols();
    // Fetch what is on screen plus a screen's worth ahead in each direction.
    const span = rows.body.end - rows.body.start;
    tab.cache.request(Math.max(0, rows.body.start - span), rows.body.end + span, cols.body.start, cols.body.end + 4);
    if (rows.frozen.end > rows.frozen.start || tab.header)
      tab.cache.request(0, Math.max(rows.frozen.end, 1), cols.body.start, cols.body.end + 4);
    if (cols.frozen.end > 0) tab.cache.request(rows.body.start, rows.body.end, 0, cols.frozen.end);

    ctx.fillStyle = colors.surface;
    ctx.fillRect(0, 0, w, h);
    ctx.textBaseline = "middle";

    const sel = tab.rect;
    const active = tab.sel.focus;
    const rowSpans = [rows.frozen, rows.body];
    const colSpans = [cols.frozen, cols.body];

    // Cells, region by region, each clipped to its own area.
    const top = [headH, bodyTop()];
    const left = [rowHeaderW, bodyLeft()];
    ctx.font = font;
    for (let ri = 0; ri < 2; ri++) {
      for (let ci = 0; ci < 2; ci++) {
        const rs = rowSpans[ri]!;
        const cs = colSpans[ci]!;
        if (rs.end <= rs.start || cs.end <= cs.start) continue;
        const clipX = left[ci]!;
        const clipY = top[ri]!;
        const clipW = ci === 0 ? bodyLeft() - rowHeaderW : w - bodyLeft();
        const clipH = ri === 0 ? bodyTop() - headH : h - bodyTop();
        ctx.save();
        ctx.beginPath();
        ctx.rect(clipX, clipY, clipW, clipH);
        ctx.clip();
        drawCells(ctx, rs, cs, sel, active);
        ctx.restore();
      }
    }

    // Column headers.
    ctx.font = headerFont;
    ctx.fillStyle = colors.header;
    ctx.fillRect(0, 0, w, headH);
    for (const [ci, cs] of colSpans.entries()) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(left[ci]!, 0, ci === 0 ? bodyLeft() - rowHeaderW : w - bodyLeft(), headH);
      ctx.clip();
      for (let c = cs.start; c < cs.end; c++) {
        const cw = tab.columns.width(c);
        if (cw === 0) continue;
        const x = colX(c, cols);
        const inSel = c >= sel.col && c < sel.col + sel.cols;
        if (inSel) {
          ctx.fillStyle = colors.accent;
          ctx.globalAlpha = 0.18;
          ctx.fillRect(x, 0, cw, headH);
          ctx.globalAlpha = 1;
        }
        ctx.fillStyle = inSel ? colors.text : colors.muted;
        const btn = tab.filterButtons || filtered.has(c) ? FILTER_BTN : 0;
        const mark = c > 0 && tab.columns.isHidden(c - 1);
        ctx.save();
        ctx.beginPath();
        ctx.rect(x + 4, 0, cw - 8 - btn, headH);
        ctx.clip();
        ctx.fillText(colTitle(c), x + 6 + (mark ? MARK_W : 0), headH / 2);
        ctx.restore();
        if (btn) drawFilterButton(ctx, x + cw - btn - 2, filtered.has(c));
        if (mark) drawHiddenMark(ctx, x);
        ctx.fillStyle = colors.border;
        ctx.fillRect(x + cw - 1, 0, 1, headH);
      }
      ctx.restore();
    }

    // Row headers.
    ctx.fillStyle = colors.header;
    ctx.fillRect(0, headH, rowHeaderW, h - headH);
    for (const [ri, rs] of rowSpans.entries()) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(0, top[ri]!, rowHeaderW, ri === 0 ? bodyTop() - headH : h - bodyTop());
      ctx.clip();
      for (let r = rs.start; r < rs.end; r++) {
        const y = rowY(r, rows);
        const inSel = r >= sel.row && r < sel.row + sel.rows;
        if (inSel) {
          ctx.fillStyle = colors.accent;
          ctx.globalAlpha = 0.18;
          ctx.fillRect(0, y, rowHeaderW, rowH);
          ctx.globalAlpha = 1;
        }
        // Filtered: row numbers in the accent colour, as a reminder that
        // rows are hidden in between.
        ctx.fillStyle = tab.info.filtered ? colors.accent : inSel ? colors.text : colors.muted;
        const label = rowLabel(r);
        ctx.fillText(label, rowHeaderW - 10 - ctx.measureText(label).width, y + rowH / 2);
        const note = tab.rowNotes.get(r);
        if (note) {
          if (note.flag) {
            ctx.fillStyle = FLAG_COLORS[note.flag] ?? colors.accent;
            ctx.fillRect(0, y + 1, 5, rowH - 2);
          }
          if (note.text) {
            ctx.fillStyle = colors.accent;
            ctx.beginPath();
            ctx.arc(11, y + rowH / 2, 2.5, 0, Math.PI * 2);
            ctx.fill();
          }
        }
        ctx.fillStyle = colors.border;
        ctx.fillRect(0, y + rowH - 1, rowHeaderW, 1);
      }
      ctx.restore();
    }
    // Corner and dividers.
    ctx.fillStyle = colors.header;
    ctx.fillRect(0, 0, rowHeaderW, headH);
    ctx.fillStyle = colors.border;
    ctx.fillRect(rowHeaderW - 1, 0, 1, h);
    ctx.fillRect(0, headH - 1, w, 1);
    if (frozenRows() > 0) {
      ctx.fillStyle = colors.accent;
      ctx.fillRect(0, bodyTop() - 2, w, 2);
    }
    if (frozenCols() > 0) {
      ctx.fillStyle = colors.accent;
      ctx.fillRect(bodyLeft() - 2, 0, 2, h);
    }
    if (tab.header && tab.info.rows > 0) {
      // The header row's own number, so it is clear row 1 is the titles.
      ctx.font = headerFont;
      ctx.fillStyle = colors.muted;
      ctx.fillText("1", rowHeaderW - 10 - ctx.measureText("1").width, headH / 2);
    }
    drawFillHandle(ctx, w, h);
    drawMove(ctx, w, h);
    drawFreezeDrag(ctx, w, h);
    scrollTick++;
    positionEditor();
    wantMarks(rows, cols);
    wantNotes(rows);
  }

  // ---- highlight rules -------------------------------------------------------
  /** What the marks on screen were asked for (rules, data, area). */
  let marksKey = "";
  // ---- notes and flags on rows ----------------------------------------------
  let notesKey = "";
  function wantNotes(rows: { frozen: Span; body: Span }): void {
    if (!tab.info.path) {
      if (tab.rowNotes.size) tab.rowNotes = new Map();
      return;
    }
    const areas = [rows.frozen, rows.body].filter((a) => a.end > a.start);
    const key = `${tab.version}|${tab.notesVersion}|${areas.map((a) => `${a.start}-${a.end}`).join(",")}`;
    if (key === notesKey) return;
    notesKey = key;
    const t = tab;
    void Promise.all(areas.map((a) => backend().noteMarks(t.id, a.start, a.end - a.start)))
      .then((all) => {
        if (notesKey !== key) return;
        const m = new Map<number, { flag: number; text: string }>();
        for (const list of all) for (const n of list) m.set(n.row, { flag: n.flag, text: n.text });
        const changed = m.size !== t.rowNotes.size || [...m].some(([k, v]) => t.rowNotes.get(k)?.flag !== v.flag || t.rowNotes.get(k)?.text !== v.text);
        t.rowNotes = m;
        if (changed) draw();
      })
      .catch(() => {
        // A file with no notes yet, or key columns gone: nothing to show.
      });
  }

  /** Ask the engine which cells on screen the highlight rules colour. */
  function wantMarks(rows: { frozen: Span; body: Span }, cols: { frozen: Span; body: Span }): void {
    const rules = tab.highlights;
    const checks = tab.checksMarked ? tab.checks : [];
    const edits = tab.showEdits;
    if (!rules.length && !checks.length && !edits) {
      if (tab.marks.size) tab.marks = new Map();
      tab.edits.clear();
      marksKey = "";
      return;
    }
    const c0 = Math.min(cols.frozen.start, cols.body.start);
    const c1 = Math.max(cols.frozen.end, cols.body.end);
    const areas = [rows.frozen, rows.body].filter((a) => a.end > a.start);
    const key = `${tab.version}|${tab.highlightVersion}|${tab.checkVersion}|${checks.length}|${edits}|${tab.header}|${areas.map((a) => `${a.start}-${a.end}`).join(",")}|${c0}-${c1}`;
    if (key === marksKey) return;
    marksKey = key;
    const t = tab;
    const snapshot = $state.snapshot(rules);
    const read = { decimal_comma: false, date_order: "mdy" as const };
    const spec = { checks: $state.snapshot(checks), keep_first: t.header, read, now: localDays() };
    void Promise.all([
      ...areas.map((a) => (snapshot.length ? backend().highlights(t.id, a.start, a.end - a.start, c0, c1 - c0, snapshot, t.header, read) : Promise.resolve([]))),
      ...areas.map((a) => (spec.checks.length ? backend().checkMarks(t.id, a.start, a.end - a.start, c0, c1 - c0, spec) : Promise.resolve([]))),
      ...areas.map((a) => (edits ? backend().editedCells(t.id, a.start, a.end - a.start, c0, c1 - c0) : Promise.resolve([]))),
    ])
      .then((all) => {
        if (marksKey !== key) return;
        const m = new Map<string, number>();
        const lists = all.slice(0, areas.length * 2) as ({ row: number; col?: number | null } & ({ color: number } | object))[][];
        t.edits.clear();
        for (const list of all.slice(areas.length * 2) as [number, number][][]) for (const [row, col] of list) t.edits.add(`${row}:${col}`);
        lists.forEach((list, i) => {
          const problem = i >= areas.length;
          for (const k of list) {
            const id = `${k.row}:${k.col ?? "*"}`;
            if (problem) m.set(id, PROBLEM);
            else if (!m.has(id) && "color" in k) m.set(id, k.color);
          }
        });
        t.marks = m;
        draw();
      })
      .catch((e: unknown) => onMessage(errorText(e)));
  }

  // ---- fill handle -----------------------------------------------------------
  /** Where the fill handle is on screen (null when off screen). */
  let handle: { x: number; y: number } | null = null;
  /** While dragging the handle: how far, and which way. */
  let fillTo = $state<{ dir: Dir; count: number } | null>(null);
  const HANDLE = 7;

  /** Screen box of cells r0..r1 × c0..c1, clamped to the view. */
  function screenBox(r0: number, c0: number, r1: number, c1: number, w: number, h: number) {
    const rows = visibleRows();
    const cols = visibleCols();
    const ry = (r: number) => (r < rows.body.start && r >= rows.frozen.end ? bodyTop() : r >= rows.body.end ? h + rowH : rowY(r, rows));
    const cx = (c: number) => (c < cols.body.start && c >= cols.frozen.end ? bodyLeft() : c >= cols.body.end ? w + 40 : colX(c, cols));
    const x0 = cx(c0);
    const y0 = ry(r0);
    const x1 = c1 >= cols.body.end ? w + 40 : cx(c1) + tab.columns.width(c1);
    const y1 = r1 >= rows.body.end ? h + rowH : ry(r1) + rowH;
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }

  function drawFillHandle(ctx: CanvasRenderingContext2D, w: number, h: number): void {
    handle = null;
    if (readOnly || editing || tab.info.rows === 0) return;
    const sel = tab.rect;
    const corner = cellRect({ row: sel.row + sel.rows - 1, col: sel.col + sel.cols - 1 });
    ctx.save();
    ctx.beginPath();
    ctx.rect(rowHeaderW, headH, w - rowHeaderW, h - headH);
    ctx.clip();
    if (fillTo && fillTo.count > 0) {
      const f = fillTo;
      const last = (n: number) => n - 1;
      const [r0, c0, r1, c1] =
        f.dir === "down"
          ? [sel.row + sel.rows, sel.col, sel.row + sel.rows + last(f.count), sel.col + last(sel.cols)]
          : f.dir === "up"
            ? [sel.row - f.count, sel.col, sel.row - 1, sel.col + last(sel.cols)]
            : f.dir === "right"
              ? [sel.row, sel.col + sel.cols, sel.row + last(sel.rows), sel.col + sel.cols + last(f.count)]
              : [sel.row, sel.col - f.count, sel.row + last(sel.rows), sel.col - 1];
      const b = screenBox(r0, c0, r1, c1, w, h);
      ctx.strokeStyle = colors.accent;
      ctx.setLineDash([4, 3]);
      ctx.lineWidth = 1.5;
      ctx.strokeRect(b.x + 1, b.y + 1, b.w - 2, b.h - 2);
      ctx.setLineDash([]);
    }
    if (corner) {
      const x = corner.x + corner.w - HANDLE / 2 - 1;
      const y = corner.y + corner.h - HANDLE / 2 - 1;
      ctx.fillStyle = colors.surface;
      ctx.fillRect(x - 1, y - 1, HANDLE + 2, HANDLE + 2);
      ctx.fillStyle = colors.accent;
      ctx.fillRect(x, y, HANDLE, HANDLE);
      handle = { x, y };
    }
    ctx.restore();
  }

  function onHandle(px: number, py: number): boolean {
    return !!handle && px >= handle.x - 3 && px <= handle.x + HANDLE + 3 && py >= handle.y - 3 && py <= handle.y + HANDLE + 3;
  }

  // ---- dragging columns, rows and blocks of cells to move them -------------
  /** A press on a selected header (or on the selection's outline) that may
   *  become a move; it starts once the pointer has gone a few pixels. */
  type Moving = {
    kind: "cols" | "rows" | "cells";
    from: Rect;
    startX: number;
    startY: number;
    active: boolean;
    /** Where the press was: the cell grabbed, or the header clicked. */
    grab: Cell;
    /** cols/rows: the index the block would start at; cells: top-left. */
    to: Cell | null;
    /** Pointer now, and its offset into the block when pressed (for the
     *  see-through copy that follows the pointer). */
    px: number;
    py: number;
    offset: number;
  };
  let moving: Moving | null = null;
  const MOVE_SLOP = 4;

  function wholeColsSel(r: Rect): boolean {
    return tab.info.rows > 0 && r.row <= tab.firstRow && r.row + r.rows >= tab.info.rows;
  }
  function wholeRowsSel(r: Rect): boolean {
    return tab.info.cols > 0 && r.col === 0 && r.cols >= tab.info.cols;
  }

  /** The selection's outline on screen, where a block can be picked up. */
  function onSelOutline(px: number, py: number): boolean {
    const r = tab.rect;
    if (tab.info.rows === 0 || wholeColsSel(r) || wholeRowsSel(r)) return false;
    const b = screenBox(r.row, r.col, r.row + r.rows - 1, r.col + r.cols - 1, viewW(), viewH());
    return px >= rowHeaderW && py >= headH && onOutline(px, py, b);
  }

  /** Where the drag would put things with the pointer at (px, py). */
  function moveTarget(m: Moving, px: number, py: number): Cell | null {
    const x = Math.min(Math.max(px, rowHeaderW), viewW() - 1);
    const y = Math.min(Math.max(py, headH), viewH() - 1);
    if (m.kind === "cols") {
      const h = hit(x, headH - 1);
      if (h.area !== "col") return null;
      const left = colX(h.col, visibleCols());
      const to = dropIndex(m.from.col, m.from.cols, gapAt(h.col, x - left, tab.columns.width(h.col)), tab.info.cols);
      return to === null ? null : { row: 0, col: to };
    }
    if (m.kind === "rows") {
      const h = hit(1, y);
      if (h.area !== "row") return null;
      const top = rowY(h.row, visibleRows());
      const to = dropIndex(m.from.row, m.from.rows, gapAt(h.row, y - top, rowH), tab.info.rows, tab.firstRow);
      return to === null ? null : { row: to, col: 0 };
    }
    const h = hit(x, y);
    if (h.area !== "cell") return m.to;
    const t = blockTarget(m.from, m.grab, h, tab.info.rows, tab.info.cols, tab.firstRow);
    return t.row === m.from.row && t.col === m.from.col ? null : t;
  }

  function drawMove(ctx: CanvasRenderingContext2D, w: number, h: number): void {
    const m = moving;
    if (!m?.active) return;
    // A see-through band the size of what is dragged follows the pointer.
    if (m.kind === "cols") {
      const bw = Math.min(w, tab.columns.x(m.from.col + m.from.cols) - tab.columns.x(m.from.col));
      ctx.save();
      ctx.fillStyle = colors.accent;
      ctx.globalAlpha = 0.15;
      ctx.fillRect(m.px - m.offset, 0, bw, h);
      ctx.restore();
    } else if (m.kind === "rows") {
      const bh = Math.min(h, m.from.rows * rowH);
      ctx.save();
      ctx.fillStyle = colors.accent;
      ctx.globalAlpha = 0.15;
      ctx.fillRect(0, m.py - m.offset, w, bh);
      ctx.restore();
    }
    if (!m.to) return;
    ctx.save();
    ctx.fillStyle = colors.accent;
    ctx.strokeStyle = colors.accent;
    if (m.kind === "cols") {
      // A bar at the gap the columns go into.
      const gap = m.to.col < m.from.col ? m.to.col : m.to.col + m.from.cols;
      const cols = visibleCols();
      const x = gap >= tab.info.cols ? colX(gap - 1, cols) + tab.columns.width(gap - 1) : colX(gap, cols);
      if (x >= rowHeaderW - 2 && x <= w + 2) ctx.fillRect(x - 2, 0, 4, h);
    } else if (m.kind === "rows") {
      const gap = m.to.row < m.from.row ? m.to.row : m.to.row + m.from.rows;
      const rows = visibleRows();
      const y = gap >= tab.info.rows ? rowY(gap - 1, rows) + rowH : rowY(gap, rows);
      if (y >= headH - 2 && y <= h + 2) ctx.fillRect(0, y - 2, w, 4);
    } else {
      ctx.beginPath();
      ctx.rect(rowHeaderW, headH, w - rowHeaderW, h - headH);
      ctx.clip();
      const b = screenBox(m.to.row, m.to.col, m.to.row + m.from.rows - 1, m.to.col + m.from.cols - 1, w, h);
      ctx.globalAlpha = 0.12;
      ctx.fillRect(b.x, b.y, b.w, b.h);
      ctx.globalAlpha = 1;
      ctx.setLineDash([4, 3]);
      ctx.lineWidth = 2;
      ctx.strokeRect(b.x + 1, b.y + 1, b.w - 2, b.h - 2);
    }
    ctx.restore();
  }

  function cancelMove(): void {
    if (!moving && !freezing) return;
    moving = null;
    freezing = null;
    cursor = "default";
    redraw();
  }

  // ---- hidden columns: a mark on the header beside them; a click shows them
  const MARK_W = 10;

  function drawHiddenMark(ctx: CanvasRenderingContext2D, edge: number): void {
    // A small arrow pointing left, at the hidden columns.
    const x = edge + 5;
    const y = headH / 2;
    const s = Math.max(3, Math.round(fontPx * 0.3));
    ctx.save();
    ctx.fillStyle = colors.accent;
    ctx.beginPath();
    ctx.moveTo(x + s, y - s);
    ctx.lineTo(x, y);
    ctx.lineTo(x + s, y + s);
    ctx.closePath();
    ctx.fill();
    ctx.restore();
  }

  /** The hidden columns just left of the header mark at (px, py), or null. */
  function hiddenAt(px: number, py: number, col: number): { from: number; to: number } | null {
    if (py >= headH || col <= 0 || !tab.columns.isHidden(col - 1)) return null;
    const left = colX(col, visibleCols());
    if (px <= left + 4 || px > left + 4 + MARK_W) return null;
    let from = col - 1;
    while (from > 0 && tab.columns.isHidden(from - 1)) from--;
    return { from, to: col };
  }

  // ---- dragging the corner's edges to freeze rows or columns ---------------
  /** Frozen rows (or columns) the drag would leave, by pointer position. */
  let freezing: { kind: "rows" | "cols"; n: number } | null = null;

  function onFreezeEdge(px: number, py: number): "rows" | "cols" | null {
    if (px < rowHeaderW && py < headH) {
      if (py >= headH - 4) return "rows";
      if (px >= rowHeaderW - 4) return "cols";
      return null;
    }
    // With rows frozen, their line in the row numbers can be dragged too.
    if (frozenRows() > 0 && px < rowHeaderW && Math.abs(py - bodyTop()) <= 3) return "rows";
    return null;
  }

  function freezeCount(kind: "rows" | "cols", px: number, py: number): number {
    if (kind === "rows") {
      const most = Math.max(0, Math.min(dataRows(), Math.floor((viewH() - headH) / rowH) - 1));
      return Math.min(most, Math.max(0, Math.round((py - headH) / rowH)));
    }
    const x = px - rowHeaderW;
    if (x <= 0 || tab.info.cols === 0) return 0;
    const c = tab.columns.at(x);
    const n = x - tab.columns.x(c) < tab.columns.width(c) / 2 ? c : c + 1;
    // Keep some room on the right to scroll the rest.
    let most = n;
    while (most > 0 && rowHeaderW + tab.columns.x(most) > viewW() - 40) most--;
    return Math.min(n, most, tab.info.cols);
  }

  function drawFreezeDrag(ctx: CanvasRenderingContext2D, w: number, h: number): void {
    const f = freezing;
    if (!f) return;
    ctx.save();
    ctx.fillStyle = colors.accent;
    ctx.globalAlpha = 0.7;
    if (f.kind === "rows") ctx.fillRect(0, headH + f.n * rowH - 2, w, 4);
    else ctx.fillRect(rowHeaderW + tab.columns.x(f.n) - 2, 0, 4, h);
    ctx.restore();
  }

  /** The fill a pointer over cell (row, col) asks for: along the axis it
   *  moved furthest from the selection, as Excel does. */
  function fillTarget(row: number, col: number): { dir: Dir; count: number } | null {
    const s = tab.rect;
    const down = row - (s.row + s.rows - 1);
    const up = s.row - row;
    const right = col - (s.col + s.cols - 1);
    const left = s.col - col;
    const v = Math.max(down, up, 0);
    const hz = Math.max(right, left, 0);
    if (v === 0 && hz === 0) return null;
    if (v >= hz) return down > 0 ? { dir: "down", count: down } : { dir: "up", count: Math.min(up, s.row - tab.firstRow) };
    return right > 0 ? { dir: "right", count: right } : { dir: "left", count: Math.min(left, s.col) };
  }

  /** Columns with a "list" data check: their cells offer a pick list. */
  const listCols = $derived(new Map(tab.checks.flatMap((k) => (k.kind.type === "list" && k.col !== null ? [[k.col, k.kind.values] as const] : []))));

  /** Width of the pick-list arrow at the right of the active cell. */
  const PICK_BTN = 16;

  function drawTick(ctx: CanvasRenderingContext2D, x: number, y: number, cw: number, on: boolean): void {
    const size = Math.min(14, rowH - 6, cw - 6);
    if (size < 6) return;
    const bx = Math.round(x + (cw - size) / 2) + 0.5;
    const by = Math.round(y + (headH - size) / 2) + 0.5;
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = on ? colors.accent : colors.muted;
    if (on) {
      ctx.fillStyle = colors.accent;
      ctx.fillRect(bx, by, size, size);
      ctx.strokeStyle = "#fff";
      ctx.beginPath();
      ctx.moveTo(bx + size * 0.22, by + size * 0.52);
      ctx.lineTo(bx + size * 0.42, by + size * 0.72);
      ctx.lineTo(bx + size * 0.78, by + size * 0.3);
      ctx.stroke();
    } else {
      ctx.strokeRect(bx, by, size, size);
    }
    ctx.lineWidth = 1;
  }

  function drawCells(
    ctx: CanvasRenderingContext2D,
    rs: Span,
    cs: Span,
    sel: { row: number; col: number; rows: number; cols: number },
    active: Cell,
  ): void {
    const pad = Math.round(fontPx * 0.45);
    const measure = (s: string) => ctx.measureText(s).width;
    const lines = Math.max(1, Math.min(MAX_ROW_LINES, tab.rowLines));
    // Rows on screen with a group line under them (the divider column's
    // value changes on the next row).
    const under = new Set<number>();
    const dc = tab.divider;
    if (dc !== null) {
      for (let r = Math.max(rs.start, tab.firstRow); r < rs.end; r++) {
        const a = tab.cache.get(r, dc);
        const b = tab.cache.get(r + 1, dc);
        if (a !== undefined && b !== undefined && r + 1 < tab.info.rows && a !== b) under.add(r);
      }
    }
    const typeColour = { number: "#2f6fd0", date: "#2e8b57" } as const;
    let x = cs.pos;
    for (let c = cs.start; c < cs.end; c++) {
      const cw = tab.columns.width(c);
      if (cw === 0) continue;
      const fmt = tab.columns.formats.get(c);
      const scale = tab.scales.length ? tab.scales.find((s) => s.col === c) : undefined;
      const range = scale ? tab.ranges.get(c) : undefined;
      const wrap = lines > 1 && tab.columns.wraps.has(c);
      let y = rs.pos;
      ctx.save();
      ctx.beginPath();
      ctx.rect(x, rs.pos, cw, (rs.end - rs.start) * rowH);
      ctx.clip();
      for (let r = rs.start; r < rs.end; r++, y += rowH) {
        if (tab.banded && (r - tab.firstRow) % 2 === 1) {
          ctx.fillStyle = colors.accent;
          ctx.globalAlpha = 0.08;
          ctx.fillRect(x, y, cw, rowH);
          ctx.globalAlpha = 1;
        }
        if (contains(sel, r, c) && !(r === active.row && c === active.col)) {
          ctx.fillStyle = colors.accent;
          ctx.globalAlpha = 0.12;
          ctx.fillRect(x, y, cw, rowH);
          ctx.globalAlpha = 1;
        }
        const mark = tab.marks.size ? (tab.marks.get(`${r}:${c}`) ?? tab.marks.get(`${r}:*`)) : undefined;
        if (mark !== undefined) {
          ctx.fillStyle = mark === PROBLEM ? PROBLEM_COLOR : (PALETTE[mark]?.color ?? colors.accent);
          ctx.globalAlpha = 0.28;
          ctx.fillRect(x, y, cw, rowH);
          ctx.globalAlpha = 1;
        }
        if (tab.showEdits && tab.edits.has(`${r}:${c}`)) {
          // My edits: a small triangle in the top right corner.
          const k = Math.min(8, rowH / 2);
          ctx.fillStyle = EDIT_COLOR;
          ctx.beginPath();
          ctx.moveTo(x + cw - k, y);
          ctx.lineTo(x + cw, y);
          ctx.lineTo(x + cw, y + k);
          ctx.closePath();
          ctx.fill();
        }
        const t = tab.cache.get(r, c);
        let filled = false;
        if (scale && range && t !== undefined && !(tab.header && r === 0)) {
          const v = readNumber(t);
          if (v !== null) {
            if (scale.kind === "bar") {
              const b = barSpan(v, range[0], range[1]);
              ctx.fillStyle = v < 0 ? PROBLEM_COLOR : colors.accent;
              ctx.globalAlpha = 0.35;
              ctx.fillRect(x + 2 + b.from * (cw - 4), y + 3, Math.max(1, (b.to - b.from) * (cw - 4)), rowH - 6);
              ctx.globalAlpha = 1;
            } else {
              ctx.fillStyle = scaleColour(scale.kind, scalePos(v, range[0], range[1]));
              ctx.fillRect(x, y, cw, rowH);
              filled = true;
            }
          }
        }
        const tick = t !== undefined && fmt?.kind === "checkbox" && !(tab.header && r === 0) ? tickOf(t) : null;
        if (t === undefined) {
          ctx.fillStyle = colors.border;
          ctx.fillRect(x + pad, y + rowH / 2 - 1, Math.min(24, cw - 2 * pad), 2);
        } else if (tick !== null) {
          drawTick(ctx, x, y, cw, tick);
        } else if (t !== "") {
          const header = tab.header && r === 0;
          const shown = fmt && !header ? formatCell(t, fmt) : t;
          const link = !fmt && !header && /[.@:]/.test(t) ? linkOf(t) : null;
          const kind = tab.valueTypes && !header && !link ? valueType(t) : "text";
          // Text on a colour-scale fill is always dark, so it reads on every shade.
          ctx.fillStyle = filled ? "#111" : link ? colors.accent : kind === "number" || kind === "date" ? typeColour[kind] : colors.text;
          const cut = tab.cache.isTruncated(r, c);
          if (wrap) {
            const out = wrapLines(shown, Math.max(8, cw - 2 * pad), measure, lines);
            if (cut && out.length) out[out.length - 1] += "…";
            // The block of lines sits in the middle of the row, as one-line text does.
            const top = y + rowH / 2 + 0.5 - ((out.length - 1) * textLineH) / 2;
            out.forEach((line, i) => ctx.fillText(line, x + pad, top + i * textLineH));
          } else {
            let text = displayText(shown);
            if (cut) text += "…";
            const right = fmt && shown !== t && fmt.kind !== "date";
            const tx = right ? Math.max(x + pad, x + cw - pad - measure(text)) : x + pad;
            const ty = y + rowH / 2 + 0.5;
            ctx.fillText(text, tx, ty);
            if (link) ctx.fillRect(tx, ty + Math.round(fontPx * 0.55), Math.min(measure(text), cw - 2 * pad), 1);
          }
        }
        if (prefs.data.gridLines) {
          ctx.fillStyle = colors.border;
          ctx.fillRect(x, y + rowH - 1, cw, 1);
        }
        if (under.has(r)) {
          ctx.fillStyle = colors.accent;
          ctx.fillRect(x, y + rowH - 2, cw, 2);
        }
      }
      ctx.restore();
      if (prefs.data.gridLines) {
        ctx.fillStyle = colors.border;
        ctx.fillRect(x + cw - 1, rs.pos, 1, (rs.end - rs.start) * rowH);
      }
      x += cw;
    }
    // Active cell outline.
    if (active.row >= rs.start && active.row < rs.end && active.col >= cs.start && active.col < cs.end) {
      const ax = cs.pos + tab.columns.x(active.col) - tab.columns.x(cs.start);
      const ay = rs.pos + (active.row - rs.start) * rowH;
      const aw = tab.columns.width(active.col);
      ctx.strokeStyle = colors.accent;
      ctx.lineWidth = 2;
      ctx.strokeRect(ax + 1, ay + 1, aw - 2, rowH - 2);
      ctx.lineWidth = 1;
      // A pick-list arrow on cells of a column with a list of allowed values.
      if (listCols.has(active.col) && !readOnly && !(tab.header && active.row === 0) && aw > PICK_BTN + 8) {
        const bx = ax + aw - PICK_BTN - 2;
        ctx.fillStyle = colors.surface;
        ctx.fillRect(bx, ay + 3, PICK_BTN, Math.min(headH, rowH) - 6);
        ctx.fillStyle = colors.accent;
        ctx.beginPath();
        const cx = bx + PICK_BTN / 2;
        const cy = ay + headH / 2;
        ctx.moveTo(cx - 4, cy - 2);
        ctx.lineTo(cx + 4, cy - 2);
        ctx.lineTo(cx, cy + 3);
        ctx.closePath();
        ctx.fill();
      }
    }
  }

  /** Width of the filter button drawn at the right of a column header. */
  const FILTER_BTN = 16;
  const filtered = $derived(new Set(tab.info.filter_cols ?? []));

  /** A small arrow button; a funnel when the column has a filter rule. */
  function drawFilterButton(ctx: CanvasRenderingContext2D, x: number, on: boolean): void {
    const h = Math.min(FILTER_BTN, headH - 6);
    const y = (headH - h) / 2;
    ctx.fillStyle = on ? colors.accent : colors.surface;
    ctx.globalAlpha = on ? 0.9 : 1;
    ctx.fillRect(x, y, FILTER_BTN, h);
    ctx.globalAlpha = 1;
    ctx.strokeStyle = colors.border;
    ctx.strokeRect(x + 0.5, y + 0.5, FILTER_BTN - 1, h - 1);
    ctx.fillStyle = on ? "#fff" : colors.muted;
    ctx.beginPath();
    const cx = x + FILTER_BTN / 2;
    const cy = headH / 2;
    if (on) {
      // Funnel.
      ctx.moveTo(cx - 5, cy - 4);
      ctx.lineTo(cx + 5, cy - 4);
      ctx.lineTo(cx + 1, cy);
      ctx.lineTo(cx + 1, cy + 4);
      ctx.lineTo(cx - 1, cy + 5);
      ctx.lineTo(cx - 1, cy);
    } else {
      ctx.moveTo(cx - 4, cy - 2);
      ctx.lineTo(cx + 4, cy - 2);
      ctx.lineTo(cx, cy + 3);
    }
    ctx.closePath();
    ctx.fill();
  }

  /** The number shown for row `r`: its place in the whole file. */
  function rowLabel(r: number): string {
    if (!tab.info.filtered) return String(r + 1);
    const n = tab.cache.rowNumber(r);
    return n === undefined ? "" : String(n + 1);
  }

  // ---- hit testing ---------------------------------------------------------
  type Hit =
    | { area: "cell"; row: number; col: number }
    | { area: "col"; col: number; edge: number | null }
    | { area: "row"; row: number }
    | { area: "corner" }
    | { area: "none" };

  function hit(px: number, py: number): Hit {
    const rows = visibleRows();
    const cols = visibleCols();
    let col = -1;
    if (px >= rowHeaderW) {
      const inFrozen = px < bodyLeft();
      const base = inFrozen ? 0 : tab.columns.x(frozenCols()) + tab.scrollX;
      const rel = inFrozen ? px - rowHeaderW : px - bodyLeft();
      col = tab.columns.at(base + rel);
      if (!inFrozen && col < frozenCols()) col = frozenCols();
    }
    let row = -1;
    if (py >= headH) {
      row = py < bodyTop() ? rows.frozen.start + Math.floor((py - headH) / rowH) : rows.body.start + Math.floor((py - rows.body.pos) / rowH);
      if (row >= tab.info.rows) row = tab.info.rows - 1;
    }
    if (py < headH && px < rowHeaderW) return { area: "corner" };
    if (py < headH && col >= 0) {
      // Near a column's right edge: resize handle.
      let edge: number | null = null;
      const right = colX(col, cols) + tab.columns.width(col);
      const prevRight = colX(col, cols);
      if (Math.abs(px - right) <= 4) edge = col;
      else if (Math.abs(px - prevRight) <= 4 && col > 0) edge = tab.columns.step(col, -1);
      return { area: "col", col, edge };
    }
    if (px < rowHeaderW && row >= 0) return { area: "row", row };
    if (row >= 0 && col >= 0) return { area: "cell", row, col };
    return { area: "none" };
  }

  // ---- selection and navigation ----------------------------------------------
  function lastRow(): number {
    return Math.max(tab.firstRow, tab.info.rows - 1);
  }

  /** Select from `anchor` to `focus`; `scroll` brings the focus into view
   *  (off for whole rows/columns, which keep the view where it is). */
  export function select(anchor: Cell, focus: Cell = anchor, scroll = true): void {
    const clamp = (c: Cell): Cell => ({
      row: Math.min(Math.max(tab.firstRow, c.row), lastRow()),
      col: Math.min(Math.max(0, c.col), Math.max(0, tab.info.cols - 1)),
    });
    tab.sel = { anchor: clamp(anchor), focus: clamp(focus) };
    if (scroll) scrollIntoView(tab.sel.focus);
    else redraw();
  }

  export function move(dr: number, dc: number, extend: boolean): void {
    const f = tab.sel.focus;
    const col = dc === 0 ? f.col : tab.columns.step(f.col, dc > 0 ? 1 : -1, Math.abs(dc));
    const next = { row: f.row + dr, col };
    if (extend) select(tab.sel.anchor, next);
    else select(next);
  }

  export function pageRows(): number {
    return Math.max(1, Math.floor((viewH() - bodyTop()) / rowH) - 1);
  }

  export function scrollIntoView(c: Cell): void {
    const fr = frozenRows();
    const off = tab.firstRow;
    if (c.row >= off + fr) {
      const y = (c.row - off - fr) * rowH;
      const vis = viewH() - bodyTop();
      if (y < tab.scrollY) tab.scrollY = y;
      else if (y + rowH > tab.scrollY + vis) tab.scrollY = y + rowH - vis;
    }
    if (c.col >= frozenCols()) {
      const x = tab.columns.x(c.col) - tab.columns.x(frozenCols());
      const vis = viewW() - bodyLeft();
      const cw = tab.columns.width(c.col);
      if (x < tab.scrollX) tab.scrollX = x;
      else if (x + cw > tab.scrollX + vis) tab.scrollX = Math.min(x, x + cw - vis);
    }
    clampScroll();
    redraw();
  }

  /** Fit a column to the widest value currently loaded on screen. */
  export function autoFit(col: number): void {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.font = headerFont;
    let best = ctx.measureText(colTitle(col)).width;
    ctx.font = font;
    const rows = visibleRows();
    for (let r = rows.body.start; r < rows.body.end; r++) {
      const t = tab.cache.get(r, col);
      if (t) best = Math.max(best, ctx.measureText(displayText(t).slice(0, 200)).width);
    }
    tab.columns.setWidth(col, Math.min(600, Math.max(40, Math.ceil(best) + 2 * Math.round(fontPx * 0.45) + 4)));
    tab.bump();
  }

  /** Make rows tall enough for the wrapped columns' text on screen (1 to
   *  MAX_ROW_LINES lines; every row has the same height). */
  export function fitRowLines(): void {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.font = font;
    const pad = Math.round(fontPx * 0.45);
    const rows = visibleRows();
    let most = 1;
    for (const c of tab.columns.wraps) {
      const cw = tab.columns.width(c);
      if (cw === 0) continue;
      const fmt = tab.columns.formats.get(c);
      for (let r = rows.body.start; r < rows.body.end && most < MAX_ROW_LINES; r++) {
        const t = tab.cache.get(r, c);
        if (!t) continue;
        const shown = fmt ? formatCell(t, fmt) : t;
        most = Math.max(most, wrapLines(shown, Math.max(8, cw - 2 * pad), (s) => ctx.measureText(s).width, MAX_ROW_LINES).length);
      }
    }
    tab.rowLines = most;
    tab.bump();
  }

  // ---- mouse -----------------------------------------------------------------
  let drag: { kind: "select" | "resize" | "rows" | "cols" | "fill"; col?: number; startX?: number; startW?: number } | null = null;

  function local(e: MouseEvent): [number, number] {
    const r = canvas.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  }

  async function onMouseDown(e: MouseEvent): Promise<void> {
    // The canvas cannot take focus, so the browser's own mousedown handling
    // would move focus to the page body after we focus the text sink, and
    // keys (Ctrl+arrows, typing) would stop reaching the grid. Stop that
    // first, before anything async runs.
    e.preventDefault();
    if (editing && !(await commitEdit())) return;
    sink.focus({ preventScroll: true });
    const [x, y] = local(e);
    const h = hit(x, y);
    const fullCols = Math.max(0, tab.info.cols - 1);
    if (e.button === 2) {
      // Right-click inside the selection keeps it; elsewhere selects the target.
      const r = tab.rect;
      if (h.area === "cell" && !contains(r, h.row, h.col)) select({ row: h.row, col: h.col });
      if (h.area === "row" && !(h.row >= r.row && h.row < r.row + r.rows)) select({ row: h.row, col: 0 }, { row: h.row, col: fullCols }, false);
      if (h.area === "col" && !(h.col >= r.col && h.col < r.col + r.cols)) select({ row: tab.firstRow, col: h.col }, { row: lastRow(), col: h.col }, false);
      return;
    }
    if (e.button !== 0) return;
    if (onHandle(x, y) && onFill) {
      if (e.detail === 2) onFillAuto?.();
      else {
        drag = { kind: "fill" };
        fillTo = null;
      }
      return;
    }
    const fz = onFreezeEdge(x, y);
    if (fz) {
      // Frozen rows and columns count from the top-left of the file.
      if (fz === "rows") tab.scrollY = 0;
      else tab.scrollX = 0;
      clampScroll();
      freezing = { kind: fz, n: fz === "rows" ? frozenRows() : frozenCols() };
      redraw();
      return;
    }
    switch (h.area) {
      case "cell":
        if (e.ctrlKey && !e.shiftKey && onOpenLink) {
          const link = linkAt(h.row, h.col);
          if (link) {
            select({ row: h.row, col: h.col });
            onOpenLink(link);
            break;
          }
        }
        if (!e.shiftKey && !e.ctrlKey && onPickButton(x, y, h.row, h.col)) {
          void openPick();
          break;
        }
        if (!e.shiftKey && !e.ctrlKey && e.detail === 1 && onMoveCells && !readOnly && onSelOutline(x, y)) {
          moving = { kind: "cells", from: tab.rect, startX: x, startY: y, active: false, grab: { row: h.row, col: h.col }, to: null, px: x, py: y, offset: 0 };
          break;
        }
        if (!e.shiftKey && !e.ctrlKey && e.detail === 1 && onTickBox(x, y, h.row, h.col)) {
          select({ row: h.row, col: h.col });
          void toggleTick(h.row, h.col);
          break;
        }
        if (e.shiftKey) select(tab.sel.anchor, { row: h.row, col: h.col });
        else select({ row: h.row, col: h.col });
        drag = { kind: "select" };
        if (e.detail === 2) startEdit();
        break;
      case "row": {
        const r = tab.rect;
        if (!e.shiftKey && !e.ctrlKey && onMoveRows && !readOnly && wholeRowsSel(r) && h.row >= r.row && h.row < r.row + r.rows) {
          moving = { kind: "rows", from: r, startX: x, startY: y, active: false, grab: { row: h.row, col: 0 }, to: null, px: x, py: y, offset: y - rowY(r.row, visibleRows()) };
          break;
        }
        if (e.shiftKey) select({ row: tab.sel.anchor.row, col: 0 }, { row: h.row, col: fullCols }, false);
        else select({ row: h.row, col: 0 }, { row: h.row, col: fullCols }, false);
        drag = { kind: "rows" };
        break;
      }
      case "col":
        if (h.edge !== null) {
          const span = selectedColsWith(h.edge);
          if (e.detail === 2) {
            // Every selected column, as in Sheets (a few hundred at most:
            // each one measures the rows on screen).
            if (span && span.cols <= MAX_FIT_COLS) for (let c = span.col; c < span.col + span.cols; c++) autoFit(c);
            else autoFit(h.edge);
            break;
          }
          drag = { kind: "resize", col: h.edge, startX: x, startW: tab.columns.width(h.edge) };
          break;
        }
        {
          const run = hiddenAt(x, y, h.col);
          if (run) {
            tab.columns.showRange(run.from, run.to);
            tab.bump();
            break;
          }
        }
        if ((tab.filterButtons || filtered.has(h.col)) && onFilterButton) {
          const cols = visibleCols();
          const right = colX(h.col, cols) + tab.columns.width(h.col);
          if (x >= right - FILTER_BTN - 4) {
            // The menu listens on the window for clicks elsewhere; it opens
            // during this event, so keep this click from reaching the window
            // and closing it straight away.
            e.stopPropagation();
            const r = canvas.getBoundingClientRect();
            onFilterButton(h.col, r.left + right - FILTER_BTN - 2, r.top + headH);
            break;
          }
        }
        {
          const r = tab.rect;
          if (!e.shiftKey && !e.ctrlKey && onMoveCols && !readOnly && wholeColsSel(r) && h.col >= r.col && h.col < r.col + r.cols) {
            moving = { kind: "cols", from: r, startX: x, startY: y, active: false, grab: { row: 0, col: h.col }, to: null, px: x, py: y, offset: x - colX(r.col, visibleCols()) };
            break;
          }
        }
        if (e.shiftKey) select({ row: tab.firstRow, col: tab.sel.anchor.col }, { row: lastRow(), col: h.col }, false);
        else select({ row: tab.firstRow, col: h.col }, { row: lastRow(), col: h.col }, false);
        drag = { kind: "cols" };
        break;
      case "corner":
        select({ row: tab.firstRow, col: 0 }, { row: lastRow(), col: fullCols }, false);
        break;
    }
  }

  let cursor = $state("default");
  function onMouseMove(e: MouseEvent): void {
    const [x, y] = local(e);
    if (freezing) {
      freezing.n = freezeCount(freezing.kind, x, y);
      cursor = freezing.kind === "rows" ? "row-resize" : "col-resize";
      redraw();
      return;
    }
    if (moving) {
      const m = moving;
      if (!m.active) {
        if (Math.abs(x - m.startX) < MOVE_SLOP && Math.abs(y - m.startY) < MOVE_SLOP) return;
        m.active = true;
      }
      cursor = m.kind === "cells" && e.ctrlKey ? "copy" : "grabbing";
      // Scroll when the pointer leaves the body, along the axis that moves.
      if (m.kind !== "cols") {
        if (y > viewH() - 4) tab.scrollY += rowH;
        if (y < bodyTop() && tab.scrollY > 0) tab.scrollY -= rowH;
      }
      if (m.kind !== "rows") {
        if (x > viewW() - 4) tab.scrollX += 40;
        if (x < bodyLeft() && tab.scrollX > 0) tab.scrollX -= 40;
      }
      clampScroll();
      m.px = x;
      m.py = y;
      m.to = moveTarget(m, x, y);
      redraw();
      return;
    }
    if (!drag) {
      const h = hit(x, y);
      const pointer = h.area === "cell" && ((e.ctrlKey && !!linkAt(h.row, h.col)) || onTickBox(x, y, h.row, h.col) || onPickButton(x, y, h.row, h.col));
      const r = tab.rect;
      const grab =
        !readOnly &&
        ((h.area === "col" && h.edge === null && !!onMoveCols && wholeColsSel(r) && h.col >= r.col && h.col < r.col + r.cols) ||
          (h.area === "row" && !!onMoveRows && wholeRowsSel(r) && h.row >= r.row && h.row < r.row + r.rows) ||
          (h.area === "cell" && !!onMoveCells && !editing && onSelOutline(x, y)));
      const fz = onFreezeEdge(x, y);
      cursor = fz
        ? fz === "rows"
          ? "row-resize"
          : "col-resize"
        : onHandle(x, y)
          ? "crosshair"
          : h.area === "col" && h.edge !== null
            ? "col-resize"
            : grab || (h.area === "col" && hiddenAt(x, y, h.col))
              ? grab
                ? "grab"
                : "pointer"
              : pointer
                ? "pointer"
                : "default";
      return;
    }
    if (drag.kind === "resize" && drag.col !== undefined) {
      tab.columns.setWidth(drag.col, Math.max(24, (drag.startW ?? 0) + x - (drag.startX ?? 0)));
      tab.bump();
      return;
    }
    // Drag-select, scrolling when the pointer leaves the body.
    if (y > viewH() - 4) tab.scrollY += rowH;
    if (y < bodyTop() && tab.scrollY > 0) tab.scrollY -= rowH;
    if (x > viewW() - 4) tab.scrollX += 40;
    if (x < bodyLeft() && tab.scrollX > 0) tab.scrollX -= 40;
    clampScroll();
    const h = hit(Math.min(Math.max(x, rowHeaderW), viewW() - 1), Math.min(Math.max(y, headH), viewH() - 1));
    if (h.area !== "cell") return;
    if (drag.kind === "fill") {
      fillTo = fillTarget(h.row, h.col);
      redraw();
      return;
    }
    const fullCols = Math.max(0, tab.info.cols - 1);
    if (drag.kind === "select") tab.sel = { anchor: tab.sel.anchor, focus: { row: Math.max(h.row, tab.firstRow), col: h.col } };
    else if (drag.kind === "rows") tab.sel = { anchor: tab.sel.anchor, focus: { row: Math.max(h.row, tab.firstRow), col: fullCols } };
    else if (drag.kind === "cols") tab.sel = { anchor: tab.sel.anchor, focus: { row: lastRow(), col: h.col } };
    redraw();
  }

  /** The whole-column selection holding column `c`, when it has more than one. */
  function selectedColsWith(c: number): Rect | null {
    const r = tab.rect;
    return wholeColsSel(r) && r.cols > 1 && c >= r.col && c < r.col + r.cols ? r : null;
  }
  const MAX_FIT_COLS = 500;

  function onMouseUp(e: MouseEvent): void {
    if (freezing) {
      const f = freezing;
      freezing = null;
      if (f.kind === "rows") {
        tab.frozenRows = f.n;
        tab.scrollY = 0;
      } else {
        tab.frozenCols = f.n;
        tab.scrollX = 0;
      }
      cursor = "default";
      clampScroll();
      tab.bump();
      return;
    }
    if (drag?.kind === "resize" && drag.col !== undefined) {
      // Dragging one selected column's edge sizes all of them.
      const span = selectedColsWith(drag.col);
      if (span) {
        tab.columns.setRange(span.col, span.cols, tab.columns.width(drag.col));
        tab.bump();
      }
    }
    if (moving) {
      const m = moving;
      moving = null;
      cursor = "default";
      if (m.active && m.to) {
        if (m.kind === "cols") onMoveCols?.(m.from.col, m.from.cols, m.to.col);
        else if (m.kind === "rows") onMoveRows?.(m.from.row, m.from.rows, m.to.row);
        else onMoveCells?.(m.from, m.to.row, m.to.col, e.ctrlKey);
      } else if (!m.active) {
        // A click without a drag: select just what was clicked, as before.
        const fullCols = Math.max(0, tab.info.cols - 1);
        if (m.kind === "cols") select({ row: tab.firstRow, col: m.grab.col }, { row: lastRow(), col: m.grab.col }, false);
        else if (m.kind === "rows") select({ row: m.grab.row, col: 0 }, { row: m.grab.row, col: fullCols }, false);
        else select(m.grab);
      }
      redraw();
      return;
    }
    if (drag?.kind === "fill") {
      const f = fillTo;
      fillTo = null;
      redraw();
      if (f && f.count > 0) onFill?.(f.dir, f.count, e.ctrlKey);
    }
    drag = null;
  }

  function onContextMenu(e: MouseEvent): void {
    e.preventDefault();
    const [x, y] = local(e);
    const h = hit(x, y);
    if (h.area === "none") return;
    onContext(e.clientX, e.clientY, h.area);
  }

  function onWheel(e: WheelEvent): void {
    if (e.ctrlKey) {
      e.preventDefault();
      prefs.setZoom(prefs.data.zoom + (e.deltaY < 0 ? 0.1 : -0.1));
      return;
    }
    e.preventDefault();
    const unit = e.deltaMode === 1 ? rowH : e.deltaMode === 2 ? viewH() : 1;
    if (e.shiftKey && e.deltaX === 0) tab.scrollX += e.deltaY * unit;
    else {
      tab.scrollY += e.deltaY * unit;
      tab.scrollX += e.deltaX * unit;
    }
    clampScroll();
    redraw();
  }

  // ---- scrollbars ------------------------------------------------------------
  const vThumb = $derived.by(() => {
    void scrollTick;
    const track = Math.max(0, height - SCROLLBAR);
    const max = maxScrollY();
    const visible = Math.max(1, viewH() - bodyTop());
    const size = max <= 0 ? track : Math.max(28, (track * visible) / (visible + max));
    const pos = max <= 0 ? 0 : ((track - size) * tab.scrollY) / max;
    return { size, pos, track, max };
  });
  const hThumb = $derived.by(() => {
    void scrollTick;
    const track = Math.max(0, width - SCROLLBAR);
    const max = maxScrollX();
    const visible = Math.max(1, viewW() - bodyLeft());
    const size = max <= 0 ? track : Math.max(28, (track * visible) / (visible + max));
    const pos = max <= 0 ? 0 : ((track - size) * tab.scrollX) / max;
    return { size, pos, track, max };
  });

  function thumbDrag(axis: "x" | "y", e: PointerEvent): void {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const start = axis === "y" ? e.clientY : e.clientX;
    const startScroll = axis === "y" ? tab.scrollY : tab.scrollX;
    const t = axis === "y" ? vThumb : hThumb;
    const move = (ev: PointerEvent) => {
      const delta = (axis === "y" ? ev.clientY : ev.clientX) - start;
      const free = Math.max(1, t.track - t.size);
      const value = startScroll + (delta / free) * t.max;
      if (axis === "y") tab.scrollY = Math.round(value / rowH) * rowH;
      else tab.scrollX = value;
      clampScroll();
      redraw();
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  }

  function trackClick(axis: "x" | "y", e: MouseEvent): void {
    if (e.target !== e.currentTarget) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const t = axis === "y" ? vThumb : hThumb;
    const at = axis === "y" ? e.clientY - r.top : e.clientX - r.left;
    const page = axis === "y" ? viewH() - bodyTop() : viewW() - bodyLeft();
    const dir = at < t.pos ? -1 : 1;
    if (axis === "y") tab.scrollY += dir * page;
    else tab.scrollX += dir * page;
    clampScroll();
    redraw();
  }

  // ---- inline editing ----------------------------------------------------------
  function cellRect(c: Cell): { x: number; y: number; w: number; h: number } | null {
    const rows = visibleRows();
    const cols = visibleCols();
    const inRows = (c.row >= rows.frozen.start && c.row < rows.frozen.end) || (c.row >= rows.body.start && c.row < rows.body.end);
    const inCols = c.col < cols.frozen.end || (c.col >= cols.body.start && c.col < cols.body.end);
    if (!inRows || !inCols) return null;
    return { x: colX(c.col, cols), y: rowY(c.row, rows), w: tab.columns.width(c.col), h: rowH };
  }

  function positionEditor(): void {
    if (!editing) return;
    const r = cellRect(editing);
    if (r) {
      editing.x = r.x;
      editing.y = r.y;
      editing.w = r.w;
      editing.h = r.h;
    }
  }

  export function isEditing(): boolean {
    return editing !== null;
  }

  /** Edit the active cell; `initial` replaces its value (typing to start). */
  export async function startEdit(initial?: string): Promise<void> {
    if (readOnly) {
      onMessage("This file is still loading; editing starts when it has been read.");
      return;
    }
    if (editing) {
      // Keys typed before the editor took focus belong to the same edit.
      if (initial !== undefined && editor) {
        editor.value += initial;
        editing.value = editor.value;
      }
      return;
    }
    const c = { ...tab.sel.focus };
    scrollIntoView(c);
    let value = initial ?? "";
    if (initial === undefined) {
      try {
        const t = await backend().cellText(tab.id, c.row, c.col);
        if (t.truncated) {
          onMessage("This cell is too long to edit here (over 1,048,576 characters).");
          return;
        }
        value = t.text;
      } catch (e) {
        onMessage(errorText(e));
        return;
      }
    }
    const r = cellRect(c) ?? { x: rowHeaderW, y: rowH, w: 120, h: rowH };
    pickTyped = initial !== undefined;
    pickIndex = -1;
    editing = { row: c.row, col: c.col, value, ...r };
    await tick();
    editor?.focus({ preventScroll: true });
    const end = editor?.value.length ?? 0;
    editor?.setSelectionRange(end, end);
  }

  export async function commitEdit(fill = false): Promise<boolean> {
    if (!editing) return true;
    const e = editing;
    const ok = await onCommit(e.row, e.col, editor?.value ?? e.value, fill);
    if (ok) {
      editing = null;
      pickHidden = false;
      sink?.focus({ preventScroll: true });
    }
    return ok;
  }

  /** Caret position in the open cell editor, if one is open. */
  export function editorCaret(): number | null {
    return editing && editor ? editor.selectionStart : null;
  }

  /** Put `text` into the active cell at `at` (end when null), opening the
   *  editor on the cell's value first when it is closed. */
  export async function insertText(text: string, at: number | null = null): Promise<void> {
    if (!editing) await startEdit();
    if (!editing || !editor) return;
    const pos = at === null ? editor.value.length : Math.min(at, editor.value.length);
    editor.focus({ preventScroll: true });
    editor.setRangeText(text, pos, pos, "end");
    editing.value = editor.value;
  }

  export function cancelEdit(): void {
    editing = null;
    pickHidden = false;
    sink?.focus({ preventScroll: true });
  }

  // ---- links, tick boxes and pick lists ----------------------------------------
  /** The checked address in a loaded cell, if it holds one. */
  function linkAt(row: number, col: number): string | null {
    if (tab.header && row === 0) return null;
    const t = tab.cache.get(row, col);
    return t && !tab.cache.isTruncated(row, col) ? linkOf(t) : null;
  }

  /** The pointer is on the tick box of a cell shown as a tick box. */
  function onTickBox(px: number, py: number, row: number, col: number): boolean {
    if (readOnly || tab.columns.formats.get(col)?.kind !== "checkbox" || (tab.header && row === 0)) return false;
    const t = tab.cache.get(row, col);
    if (t === undefined || tickOf(t) === null) return false;
    const r = cellRect({ row, col });
    if (!r) return false;
    const size = Math.min(14, rowH - 6, r.w - 6) + 6;
    const cx = r.x + r.w / 2;
    const cy = r.y + headH / 2;
    return Math.abs(px - cx) <= size / 2 && Math.abs(py - cy) <= size / 2;
  }

  async function toggleTick(row: number, col: number): Promise<void> {
    const t = tab.cache.get(row, col);
    if (t === undefined) return;
    const others: string[] = [];
    for (let r = Math.max(tab.firstRow, row - 20); r < row + 20; r++) {
      const v = r === row ? undefined : tab.cache.get(r, col);
      if (v) others.push(v);
    }
    const next = toggled(t, others);
    if (next !== null) await onCommit(row, col, next, false);
  }

  /** The pointer is on the pick-list arrow of the active cell. */
  function onPickButton(px: number, py: number, row: number, col: number): boolean {
    const a = tab.sel.focus;
    if (readOnly || row !== a.row || col !== a.col || !listCols.has(col) || (tab.header && row === 0)) return false;
    const r = cellRect(a);
    return !!r && r.w > PICK_BTN + 8 && px >= r.x + r.w - PICK_BTN - 4 && py < r.y + headH;
  }

  let pickIndex = $state(-1);
  let pickHidden = $state(false);
  /** The user typed in the editor (the list then narrows to the text). */
  let pickTyped = $state(false);
  const picks = $derived.by(() => {
    if (!editing || pickHidden || (tab.header && editing.row === 0)) return null;
    const values = listCols.get(editing.col);
    if (!values?.length) return null;
    return pickMatches(values, pickTyped ? editing.value : "");
  });

  /** Edit the active cell with its column's list of allowed values open
   *  (Alt+Down or the arrow). False when the column has no list. */
  export async function openPick(): Promise<boolean> {
    const a = tab.sel.focus;
    if (!listCols.has(a.col) || (tab.header && a.row === 0)) return false;
    pickHidden = false;
    pickTyped = false;
    if (!editing) await startEdit();
    const values = listCols.get(a.col) ?? [];
    pickIndex = editing ? values.indexOf(editing.value) : -1;
    return true;
  }

  async function choosePick(v: string): Promise<void> {
    if (!editing) return;
    editing.value = v;
    if (editor) editor.value = v;
    await commitEdit();
  }

  /** Type-ahead requests: only the newest one's answer is used. */
  let suggestAsked = 0;

  function onEditorInput(e: Event): void {
    if (!editing) return;
    const ta = e.currentTarget as HTMLTextAreaElement;
    editing.value = ta.value;
    pickTyped = true;
    pickHidden = false;
    pickIndex = -1;
    // Type-ahead (Calc's AutoInput): typing at the end of a one-line cell
    // offers the rest of a value already in the column, selected, so more
    // typing replaces it and Enter keeps it.
    const ie = e as InputEvent;
    const n = ++suggestAsked;
    if (
      !prefs.data.typeahead ||
      ie.inputType !== "insertText" ||
      listCols.has(editing.col) ||
      (tab.header && editing.row === 0) ||
      ta.value.includes("\n") ||
      ta.selectionStart !== ta.value.length
    )
      return;
    const typed = ta.value;
    const at = { row: editing.row, col: editing.col };
    void backend()
      .typeahead(tab.id, at.col, typed, tab.header)
      .then((s) => {
        const c = completion(typed, s);
        if (!c || n !== suggestAsked || !editing || !editor) return;
        if (editing.row !== at.row || editing.col !== at.col || editor.value !== typed) return;
        editor.value = c.text;
        editing.value = c.text;
        editor.setSelectionRange(c.from, c.text.length);
      })
      .catch(() => {});
  }

  async function onEditorKey(e: KeyboardEvent): Promise<void> {
    e.stopPropagation();
    if (picks && picks.length && !e.ctrlKey) {
      if (e.key === "ArrowDown" && !e.altKey) {
        e.preventDefault();
        pickIndex = Math.min(picks.length - 1, pickIndex + 1);
        return;
      }
      if (e.key === "ArrowUp" && !e.altKey) {
        e.preventDefault();
        pickIndex = Math.max(-1, pickIndex - 1);
        return;
      }
      if (e.key === "Enter" && !e.shiftKey && !e.altKey && pickIndex >= 0) {
        e.preventDefault();
        await choosePick(picks[pickIndex]!);
        return;
      }
    }
    if (picks && e.key === "Escape") {
      e.preventDefault();
      pickHidden = true;
      return;
    }
    if (e.key === "ArrowDown" && e.altKey && listCols.has(editing?.col ?? -1)) {
      e.preventDefault();
      pickHidden = false;
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    } else if (e.key === "Enter" && e.ctrlKey) {
      // Ctrl+Enter: put the value into every selected cell.
      e.preventDefault();
      await commitEdit(true);
    } else if ((e.key === ";" || e.key === ":") && e.ctrlKey && editor) {
      // Ctrl+; today's date, Ctrl+Shift+; the time.
      e.preventDefault();
      const s = editor.selectionStart;
      editor.setRangeText(e.shiftKey ? nowTime() : today(), s, editor.selectionEnd, "end");
    } else if (e.key === "Enter" && !e.altKey && !e.shiftKey && !e.ctrlKey) {
      e.preventDefault();
      if (await commitEdit()) move(1, 0, false);
    } else if (e.key === "Enter" && e.shiftKey && !e.altKey) {
      e.preventDefault();
      if (await commitEdit()) move(-1, 0, false);
    } else if (e.key === "Enter" && e.altKey && editor) {
      // Alt+Enter: a line break inside the cell.
      e.preventDefault();
      const s = editor.selectionStart;
      editor.setRangeText("\n", s, editor.selectionEnd, "end");
    } else if (e.key === "Tab") {
      e.preventDefault();
      if (await commitEdit()) move(0, e.shiftKey ? -1 : 1, false);
    }
  }

  function pad2(n: number): string {
    return String(n).padStart(2, "0");
  }
  export function today(): string {
    const d = new Date();
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  }
  export function nowTime(): string {
    const d = new Date();
    return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  }

  function onSinkInput(e: Event): void {
    if ((e as InputEvent).isComposing) return;
    const text = sink.value;
    sink.value = "";
    if (text) void startEdit(text);
  }

  /** Where to open a column's filter menu (window coordinates). */
  export function headerPoint(col: number): { x: number; y: number } {
    const r = canvas.getBoundingClientRect();
    const cols = visibleCols();
    const x = colX(col, cols) + tab.columns.width(col) - FILTER_BTN - 2;
    return { x: r.left + Math.max(rowHeaderW, x), y: r.top + headH };
  }

  /** A column's title as the header shows it. */
  export function title(col: number): string {
    return colTitle(col);
  }

  export function focus(): void {
    sink?.focus({ preventScroll: true });
  }

  $effect(() => {
    const t = tab;
    t.onRedraw = redraw;
    return () => {
      if (t.onRedraw === redraw) t.onRedraw = null;
    };
  });

  onMount(() => {
    readColors();
    const ro = new ResizeObserver(() => {
      width = host.clientWidth;
      height = host.clientHeight;
    });
    ro.observe(host);
    // The grid scrolls itself; never let the browser scroll the host box
    // (it would when focusing the editor near an edge).
    const noNativeScroll = () => {
      host.scrollTop = 0;
      host.scrollLeft = 0;
    };
    host.addEventListener("scroll", noNativeScroll);
    const media = matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () => {
      readColors();
      redraw();
    };
    media.addEventListener("change", onScheme);
    window.addEventListener("mouseup", onMouseUp);
    // Esc while dragging to move: drop nothing.
    const onKey = (e: KeyboardEvent) => {
      if ((moving || freezing) && e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        cancelMove();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      ro.disconnect();
      host.removeEventListener("scroll", noNativeScroll);
      media.removeEventListener("change", onScheme);
      window.removeEventListener("mouseup", onMouseUp);
      if (frame) cancelAnimationFrame(frame);
    };
  });
</script>

<div
  class="grid"
  bind:this={host}
  role="grid"
  aria-label="Data grid"
  aria-rowcount={tab.info.rows}
  aria-colcount={tab.info.cols}
  style:cursor
>
  <textarea
    bind:this={sink}
    class="sink"
    data-grid-focus
    aria-label="Grid keyboard input"
    autocomplete="off"
    spellcheck="false"
    oninput={onSinkInput}
    oncompositionend={onSinkInput}
  ></textarea>
  <canvas
    bind:this={canvas}
    onmousedown={onMouseDown}
    onmousemove={onMouseMove}
    oncontextmenu={onContextMenu}
    onwheel={onWheel}
  ></canvas>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="vbar" style:height="{vThumb.track}px" onclick={(e) => trackClick("y", e)}>
    <div class="thumb" style:height="{vThumb.size}px" style:top="{vThumb.pos}px" onpointerdown={(e) => thumbDrag("y", e)}></div>
  </div>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="hbar" style:width="{hThumb.track}px" onclick={(e) => trackClick("x", e)}>
    <div class="thumb" style:width="{hThumb.size}px" style:left="{hThumb.pos}px" onpointerdown={(e) => thumbDrag("x", e)}></div>
  </div>
  {#if editing}
    <textarea
      bind:this={editor}
      class="editor"
      spellcheck={prefs.data.spellcheck}
      style:left="{editing.x}px"
      style:top="{editing.y}px"
      style:min-width="{editing.w}px"
      style:height="{Math.max(editing.h, editing.value.includes('\n') ? editing.h * 3 : editing.h)}px"
      style:font
      value={editing.value}
      oninput={onEditorInput}
      onkeydown={onEditorKey}
      onblur={() => void commitEdit()}
      aria-label="Cell editor"
      aria-autocomplete={picks ? "list" : undefined}
    ></textarea>
    {#if picks}
      <div
        class="picks"
        role="listbox"
        aria-label="Allowed values"
        style:left="{editing.x}px"
        style:top="{editing.y + Math.max(editing.h, editing.value.includes('\n') ? editing.h * 3 : editing.h)}px"
        style:min-width="{Math.max(editing.w, 160)}px"
        style:font
      >
        {#each picks as v, i (i)}
          <!-- Mousedown keeps the editor focused, so it does not commit first. -->
          <div
            class="pick"
            class:on={i === pickIndex}
            role="option"
            tabindex="-1"
            aria-selected={i === pickIndex}
            onmousedown={(e) => {
              e.preventDefault();
              void choosePick(v);
            }}
          >
            {v === "" ? "(blank)" : v}
          </div>
        {:else}
          <div class="pick none">Not in the list of allowed values</div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .grid {
    position: relative;
    overflow: hidden;
    outline: none;
    background: var(--surface);
    min-height: 0;
    user-select: none;
  }
  canvas {
    position: absolute;
    left: 0;
    top: 0;
    display: block;
  }
  .vbar,
  .hbar {
    position: absolute;
    background: var(--bg);
  }
  .vbar {
    right: 0;
    top: 0;
    width: 12px;
  }
  .hbar {
    left: 0;
    bottom: 0;
    height: 12px;
  }
  .thumb {
    position: absolute;
    background: var(--border);
    border-radius: 6px;
    cursor: default;
  }
  .vbar .thumb {
    left: 2px;
    width: 8px;
  }
  .hbar .thumb {
    top: 2px;
    height: 8px;
  }
  .thumb:hover {
    background: var(--muted);
  }
  .sink {
    position: absolute;
    left: 0;
    top: 0;
    width: 1px;
    height: 1px;
    opacity: 0;
    padding: 0;
    border: 0;
    resize: none;
    pointer-events: none;
  }
  .picks {
    position: absolute;
    z-index: 3;
    max-height: 240px;
    max-width: 60vw;
    overflow-y: auto;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.18);
    padding: 2px 0;
  }
  .pick {
    padding: 3px 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: default;
  }
  .pick:hover,
  .pick.on {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .pick.none {
    color: var(--muted);
    font-style: italic;
  }
  .editor {
    position: absolute;
    z-index: 2;
    margin: 0;
    padding: 0 0.45em;
    border: 2px solid var(--accent);
    background: var(--surface);
    color: var(--text);
    resize: none;
    outline: none;
    line-height: 1.6;
    max-width: 60vw;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.18);
    field-sizing: content;
  }
</style>
