<script lang="ts">
  // Find and replace bar. Searches run in the engine (safe regular
  // expressions, whole files of any size); results select cells in the grid.
  import { backend, errorText, type DocInfo, type FindAll, type FindCount, type FindSpec, type ReplaceReport } from "./backend";
  import { onDestroy, untrack } from "svelte";
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
    onChanged: (info: DocInfo) => void;
    onClose: () => void;
  }
  let { tab, replace = $bindable(), options = $bindable(), block = $bindable(), run, onSelect, onChanged, onClose }: Props = $props();

  // Picking "Selection" or "Selected columns" while the box is open takes
  // the block selected then; "Whole file" lets it go.
  let lastScope = options.scope;
  $effect(() => {
    const s = options.scope;
    if (s === lastScope) return;
    lastScope = s;
    if (s === "all") block = null;
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
  let results = $state<FindAll | null>(null);
  let pending = $state<ReplaceReport | null>(null);

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
      if (!options.text) {
        count = null;
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
        })
        .catch((e: unknown) => {
          if (seq !== countSeq) return;
          const kind = (e as { kind?: string } | null)?.kind;
          // Busy or cancelled: try once more shortly; a bad pattern: no count.
          if (kind === "busy" || kind === "cancelled") recount(600);
          else count = null;
        });
    }, delay);
  }

  /** What the count says now. */
  const countText = $derived.by(() => {
    if (!count || !options.text) return "";
    if (count.cells === 0) return "No matches";
    const total = count.cells.toLocaleString();
    const at = countAt;
    if (count.here && at && at[0] === tab.cell.row && at[1] === tab.cell.col) return `${(count.before + 1).toLocaleString()} of ${total}`;
    return `${total} match${count.cells === 1 ? "" : "es"}`;
  });

  // Whatever changes the search (text, options, block, the file) counts
  // again and lights up the matches on screen.
  $effect(() => {
    void [options.text, options.kind, options.matchCase, options.wholeCell, options.wholeWord, options.ignoreAccents, options.escapes, options.includeHidden, options.scope, block, tab.version];
    const s = options.text ? $state.snapshot(spec()) : null;
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

  /** The search as set up now. A one-cell "selection" scope searches all. */
  function spec(): FindSpec {
    const r = options.scope !== "all" && block ? block : rectOf(tab.sel);
    const o = { ...options };
    if (o.scope === "selection" && r.rows * r.cols <= 1) o.scope = "all";
    return findSpecOf(o, { row: r.row, rows: r.rows, col: r.col, cols: r.cols }, tab.header, tab.columns.hiddenList());
  }


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

  export async function next(backward = false): Promise<void> {
    if (!ready()) return;
    const f = tab.cell;
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
    try {
      const r = await run("Searching", () => backend().findAll(tab.id, spec()));
      if (!r) return;
      results = r;
      status = r.cells === 0 ? "No matches." : "";
    } catch (e) {
      status = errorText(e);
    }
  }

  /** Replace in the active cell if it matches, then go to the next match. */
  async function replaceOne(): Promise<void> {
    if (!ready()) return;
    rememberReplace(replacement);
    const f = tab.cell;
    const one: FindSpec = { ...spec(), scope: { kind: "range", row: f.row, rows: 1, col: f.col, cols: 1 }, skip_first: false };
    try {
      const r = await backend().replaceAll(tab.id, one, replacement);
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
    rememberReplace(replacement);
    try {
      const s = spec();
      const rep = replacement;
      const r = await run("Looking for matches", () => backend().replacePreview(tab.id, s, rep));
      if (!r) return;
      previewed = { spec: s, replacement: rep };
      if (r.cells === 0) {
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
  // box, the scope or block, hidden columns, the file) closes the preview,
  // so its Replace all never applies something it did not show.
  $effect(() => {
    void [options.text, options.kind, options.matchCase, options.wholeCell, options.wholeWord, options.ignoreAccents, options.escapes, options.includeHidden, options.scope, block, replacement, tab.version];
    untrack(() => {
      if (!pending) return;
      pending = null;
      previewed = null;
      status = "The search changed, so the preview was closed: press Replace all… to see the changes again.";
    });
  });

  async function replaceAll(): Promise<void> {
    const what = previewed;
    pending = null;
    previewed = null;
    if (!what) return;
    try {
      const r = await run("Replacing", () => backend().replaceConfirm(tab.id, what.spec, what.replacement));
      if (!r) return;
      if (r.info) onChanged(r.info);
      status = `Replaced ${r.hits.toLocaleString()} match${r.hits === 1 ? "" : "es"} in ${r.cells.toLocaleString()} cell${r.cells === 1 ? "" : "s"}. Undo (Ctrl+Z) reverts it all.`;
      results = null;
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
  const shown = $derived.by(() => {
    if (!results) return [];
    let h = results.hits;
    if (onlyCol !== null) h = h.filter((x) => x.col === onlyCol);
    if (order === "cols") h = [...h].sort((a, b) => a.col - b.col || a.row - b.row);
    return h;
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
    const h = shown[cursor]!;
    onSelect(h.row, h.col);
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
    if ((!results && !pending) || !tab.header) return;
    const t = tab;
    const cols = Math.min(t.info.cols, 2000);
    untrack(() => {
      void backend()
        .viewport(t.id, 0, 1, 0, cols, 80)
        .then((v) => (heads = v.cells.map((x) => x.replace(/[\r\n\t]+/g, " "))))
        .catch(() => {});
    });
  });

  function title(c: number): string {
    const t = tab.header ? (heads[c] || tab.cache.get(0, c)) : undefined;
    return t ? `${t} (column ${c + 1})` : `Column ${c + 1}`;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="find" role="search" aria-label="Find and replace" onkeydown={onBarKey}>
  <div class="line">
    <button type="button" class="twist" aria-label={replace ? "Hide replace" : "Show replace"} title="Replace (Ctrl+H)" onclick={() => (replace = !replace)}>{replace ? "▾" : "▸"}</button>
    <input bind:this={findInput} class="text" aria-label="Find" placeholder="Find" bind:value={options.text} onkeydown={(e) => onKey(e, false)} oninput={() => { results = null; pending = null; status = ""; }} />
    {#if countText}<span class="count" aria-live="polite">{countText}</span>{/if}
    <button type="button" title="Find previous (Shift+F3, Shift+Enter)" aria-label="Find previous" onclick={() => void next(true)}>↑</button>
    <button type="button" title="Find next (F3, Enter)" aria-label="Find next" onclick={() => void next(false)}>↓</button>
    <button type="button" title="Find all (Alt+Enter)" onclick={() => void findAll()}>Find all</button>
    <select aria-label="Search in" title="Alt+L: selection or whole file" bind:value={options.scope}>
      <option value="all">Whole file</option>
      <option value="selection">{blockCells && options.scope === "selection" ? `Selection ${blockCells}` : "Selection"}</option>
      <option value="column">{blockCols && options.scope === "column" ? `Selected columns ${blockCols}` : "Selected columns"}</option>
    </select>
    <span class="spacer"></span>
    <button type="button" class="x" aria-label="Find tips" title="Wildcards, patterns and keys" aria-expanded={help} onclick={() => (help = !help)}>?</button>
    <button type="button" class="x" aria-label="Close find" title="Close (Esc)" onclick={onClose}>✕</button>
  </div>
  {#if replace}
    <div class="line">
      <span class="twist"></span>
      <input bind:this={replaceInput} class="text" aria-label="Replace with" placeholder={options.kind === "regex" ? "Replace with ($1 = first group)" : "Replace with"} bind:value={replacement} onkeydown={(e) => onKey(e, true)} />
      <button type="button" onclick={() => void replaceOne()}>Replace</button>
      <button type="button" title="Replace all (Ctrl+Alt+Enter)" onclick={() => void previewAll()}>Replace all…</button>
    </div>
  {/if}
  <div class="line opts">
    <select aria-label="Pattern kind" title="Alt+R: regular expression on or off" bind:value={options.kind}>
      <option value="plain">Plain text</option>
      <option value="wildcard">Wildcards (* ? ~)</option>
      <option value="regex">Regular expression</option>
    </select>
    <label title="Alt+C"><input type="checkbox" bind:checked={options.matchCase} /> Match case</label>
    <label title="Alt+W: &quot;Ann&quot; finds &quot;Ann Lee&quot;, not &quot;Annette&quot;"><input type="checkbox" bind:checked={options.wholeWord} /> Whole word</label>
    <label><input type="checkbox" bind:checked={options.wholeCell} /> Whole cell</label>
    <label title="&quot;Jose&quot; finds &quot;José&quot; too"><input type="checkbox" bind:checked={options.ignoreAccents} /> Ignore accents</label>
    {#if options.kind !== "regex"}
      <label title="\t tab, \n line break, \r return, \\ backslash, \u&#123;A0&#125; any character (in Replace with too)"><input type="checkbox" bind:checked={options.escapes} /> \t \n</label>
    {/if}
    <label title="Hidden columns are skipped unless this is ticked"><input type="checkbox" bind:checked={options.includeHidden} /> Include hidden</label>
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
        <span class="muted">With <code>\t \n</code> ticked, <code>\t</code> is a tab and <code>\n</code> a line break (in Replace with too). In a regular expression's replacement <code>\n</code> and <code>\t</code> always work that way, so write <code>\\</code> for a backslash. Whole word needs spaces or punctuation around a word, so it does not split Chinese or Japanese text. Recent searches are kept only until you close the app.</span>
        <button type="button" onclick={() => { clearFindHistory(); status = "Search history cleared."; }}>Clear search history</button>
      </div>
    </div>
  {/if}
  {#if pending}
    <div class="confirm" role="alert">
      <div>
        Replace {pending.hits.toLocaleString()} match{pending.hits === 1 ? "" : "es"} in {pending.cells.toLocaleString()} cell{pending.cells === 1 ? "" : "s"}?
        It is one step to undo.
      </div>
      <div class="pv">
        <table>
          <thead><tr><th>Where</th><th>Before</th><th>After</th></tr></thead>
          <tbody>
            {#each pageRows as c, i (page * PAGE + i)}
              {@const d = cellDiff(c.before, c.after)}
              <tr>
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
      <div class="line">
        <button type="button" class="primary" onclick={() => void replaceAll()}>Replace all</button>
        <button type="button" onclick={() => (pending = null)}>Cancel</button>
      </div>
    </div>
  {/if}
  {#if status}<div class="status" role="status">{status}</div>{/if}
  {#if results && results.cells > 0}
    <div class="results">
      <div class="rhead">
        {results.cells.toLocaleString()} cell{results.cells === 1 ? "" : "s"} match{results.cells === 1 ? "es" : ""}{results.hits.length < results.cells ? ` (first ${results.hits.length.toLocaleString()} listed)` : ""}
        <button type="button" class="x" aria-label="Close results" onclick={() => (results = null)}>✕</button>
      </div>
      {#if results.per_col && results.per_col.length > 1}
        <div class="percol" aria-label="Matches per column">
          <button type="button" class:on={onlyCol === null} onclick={() => (onlyCol = null)}>All columns</button>
          {#each results.per_col.slice(0, 40) as [c, n] (c)}
            <button type="button" class:on={onlyCol === c} title="Show only this column's matches" onclick={() => (onlyCol = onlyCol === c ? null : c)}>{title(c)}: {n.toLocaleString()}</button>
          {/each}
          <label class="order">Order <select aria-label="Order of matches" bind:value={order}><option value="rows">by row</option><option value="cols">by column</option></select></label>
        </div>
      {/if}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="rlist" role="listbox" aria-label="Matches" tabindex="0" bind:this={listBox} bind:clientHeight={listH} onscroll={(e) => (listTop = (e.currentTarget as HTMLDivElement).scrollTop)} onkeydown={onListKey}>
        <div style:height="{shown.length * ROW_H}px" style:position="relative">
          {#each shown.slice(first, last) as h, j (first + j)}
            {@const i = first + j}
            {@const chars = [...h.text]}
            <div class="hit" class:cur={i === cursor} role="option" aria-selected={i === cursor} tabindex="-1" style:top="{i * ROW_H}px" onclick={() => go(i)} onkeydown={() => {}}>
              <span class="where">{title(h.col)}, row {h.number + 1}</span>
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
