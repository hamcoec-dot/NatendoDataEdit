<script lang="ts">
  // Find and replace bar. Searches run in the engine (safe regular
  // expressions, whole files of any size); results select cells in the grid.
  import { backend, errorText, type DocInfo, type FindAll, type FindCount, type FindSpec, type ReplaceReport } from "./backend";
  import { onDestroy } from "svelte";
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
    tab.findSpec = null;
    tab.findVersion++;
    tab.onRedraw?.();
  });

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
    return true;
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

  async function findAll(): Promise<void> {
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

  async function previewAll(): Promise<void> {
    if (!ready()) return;
    try {
      const r = await run("Looking for matches", () => backend().replacePreview(tab.id, spec(), replacement));
      if (!r) return;
      if (r.cells === 0) {
        status = "Nothing to replace.";
        pending = null;
        return;
      }
      pending = r;
      status = "";
    } catch (e) {
      status = errorText(e);
    }
  }

  async function replaceAll(): Promise<void> {
    pending = null;
    try {
      const r = await run("Replacing", () => backend().replaceAll(tab.id, spec(), replacement));
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
      onClose();
    }
  }

  function title(c: number): string {
    const t = tab.header ? tab.cache.get(0, c) : undefined;
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
    <select aria-label="Search in" title="Alt+L: selection or whole file" bind:value={options.scope}>
      <option value="all">Whole file</option>
      <option value="selection">{blockCells && options.scope === "selection" ? `Selection ${blockCells}` : "Selection"}</option>
      <option value="column">{blockCols && options.scope === "column" ? `Selected columns ${blockCols}` : "Selected columns"}</option>
    </select>
    <span class="spacer"></span>
    <button type="button" class="x" aria-label="Close find" title="Close (Esc)" onclick={onClose}>✕</button>
  </div>
  {#if replace}
    <div class="line">
      <span class="twist"></span>
      <input class="text" aria-label="Replace with" placeholder={options.kind === "regex" ? "Replace with ($1 = first group)" : "Replace with"} bind:value={replacement} onkeydown={(e) => onKey(e, true)} />
      <button type="button" onclick={() => void replaceOne()}>Replace</button>
      <button type="button" title="Replace all (Ctrl+Alt+Enter)" onclick={() => void previewAll()}>Replace all…</button>
    </div>
  {/if}
  {#if pending}
    <div class="confirm" role="alert">
      <div>
        Replace {pending.hits.toLocaleString()} match{pending.hits === 1 ? "" : "es"} in {pending.cells.toLocaleString()} cell{pending.cells === 1 ? "" : "s"}?
        It is one step to undo.
      </div>
      <table>
        <tbody>
          {#each pending.preview.slice(0, 8) as c, i (i)}
            <tr><td class="where">Row {c.number + 1}, column {c.col + 1}</td><td class="before">{c.before}</td><td>→</td><td class="after">{c.after}</td></tr>
          {/each}
        </tbody>
      </table>
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
      <div class="rlist">
        {#each results.hits.slice(0, 2000) as h, i (i)}
          {@const chars = [...h.text]}
          <button type="button" class="hit" onclick={() => onSelect(h.row, h.col)}>
            <span class="where">{title(h.col)}, row {h.number + 1}</span>
            <span class="snip">{chars.slice(0, h.start).join("")}<mark>{chars.slice(h.start, h.start + h.len).join("")}</mark>{chars.slice(h.start + h.len).join("")}</span>
          </button>
        {/each}
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
  .before {
    color: var(--muted);
    text-decoration: line-through;
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
    max-height: 180px;
    overflow: auto;
  }
  .hit {
    display: flex;
    gap: 10px;
    width: 100%;
    border: none;
    border-top: 1px solid var(--border);
    border-radius: 0;
    background: none;
    text-align: left;
    padding: 2px 8px;
  }
  .hit:hover {
    background: var(--bg);
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
