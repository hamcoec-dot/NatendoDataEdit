<script lang="ts">
  // Find and replace bar. Searches run in the engine (safe regular
  // expressions, whole files of any size); results select cells in the grid.
  import { backend, errorText, type DocInfo, type FindAll, type FindCount, type FindRowsAction, type FindSpec, type ReplaceReport } from "./backend";
  import { onDestroy, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import AllNone from "./AllNone.svelte";
  import { FLAG_NAMES, type NoteMark } from "./notes";
  import { MAX_SAVED_SEARCHES, prefs, type SavedSearch } from "./prefs.svelte";
  import { cellDiff } from "./textDiff";
  import { clearFindHistory, findHistory, rememberFind, rememberReplace, stepHistory } from "./findHistory.svelte";
  import type { Tab } from "./tab.svelte";
  import { findSpecOf, type FindOptions } from "./filters";
  import { rectOf, type Rect } from "./selection";
  import { colLetters } from "./goto";

  interface Props {
    tab: Tab;
    replace: boolean;
    options: FindOptions;
    /** Runs a long engine call with progress shown and Cancel offered. */
    run: <T>(label: string, f: () => Promise<T>) => Promise<T | null>;
    onSelect: (row: number, col: number) => void;
    /** The block "Selection" and "Selected columns" search in, kept from
     *  when Find opened (or the scope was picked): a hit selects one cell,
     *  so the live selection can't be used after the first Find next. */
    block: Rect | null;
    /** Every open tab ("All open tabs" searches them all). */
    tabs: Tab[];
    /** A change to a tab (the one shown unless `t` says another). */
    onChanged: (info: DocInfo, t?: Tab) => void;
    /** Go to a cell of another tab (a match found there). */
    onGoTab: (t: Tab, row: number, col: number) => void;
    /** A new tab made from the matching rows. */
    onNewTab: (info: DocInfo) => void;
    /** Rename columns of the shown tab (replacing in column titles). */
    onRenameColumns: (renames: [number, string][]) => Promise<void>;
    /** Open "Replace from a list…". */
    onBatch: () => void;
    /** Copy the Find all list (after the privacy check); false if stopped. */
    onCopy: (text: string) => Promise<boolean>;
    onClose: () => void;
  }
  let { tab, tabs, replace = $bindable(), options = $bindable(), block = $bindable(), run, onSelect, onChanged, onGoTab, onNewTab, onRenameColumns, onBatch, onCopy, onClose }: Props = $props();

  /** Number, date and similar patterns test the whole cell; the text
   *  options don't apply to them. */
  const valueKind = $derived(!options.blank && (options.kind === "number" || options.kind === "date" || options.kind === "similar"));
  const placeholders: Record<string, string> = {
    number: "A number: 5, 10..20, >5, <=100",
    date: "A date: 1/5/2024, 1/1/2024..3/31/2024, >1/5/2024",
    similar: "A name or word: Jon Smyth finds John Smith",
  };

  // Picking "Selection" or "Selected columns" while the box is open takes
  // the block selected then; "Whole file" lets it go.
  let lastScope = options.scope;
  $effect(() => {
    const s = options.scope;
    if (s === lastScope) return;
    lastScope = s;
    if (s === "all" || s === "pick" || s === "tabs") block = null;
    else {
      const r = rectOf(tab.sel);
      block = r.rows * r.cols > 1 ? { ...r } : null;
    }
  });

  // The block in the scope box, so it is clear which one is searched.
  const blockCells = $derived(block ? `${colLetters(block.col)}${block.row + 1}:${colLetters(block.col + block.cols - 1)}${block.row + block.rows}` : "");
  const blockCols = $derived(block ? (block.cols > 1 ? `${colLetters(block.col)}:${colLetters(block.col + block.cols - 1)}` : colLetters(block.col)) : "");

  let findInput = $state<HTMLInputElement>();
  let replaceInput = $state<HTMLInputElement>();
  let replacement = $state("");
  let status = $state("");
  /** Find all's matches, a group per tab searched. */
  let results = $state<{ t: Tab; r: FindAll }[] | null>(null);
  /** The search Find all ran on the shown tab (for acting on the rows). */
  let resultsSpec: FindSpec | null = null;
  let resultsRev: number | undefined;
  /** The versions of the tabs Find all searched: a change closes the list,
   *  so its row actions never act on rows it doesn't show. */
  let resultsAt = "";
  // A tab's edits (its place in the undo history) and the rows shown: a
  // redraw alone doesn't count.
  const versions = (ts: Tab[]): string => ts.map((t) => `${t.id}:${t.info.history_pos}:${t.info.rows}:${t.info.total_rows}`).join(",");
  $effect(() => {
    const now = results ? versions(results.map((g) => g.t)) : "";
    untrack(() => {
      if (!results) {
        resultsAt = "";
        return;
      }
      if (!resultsAt) resultsAt = now;
      else if (now !== resultsAt) {
        results = null;
        deleteAsk = false;
        resultsAt = "";
        status = "The file changed, so the Find all list was closed. Run Find all again.";
      }
    });
  });
  /** Row notes that match ("Look in: Notes"). */
  let notes = $state<NoteMark[] | null>(null);
  let pending = $state<ReplaceReport | null>(null);
  /** Column titles the open preview renames: [column, old, new]. */
  let titleChanges = $state<[number, string, string][]>([]);
  /** All open tabs: each tab's part of the open preview. */
  let tabParts = $state<{ t: Tab; spec: FindSpec; r: ReplaceReport }[]>([]);
  /** Changes unticked in the preview ("row:col"); they are left as they are. */
  const unticked = new SvelteSet<string>();
  /** The last Replace all changed rows here: "Show changed rows" is offered. */
  /** The tab and its version right after the last Replace all. */
  let changedAt = $state<[number, number] | null>(null);
  const canShowChanged = $derived(changedAt !== null && changedAt[0] === tab.id && changedAt[1] === tab.info.history_pos && !tab.info.filtered);
  let deleteAsk = $state(false);
  let flag = $state(1);
  /** Looking across every open tab. */
  const allTabs = $derived(options.scope === "tabs" && options.lookIn === "cells");
  /** Column titles are searched too (and replaced by renaming). */
  const withTitles = $derived(options.titles && tab.header && options.lookIn === "cells");

  // ---- the count ("3 of 1,204") and matches lit up in the grid ----
  let count = $state<FindCount | null>(null);
  /** The cell the count's "3 of" is about. */
  let countAt = $state<[number, number] | null>(null);
  let countTimer: ReturnType<typeof setTimeout> | null = null;
  let countSeq = 0;

  /** Count again after typing stops (300 ms); `at` is the active cell. */
  function recount(delay = 300): void {
    if (countTimer) clearTimeout(countTimer);
    const seq = ++countSeq;
    countTimer = setTimeout(() => {
      countTimer = null;
      if (!searching()) {
        count = null;
        tab.findCount = null;
        return;
      }
      const s = spec();
      const at: [number, number] = [tab.cell.row, tab.cell.col];
      backend()
        .findCount(tab.id, s, at)
        .then((c) => {
          if (seq !== countSeq) return;
          count = c;
          countAt = at;
          tab.findCount = c.cells;
        })
        .catch((e: unknown) => {
          if (seq !== countSeq) return;
          const kind = (e as { kind?: string } | null)?.kind;
          // Busy or cancelled: try once more shortly; a bad pattern: no count.
          if (kind === "busy" || kind === "cancelled") recount(600);
          else {
            count = null;
            tab.findCount = null;
          }
        });
    }, delay);
  }

  /** What the count says now. */
  const countText = $derived.by(() => {
    if (!count || !searching()) return "";
    if (count.cells === 0) return "No matches";
    const total = count.cells.toLocaleString();
    const at = countAt;
    if (count.here && at && at[0] === tab.cell.row && at[1] === tab.cell.col) return `${(count.before + 1).toLocaleString()} of ${total}`;
    return `${total} match${count.cells === 1 ? "" : "es"}`;
  });

  // Whatever changes the search (text, options, block, the file) counts
  // again and lights up the matches on screen.
  $effect(() => {
    void [options.text, options.kind, options.matchCase, options.wholeCell, options.wholeWord, options.ignoreAccents, options.escapes, options.includeHidden, options.scope, options.blank, options.titles, options.lookIn, options.cols.length, block, tab.version];
    const s = searching() ? $state.snapshot(spec()) : null;
    tab.findSpec = s;
    tab.findVersion++;
    tab.onRedraw?.();
    recount();
  });

  onDestroy(() => {
    if (countTimer) clearTimeout(countTimer);
    countSeq++;
    void backend().findCountCancel(tab.id).catch(() => {});
    tab.findSpec = null;
    tab.findCount = null;
    tab.findVersion++;
    tab.onRedraw?.();
  });

  export function focusReplace(): void {
    void Promise.resolve().then(() => {
      replaceInput?.focus();
      replaceInput?.select();
    });
  }

  export function focus(): void {
    findInput?.focus();
    findInput?.select();
  }

  $effect(() => {
    void replace;
    focus();
  });

  /** There is something to look for (in the cells, not the notes). */
  function searching(): boolean {
    return options.lookIn === "cells" && (options.blank || options.text !== "") && !(options.scope === "pick" && options.cols.length === 0);
  }

  /** The search as set up now. A one-cell "selection" scope searches all. */
  function spec(): FindSpec {
    const r = options.scope !== "all" && block ? block : rectOf(tab.sel);
    const o = { ...options, cols: [...options.cols] };
    if (o.scope === "selection" && r.rows * r.cols <= 1) o.scope = "all";
    if (o.scope === "tabs") o.scope = "all";
    return findSpecOf(o, { row: r.row, rows: r.rows, col: r.col, cols: r.cols }, tab.header, tab.columns.hiddenList());
  }

  /** The search for another tab (all of it). */
  function specFor(t: Tab): FindSpec {
    if (t === tab) return spec();
    return findSpecOf({ ...options, cols: [], scope: "all" }, { row: 0, rows: 0, col: 0, cols: 0 }, t.header, t.columns.hiddenList());
  }

  /** The tabs searched: every open one, or the shown one. */
  function targets(): Tab[] {
    return allTabs ? [tab, ...tabs.filter((t) => t !== tab)] : [tab];
  }

  // ---- Special… : characters that are hard to type or to see ----
  let special = $state(false);
  const SPECIALS: { label: string; plain: string; regex: string; regexOnly?: boolean }[] = [
    { label: "Non-breaking space", plain: "\\u{A0}", regex: "\\x{A0}" },
    { label: "Zero-width space", plain: "\\u{200B}", regex: "\\x{200B}" },
    { label: "Tab", plain: "\\t", regex: "\\t" },
    { label: "Line break", plain: "\\n", regex: "\\n" },
    { label: "Curly quotes \u201c \u201d", plain: "\u201c", regex: "[\u201c\u201d]" },
    { label: "Curly apostrophe \u2019", plain: "\u2019", regex: "\u2019" },
    { label: "Any digit", plain: "", regex: "\\d", regexOnly: true },
    { label: "Any letter", plain: "", regex: "\\pL", regexOnly: true },
  ];
  function insertSpecial(i: number): void {
    const s = SPECIALS[i]!;
    const regex = options.kind === "regex";
    if (s.regexOnly && !regex) return;
    const text = regex ? s.regex : s.plain;
    // \u{..} and \t stand for the character only with escapes on.
    if (!regex && text.startsWith("\\")) options.escapes = true;
    options.blank = false;
    const el = findInput;
    const at = el?.selectionStart ?? options.text.length;
    const end = el?.selectionEnd ?? at;
    options.text = options.text.slice(0, at) + text + options.text.slice(end);
    special = false;
    void Promise.resolve().then(() => {
      el?.focus();
      el?.setSelectionRange(at + text.length, at + text.length);
    });
  }

  // ---- the column picker (scope "Columns…") ----
  let picking = $state(false);
  const pickTitles = $derived(Array.from({ length: Math.min(tab.info.cols, 2000) }, (_, c) => (tab.header ? (heads[c] || tab.cache.get(0, c) || `Column ${c + 1}`) : `Column ${c + 1}`)));
  $effect(() => {
    if (options.scope === "pick") untrack(() => (picking = options.cols.length === 0));
    else untrack(() => (picking = false));
  });


  /** Alt+C / Alt+W / Alt+R / Alt+L while the bar has focus (as in VS Code):
   *  match case, whole word, regular expression, in selection. */
  function onBarKey(e: KeyboardEvent): void {
    if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const k = e.key.toLowerCase();
    if (k === "c") options.matchCase = !options.matchCase;
    else if (k === "w") options.wholeWord = !options.wholeWord;
    else if (k === "r") options.kind = options.kind === "regex" ? "plain" : "regex";
    else if (k === "l") options.scope = options.scope === "all" ? "selection" : "all";
    else return;
    e.preventDefault();
    results = null;
    pending = null;
  }

  function ready(): boolean {
    if (options.scope === "pick" && options.cols.length === 0 && options.lookIn === "cells") {
      status = "Tick the columns to search in.";
      picking = true;
      return false;
    }
    if (options.blank && options.lookIn === "cells") return true;
    if (options.text === "") {
      status = "Type what to look for.";
      focus();
      return false;
    }
    rememberFind(options.text);
    return true;
  }

  let help = $state(false);

  // ---- the Replace all preview, a page at a time (every change) ----
  const PAGE = 50;
  let page = $state(0);
  let pageRows = $state<ReplaceReport["preview"]>([]);
  const pages = $derived(pending ? Math.max(1, Math.ceil(pending.cells / PAGE)) : 1);
  async function showPage(n: number): Promise<void> {
    if (!pending) return;
    const want = Math.max(0, Math.min(pages - 1, n));
    try {
      pageRows = await backend().replacePage(tab.id, want * PAGE, PAGE);
      page = want;
    } catch (e) {
      status = errorText(e);
    }
  }

  /** Row notes that match the text. */
  async function matchingNotes(): Promise<NoteMark[] | null> {
    const pattern = spec().pattern;
    try {
      return await backend().findNotes(tab.id, pattern);
    } catch (e) {
      status = errorText(e);
      return null;
    }
  }

  export async function next(backward = false): Promise<void> {
    if (!ready()) return;
    const f = tab.cell;
    if (options.lookIn === "notes") {
      const list = await matchingNotes();
      if (!list) return;
      if (!list.length) {
        status = "No notes match.";
        return;
      }
      const rows = list.map((n) => n.row).sort((a, b) => a - b);
      let to = backward ? [...rows].reverse().find((r) => r < f.row) : rows.find((r) => r > f.row);
      const wrapped = to === undefined;
      if (wrapped) to = backward ? rows[rows.length - 1] : rows[0];
      onSelect(to!, f.col);
      const n = list.find((x) => x.row === to);
      status = `${wrapped ? "Started over. " : ""}Note: ${(n?.text ?? "").slice(0, 120)}`;
      return;
    }
    try {
      const hit = await run("Searching", () => backend().findNext(tab.id, spec(), [f.row, f.col], backward));
      if (hit === null) return;
      if (!hit) {
        status = "No matches.";
        return;
      }
      onSelect(hit.row, hit.col);
      status = hit.wrapped ? `Found (search started over from the ${backward ? "end" : "top"}).` : "";
      recount(0);
    } catch (e) {
      status = errorText(e);
    }
  }

  export async function findAll(): Promise<void> {
    if (!ready()) return;
    deleteAsk = false;
    if (options.lookIn === "notes") {
      const list = await matchingNotes();
      if (!list) return;
      notes = list;
      results = null;
      status = list.length ? "" : "No notes match.";
      return;
    }
    notes = null;
    const groups: { t: Tab; r: FindAll }[] = [];
    const skipped: string[] = [];
    for (const t of targets()) {
      const s = specFor(t);
      try {
        const r = await run(allTabs ? `Searching ${t.info.name}` : "Searching", () => backend().findAll(t.id, s));
        if (!r) return;
        if (t === tab) {
          resultsSpec = s;
          resultsRev = r.revision;
        }
        if (r.cells > 0) groups.push({ t, r });
      } catch (e) {
        if (!allTabs) {
          status = errorText(e);
          return;
        }
        skipped.push(`${t.info.name} (${errorText(e)})`);
      }
    }
    results = groups;
    resultsAt = "";
    // Going down columns: the list follows the same order.
    if (options.byColumns) order = "cols";
    const n = groups.reduce((a, g) => a + g.r.cells, 0);
    status = (n === 0 ? "No matches." : "") + (skipped.length ? ` Not searched: ${skipped.join("; ")}.` : "");
  }

  /** Act on the rows Find all found in the shown tab. */
  async function actOnRows(action: FindRowsAction): Promise<void> {
    const s = resultsSpec;
    if (!s) return;
    deleteAsk = false;
    const label = action.kind === "delete" ? "Deleting rows" : action.kind === "copy" ? "Copying rows" : action.kind === "show" ? "Filtering" : "Flagging rows";
    try {
      const rep = await run(label, () => backend().findRows(tab.id, s, action, resultsRev));
      if (!rep) return;
      const info = rep.info;
      if (action.kind === "copy") {
        onNewTab(info);
        status = `Copied ${plural(info.rows, "row")} to a new tab (not saved yet)${rep.left_out_cols ? `; ${plural(rep.left_out_cols, "hidden column")} left out` : ""}.`;
        return;
      }
      results = null;
      onChanged(info);
      if (action.kind === "show") status = "Showing only the rows with a match. Show all rows from the status bar, or undo.";
      else if (action.kind === "delete") status = "Deleted the rows with a match. Ctrl+Z puts them back.";
      else {
        tab.notesVersion++;
        tab.bump();
        const colour = FLAG_NAMES[action.flag]?.[1]?.toLowerCase() ?? "";
        status = `Flagged the rows with a match ${colour}${rep.recolored ? ` (${plural(rep.recolored, "row")} had another colour and ${rep.recolored === 1 ? "is" : "are"} now ${colour})` : ""}. Flags are kept with the notes, so Ctrl+Z doesn't take them off: clear them from Notes.`;
      }
    } catch (e) {
      status = errorText(e);
    }
  }

  /** Find all listed matches in hidden rows: row actions act on shown rows. */
  const hiddenListed = $derived(results?.some((g) => g.r.hits.some((h) => h.hidden)) ?? false);

  /** The Find all list as tab-separated text (tab, row, column, text). */
  async function copyList(): Promise<void> {
    if (!results) return;
    const many = results.length > 1;
    const head = [...(many ? ["Tab"] : []), "Row", "Column", "Cell"].join("\t");
    const lines = [head];
    for (const { t, h } of shown) lines.push([...(many ? [t.info.name] : []), `${h.number + 1}${h.hidden ? " (hidden)" : ""}`, title(h.col, t), h.text].map((x) => x.replace(/[\t\r\n]+/g, " ")).join("\t"));
    if (await onCopy(lines.join("\r\n") + "\r\n")) status = `Copied the list of ${plural(shown.length, "match", "matches")} (as listed, each cell around its match).`;
  }

  // ---- saved searches (kept in Options only when Save is pressed) ----
  let savedOpen = $state(false);
  let saveName = $state("");
  function saveSearch(): void {
    const name = saveName.trim().slice(0, 100);
    if (!name) return;
    const s: SavedSearch = {
      name,
      text: options.text,
      replacement,
      kind: options.kind,
      blank: options.blank,
      matchCase: options.matchCase,
      wholeCell: options.wholeCell,
      wholeWord: options.wholeWord,
      ignoreAccents: options.ignoreAccents,
      escapes: options.escapes,
      preserveCase: options.preserveCase,
    };
    const list = prefs.data.savedSearches.filter((x) => x.name !== name);
    if (list.length >= MAX_SAVED_SEARCHES) {
      status = `At most ${MAX_SAVED_SEARCHES} searches can be saved: delete one first.`;
      return;
    }
    prefs.data.savedSearches = [...list, s].sort((a, b) => a.name.localeCompare(b.name));
    prefs.save();
    saveName = "";
    status = `Saved "${name}". It is kept on this computer until you delete it.`;
  }
  function useSearch(s: SavedSearch): void {
    options.text = s.text;
    options.kind = s.kind;
    options.blank = s.blank;
    options.matchCase = s.matchCase;
    options.wholeCell = s.wholeCell;
    options.wholeWord = s.wholeWord;
    options.ignoreAccents = s.ignoreAccents;
    options.escapes = s.escapes;
    options.preserveCase = s.preserveCase;
    if (s.replacement) {
      replacement = s.replacement;
      replace = true;
    }
    savedOpen = false;
    results = null;
    pending = null;
    status = `Using "${s.name}".`;
    focus();
  }
  function forgetSearch(s: SavedSearch): void {
    prefs.data.savedSearches = prefs.data.savedSearches.filter((x) => x !== s);
    prefs.save();
  }

  /** How many rows Find all's list holds (when it lists every match). */
  const matchRows = $derived.by(() => {
    const g = results?.length === 1 && results[0]!.t === tab ? results[0]!.r : null;
    if (!g || g.hits.length < g.cells || g.hits.some((h) => h.hidden)) return null;
    return new Set(g.hits.map((h) => h.row)).size;
  });

  /** Replace in the active cell if it matches, then go to the next match. */
  async function replaceOne(): Promise<void> {
    if (!ready()) return;
    rememberReplace(replacement);
    const f = tab.cell;
    const one: FindSpec = { ...spec(), scope: { kind: "range", row: f.row, rows: 1, col: f.col, cols: 1 }, skip_first: false };
    try {
      const r = await backend().replaceCell(tab.id, one, replacement);
      if (r.info) onChanged(r.info);
    } catch (e) {
      status = errorText(e);
      return;
    }
    await next(false);
  }

  /** The search and replacement the open preview shows: its Replace all
   *  applies exactly these (the backend refuses if anything changed). */
  let previewed: { spec: FindSpec; replacement: string } | null = null;

  async function previewAll(): Promise<void> {
    if (!ready()) return;
    if (options.lookIn === "notes") {
      status = "Replace works on cells. Set Look in to Cells.";
      return;
    }
    rememberReplace(replacement);
    unticked.clear();
    titleChanges = [];
    tabParts = [];
    changedAt = null;
    const rep = replacement;
    try {
      if (allTabs) {
        const parts: { t: Tab; spec: FindSpec; r: ReplaceReport }[] = [];
        for (const t of targets()) {
          const s = specFor(t);
          const r = await run(`Looking in ${t.info.name}`, () => backend().replacePreview(t.id, s, rep));
          if (!r) return;
          if (r.cells > 0) parts.push({ t, spec: s, r });
        }
        previewed = null;
        if (!parts.length) {
          status = "Nothing to replace.";
          pending = null;
          return;
        }
        tabParts = parts;
        partsAt = "";
        pending = { cells: parts.reduce((a, p) => a + p.r.cells, 0), hits: parts.reduce((a, p) => a + p.r.hits, 0), preview: [], info: null };
        status = "";
        return;
      }
      const all = spec();
      // Titles are renamed (so formulas and views follow), not replaced as cells.
      const s = withTitles ? { ...all, skip_first: true } : all;
      const r = await run("Looking for matches", () => backend().replacePreview(tab.id, s, rep));
      if (!r) return;
      if (withTitles) {
        const t = await backend().replaceTitles(tab.id, all, rep);
        titleChanges = t.map(([c, n]) => [c, heads[c] ?? tab.cache.get(0, c) ?? "", n]);
      }
      previewed = { spec: s, replacement: rep };
      if (r.cells === 0 && titleChanges.length === 0) {
        status = "Nothing to replace.";
        pending = null;
        return;
      }
      pending = r;
      page = 0;
      pageRows = r.preview.slice(0, PAGE);
      status = "";
    } catch (e) {
      status = errorText(e);
    }
  }

  // Anything that shapes the search (the text, the replacement, a tick
  // box, the scope or block, a change to the file) closes the preview. The
  // preview's Replace all also sends the search it showed (hidden columns
  // as they were then), and the backend refuses it if that no longer fits.
  $effect(() => {
    void [options.text, options.kind, options.matchCase, options.wholeCell, options.wholeWord, options.ignoreAccents, options.escapes, options.includeHidden, options.scope, options.blank, options.titles, options.lookIn, options.cols.length, block, replacement, tab.version];
    untrack(() => {
      if (!pending) return;
      pending = null;
      previewed = null;
      tabParts = [];
      titleChanges = [];
      status = "The search changed, so the preview was closed: press Replace all… to see the changes again.";
    });
  });

  const plural = (n: number, one: string, many = `${one}s`): string => `${n.toLocaleString()} ${n === 1 ? one : many}`;

  // All open tabs: a change to any of them closes the preview (each tab's
  // confirm would refuse it anyway, leaving the replace half done).
  let partsAt = "";
  $effect(() => {
    const now = tabParts.length ? versions(tabParts.map((p) => p.t)) : "";
    untrack(() => {
      if (!tabParts.length) {
        partsAt = "";
        return;
      }
      if (!partsAt) partsAt = now;
      else if (now !== partsAt && pending) {
        pending = null;
        tabParts = [];
        partsAt = "";
        status = "A tab changed, so the preview was closed: press Replace all… to see the changes again.";
      }
    });
  });

  async function replaceAll(): Promise<void> {
    const what = previewed;
    const parts = tabParts;
    const titles = titleChanges;
    const skip = [...unticked].map((k) => k.split(":").map(Number) as [number, number]);
    const cells = pending?.cells ?? 0;
    pending = null;
    previewed = null;
    tabParts = [];
    titleChanges = [];
    results = null;
    if (parts.length) {
      // Every open tab: one undo step in each.
      const done: string[] = [];
      let hits = 0;
      for (const p of parts) {
        try {
          const r = await run(`Replacing in ${p.t.info.name}`, () => backend().replaceConfirm(p.t.id, p.spec, replacement));
          if (!r) break;
          if (r.info) onChanged(r.info, p.t);
          hits += r.hits;
          done.push(p.t.info.name);
        } catch (e) {
          status = `${done.length ? `Replaced in ${done.join(", ")}. ` : ""}${p.t.info.name}: ${errorText(e)}`;
          return;
        }
      }
      status = `Replaced ${plural(hits, "match", "matches")} in ${plural(done.length, "tab")}. Undo (Ctrl+Z) in each tab reverts it there.`;
      return;
    }
    if (!what) return;
    try {
      let msg = "";
      if (cells - skip.length > 0) {
        const r = await run("Replacing", () => backend().replaceConfirm(tab.id, what.spec, what.replacement, skip));
        if (!r) return;
        if (r.info) onChanged(r.info);
        msg = `Replaced in ${plural(r.cells, "cell")}${skip.length ? ` (${plural(skip.length, "change")} left out)` : ""}.`;
        if (r.cells > 0 && !titles.length) changedAt = [tab.id, tab.info.history_pos];
      }
      if (titles.length) {
        await onRenameColumns(titles.map(([c, , n]) => [c, n]));
        msg += ` Renamed ${plural(titles.length, "column")}.`;
      }
      const steps = (cells - skip.length > 0 ? 1 : 0) + titles.length;
      status = `${msg.trim()} ${steps > 1 ? `Undo (Ctrl+Z) ${steps} times reverts it all (each renamed title is its own step).` : "Undo (Ctrl+Z) reverts it."}${skip.length ? " With changes unticked it isn't kept as a recipe step." : ""}`;
    } catch (e) {
      status = errorText(e);
    }
  }

  async function showChanged(): Promise<void> {
    try {
      const info = await run("Filtering", () => backend().showChanged(tab.id, tab.header));
      if (!info) return;
      onChanged(info);
      changedAt = null;
      status = "Showing only the rows the last Replace all changed. Show all rows from the status bar, or undo.";
    } catch (e) {
      status = errorText(e);
    }
  }

  function onKey(e: KeyboardEvent, inReplace: boolean): void {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      // Alt+Enter: Find all; Ctrl+Alt+Enter: Replace all (with its preview).
      if (e.altKey && e.ctrlKey) void previewAll();
      else if (e.altKey) void findAll();
      else if (inReplace && !e.shiftKey) void replaceOne();
      else void next(e.shiftKey);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (help) help = false;
      else onClose();
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.altKey && !e.ctrlKey) {
      // Up: an earlier search; Down: back to a later one.
      const list = inReplace ? findHistory.replaces : findHistory.finds;
      const v = stepHistory(list, inReplace ? replacement : options.text, e.key === "ArrowUp");
      if (v === null) return;
      e.preventDefault();
      e.stopPropagation();
      if (inReplace) replacement = v;
      else {
        options.text = v;
        results = null;
        pending = null;
      }
    }
  }

  // ---- the Find all list: every listed hit drawn on demand ----
  const ROW_H = 22;
  let listBox = $state<HTMLDivElement>();
  let listTop = $state(0);
  let listH = $state(180);
  let cursor = $state(-1);
  let order = $state<"rows" | "cols">("rows");
  let onlyCol = $state<number | null>(null);
  /** One group: per-column counts and the order menu apply. */
  const single = $derived(results?.length === 1 ? results[0]! : null);
  const totalCells = $derived(results?.reduce((a, g) => a + g.r.cells, 0) ?? 0);
  const listed = $derived(results?.reduce((a, g) => a + g.r.hits.length, 0) ?? 0);
  const shown = $derived.by(() => {
    if (!results) return [];
    if (single) {
      let h = single.r.hits;
      if (onlyCol !== null) h = h.filter((x) => x.col === onlyCol);
      if (order === "cols") h = [...h].sort((a, b) => a.col - b.col || a.row - b.row);
      return h.map((x) => ({ t: single.t, h: x }));
    }
    return results.flatMap((g) => g.r.hits.map((x) => ({ t: g.t, h: x })));
  });
  const first = $derived(Math.max(0, Math.floor(listTop / ROW_H) - 5));
  const last = $derived(Math.min(shown.length, Math.ceil((listTop + listH) / ROW_H) + 5));

  $effect(() => {
    void results;
    cursor = -1;
    onlyCol = null;
    listTop = 0;
    if (listBox) listBox.scrollTop = 0;
  });

  function go(i: number): void {
    if (!shown.length) return;
    cursor = Math.max(0, Math.min(shown.length - 1, i));
    const { t, h } = shown[cursor]!;
    if (t === tab) onSelect(h.row, h.col);
    else onGoTab(t, h.row, h.col);
    if (h.hidden) status = `Row ${(h.number + 1).toLocaleString()} is hidden, so the nearest shown row is selected. Show all rows to see it.`;
    const box = listBox;
    if (box) {
      const y = cursor * ROW_H;
      if (y < box.scrollTop) box.scrollTop = y;
      else if (y + ROW_H > box.scrollTop + box.clientHeight) box.scrollTop = y + ROW_H - box.clientHeight;
    }
  }

  function onListKey(e: KeyboardEvent): void {
    const page = Math.max(1, Math.floor(listH / ROW_H) - 1);
    const k = e.key;
    if (k === "ArrowDown") go(cursor + 1);
    else if (k === "ArrowUp") go(cursor < 0 ? 0 : cursor - 1);
    else if (k === "PageDown") go(cursor + page);
    else if (k === "PageUp") go(cursor - page);
    else if (k === "Home") go(0);
    else if (k === "End") go(shown.length - 1);
    else if (k === "Enter") go(cursor < 0 ? 0 : cursor);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  // Column titles for the lists, read from the file (row 0 may not be in
  // the cell cache after scrolling far down).
  let heads = $state<string[]>([]);
  $effect(() => {
    if ((!results && !pending && !picking) || !tab.header) return;
    const t = tab;
    const cols = Math.min(t.info.cols, 2000);
    untrack(() => {
      void backend()
        .viewport(t.id, 0, 1, 0, cols, 80)
        .then((v) => (heads = v.cells.map((x) => x.replace(/[\r\n\t]+/g, " "))))
        .catch(() => {});
    });
  });

  function title(c: number, of: Tab = tab): string {
    const t = of.header ? (of === tab ? heads[c] || of.cache.get(0, c) : of.cache.get(0, c)) : undefined;
    return t ? `${t} (column ${c + 1})` : `Column ${c + 1}`;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="find" role="search" aria-label="Find and replace" onkeydown={onBarKey}>
  <div class="line">
    <button type="button" class="twist" aria-label={replace ? "Hide replace" : "Show replace"} title="Replace (Ctrl+H)" onclick={() => (replace = !replace)}>{replace ? "▾" : "▸"}</button>
    <input bind:this={findInput} class="text" aria-label="Find" placeholder={options.blank ? "Empty cells (Blank cells is picked below)" : options.lookIn === "notes" ? "Find in row notes" : (placeholders[options.kind] ?? "Find")} disabled={options.blank && options.lookIn === "cells"} bind:value={options.text} onkeydown={(e) => onKey(e, false)} oninput={() => { results = null; notes = null; pending = null; status = ""; }} />
    <span class="menuwrap">
      <button type="button" title="Insert a character that is hard to type or to see" aria-haspopup="menu" aria-expanded={special} onclick={() => (special = !special)}>Special…</button>
      {#if special}
        <div class="menu" role="menu" aria-label="Special characters">
          {#each SPECIALS as s, i (s.label)}
            <button type="button" role="menuitem" disabled={s.regexOnly && options.kind !== "regex"} title={s.regexOnly && options.kind !== "regex" ? "Pick Regular expression first" : ""} onclick={() => insertSpecial(i)}>{s.label}</button>
          {/each}
        </div>
      {/if}
    </span>
    {#if countText}<span class="count" aria-live="polite">{countText}</span>{/if}
    <button type="button" title="Find previous (Shift+F3, Shift+Enter)" aria-label="Find previous" onclick={() => void next(true)}>↑</button>
    <button type="button" title="Find next (F3, Enter)" aria-label="Find next" onclick={() => void next(false)}>↓</button>
    <button type="button" title="Find all (Alt+Enter)" onclick={() => void findAll()}>Find all</button>
    <select aria-label="Search in" title="Alt+L: selection or whole file" bind:value={options.scope}>
      <option value="all">Whole file</option>
      <option value="selection">{blockCells && options.scope === "selection" ? `Selection ${blockCells}` : "Selection"}</option>
      <option value="column">{blockCols && options.scope === "column" ? `Selected columns ${blockCols}` : "Selected columns"}</option>
      <option value="pick">{options.scope === "pick" && options.cols.length ? `Columns: ${options.cols.length} picked` : "Columns…"}</option>
      <option value="tabs">All open tabs</option>
    </select>
    {#if options.scope === "pick" && !picking}<button type="button" onclick={() => (picking = true)}>Change columns</button>{/if}
    <span class="spacer"></span>
    <span class="menuwrap">
      <button type="button" title="Searches saved by name" aria-haspopup="menu" aria-expanded={savedOpen} onclick={() => (savedOpen = !savedOpen)}>Saved ▾</button>
      {#if savedOpen}
        <div class="menu right" role="menu" aria-label="Saved searches">
          {#each prefs.data.savedSearches as s (s.name)}
            <div class="saved">
              <button type="button" role="menuitem" title={`${s.blank ? "Blank cells" : s.text}${s.replacement ? ` → ${s.replacement}` : ""}`} onclick={() => useSearch(s)}>{s.name}</button>
              <button type="button" class="x" aria-label={`Delete ${s.name}`} title="Delete this saved search" onclick={() => forgetSearch(s)}>✕</button>
            </div>
          {:else}
            <span class="muted pad">No saved searches yet.</span>
          {/each}
          <div class="saveline">
            <input aria-label="Name for this search" placeholder="Name this search" maxlength="100" bind:value={saveName} onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); saveSearch(); } }} />
            <button type="button" disabled={!saveName.trim() || (!options.text && !options.blank)} onclick={saveSearch}>Save</button>
          </div>
        </div>
      {/if}
    </span>
    <button type="button" class="x" aria-label="Find tips" title="Wildcards, patterns and keys" aria-expanded={help} onclick={() => (help = !help)}>?</button>
    <button type="button" class="x" aria-label="Close find" title="Close (Esc)" onclick={onClose}>✕</button>
  </div>
  {#if replace}
    <div class="line">
      <span class="twist"></span>
      <input bind:this={replaceInput} class="text" aria-label="Replace with" placeholder={options.kind === "regex" ? "Replace with ($1 = first group)" : "Replace with"} bind:value={replacement} onkeydown={(e) => onKey(e, true)} />
      <button type="button" disabled={allTabs} title={allTabs ? "Replace one at a time works in the shown tab: pick Whole file" : ""} onclick={() => void replaceOne()}>Replace</button>
      <button type="button" title="Replace all (Ctrl+Alt+Enter)" onclick={() => void previewAll()}>Replace all…</button>
      <button type="button" title="Many find-and-replace pairs at once, from a pasted list or another tab" onclick={onBatch}>From a list…</button>
      {#if !valueKind}<label title="SMITH becomes JONES, Smith becomes Jones"><input type="checkbox" bind:checked={options.preserveCase} /> Preserve case</label>{/if}
    </div>
  {/if}
  {#if picking}
    <div class="picker" role="group" aria-label="Columns to search">
      <AllNone count={pickTitles.length} picked={options.cols} onChange={(v) => (options.cols = v)} />
      <div class="plist">
        {#each pickTitles as t, c (c)}
          <label><input type="checkbox" checked={options.cols.includes(c)} onchange={() => (options.cols = options.cols.includes(c) ? options.cols.filter((x) => x !== c) : [...options.cols, c])} /> {t}</label>
        {/each}
      </div>
      <div class="line"><button type="button" class="primary" disabled={!options.cols.length} onclick={() => { picking = false; focus(); }}>Done</button><span class="muted">{options.cols.length ? `${options.cols.length} picked` : "Tick the columns to search in."}</span></div>
    </div>
  {/if}
  <div class="line opts">
    <select aria-label="Pattern kind" title="Alt+R: regular expression on or off" value={options.blank ? "blank" : options.kind} onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; options.blank = v === "blank"; if (v !== "blank") options.kind = v as typeof options.kind; results = null; pending = null; }}>
      <option value="plain">Plain text</option>
      <option value="wildcard">Wildcards (* ? ~)</option>
      <option value="regex">Regular expression</option>
      <option value="blank">Blank cells</option>
      <option value="number">Numbers by value</option>
      <option value="date">Dates by value</option>
      <option value="similar">Similar spelling</option>
    </select>
    <select aria-label="Look in" title="Search the cells or the notes on rows" bind:value={options.lookIn}>
      <option value="cells">In cells</option>
      <option value="notes">In row notes</option>
    </select>
    {#if tab.header}<label title="Search the first row (the column titles) too; replacing there renames the column"><input type="checkbox" bind:checked={options.titles} /> Column titles</label>{/if}
    {#if !valueKind}
      <label title="Alt+C"><input type="checkbox" bind:checked={options.matchCase} /> Match case</label>
      <label title="Alt+W: &quot;Ann&quot; finds &quot;Ann Lee&quot;, not &quot;Annette&quot;"><input type="checkbox" bind:checked={options.wholeWord} /> Whole word</label>
      <label><input type="checkbox" bind:checked={options.wholeCell} /> Whole cell</label>
      <label title="&quot;Jose&quot; finds &quot;José&quot; too"><input type="checkbox" bind:checked={options.ignoreAccents} /> Ignore accents</label>
      {#if options.kind !== "regex"}
        <label title="\t tab, \n line break, \r return, \\ backslash, \u&#123;A0&#125; any character (in Replace with too)"><input type="checkbox" bind:checked={options.escapes} /> \t \n</label>
      {/if}
    {:else}
      <span class="muted">{options.kind === "similar" ? "Whole cells, a letter or two different; case, accents and punctuation aside." : "Whole cells by value; Replace puts the replacement in the whole cell."}</span>
    {/if}
    <label title="Hidden columns and hidden rows (filtered out or hidden by hand) are skipped unless this is ticked; Find next stays on shown rows"><input type="checkbox" bind:checked={options.includeHidden} /> Include hidden</label>
    <label title="Find next goes down each column before moving right"><input type="checkbox" bind:checked={options.byColumns} /> By columns</label>
  </div>
  {#if help}
    <div class="help" role="note" aria-label="Find tips">
      <div class="cols">
        <div>
          <b>Wildcards</b>
          <table>
            <tbody>
              <tr><td><code>*</code></td><td>any text: <code>Sm*th</code> finds Smith, Smyth</td></tr>
              <tr><td><code>?</code></td><td>one character: <code>B?b</code> finds Bob, Bib</td></tr>
              <tr><td><code>~*</code> <code>~?</code></td><td>a real * or ?</td></tr>
            </tbody>
          </table>
        </div>
        <div>
          <b>Regular expressions</b>
          <table>
            <tbody>
              <tr><td><code>\d</code> <code>\s</code></td><td>a digit, a space</td></tr>
              <tr><td><code>^</code> <code>$</code></td><td>start, end of the cell</td></tr>
              <tr><td><code>a|b</code></td><td>either</td></tr>
              <tr><td><code>(\d&#123;3&#125;)-(\d&#123;4&#125;)</code></td><td>groups; replace with <code>$2-$1</code> (or <code>$&#123;1&#125;a</code>)</td></tr>
              <tr><td><code>\U$1</code> <code>\L$1</code></td><td>in the replacement: upper or lower case until <code>\E</code></td></tr>
              <tr><td><code>\u$1</code> <code>\l$1</code></td><td>first letter upper or lower: <code>(\w+)</code> → <code>\u$1</code> makes "smith" "Smith"; <code>\\U</code> is a real backslash and U</td></tr>
            </tbody>
          </table>
        </div>
        <div>
          <b>Keys</b>
          <table>
            <tbody>
              <tr><td>Enter, Shift+Enter</td><td>next, previous (also F3, Shift+F3, Shift+F4)</td></tr>
              <tr><td>Alt+Enter</td><td>find all</td></tr>
              <tr><td>Ctrl+Alt+Enter</td><td>replace all (shows what changes first)</td></tr>
              <tr><td>Alt+C, Alt+W, Alt+R, Alt+L</td><td>match case, whole word, regular expression, selection</td></tr>
              <tr><td>Up, Down</td><td>earlier searches</td></tr>
            </tbody>
          </table>
        </div>
      </div>
      <div class="line">
        <span class="muted">Special… inserts characters that are hard to see, such as a non-breaking space. Blank cells finds empty cells. With <code>\t \n</code> ticked, <code>\t</code> is a tab and <code>\n</code> a line break (in Replace with too). In a regular expression's replacement <code>\n</code> and <code>\t</code> always work that way, so write <code>\\</code> for a backslash. Whole word needs spaces or punctuation around a word, so it does not split Chinese or Japanese text. Recent searches are kept only until you close the app.</span>
        <button type="button" onclick={() => { clearFindHistory(); status = "Search history cleared."; }}>Clear search history</button>
      </div>
    </div>
  {/if}
  {#if pending}
    <div class="confirm" role="alert">
      <div>
        {#if tabParts.length}
          Replace {plural(pending.hits, "match", "matches")} in {plural(pending.cells, "cell")} across {plural(tabParts.length, "tab")}? Each tab can undo its part in one step.
        {:else}
          Replace {plural(pending.hits, "match", "matches")} in {plural(pending.cells - unticked.size, "cell")}{titleChanges.length ? ` and rename ${plural(titleChanges.length, "column")}` : ""}?
          {titleChanges.length ? `It takes ${plural((pending.cells - unticked.size > 0 ? 1 : 0) + titleChanges.length, "step")} to undo (one per renamed title).` : "It is one step to undo."}{pending.hidden_cells ? ` ${plural(pending.hidden_cells, "cell")} ${pending.hidden_cells === 1 ? "is" : "are"} in hidden rows or columns.` : ""}{unticked.size ? ` ${plural(unticked.size, "change")} unticked will be left as they are.` : ""}
        {/if}
      </div>
      {#if titleChanges.length}
        <div class="pv">
          <table>
            <thead><tr><th>Column</th><th>Title now</th><th>New title</th></tr></thead>
            <tbody>
              {#each titleChanges as [c, before, after] (c)}
                {@const d = cellDiff(before, after)}
                <tr><td class="where">{c + 1}</td><td class="cell">{d.head}<del>{d.del}</del>{d.tail}</td><td class="cell">{d.head}<ins>{d.ins}</ins>{d.tail}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {#if tabParts.length}
        <div class="pv">
          <table>
            <thead><tr><th>Tab</th><th>Cells</th><th>First change</th></tr></thead>
            <tbody>
              {#each tabParts as p (p.t.id)}
                {@const c = p.r.preview[0]}
                <tr>
                  <td class="where">{p.t.info.name}</td>
                  <td>{p.r.cells.toLocaleString()}</td>
                  <td class="cell">{#if c}{@const d = cellDiff(c.before, c.after)}{d.head}<del>{d.del}</del><ins>{d.ins}</ins>{d.tail}{/if}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else if pending.cells > 0}
      <div class="pv">
        <table>
          <thead><tr><th><span class="sr">Replace</span></th><th>Where</th><th>Before</th><th>After</th></tr></thead>
          <tbody>
            {#each pageRows as c, i (page * PAGE + i)}
              {@const d = cellDiff(c.before, c.after)}
              {@const key = `${c.number}:${c.col}`}
              <tr class:off={unticked.has(key)}>
                <td><input type="checkbox" aria-label="Replace this one" title="Untick to leave this cell as it is" checked={!unticked.has(key)} onchange={() => (unticked.has(key) ? unticked.delete(key) : unticked.add(key))} /></td>
                <td class="where"><button type="button" class="link" title="Go to this cell" onclick={() => onSelect(c.row, c.col)}>{title(c.col)}, row {c.number + 1}</button></td>
                <td class="cell">{d.head}<del>{d.del}</del>{d.tail}</td>
                <td class="cell">{d.head}<ins>{d.ins}</ins>{d.tail}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if pages > 1}
        <div class="line pager">
          <button type="button" aria-label="Previous changes" disabled={page === 0} onclick={() => void showPage(page - 1)}>‹</button>
          <span class="muted">Changes {(page * PAGE + 1).toLocaleString()}–{Math.min(pending.cells, (page + 1) * PAGE).toLocaleString()} of {pending.cells.toLocaleString()}</span>
          <button type="button" aria-label="Next changes" disabled={page >= pages - 1} onclick={() => void showPage(page + 1)}>›</button>
        </div>
      {/if}
      {/if}
      <div class="line">
        <button type="button" class="primary" disabled={!tabParts.length && pending.cells - unticked.size <= 0 && !titleChanges.length} onclick={() => void replaceAll()}>Replace all</button>
        <button type="button" onclick={() => (pending = null)}>Cancel</button>
      </div>
    </div>
  {/if}
  {#if status || canShowChanged}
    <div class="status" role="status">
      {status}
      {#if canShowChanged}<button type="button" onclick={() => void showChanged()}>Show changed rows</button>{/if}
    </div>
  {/if}
  {#if notes && notes.length}
    <div class="results">
      <div class="rhead">
        {notes.length ? `${plural(notes.length, "note")} match` : "No notes match"}
        <button type="button" class="x" aria-label="Close results" onclick={() => (notes = null)}>✕</button>
      </div>
      <div class="nlist" aria-label="Matching notes">
        {#each notes.slice(0, 2000) as n (n.row)}
          <button type="button" class="hit note" onclick={() => onSelect(n.row, tab.cell.col)}>
            <span class="where">Row {n.row + 1}{n.flag ? ` · ${FLAG_NAMES[n.flag]?.[1] ?? ""} flag` : ""}</span>
            <span class="snip">{n.text}</span>
          </button>
        {/each}
      </div>
    </div>
  {/if}
  {#if results && totalCells > 0}
    <div class="results">
      <div class="rhead">
        <span>{plural(totalCells, "cell")} match{totalCells === 1 ? "es" : ""}{results.length > 1 ? ` in ${plural(results.length, "tab")}` : ""}{listed < totalCells ? ` (first ${listed.toLocaleString()} listed)` : ""}</span>
        <span class="rbtns">
          <button type="button" title="Copy this list (row, column and cell) as tab-separated text" onclick={() => void copyList()}>Copy list</button>
          <button type="button" class="x" aria-label="Close results" onclick={() => (results = null)}>✕</button>
        </span>
      </div>
      {#if single && single.t === tab && !allTabs}
        <div class="acts" role="group" aria-label="Rows with a match">
          <span class="muted">{hiddenListed ? "Shown rows with a match" : "Rows with a match"}{matchRows !== null ? ` (${matchRows.toLocaleString()})` : ""}:</span>
          <button type="button" title="Hide the other rows (undo, or Show all rows, brings them back)" onclick={() => void actOnRows({ kind: "show" })}>Show only these</button>
          <button type="button" title="Put these rows in a new tab; this file does not change" onclick={() => void actOnRows({ kind: "copy" })}>Copy to new tab</button>
          <span class="flagpick">
            <select aria-label="Flag colour" bind:value={flag}>
              {#each FLAG_NAMES.slice(1) as [n, name] (n)}<option value={n}>{name}</option>{/each}
            </select>
            <button type="button" title="Flag these rows (a row note); needs the columns that identify rows" onclick={() => void actOnRows({ kind: "flag", flag })}>Flag</button>
          </span>
          {#if deleteAsk}
            <span class="ask" role="alert">Delete {matchRows !== null ? plural(matchRows, "row") : "every row with a match"}? <button type="button" class="danger" onclick={() => void actOnRows({ kind: "delete" })}>Delete rows</button> <button type="button" onclick={() => (deleteAsk = false)}>Cancel</button></span>
          {:else}
            <button type="button" onclick={() => (deleteAsk = true)}>Delete rows…</button>
          {/if}
        </div>
      {/if}
      {#if single && single.r.per_col && single.r.per_col.length > 1}
        <div class="percol" aria-label="Matches per column">
          <button type="button" class:on={onlyCol === null} onclick={() => (onlyCol = null)}>All columns</button>
          {#each single.r.per_col.slice(0, 40) as [c, n] (c)}
            <button type="button" class:on={onlyCol === c} title="Show only this column's matches" onclick={() => (onlyCol = onlyCol === c ? null : c)}>{title(c)}: {n.toLocaleString()}</button>
          {/each}
          <label class="order">Order <select aria-label="Order of matches" bind:value={order}><option value="rows">by row</option><option value="cols">by column</option></select></label>
        </div>
      {/if}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="rlist" role="listbox" aria-label="Matches" tabindex="0" bind:this={listBox} bind:clientHeight={listH} onscroll={(e) => (listTop = (e.currentTarget as HTMLDivElement).scrollTop)} onkeydown={onListKey}>
        <div style:height="{shown.length * ROW_H}px" style:position="relative">
          {#each shown.slice(first, last) as { t, h }, j (first + j)}
            {@const i = first + j}
            {@const chars = [...h.text]}
            <div class="hit" class:cur={i === cursor} role="option" aria-selected={i === cursor} tabindex="-1" style:top="{i * ROW_H}px" onclick={() => go(i)} onkeydown={() => {}}>
              <span class="where">{results.length > 1 ? `${t.info.name}: ` : ""}{title(h.col, t)}, row {h.number + 1}{h.hidden ? " (hidden)" : ""}</span>
              <span class="snip">{chars.slice(0, h.start).join("")}<mark>{chars.slice(h.start, h.start + h.len).join("")}</mark>{chars.slice(h.start + h.len).join("")}</span>
            </div>
          {/each}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .find {
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 4px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .line {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .text {
    flex: 0 1 320px;
    min-width: 160px;
  }
  input,
  select,
  button {
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 8px;
  }
  input[type="checkbox"] {
    padding: 0;
  }
  button {
    cursor: pointer;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .twist {
    width: 24px;
    padding: 2px 0;
    border: none;
    background: none;
  }
  .x {
    border: none;
    background: none;
  }
  .spacer {
    flex: 1;
  }
  .help {
    margin-left: 30px;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
  }
  .help .cols {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
  }
  .help td {
    white-space: normal;
    max-width: 360px;
  }
  .muted {
    color: var(--muted);
  }
  code {
    font-family: ui-monospace, Consolas, monospace;
  }
  .opts {
    padding-left: 30px;
  }
  .count {
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
    min-width: 70px;
  }
  .status {
    color: var(--muted);
    padding-left: 30px;
  }
  .confirm {
    margin-left: 30px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  table {
    border-collapse: collapse;
    font-size: 12px;
  }
  td {
    padding: 1px 6px;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pv {
    max-height: 240px;
    overflow: auto;
  }
  th {
    text-align: left;
    font-weight: 600;
    color: var(--muted);
    padding: 1px 6px;
  }
  td.cell {
    max-width: 420px;
  }
  del {
    background: color-mix(in srgb, #e5484d 30%, transparent);
    text-decoration: line-through;
  }
  ins {
    background: color-mix(in srgb, #30a46c 30%, transparent);
    text-decoration: none;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--muted);
    text-decoration: underline dotted;
  }
  .where {
    color: var(--muted);
    white-space: nowrap;
  }
  .results {
    margin-left: 30px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .rhead {
    display: flex;
    justify-content: space-between;
    padding: 2px 8px;
    color: var(--muted);
  }
  .rlist {
    height: 180px;
    min-height: 66px;
    max-height: 70vh;
    overflow: auto;
    resize: vertical;
  }
  .rlist:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -2px;
  }
  .hit {
    position: absolute;
    left: 0;
    right: 0;
    height: 22px;
    box-sizing: border-box;
    display: flex;
    gap: 10px;
    border-top: 1px solid var(--border);
    text-align: left;
    padding: 2px 8px;
    cursor: pointer;
  }
  .hit:hover {
    background: var(--bg);
  }
  .hit.cur {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .percol {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 2px 8px 4px;
    font-size: 12px;
  }
  .percol button {
    padding: 0 6px;
    border-radius: 10px;
  }
  .percol button.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .order {
    margin-left: auto;
    color: var(--muted);
  }
  .menuwrap {
    position: relative;
  }
  .menu {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px;
    min-width: 200px;
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.18);
  }
  .menu button {
    border: none;
    background: none;
    text-align: left;
  }
  .menu.right {
    left: auto;
    right: 0;
    min-width: 260px;
  }
  .saved {
    display: flex;
    justify-content: space-between;
  }
  .saved button:first-child {
    flex: 1;
  }
  .saveline {
    display: flex;
    gap: 4px;
    border-top: 1px solid var(--border);
    padding-top: 4px;
    margin-top: 2px;
  }
  .saveline input {
    flex: 1;
    min-width: 0;
  }
  .pad {
    padding: 2px 8px;
  }
  .rbtns {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .rbtns button:not(.x) {
    padding: 0 6px;
    font-size: 12px;
  }
  .menu button:hover:not(:disabled) {
    background: var(--bg);
  }
  .menu button:disabled {
    color: var(--muted);
    cursor: default;
  }
  .picker {
    margin-left: 30px;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .plist {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    max-height: 160px;
    overflow: auto;
    font-size: 12px;
  }
  .plist label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 2px 8px 4px;
    font-size: 12px;
  }
  .acts button,
  .acts select {
    padding: 0 6px;
  }
  .ask {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }
  button.danger {
    background: #e5484d;
    border-color: #e5484d;
    color: #fff;
  }
  .nlist {
    max-height: 200px;
    overflow: auto;
  }
  .hit.note {
    position: static;
    width: 100%;
    border-radius: 0;
    border-left: none;
    border-right: none;
    border-bottom: none;
    background: none;
  }
  tr.off td.cell {
    opacity: 0.45;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .snip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  mark {
    background: var(--accent);
    color: var(--accent-ink);
    border-radius: 2px;
  }
</style>
